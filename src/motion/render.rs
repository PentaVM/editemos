//! Scene -> video with alpha. The browser seeks the scene to each frame time and
//! screenshots it with a transparent background; ffmpeg encodes the sequence.
//! Frames are captured by several tabs of one browser in parallel.

use super::lint::{lint, Issue};
use super::LIB_JS;
use crate::cdp::{Browser, Cdp};
use crate::error::{fail, Result};
use crate::util::{path_str, short_id};
use serde::Serialize;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

#[derive(Clone)]
pub struct RenderOptions {
    pub scene: PathBuf,
    pub out: PathBuf,
    pub fps: f64,
    pub width: u32,
    pub height: u32,
    pub duration: Option<f64>,
    /// prores4444 | pngmov | vp9 | pngseq (default by extension)
    pub format: Option<String>,
    pub subframes: u32,
    pub shutter_deg: f64,
    pub contact: Option<(u32, u32)>,
    pub over: Option<PathBuf>,
    pub lint: bool,
    pub strict: bool,
    pub keep_frames: bool,
    pub tokens: Option<Value>,
    /// Render a single PNG at this time instead of a video.
    pub poster: Option<f64>,
    /// Browser tabs capturing in parallel (default: half the cores, at most 3).
    pub jobs: Option<usize>,
}

impl Default for RenderOptions {
    fn default() -> Self {
        RenderOptions {
            scene: PathBuf::new(),
            out: PathBuf::new(),
            fps: 30.0,
            width: 1920,
            height: 1080,
            duration: None,
            format: None,
            subframes: 1,
            shutter_deg: 180.0,
            contact: Some((4, 3)),
            over: None,
            lint: true,
            strict: false,
            keep_frames: false,
            tokens: None,
            poster: None,
            jobs: None,
        }
    }
}

#[derive(Serialize, Debug)]
pub struct RenderResult {
    pub ok: bool,
    pub scene: String,
    pub out: String,
    pub format: String,
    pub fps: f64,
    pub size: String,
    pub duration: f64,
    pub frames: u32,
    pub subframes: u32,
    pub jobs: u32,
    pub bytes: u64,
    pub capture_seconds: f64,
    pub lint: Vec<Issue>,
    pub contact: Option<String>,
    pub frames_dir: Option<String>,
    /// The moments the scene settles (moves end), checked for layout and shown in `review`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub checkpoints: Vec<f64>,
    /// One frame per checkpoint, in order: what a reviewer should look at.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review: Option<String>,
    /// Sound cues the scene declared (synthesised and muxed into the render).
    pub cues: usize,
    /// The soundtrack, when there is one (inside the video; a .wav for png sequences).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio: Option<String>,
}

pub fn find_ffmpeg() -> Option<PathBuf> {
    for var in ["EDITEMOS_FFMPEG", "KDENLIVE_MCP_FFMPEG_PATH"] {
        if let Ok(p) = std::env::var(var) {
            if Path::new(&p).exists() {
                return Some(PathBuf::from(p));
            }
        }
    }
    for p in [r"C:\Program Files\kdenlive\bin\ffmpeg.exe", r"C:\Program Files\Kdenlive\bin\ffmpeg.exe"] {
        if Path::new(p).exists() {
            return Some(PathBuf::from(p));
        }
    }
    let ok = Command::new("ffmpeg").arg("-version").stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false);
    if ok {
        Some(PathBuf::from("ffmpeg"))
    } else {
        None
    }
}

/// ffmpeg wants rationals for NTSC rates.
pub fn fps_arg(fps: f64) -> String {
    for (v, s) in [(23.976, "24000/1001"), (29.97, "30000/1001"), (47.952, "48000/1001"), (59.94, "60000/1001"), (119.88, "120000/1001")] {
        if (fps - v).abs() < 0.01 {
            return s.to_string();
        }
    }
    if (fps - fps.round()).abs() < 1e-6 {
        format!("{}", fps.round() as i64)
    } else {
        format!("{fps}")
    }
}

fn run_ffmpeg(ffmpeg: &Path, args: &[String], label: &str) -> Result<()> {
    let out = Command::new(ffmpeg).args(["-hide_banner", "-loglevel", "error", "-y"]).args(args).stdin(Stdio::null()).output()?;
    if !out.status.success() {
        return fail(format!("ffmpeg failed ({label}): {}", crate::util::truncate_chars(String::from_utf8_lossy(&out.stderr).trim(), 600)));
    }
    Ok(())
}

fn file_url(p: &Path) -> String {
    let s = fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf()).to_string_lossy().replace('\\', "/");
    let s = s.trim_start_matches("//?/").to_string();
    if s.starts_with('/') {
        format!("file://{s}")
    } else {
        format!("file:///{s}")
    }
}

fn encode(ffmpeg: &Path, frames: &Path, k: u32, fps: f64, format: &str, out: &Path) -> Result<()> {
    let pattern = path_str(&frames.join("f_%07d.png"));
    let in_fps = fps_arg(fps * k as f64);
    let blur = if k > 1 {
        format!("format=rgba,premultiply=inplace=1,tmix=frames={k},unpremultiply=inplace=1,select='not(mod(n+1\\,{k}))',setpts=N/({fps:.6}*TB)")
    } else {
        String::new()
    };
    let chain = |pix: &str| if blur.is_empty() { format!("format={pix}") } else { format!("{blur},format={pix}") };
    let mut args: Vec<String> = vec!["-framerate".into(), in_fps, "-i".into(), pattern];
    match format {
        "prores4444" => args.extend([
            "-vf".to_string(),
            chain("yuva444p10le"),
            "-r".into(),
            fps_arg(fps),
            "-c:v".into(),
            "prores_ks".into(),
            "-profile:v".into(),
            "4444".into(),
            // 8-bit alpha (the frames are 8-bit PNGs) at a fixed high quality:
            // about eight times faster to encode than 16-bit alpha with rate control
            "-alpha_bits".into(),
            "8".into(),
            "-qscale:v".into(),
            "9".into(),
            "-vendor".into(),
            "apl0".into(),
            "-pix_fmt".into(),
            "yuva444p10le".into(),
            path_str(out),
        ]),
        "qtrle" => args.extend(["-vf".to_string(), chain("argb"), "-r".into(), fps_arg(fps), "-c:v".into(), "qtrle".into(), "-pix_fmt".into(), "argb".into(), path_str(out)]),
        "pngmov" => args.extend(["-vf".to_string(), chain("rgba"), "-r".into(), fps_arg(fps), "-c:v".into(), "png".into(), "-pix_fmt".into(), "rgba".into(), path_str(out)]),
        "vp9" => args.extend([
            "-vf".to_string(),
            chain("yuva420p"),
            "-r".into(),
            fps_arg(fps),
            "-c:v".into(),
            "libvpx-vp9".into(),
            "-pix_fmt".into(),
            "yuva420p".into(),
            "-b:v".into(),
            "0".into(),
            "-crf".into(),
            "24".into(),
            "-auto-alt-ref".into(),
            "0".into(),
            path_str(out),
        ]),
        other => return fail(format!("unknown format '{other}' (prores4444, pngmov, vp9, pngseq)")),
    }
    run_ffmpeg(ffmpeg, &args, format)
}

/// Frames spread evenly over the scene, one per cell.
fn uniform_frames(total: u32, n: u32) -> Vec<u32> {
    (0..n).map(|j| (((j as f64 + 0.5) / n as f64 * total as f64 - 0.5).round() as i64).clamp(0, total as i64 - 1) as u32).collect()
}

#[allow(clippy::too_many_arguments)]
fn contact_sheet(ffmpeg: &Path, frames: &Path, k: u32, pick_frames: &[u32], w: u32, h: u32, grid: (u32, u32), over: Option<&Path>, out: &Path) -> Result<()> {
    let n = grid.0 * grid.1;
    let pick = std::env::temp_dir().join(format!("editemos-pick-{}", short_id()));
    fs::create_dir_all(&pick)?;
    for j in 0..n {
        // a grid larger than the list repeats the last frame rather than leaving a hole
        let i = pick_frames.get(j as usize).or(pick_frames.last()).copied().unwrap_or(0);
        let src = frames.join(format!("f_{:07}.png", i * k + (k >> 1) + 1));
        fs::copy(&src, pick.join(format!("p_{:03}.png", j + 1)))?;
    }
    let mut args: Vec<String> = Vec::new();
    let bg;
    if let Some(o) = over {
        args.extend(["-loop".into(), "1".into(), "-t".into(), (n + 1).to_string(), "-framerate".into(), "1".into(), "-i".into(), path_str(o)]);
        bg = format!("[0:v]scale={w}:{h}:force_original_aspect_ratio=increase,crop={w}:{h},format=rgba[bg]");
    } else {
        args.extend(["-f".into(), "lavfi".into(), "-i".into(), format!("color=c=0x303030:s={w}x{h}:r=1:d={}", n + 1)]);
        bg = "[0:v]format=rgba[bg]".to_string();
    }
    args.extend(["-framerate".into(), "1".into(), "-i".into(), path_str(&pick.join("p_%03d.png"))]);
    let graph = format!("{bg};[1:v]format=rgba[fg];[bg][fg]overlay=eof_action=pass:format=auto,scale=480:-2,tile={}x{}:padding=6:margin=6[o]", grid.0, grid.1);
    args.extend(["-filter_complex".into(), graph, "-map".into(), "[o]".into(), "-frames:v".into(), "1".into(), path_str(out)]);
    let r = run_ffmpeg(ffmpeg, &args, "contact sheet");
    let _ = fs::remove_dir_all(&pick);
    r
}

/// When the scene comes to rest: a couple of frames after each move ends, merged
/// when close together, always including the last frame. These are the stills a
/// reviewer judges (is the text readable, did everything land before the next
/// thing appeared) and where the layout is checked.
pub fn checkpoints(report: &Value, duration: f64, fps: f64, max: usize) -> Vec<f64> {
    let last = (duration - 1.0 / fps).max(0.0);
    let mut ends: Vec<f64> = report
        .get("tweens")
        .and_then(|t| t.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|t| {
                    let at = t.get("at")?.as_f64()?;
                    let dur = t.get("dur")?.as_f64()?;
                    (dur > 0.0).then_some(at + dur)
                })
                .filter(|e| *e > 0.0 && *e < duration)
                .map(|e| (e + 2.0 / fps).min(last))
                .collect()
        })
        .unwrap_or_default();
    ends.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mut kept: Vec<f64> = Vec::new();
    for e in ends {
        if kept.last().map(|l| e - l >= 0.25).unwrap_or(true) {
            kept.push(e);
        }
    }
    if kept.last().map(|l| last - l >= 0.25).unwrap_or(true) {
        kept.push(last);
    }
    if kept.len() == 1 && last > 0.5 {
        kept.insert(0, duration * 0.5);
    }
    if kept.len() > max && max >= 2 {
        let n = kept.len();
        kept = (0..max).map(|j| kept[j * (n - 1) / (max - 1)]).collect();
    }
    kept.into_iter().map(|t| (t * 1000.0).round() / 1000.0).collect()
}

fn inspect_issues(page: &mut Cdp, times: &[f64], allow: &Value) -> Result<Vec<Issue>> {
    let mut seen: Vec<(String, String)> = Vec::new();
    let mut out = Vec::new();
    for t in times {
        page.eval(&format!("window.__motion.seek({t:.6})"), true)?;
        page.eval(RAF2, true)?;
        let v = page.eval(&format!("JSON.stringify(window.__motion.inspect ? window.__motion.inspect({t:.3}) : [])"), false)?;
        let list: Vec<Value> = v.as_str().and_then(|s| serde_json::from_str(s).ok()).unwrap_or_default();
        for x in list {
            let code = x["code"].as_str().unwrap_or("").to_string();
            let what = x["what"].as_str().unwrap_or("").to_string();
            if code.is_empty() || seen.iter().any(|(c, w)| *c == code && *w == what) {
                continue;
            }
            seen.push((code.clone(), what.clone()));
            let message = match code.as_str() {
                "TEXT_OFFSCREEN" => format!("{what} is cut by the frame edge at {t:.2}s"),
                "TEXT_OUTSIDE_SAFE" => format!("{what} sits outside the 5% safe margin at {t:.2}s (platform UI and overscan cover it)"),
                "TEXT_OVERLAP" => format!("{what} overlap at {t:.2}s"),
                "TEXT_TINY" => format!("{what} is too small to read on a phone at {t:.2}s"),
                "LOW_CONTRAST" => format!("{what} contrast against its own background at {t:.2}s (needs 3:1 or more)"),
                _ => format!("{what} at {t:.2}s"),
            };
            let level = if code == "TEXT_OFFSCREEN" { "error" } else { "warn" };
            match allow.get(&code).and_then(|r| r.as_str()) {
                Some(why) => out.push(Issue { level: "info".into(), code, message: format!("{message} (allowed: {why})") }),
                None => out.push(Issue { level: level.into(), code, message }),
            }
        }
    }
    Ok(out)
}

/// Mix the scene's sound (synthesised cues plus any audio files it placed) and
/// put it into the render. Returns where the sound ended up.
fn add_sound(ffmpeg: &Path, cues: &[super::sound::Cue], scene: &Path, duration: f64, format: &str, out: &Path, issues: &mut Vec<Issue>) -> Result<Option<PathBuf>> {
    if cues.is_empty() {
        return Ok(None);
    }
    let tmp = TempDir { path: std::env::temp_dir().join(format!("editemos-sound-{}", short_id())), keep: false };
    fs::create_dir_all(&tmp.path)?;
    let bed = tmp.path.join("synth.wav");
    super::sound::write_wav(&bed, &super::sound::mix(cues, duration))?;
    let base = scene.parent().unwrap_or(Path::new("."));
    let mut args: Vec<String> = Vec::new();
    let is_seq = format == "pngseq";
    if !is_seq {
        args.extend(["-i".into(), path_str(out)]);
    }
    args.extend(["-i".into(), path_str(&bed)]);
    let first_audio = if is_seq { 0 } else { 1 };
    let mut graph = String::new();
    let mut labels = format!("[{first_audio}:a]");
    let mut n_inputs = 1;
    for c in cues.iter().filter(|c| c.kind == "file") {
        let Some(src) = c.src.as_ref() else { continue };
        let p = if Path::new(src).is_absolute() { PathBuf::from(src) } else { base.join(src) };
        if !p.exists() {
            issues.push(Issue { level: "warn".into(), code: "SFX_FILE_MISSING".into(), message: format!("audio file not found, left out: {}", p.display()) });
            continue;
        }
        if let Some(trim) = c.trim.filter(|t| *t > 0.0) {
            args.extend(["-ss".into(), format!("{trim:.3}")]);
        }
        args.extend(["-i".into(), path_str(&p)]);
        let idx = first_audio + n_inputs;
        let ms = (c.at * 1000.0).round().max(0.0) as i64;
        graph.push_str(&format!("[{idx}:a]aresample={},aformat=channel_layouts=stereo,adelay={ms}|{ms},volume={:.3}[f{idx}];", super::sound::RATE, c.gain));
        labels.push_str(&format!("[f{idx}]"));
        n_inputs += 1;
    }
    let mixed = if n_inputs > 1 { format!("{graph}{labels}amix=inputs={n_inputs}:normalize=0:duration=first[a]") } else { format!("[{first_audio}:a]anull[a]") };
    args.extend(["-filter_complex".into(), mixed]);
    if is_seq {
        let wav = out.join("audio.wav");
        args.extend(["-map".into(), "[a]".into(), "-c:a".into(), "pcm_s16le".into(), path_str(&wav)]);
        run_ffmpeg(ffmpeg, &args, "sound")?;
        return Ok(Some(wav));
    }
    let ext = out.extension().and_then(|e| e.to_str()).unwrap_or("mov").to_string();
    let muxed = tmp.path.join(format!("muxed.{ext}"));
    let codec: &[&str] = if format == "vp9" { &["-c:a", "libopus", "-b:a", "160k"] } else { &["-c:a", "pcm_s16le"] };
    args.extend(["-map".into(), "0:v".into(), "-map".into(), "[a]".into(), "-c:v".into(), "copy".into()]);
    args.extend(codec.iter().map(|s| s.to_string()));
    args.extend(["-t".into(), format!("{duration:.4}"), path_str(&muxed)]);
    run_ffmpeg(ffmpeg, &args, "sound")?;
    fs::copy(&muxed, out)?;
    Ok(Some(out.to_path_buf()))
}

/// A temporary folder that is removed on drop (also on early error returns).
struct TempDir {
    path: PathBuf,
    keep: bool,
}

impl Drop for TempDir {
    fn drop(&mut self) {
        if !self.keep {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

// Two animation frames so layout and style are settled; a timer fallback keeps a
// throttled tab from stalling the whole render.
const RAF2: &str = "Promise.race([new Promise(r => requestAnimationFrame(() => requestAnimationFrame(() => r(true)))), new Promise(r => setTimeout(() => r(false), 400))])";
const RESET: &str = "(() => { const s = document.createElement('style'); s.textContent = 'html,body{margin:0;background:transparent!important;overflow:hidden}'; document.head.appendChild(s); return true; })()";

fn resolve_scene(scene: &Path) -> Result<PathBuf> {
    let mut scene = scene.to_path_buf();
    if scene.is_dir() {
        scene = scene.join("index.html");
    }
    if !scene.exists() {
        return fail(format!("scene not found: {}", scene.display()));
    }
    Ok(scene)
}

/// Point a tab at the scene with transparent background and the motion runtime
/// injected, and wait until the scene has mounted.
fn prepare_page(page: &mut Cdp, scene: &Path, width: u32, height: u32, tokens: &Value) -> Result<()> {
    page.set_viewport(width, height)?;
    page.transparent_background()?;
    // Pretend every tab has focus: unfocused tabs stop producing animation frames.
    let _ = page.call("Emulation.setFocusEmulationEnabled", serde_json::json!({"enabled": true}));
    page.inject_on_new_document(&format!("window.__EDITEMOS_TOKENS__ = {};\n{}", tokens, LIB_JS))?;
    page.navigate(&file_url(scene), Duration::from_secs(30))?;
    page.eval("document.fonts.ready.then(() => true)", true)?;
    page.eval(RESET, false)?;
    let t0 = Instant::now();
    while page.eval("!!(window.__motion && window.__motion.seek)", false)? != Value::Bool(true) {
        if t0.elapsed() > Duration::from_secs(10) {
            return fail("window.__motion not found: did the scene call Motion.mount(...)?");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}

/// A scene loaded in a headless browser, ready to be seeked.
struct Opened {
    browser: Browser,
    page: Cdp,
    scene: PathBuf,
    tokens: Value,
}

fn open_scene(scene: &Path, width: u32, height: u32, tokens: Option<&Value>) -> Result<Opened> {
    let scene = resolve_scene(scene)?;
    let tokens = tokens.cloned().unwrap_or(Value::Object(Default::default()));
    let browser = Browser::launch(width, height)?;
    let mut page = browser.page()?;
    prepare_page(&mut page, &scene, width, height, &tokens)?;
    Ok(Opened { browser, page, scene, tokens })
}

fn scene_report(page: &mut Cdp) -> Result<Value> {
    Ok(match page.eval("JSON.stringify(window.__motion.report ? window.__motion.report() : {tweens: []})", false)? {
        Value::String(s) => serde_json::from_str(&s).unwrap_or(Value::Null),
        _ => Value::Null,
    })
}

/// Load a scene and run the anti-generic linter without rendering any frames.
pub fn lint_scene(scene: &Path, fps: f64, width: u32, height: u32, tokens: Option<&Value>) -> Result<Vec<Issue>> {
    let mut o = open_scene(scene, width, height, tokens)?;
    let report = scene_report(&mut o.page)?;
    drop(o.page);
    drop(o.browser);
    Ok(lint(&report, fps))
}

struct Capture<'a> {
    dir: &'a Path,
    k: u32,
    fps: f64,
    duration: f64,
    shutter: f64,
    total: u32,
    done: AtomicU32,
}

/// Capture output frames `first, first+step, ...` (each as `k` sub-frames).
fn capture_frames(page: &mut Cdp, c: &Capture, first: u32, step: u32) -> Result<()> {
    let mut i = first;
    while i < c.total {
        for s in 0..c.k {
            let off = if c.k > 1 { ((s as f64 + 0.5) / c.k as f64 - 0.5) * c.shutter } else { 0.0 };
            let t = ((i as f64 / c.fps) + off / c.fps).clamp(0.0, c.duration);
            page.eval(&format!("window.__motion.seek({t:.6})"), true)?;
            page.eval(RAF2, true)?;
            let png = page.screenshot(false)?;
            fs::write(c.dir.join(format!("f_{:07}.png", i * c.k + s + 1)), png)?;
        }
        c.done.fetch_add(1, Ordering::Relaxed);
        i += step;
    }
    Ok(())
}

fn default_jobs() -> usize {
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2);
    (cores / 2).clamp(1, 3)
}

pub fn render(o: &RenderOptions) -> Result<RenderResult> {
    let k = o.subframes.max(1);
    let ext = o.out.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let format = o.format.clone().unwrap_or_else(|| match ext.as_str() {
        "webm" => "vp9".into(),
        "mov" => "prores4444".into(),
        "" => "pngseq".into(),
        _ => "prores4444".into(),
    });
    let ffmpeg = find_ffmpeg();
    if ffmpeg.is_none() && o.poster.is_none() {
        return fail("ffmpeg not found (set EDITEMOS_FFMPEG)");
    }

    crate::util::trace("render: start");
    let Opened { browser, mut page, scene, tokens } = open_scene(&o.scene, o.width, o.height, o.tokens.as_ref())?;
    crate::util::trace("render: scene loaded");

    if let Some(t) = o.poster {
        page.eval(&format!("window.__motion.seek({t:.6})"), true)?;
        page.eval(RAF2, true)?;
        let png = page.screenshot(false)?;
        if let Some(parent) = o.out.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&o.out, &png)?;
        return Ok(RenderResult {
            ok: true,
            scene: path_str(&scene),
            out: path_str(&o.out),
            format: "png".into(),
            fps: o.fps,
            size: format!("{}x{}", o.width, o.height),
            duration: 0.0,
            frames: 1,
            subframes: 1,
            jobs: 1,
            bytes: png.len() as u64,
            capture_seconds: 0.0,
            lint: Vec::new(),
            contact: None,
            frames_dir: None,
            checkpoints: Vec::new(),
            review: None,
            cues: 0,
            audio: None,
        });
    }

    let scene_duration = page.eval("window.__motion.duration", false)?.as_f64().unwrap_or(0.0);
    let duration = o.duration.unwrap_or(scene_duration);
    if duration <= 0.0 {
        return fail("scene duration is 0; pass --duration or Motion.mount(..., {duration})");
    }
    let total = ((duration * o.fps).round() as u32).max(1);

    let report = scene_report(&mut page)?;
    let mut issues = if o.lint { lint(&report, o.fps) } else { Vec::new() };
    let cues: Vec<super::sound::Cue> = report.get("sfx").cloned().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
    let marks = checkpoints(&report, duration, o.fps, 6);
    if o.lint {
        issues.extend(inspect_issues(&mut page, &marks, &report["allow"])?);
    }
    if o.strict && issues.iter().any(|i| i.level == "error") {
        let msg: Vec<String> = issues.iter().filter(|i| i.level == "error").map(|i| format!("{}: {}", i.code, i.message)).collect();
        return fail(format!("lint errors (strict): {}", msg.join(" | ")));
    }

    let is_seq = format == "pngseq";
    let frames = TempDir { path: if is_seq { o.out.clone() } else { std::env::temp_dir().join(format!("editemos-frames-{}", short_id())) }, keep: is_seq || o.keep_frames };
    fs::create_dir_all(&frames.path)?;

    let jobs = o.jobs.unwrap_or_else(default_jobs).clamp(1, 8).min(total as usize) as u32;
    let cap = Capture { dir: &frames.path, k, fps: o.fps, duration, shutter: o.shutter_deg / 360.0, total, done: AtomicU32::new(0) };
    let port = browser.port();
    let started = Instant::now();
    let results: Vec<Result<()>> = std::thread::scope(|sc| {
        let cap = &cap;
        let scene = &scene;
        let tokens = &tokens;
        let handles: Vec<_> = (1..jobs)
            .map(|w| {
                sc.spawn(move || -> Result<()> {
                    crate::util::trace(&format!("worker {w}: opening tab"));
                    let mut p = crate::cdp::new_page_on(port, o.width, o.height)?;
                    crate::util::trace(&format!("worker {w}: tab open"));
                    prepare_page(&mut p, scene, o.width, o.height, tokens)?;
                    crate::util::trace(&format!("worker {w}: scene ready"));
                    capture_frames(&mut p, cap, w, jobs)
                })
            })
            .collect();
        let mut rs = vec![capture_frames(&mut page, cap, 0, jobs)];
        for h in handles {
            rs.push(h.join().unwrap_or_else(|_| fail("a capture worker crashed")));
        }
        rs
    });
    for r in results {
        r?;
    }
    let capture_seconds = (started.elapsed().as_secs_f64() * 10.0).round() / 10.0;
    crate::util::trace(&format!("render: captured {total}x{k} frames with {jobs} tab(s)"));
    drop(page);
    drop(browser);

    let bytes: u64 = if !is_seq {
        if let Some(parent) = o.out.parent() {
            fs::create_dir_all(parent)?;
        }
        encode(ffmpeg.as_ref().unwrap(), &frames.path, k, o.fps, &format, &o.out)?;
        fs::metadata(&o.out).map(|m| m.len()).unwrap_or(0)
    } else {
        fs::read_dir(&frames.path)?.flatten().filter_map(|e| e.metadata().ok()).map(|m| m.len()).sum()
    };
    crate::util::trace("render: encoded");
    let audio = add_sound(ffmpeg.as_ref().unwrap(), &cues, &scene, duration, &format, &o.out, &mut issues)?.map(|p| path_str(&p));

    let side = |suffix: &str| {
        if is_seq {
            o.out.with_file_name(format!("{}.{suffix}", o.out.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()))
        } else {
            o.out.with_extension(suffix)
        }
    };
    let mut contact = None;
    let mut review = None;
    if let (Some(grid), Some(ff)) = (o.contact, ffmpeg.as_ref()) {
        let out = side("contact.png");
        contact_sheet(ff, &frames.path, k, &uniform_frames(total, grid.0 * grid.1), o.width, o.height, grid, o.over.as_deref(), &out)?;
        contact = Some(path_str(&out));
        if !marks.is_empty() {
            let idx: Vec<u32> = marks.iter().map(|t| ((t * o.fps).round() as i64).clamp(0, total as i64 - 1) as u32).collect();
            let n = idx.len() as u32;
            let g = if n <= 3 { (n, 1) } else { (3, n.div_ceil(3)) };
            let out = side("review.png");
            contact_sheet(ff, &frames.path, k, &idx, o.width, o.height, g, o.over.as_deref(), &out)?;
            review = Some(path_str(&out));
        }
    }
    let kept = if o.keep_frames && !is_seq { Some(path_str(&frames.path)) } else { None };
    Ok(RenderResult {
        ok: true,
        scene: path_str(&scene),
        out: path_str(&o.out),
        format,
        fps: o.fps,
        size: format!("{}x{}", o.width, o.height),
        duration,
        frames: total,
        subframes: k,
        jobs,
        bytes,
        capture_seconds,
        lint: issues,
        contact,
        frames_dir: kept,
        checkpoints: marks,
        review,
        cues: cues.len(),
        audio,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkpoints_follow_the_moves() {
        let r = serde_json::json!({"tweens": [{"at": 0.1, "dur": 0.4}, {"at": 0.12, "dur": 0.42}, {"at": 1.0, "dur": 0.6}, {"at": 2.0, "dur": 0.0}]});
        // two moves ending together are one still; a hard set is not a move; the last frame is always checked
        assert_eq!(checkpoints(&r, 3.0, 30.0, 6), vec![0.567, 1.667, 2.967]);
        let many: Vec<Value> = (0..20).map(|i| serde_json::json!({"at": i as f64 * 0.4, "dur": 0.2})).collect();
        let c = checkpoints(&serde_json::json!({ "tweens": many }), 9.0, 30.0, 6);
        assert_eq!(c.len(), 6);
        assert_eq!(c[0], 0.267);
        assert_eq!(*c.last().unwrap(), 8.967);
        // nothing moves: the middle and the end
        assert_eq!(checkpoints(&serde_json::json!({}), 4.0, 25.0, 6), vec![2.0, 3.96]);
    }

    #[test]
    fn ntsc_rates() {
        assert_eq!(fps_arg(29.97), "30000/1001");
        assert_eq!(fps_arg(59.94), "60000/1001");
        assert_eq!(fps_arg(30.0), "30");
        assert_eq!(fps_arg(25.0), "25");
    }
}
