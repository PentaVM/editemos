//! Kdenlive on the same machine: is it running, which project is open, what is in
//! a project file, how did a person change it, and how to hand motion back.
//!
//! Kdenlive has no live API on Windows, so "connecting" means: read the saved
//! file, work on versioned copies, never touch the original.

use crate::error::{fail, Result};
use crate::util::{path_str, truncate_chars};
use quick_xml::events::Event;
use quick_xml::Reader;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/* ------------------------------ locating ------------------------------ */

pub fn install_dir() -> Option<PathBuf> {
    if let Ok(d) = std::env::var("KDENLIVE_DIR") {
        if Path::new(&d).exists() {
            return Some(PathBuf::from(d));
        }
    }
    let mut c = vec![PathBuf::from(r"C:\Program Files\kdenlive"), PathBuf::from(r"C:\Program Files\Kdenlive")];
    if let Ok(l) = std::env::var("LOCALAPPDATA") {
        c.push(PathBuf::from(l).join("Programs").join("Kdenlive"));
    }
    c.into_iter().find(|p| p.join("bin").join("kdenlive.exe").exists())
}

pub fn bin(name: &str) -> Option<PathBuf> {
    let var = format!("EDITEMOS_{}", name.to_ascii_uppercase());
    if let Ok(p) = std::env::var(&var) {
        if Path::new(&p).exists() {
            return Some(PathBuf::from(p));
        }
    }
    if cfg!(windows) {
        return install_dir().map(|d| d.join("bin").join(format!("{name}.exe"))).filter(|p| p.exists());
    }
    // macOS app bundle, then whatever is on PATH (distro package, AppImage wrapper)
    let bundle = Path::new("/Applications/kdenlive.app/Contents/MacOS").join(name);
    if bundle.exists() {
        return Some(bundle);
    }
    let names: &[&str] = if name == "melt" { &["melt", "melt-7", "mlt-melt"] } else { &[name] };
    std::env::var_os("PATH").and_then(|paths| std::env::split_paths(&paths).flat_map(|d| names.iter().map(move |n| d.join(n))).find(|p| p.is_file()))
}

fn run_timeout(cmd: &mut Command, secs: u64) -> Option<std::process::Output> {
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().ok()?;
    let t0 = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return child.wait_with_output().ok(),
            Ok(None) => {
                if t0.elapsed() > Duration::from_secs(secs) {
                    let _ = child.kill();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => return None,
        }
    }
}

/* ------------------------------ status ------------------------------ */

#[derive(Serialize, Clone, Debug, Default)]
pub struct Window {
    pub pid: u32,
    pub title: String,
    pub has_project: bool,
    pub doc_name: Option<String>,
    pub saved: bool,
    pub modified: bool,
    pub profile: Option<String>,
}

#[derive(Serialize, Debug, Default)]
pub struct Status {
    pub installed: bool,
    pub dir: Option<String>,
    pub version: Option<String>,
    pub running: bool,
    pub started: bool,
    pub windows: Vec<Window>,
    pub active: Option<Window>,
    pub needs_save: bool,
    pub project_candidates: Vec<String>,
    pub melt: Option<String>,
    pub ffmpeg: Option<String>,
}

/// "<doc> / <profile> - Kdenlive" (a `*` after the doc name means unsaved changes).
pub fn parse_title(title: &str) -> (bool, Option<String>, bool, Option<String>) {
    let Some(inner) = title.strip_suffix(" - Kdenlive") else { return (false, None, false, None) };
    let mut parts = inner.splitn(2, " / ");
    let mut doc = parts.next().unwrap_or("").to_string();
    let profile = parts.next().map(String::from);
    let modified = doc.ends_with('*');
    if modified {
        doc = doc.trim_end_matches('*').trim().to_string();
    }
    (true, Some(doc), modified, profile)
}

/// Linux and macOS: the process list says whether Kdenlive runs; the window title
/// (and with it the open document) comes from wmctrl or AppleScript when available.
fn windows_unix() -> Vec<Window> {
    let pids = Command::new("pgrep").args(["-x", "kdenlive"]).stderr(Stdio::null()).output().map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
    let Some(pid) = pids.lines().next().and_then(|l| l.trim().parse::<u32>().ok()) else { return Vec::new() };
    let title = if cfg!(target_os = "macos") {
        Command::new("osascript")
            .args(["-e", "tell application \"System Events\" to get name of front window of (first process whose name is \"kdenlive\")"])
            .stderr(Stdio::null())
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default()
    } else {
        Command::new("wmctrl")
            .arg("-l")
            .stderr(Stdio::null())
            .output()
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .find(|l| l.contains("Kdenlive"))
                    .map(|l| l.splitn(4, char::is_whitespace).last().unwrap_or("").trim().to_string())
                    .unwrap_or_default()
            })
            .unwrap_or_default()
    };
    let title = if title.is_empty() { "Kdenlive".to_string() } else { title };
    let (has, doc, modified, profile) = parse_title(&title);
    let saved = doc.as_deref().map(|d| d.to_ascii_lowercase().ends_with(".kdenlive")).unwrap_or(false);
    vec![Window { pid, title, has_project: has, doc_name: doc, saved, modified, profile }]
}

fn windows() -> Vec<Window> {
    if !cfg!(windows) {
        return windows_unix();
    }
    let out = Command::new("tasklist").args(["/v", "/fo", "csv", "/nh", "/fi", "IMAGENAME eq kdenlive.exe"]).output();
    let Ok(out) = out else { return Vec::new() };
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let mut v = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if !line.starts_with('"') {
            continue;
        }
        let fields: Vec<&str> = line.trim_matches('"').split("\",\"").collect();
        if fields.len() < 9 {
            continue;
        }
        let pid = fields[1].parse::<u32>().unwrap_or(0);
        let title = fields[fields.len() - 1].to_string();
        let (has, doc, modified, profile) = parse_title(&title);
        let saved = doc.as_deref().map(|d| d.to_ascii_lowercase().ends_with(".kdenlive")).unwrap_or(false);
        v.push(Window { pid, title, has_project: has, doc_name: doc, saved, modified, profile });
    }
    v
}

fn recent_files() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut configs: Vec<PathBuf> =
        ["LOCALAPPDATA", "APPDATA", "XDG_CONFIG_HOME"].iter().filter_map(|v| std::env::var(v).ok()).map(|b| PathBuf::from(b).join("kdenliverc")).collect();
    if let Ok(h) = std::env::var("HOME") {
        configs.push(PathBuf::from(&h).join(".config/kdenliverc"));
        configs.push(PathBuf::from(&h).join("Library/Preferences/kdenliverc"));
    }
    for cfg_file in configs {
        if let Ok(text) = fs::read_to_string(cfg_file) {
            for l in text.lines() {
                if let Some((k, v)) = l.split_once('=') {
                    if k.starts_with("File") && k[4..].chars().all(|c| c.is_ascii_digit()) {
                        let v = v.trim();
                        out.push(if cfg!(windows) { PathBuf::from(v.trim_start_matches("file:///").replace('/', "\\")) } else { PathBuf::from(v.trim_start_matches("file://")) });
                    }
                }
            }
        }
    }
    out
}

fn find_named(name: &str, roots: &[PathBuf]) -> Vec<PathBuf> {
    let skip = ["node_modules", ".git", "AppData", "$RECYCLE.BIN", "frames", ".kdenlive-mcp-backups"];
    let mut hits: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    let mut seen = 0usize;
    fn walk(dir: &Path, depth: u32, name: &str, skip: &[&str], seen: &mut usize, hits: &mut Vec<(std::time::SystemTime, PathBuf)>) {
        if depth > 6 || *seen > 20_000 {
            return;
        }
        let Ok(rd) = fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            *seen += 1;
            let p = e.path();
            let n = e.file_name().to_string_lossy().to_string();
            if p.is_dir() {
                if !skip.contains(&n.as_str()) {
                    walk(&p, depth + 1, name, skip, seen, hits);
                }
            } else if n.eq_ignore_ascii_case(name) {
                if let Ok(m) = e.metadata().and_then(|m| m.modified()) {
                    hits.push((m, p));
                }
            }
        }
    }
    for r in roots {
        if r.exists() {
            walk(r, 0, name, &skip, &mut seen, &mut hits);
        }
    }
    hits.sort_by_key(|a| std::cmp::Reverse(a.0));
    hits.into_iter().map(|h| h.1).collect()
}

pub fn status(start: bool, project: Option<&Path>) -> Status {
    let dir = install_dir();
    let exe = bin("kdenlive");
    let mut st = Status {
        installed: exe.is_some(),
        dir: dir.as_ref().map(|d| path_str(d)),
        melt: bin("melt").map(|p| path_str(&p)),
        ffmpeg: crate::motion::render::find_ffmpeg().map(|p| path_str(&p)),
        ..Default::default()
    };
    if let Some(e) = &exe {
        let mut cmd = Command::new(e);
        cmd.arg("--version");
        st.version = run_timeout(&mut cmd, 10).map(|o| String::from_utf8_lossy(&o.stdout).lines().next().unwrap_or("").trim().to_string()).filter(|v| !v.is_empty());
    }
    let mut wins = windows();
    if start && wins.is_empty() {
        if let Some(e) = &exe {
            let mut c = Command::new(e);
            if let Some(p) = project.filter(|p| p.exists()) {
                c.arg(p);
            }
            if c.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().is_ok() {
                st.started = true;
                let t0 = Instant::now();
                while t0.elapsed() < Duration::from_secs(25) {
                    std::thread::sleep(Duration::from_millis(800));
                    wins = windows();
                    if wins.iter().any(|w| w.has_project) {
                        break;
                    }
                }
            }
        }
    }
    st.running = !wins.is_empty();
    let open: Vec<&Window> = wins.iter().filter(|w| w.has_project).collect();
    let active = open.iter().find(|w| w.saved).or(open.first()).map(|w| (*w).clone());
    if let Some(a) = &active {
        st.needs_save = !a.saved;
        if a.saved {
            let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).map(PathBuf::from).unwrap_or_default();
            let mut roots = vec![home.join("Videos"), home.join("Documents"), home.join("Desktop"), home.join("Downloads")];
            if let Ok(r) = std::env::var("EDITEMOS_ROOT") {
                roots.push(PathBuf::from(r));
            }
            let name = a.doc_name.clone().unwrap_or_default();
            let mut c: Vec<PathBuf> =
                recent_files().into_iter().filter(|p| p.file_name().map(|n| n.to_string_lossy().eq_ignore_ascii_case(&name)).unwrap_or(false) && p.exists()).collect();
            c.extend(find_named(&name, &roots));
            let mut seen = Vec::new();
            for p in c {
                let s = path_str(&p);
                if !seen.contains(&s) {
                    seen.push(s);
                }
            }
            st.project_candidates = seen.into_iter().take(5).collect();
        }
    }
    st.active = active;
    st.windows = wins;
    st
}

/* ------------------------------ XML ------------------------------ */

#[derive(Debug, Default, Clone)]
struct El {
    name: String,
    attrs: HashMap<String, String>,
    children: Vec<El>,
    text: String,
}

fn parse_xml(src: &str) -> Result<El> {
    let mut reader = Reader::from_str(src);
    let mut stack: Vec<El> = vec![El { name: "#root".into(), ..Default::default() }];
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => stack.push(make_el(&e)),
            Ok(Event::Empty(e)) => {
                let el = make_el(&e);
                stack.last_mut().unwrap().children.push(el);
            }
            Ok(Event::End(_)) => {
                if stack.len() > 1 {
                    let el = stack.pop().unwrap();
                    stack.last_mut().unwrap().children.push(el);
                }
            }
            Ok(Event::Text(t)) => {
                if let Ok(s) = t.unescape() {
                    stack.last_mut().unwrap().text.push_str(&s);
                }
            }
            Ok(Event::CData(t)) => stack.last_mut().unwrap().text.push_str(&String::from_utf8_lossy(&t.into_inner())),
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(e) => return fail(format!("XML error at byte {}: {e}", reader.buffer_position())),
        }
    }
    while stack.len() > 1 {
        let el = stack.pop().unwrap();
        stack.last_mut().unwrap().children.push(el);
    }
    Ok(stack.pop().unwrap())
}

fn make_el(e: &quick_xml::events::BytesStart<'_>) -> El {
    let mut attrs = HashMap::new();
    for a in e.attributes().flatten() {
        let k = String::from_utf8_lossy(a.key.as_ref()).to_string();
        let v = a.unescape_value().map(|c| c.to_string()).unwrap_or_default();
        attrs.insert(k, v);
    }
    El { name: String::from_utf8_lossy(e.name().as_ref()).to_string(), attrs, children: Vec::new(), text: String::new() }
}

fn props(el: &El) -> HashMap<String, String> {
    el.children.iter().filter(|c| c.name == "property").map(|c| (c.attrs.get("name").cloned().unwrap_or_default(), c.text.trim().to_string())).collect()
}

fn to_frames(v: Option<&String>, fps: f64) -> Option<i64> {
    let s = v?;
    if let Ok(n) = s.parse::<i64>() {
        return Some(n);
    }
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() == 3 {
        let secs = parts[0].parse::<f64>().ok()? * 3600.0 + parts[1].parse::<f64>().ok()? * 60.0 + parts[2].parse::<f64>().ok()?;
        return Some((secs * fps).round() as i64);
    }
    s.parse::<f64>().ok().map(|f| f.round() as i64)
}

/* ------------------------------ inspect ------------------------------ */

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Clip {
    pub start: f64,
    pub end: f64,
    pub dur: f64,
    #[serde(rename = "in")]
    pub in_s: f64,
    pub name: Option<String>,
    pub source: Option<String>,
    pub service: Option<String>,
    pub speed: Option<f64>,
    pub effects: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Track {
    pub index: usize,
    pub id: String,
    pub name: Option<String>,
    pub kind: String,
    pub hide: Option<String>,
    pub clip_count: usize,
    pub end: f64,
    pub clips: Vec<Clip>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Summary {
    pub file: String,
    pub mlt_version: Option<String>,
    pub kdenlive_version: Option<String>,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub duration: f64,
    pub frames: i64,
    pub track_count: usize,
    pub tracks: Vec<Track>,
    pub transitions: usize,
    pub effect_count: BTreeMap<String, usize>,
    pub timeline_tractor: Option<String>,
}

fn r3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

pub fn inspect(file: &Path) -> Result<Summary> {
    let xml = fs::read_to_string(file)?;
    let root = parse_xml(&xml)?;
    let Some(mlt) = root.children.iter().find(|c| c.name == "mlt") else {
        return fail("not an MLT/Kdenlive document (no <mlt> element)");
    };
    let prof = mlt.children.iter().find(|c| c.name == "profile").map(|p| p.attrs.clone()).unwrap_or_default();
    let num = |k: &str| prof.get(k).and_then(|v| v.parse::<f64>().ok());
    let fps = match (num("frame_rate_num"), num("frame_rate_den")) {
        (Some(n), Some(d)) if d > 0.0 => n / d,
        _ => 25.0,
    };
    let sec = |frames: i64| r3(frames as f64 / fps);
    let by_id: HashMap<&str, &El> = mlt.children.iter().filter_map(|c| c.attrs.get("id").map(|i| (i.as_str(), c))).collect();

    let filters_of = |el: &El| -> Vec<String> {
        el.children
            .iter()
            .filter(|c| c.name == "filter")
            .map(|f| {
                let p = props(f);
                p.get("mlt_service").or(p.get("kdenlive_id")).cloned().unwrap_or_else(|| "filter".into())
            })
            .collect()
    };

    let playlist_clips = |pl: &El| -> Vec<(i64, i64, Clip)> {
        let mut out = Vec::new();
        let mut cursor = 0i64;
        for c in &pl.children {
            if c.name == "blank" {
                cursor += to_frames(c.attrs.get("length"), fps).unwrap_or(0);
                continue;
            }
            if c.name != "entry" {
                continue;
            }
            let i = to_frames(c.attrs.get("in"), fps).unwrap_or(0);
            let o = to_frames(c.attrs.get("out"), fps).unwrap_or(i);
            let dur = o - i + 1;
            let prod = c.attrs.get("producer").and_then(|id| by_id.get(id.as_str()));
            let (mut source, mut service, mut name, mut speed) = (None, None, None, None);
            let mut fx = filters_of(c);
            if let Some(p) = prod {
                let pr = props(p);
                source = pr.get("resource").cloned();
                service = pr.get("mlt_service").cloned().or_else(|| Some(p.name.clone()));
                name = pr.get("kdenlive:clipname").cloned().or_else(|| {
                    source.as_ref().map(|s| {
                        Path::new(s.trim_start_matches(|ch: char| ch.is_ascii_digit() || ch == '.' || ch == ':'))
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_else(|| s.clone())
                    })
                });
                speed = pr.get("warp_speed").and_then(|s| s.parse::<f64>().ok());
                fx.extend(filters_of(p));
            }
            out.push((cursor, dur, Clip { start: sec(cursor), end: sec(cursor + dur), dur: sec(dur), in_s: sec(i), name, source, service, speed, effects: fx }));
            cursor += dur;
        }
        out
    };

    // the timeline tractor: not referenced by another tractor, with the most tracks
    let tractors: Vec<&El> = mlt.children.iter().filter(|c| c.name == "tractor").collect();
    let referenced: Vec<&str> = tractors.iter().flat_map(|t| t.children.iter().filter(|c| c.name == "track").filter_map(|c| c.attrs.get("producer").map(|s| s.as_str()))).collect();
    let timeline = tractors
        .iter()
        .filter(|t| t.attrs.get("id").map(|i| !referenced.contains(&i.as_str())).unwrap_or(true))
        .max_by_key(|t| t.children.iter().filter(|c| c.name == "track").count())
        .or(tractors.last())
        .copied();

    let mut tracks = Vec::new();
    let mut total = 0i64;
    let mut effect_count: BTreeMap<String, usize> = BTreeMap::new();
    let mut transitions = 0;
    if let Some(t) = timeline {
        for (i, tr) in t.children.iter().filter(|c| c.name == "track").enumerate() {
            let pid = tr.attrs.get("producer").cloned().unwrap_or_default();
            let Some(el) = by_id.get(pid.as_str()) else { continue };
            let mut lists: Vec<&El> = Vec::new();
            let (mut name, mut audio) = (None, false);
            if el.name == "playlist" {
                lists.push(el);
                let p = props(el);
                audio = p.get("kdenlive:audio_track").map(|v| v == "1").unwrap_or(false);
                name = p.get("kdenlive:track_name").cloned();
            } else if el.name == "tractor" {
                let p = props(el);
                audio = p.get("kdenlive:audio_track").map(|v| v == "1").unwrap_or(false);
                name = p.get("kdenlive:track_name").cloned();
                for sub in el.children.iter().filter(|c| c.name == "track") {
                    if let Some(pl) = sub.attrs.get("producer").and_then(|id| by_id.get(id.as_str())) {
                        if pl.name == "playlist" {
                            lists.push(pl);
                            if props(pl).get("kdenlive:audio_track").map(|v| v == "1").unwrap_or(false) {
                                audio = true;
                            }
                        }
                    }
                }
            }
            if lists.is_empty() {
                continue;
            }
            let mut clips: Vec<(i64, i64, Clip)> = lists.iter().flat_map(|l| playlist_clips(l)).collect();
            clips.sort_by_key(|c| c.0);
            let end = clips.iter().map(|c| c.0 + c.1).max().unwrap_or(0);
            total = total.max(end);
            for (_, _, c) in &clips {
                for e in &c.effects {
                    *effect_count.entry(e.clone()).or_insert(0) += 1;
                }
            }
            tracks.push(Track {
                index: i,
                id: pid,
                name,
                kind: if audio { "audio".into() } else { "video".into() },
                hide: tr.attrs.get("hide").cloned(),
                clip_count: clips.len(),
                end: sec(end),
                clips: clips.into_iter().map(|c| c.2).collect(),
            });
        }
        transitions = t.children.iter().filter(|c| c.name == "transition").count();
    }
    let kdenlive_version = mlt
        .children
        .iter()
        .find(|c| c.name == "playlist" && c.attrs.get("id").map(|i| i == "main_bin").unwrap_or(false))
        .and_then(|p| props(p).get("kdenlive:docproperties.kdenliveversion").cloned());
    Ok(Summary {
        file: path_str(file),
        mlt_version: mlt.attrs.get("version").cloned(),
        kdenlive_version,
        width: num("width").unwrap_or(0.0) as u32,
        height: num("height").unwrap_or(0.0) as u32,
        fps: r3(fps),
        duration: sec(total),
        frames: total,
        track_count: tracks.len(),
        tracks,
        transitions,
        effect_count,
        timeline_tractor: timeline.and_then(|t| t.attrs.get("id").cloned()),
    })
}

/* ------------------------------ diff ------------------------------ */

#[derive(Serialize, Debug, Default)]
pub struct TrackDiff {
    pub track: String,
    pub status: String,
    pub clips_before: usize,
    pub clips_after: usize,
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub moved: Vec<String>,
}

#[derive(Serialize, Debug, Default)]
pub struct Diff {
    pub duration_delta: f64,
    pub tracks: Vec<TrackDiff>,
    pub effects_added: BTreeMap<String, i64>,
    pub effects_removed: BTreeMap<String, i64>,
    pub transitions_delta: i64,
}

impl Diff {
    /// True when the two versions are equivalent for editing purposes.
    pub fn is_empty(&self) -> bool {
        self.duration_delta.abs() < 0.04
            && self.transitions_delta == 0
            && self.effects_added.is_empty()
            && self.effects_removed.is_empty()
            && self.tracks.iter().all(|t| t.status == "both" && t.added.is_empty() && t.removed.is_empty() && t.moved.is_empty())
    }

    /// One short paragraph a person (or an agent) can read as "what changed".
    pub fn describe(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if self.duration_delta.abs() >= 0.04 {
            parts.push(format!("duration {:+.1}s", self.duration_delta));
        }
        for t in &self.tracks {
            match t.status.as_str() {
                "added" => parts.push(format!("track {} added ({} clips)", t.track, t.clips_after)),
                "removed" => parts.push(format!("track {} removed", t.track)),
                _ => {
                    let mut bits = Vec::new();
                    if !t.added.is_empty() {
                        bits.push(format!("+{} clips", t.added.len()));
                    }
                    if !t.removed.is_empty() {
                        bits.push(format!("-{} clips", t.removed.len()));
                    }
                    if !t.moved.is_empty() {
                        bits.push(format!("{} moved", t.moved.len()));
                    }
                    if !bits.is_empty() {
                        parts.push(format!("{}: {}", t.track, bits.join(", ")));
                    }
                }
            }
        }
        if !self.effects_added.is_empty() {
            parts.push(format!("effects added: {}", self.effects_added.iter().map(|(k, v)| format!("{k} x{v}")).collect::<Vec<_>>().join(", ")));
        }
        if !self.effects_removed.is_empty() {
            parts.push(format!("effects removed: {}", self.effects_removed.iter().map(|(k, v)| format!("{k} x{v}")).collect::<Vec<_>>().join(", ")));
        }
        if self.transitions_delta != 0 {
            parts.push(format!("transitions {:+}", self.transitions_delta));
        }
        if parts.is_empty() {
            "no editing changes".to_string()
        } else {
            parts.join("; ")
        }
    }
}

fn sig(c: &Clip) -> String {
    format!("{}|in={}|dur={}", c.name.clone().or(c.source.clone()).unwrap_or_default().to_lowercase(), c.in_s, c.dur)
}

pub fn diff(a: &Summary, b: &Summary) -> Diff {
    let mut d = Diff { duration_delta: r3(b.duration - a.duration), transitions_delta: b.transitions as i64 - a.transitions as i64, ..Default::default() };
    let mut names: Vec<String> = a.tracks.iter().chain(b.tracks.iter()).map(|t| t.name.clone().unwrap_or_else(|| t.id.clone())).collect();
    names.sort();
    names.dedup();
    for n in names {
        let ta = a.tracks.iter().find(|t| t.name.clone().unwrap_or_else(|| t.id.clone()) == n);
        let tb = b.tracks.iter().find(|t| t.name.clone().unwrap_or_else(|| t.id.clone()) == n);
        let ma: HashMap<String, &Clip> = ta.map(|t| t.clips.iter().map(|c| (sig(c), c)).collect()).unwrap_or_default();
        let mb: HashMap<String, &Clip> = tb.map(|t| t.clips.iter().map(|c| (sig(c), c)).collect()).unwrap_or_default();
        let mut td = TrackDiff {
            track: n,
            status: match (ta.is_some(), tb.is_some()) {
                (true, true) => "both",
                (true, false) => "removed",
                _ => "added",
            }
            .into(),
            clips_before: ta.map(|t| t.clip_count).unwrap_or(0),
            clips_after: tb.map(|t| t.clip_count).unwrap_or(0),
            ..Default::default()
        };
        for (k, c) in &mb {
            match ma.get(k) {
                None => td.added.push(format!("{k} @ {}s", c.start)),
                Some(o) if (o.start - c.start).abs() > 0.04 => td.moved.push(format!("{k}: {}s -> {}s", o.start, c.start)),
                _ => {}
            }
        }
        for (k, c) in &ma {
            if !mb.contains_key(k) {
                td.removed.push(format!("{k} @ {}s", c.start));
            }
        }
        td.added.sort();
        td.removed.sort();
        td.moved.sort();
        d.tracks.push(td);
    }
    let keys: Vec<&String> = a.effect_count.keys().chain(b.effect_count.keys()).collect();
    for k in keys {
        let delta = *b.effect_count.get(k).unwrap_or(&0) as i64 - *a.effect_count.get(k).unwrap_or(&0) as i64;
        if delta > 0 {
            d.effects_added.insert(k.clone(), delta);
        } else if delta < 0 {
            d.effects_removed.insert(k.clone(), -delta);
        }
    }
    d
}

/* ------------------------------ overlay ------------------------------ */

#[derive(Serialize, Debug)]
pub struct OverlayResult {
    pub out: String,
    pub track_index: usize,
    pub start_frame: i64,
    pub frames: i64,
    pub verified: bool,
    pub note: String,
}

fn probe_duration(ffprobe: &Path, clip: &Path) -> Option<f64> {
    let out = Command::new(ffprobe).args(["-v", "error", "-show_entries", "format=duration", "-of", "default=nw=1:nk=1"]).arg(clip).output().ok()?;
    String::from_utf8_lossy(&out.stdout).trim().parse::<f64>().ok()
}

/// Insert `clip` (a render with alpha) on a new top video track of a COPY of `src`.
/// Experimental on documents saved by the Kdenlive GUI; verified with melt.
pub fn overlay(src: &Path, clip: &Path, at_s: f64, dur_s: Option<f64>, out: &Path, label: &str) -> Result<OverlayResult> {
    if out == src {
        return fail("refusing to overwrite the source project: pass a different --out");
    }
    let summary = inspect(src)?;
    let tractor_id = summary.timeline_tractor.clone().ok_or_else(|| crate::error::Error("no timeline tractor found".into()))?;
    let fps = summary.fps;
    let dur = match dur_s {
        Some(d) => d,
        None => {
            let ffprobe = bin("ffprobe").ok_or_else(|| crate::error::Error("ffprobe not found; pass --dur".into()))?;
            probe_duration(&ffprobe, clip).ok_or_else(|| crate::error::Error("could not read the clip duration; pass --dur".into()))?
        }
    };
    let frames = ((dur * fps).round() as i64).max(1);
    let start = (at_s * fps).round() as i64;
    let xml = fs::read_to_string(src)?;
    let start_tag = format!("id=\"{tractor_id}\"");
    let Some(pos) = xml.find(&"<tractor".to_string()).and_then(|_| {
        // locate the tractor element whose id matches
        let mut from = 0;
        while let Some(i) = xml[from..].find("<tractor") {
            let abs = from + i;
            let end = xml[abs..].find('>').map(|e| abs + e).unwrap_or(abs);
            if xml[abs..end].contains(&start_tag) {
                return Some(abs);
            }
            from = abs + 8;
        }
        None
    }) else {
        return fail("timeline tractor not found in the file");
    };
    let Some(close_rel) = xml[pos..].find("</tractor>") else { return fail("unterminated tractor") };
    let close = pos + close_rel;
    let track_count = xml[pos..close].matches("<track ").count();
    let n = crate::util::short_id();
    let pid = format!("editemos_ovl_{n}");
    let plid = format!("editemos_pl_{n}");
    let ext = clip.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let service = if matches!(ext.as_str(), "json" | "lottie" | "rawr") { "glaxnimate" } else { "avformat" };
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;");
    let resource = esc(path_str(&fs::canonicalize(clip).unwrap_or_else(|_| clip.to_path_buf())).trim_start_matches(r"\\?\"));
    let defs = format!(
        " <producer id=\"{pid}\" in=\"0\" out=\"{o}\">\n  <property name=\"resource\">{resource}</property>\n  <property name=\"mlt_service\">{service}</property>\n  <property name=\"kdenlive:clipname\">{label}</property>\n  <property name=\"kdenlive:id\">{pid}</property>\n </producer>\n <playlist id=\"{plid}\">\n  <property name=\"kdenlive:track_name\">{label}</property>\n  <blank length=\"{start}\"/>\n  <entry producer=\"{pid}\" in=\"0\" out=\"{o}\"/>\n </playlist>\n",
        o = frames - 1,
        label = esc(label),
    );
    let track = format!(
        "  <track producer=\"{plid}\"/>\n  <transition>\n   <property name=\"a_track\">0</property>\n   <property name=\"b_track\">{track_count}</property>\n   <property name=\"mlt_service\">frei0r.cairoblend</property>\n   <property name=\"always_active\">1</property>\n  </transition>\n"
    );
    let mut new_xml = String::with_capacity(xml.len() + defs.len() + track.len());
    new_xml.push_str(&xml[..pos]);
    new_xml.push_str(&defs);
    new_xml.push_str(&xml[pos..close]);
    new_xml.push_str(&track);
    new_xml.push_str(&xml[close..]);
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(out, new_xml)?;

    // verify: re-read it and let melt load a frame
    let re = inspect(out);
    let mut verified = re.is_ok();
    let mut note = String::from("inserted on a new top track");
    if let Ok(r) = &re {
        if r.track_count != summary.track_count + 1 {
            note = format!("inserted, but the track count went {} -> {}", summary.track_count, r.track_count);
        }
    }
    if let Some(melt) = bin("melt") {
        let mut cmd = Command::new(melt);
        cmd.arg(out).args(["in=0", "out=0", "-consumer", "null", "real_time=-1", "-silent"]);
        match run_timeout(&mut cmd, 60) {
            Some(o) if o.status.success() => {}
            _ => {
                verified = false;
                note = "melt could not load the result; the original is untouched".into();
            }
        }
    }
    Ok(OverlayResult { out: path_str(out), track_index: track_count, start_frame: start, frames, verified, note })
}

/* ------------------------------ draft render ------------------------------ */

#[derive(Serialize, Debug)]
pub struct DraftResult {
    pub out: String,
    pub frames: i64,
    pub seconds: f64,
    pub bytes: u64,
}

/// Render a draft with melt. Kdenlive projects contain an endless black track, so
/// melt never finishes without an explicit range: it comes from the timeline length.
pub fn render_draft(project: &Path, out: &Path, timeout_s: u64) -> Result<DraftResult> {
    let Some(melt) = bin("melt") else { return fail("melt not found (install Kdenlive or set EDITEMOS_MELT)") };
    let s = inspect(project)?;
    if s.frames <= 0 {
        return fail("the project has no clips to render");
    }
    if let Some(p) = out.parent() {
        fs::create_dir_all(p)?;
    }
    let started = Instant::now();
    let mut cmd = Command::new(melt);
    cmd.arg(project).arg("in=0").arg(format!("out={}", s.frames - 1)).args([
        "-consumer",
        &format!("avformat:{}", path_str(out)),
        "vcodec=libx264",
        "acodec=aac",
        "preset=veryfast",
        "crf=23",
        "-silent",
    ]);
    let o = run_timeout(&mut cmd, timeout_s).ok_or_else(|| crate::error::Error(format!("melt did not finish within {timeout_s}s")))?;
    if !o.status.success() {
        return fail(format!("melt failed: {}", truncate_chars(String::from_utf8_lossy(&o.stderr).trim(), 300)));
    }
    Ok(DraftResult { out: path_str(out), frames: s.frames, seconds: r3(started.elapsed().as_secs_f64()), bytes: fs::metadata(out).map(|m| m.len()).unwrap_or(0) })
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = r#"<?xml version='1.0' encoding='utf-8'?>
<mlt LC_NUMERIC="C" producer="main_bin" version="7.38.0">
 <profile frame_rate_num="30" frame_rate_den="1" width="1280" height="720"/>
 <producer id="producer0" in="0" out="89"><property name="resource">C:\media\a.mp4</property><property name="mlt_service">avformat</property><property name="kdenlive:clipname">a.mp4</property></producer>
 <playlist id="main_bin"><property name="kdenlive:docproperties.kdenliveversion">26.08.1</property></playlist>
 <playlist id="playlist0"><entry producer="producer0" in="0" out="59"/><blank length="30"/><entry producer="producer0" in="30" out="59"><filter><property name="mlt_service">brightness</property></filter></entry></playlist>
 <tractor id="tractor0"><property name="kdenlive:track_name">V1</property><track producer="playlist0"/></tractor>
 <tractor id="tractor1"><track producer="black_track"/><track producer="tractor0"/></tractor>
</mlt>"#;

    #[test]
    fn title_parsing() {
        let (has, doc, modified, profile) = parse_title("proyecto.kdenlive* / HD 1080p 59.94 fps - Kdenlive");
        assert!(has && modified);
        assert_eq!(doc.as_deref(), Some("proyecto.kdenlive"));
        assert_eq!(profile.as_deref(), Some("HD 1080p 59.94 fps"));
        assert!(!parse_title("Kdenlive").0);
    }

    #[test]
    fn inspect_and_diff() {
        let dir = std::env::temp_dir().join(format!("editemos-kd-{}", crate::util::short_id()));
        fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.kdenlive");
        fs::write(&a, DOC).unwrap();
        let s = inspect(&a).unwrap();
        assert_eq!((s.width, s.height, s.fps), (1280, 720, 30.0));
        assert_eq!(s.tracks.len(), 1);
        assert_eq!(s.tracks[0].clips.len(), 2);
        assert_eq!(s.tracks[0].clips[1].start, 3.0);
        assert_eq!(s.duration, 4.0);
        assert_eq!(s.kdenlive_version.as_deref(), Some("26.08.1"));
        assert_eq!(s.effect_count.get("brightness"), Some(&1));
        let b = dir.join("b.kdenlive");
        fs::write(&b, DOC.replace("<blank length=\"30\"/>", "<blank length=\"60\"/>")).unwrap();
        let d = diff(&s, &inspect(&b).unwrap());
        assert_eq!(d.duration_delta, 1.0);
        assert_eq!(d.tracks[0].moved.len(), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn overlay_inserts_a_track_without_touching_the_source() {
        let dir = std::env::temp_dir().join(format!("editemos-ov-{}", crate::util::short_id()));
        fs::create_dir_all(&dir).unwrap();
        let src = dir.join("src.kdenlive");
        fs::write(&src, DOC).unwrap();
        let clip = dir.join("o.mov");
        fs::write(&clip, b"x").unwrap();
        let out = dir.join("out.kdenlive");
        let r = overlay(&src, &clip, 1.0, Some(2.0), &out, "Motion").unwrap();
        assert_eq!(r.frames, 60);
        assert_eq!(r.start_frame, 30);
        let s = inspect(&out).unwrap();
        assert_eq!(s.track_count, 2);
        assert_eq!(fs::read_to_string(&src).unwrap(), DOC);
        assert!(overlay(&src, &clip, 0.0, Some(1.0), &src, "x").is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
