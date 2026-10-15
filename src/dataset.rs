//! The training set an edit kind learns from. Every clip a person approves,
//! corrects or rejects becomes one example: the plan that was built, the cut as
//! it came out, the events it was built around (goals, impacts), the verdict and,
//! when corrected, the plan the person wanted instead.
//!
//! An example never contains footage. It points at its source by link and
//! section (and a hash of the local file), so a set of examples can be shared in
//! a kind folder and anyone with the right to that footage can rebuild the
//! frames on their own machine (`export --frames`).

use crate::error::{fail, Result};
use crate::story::Story;
use crate::util::{now_iso, path_str, sha256_hex, short_id, write_atomic};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Where a recording came from, written next to it when it is fetched.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Source {
    /// The page the footage was taken from (a VOD, a video).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// The part of it that was fetched, as given to yt-dlp (e.g. "01:40:00-02:00:00").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    /// Seconds into `url` where the local file starts: add it to a time in the file.
    #[serde(default)]
    pub offset: f64,
    /// Hash of the first 8 MB of the local file, to recognise it again.
    #[serde(default)]
    pub sha256_head: String,
    #[serde(default)]
    pub duration: f64,
}

/// Something that happened in the source, in local seconds.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Event {
    pub t: f64,
    /// "goal" (a score change), "hit" (an impact the editor marked) ...
    pub what: String,
    /// For a goal: which scoreboard region (team) changed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team: Option<usize>,
}

/// What `clips build` leaves next to each clip, so it can become an example.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct BuildRecord {
    pub source: String,
    pub cut: Vec<(f64, f64)>,
    #[serde(default)]
    pub events: Vec<Event>,
    #[serde(default)]
    pub checks: Vec<String>,
    #[serde(default)]
    pub tool: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Example {
    pub id: String,
    pub kind: String,
    pub created: String,
    /// editemos version that built it.
    pub tool: String,
    pub source: Source,
    /// The part of the source the plan draws from, with some margin: what a model sees.
    pub window: (f64, f64),
    /// The plan as it was built.
    pub plan: Story,
    /// The segments as they were cut (edges moved onto changes of shot).
    pub cut: Vec<(f64, f64)>,
    #[serde(default)]
    pub events: Vec<Event>,
    /// approved | corrected | rejected | pending
    pub verdict: String,
    /// When corrected: the plan the person wanted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correction: Option<Story>,
    /// The person's own words about it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub notes: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checks: Vec<String>,
}

pub const VERDICTS: &[&str] = &["approved", "corrected", "rejected", "pending"];

pub fn sidecar(video: &Path) -> PathBuf {
    let mut s = video.as_os_str().to_owned();
    s.push(".source.json");
    PathBuf::from(s)
}

/// Seconds of a section start: "01:40:00-02:00:00", "6000-7200", "*01:40:00-...".
pub fn section_offset(section: &str) -> f64 {
    let start = section.trim_start_matches('*').split('-').next().unwrap_or("").trim();
    if start.contains(':') {
        start.split(':').rev().enumerate().map(|(i, p)| p.trim().parse::<f64>().unwrap_or(0.0) * 60f64.powi(i as i32)).sum()
    } else {
        start.parse().unwrap_or(0.0)
    }
}

pub fn head_hash(file: &Path) -> String {
    let mut buf = Vec::new();
    if let Ok(f) = fs::File::open(file) {
        let _ = f.take(8 * 1024 * 1024).read_to_end(&mut buf);
    }
    sha256_hex(&buf)
}

/// Record where a local recording came from (done by `clips` when it fetches a
/// link; by hand with `dataset source` for files fetched some other way).
pub fn write_source(video: &Path, url: Option<&str>, section: Option<&str>) -> Result<Source> {
    let s = Source {
        url: url.map(String::from),
        section: section.map(String::from),
        offset: section.map(section_offset).unwrap_or(0.0),
        sha256_head: head_hash(video),
        duration: crate::motion::render::find_ffmpeg().map(|ff| duration_of(&ff, video)).unwrap_or(0.0),
    };
    write_atomic(&sidecar(video), serde_json::to_string_pretty(&s)?.as_bytes())?;
    Ok(s)
}

pub fn read_source(video: &Path) -> Source {
    fs::read_to_string(sidecar(video)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_else(|| Source { sha256_head: head_hash(video), ..Default::default() })
}

fn duration_of(ffmpeg: &Path, file: &Path) -> f64 {
    let out = Command::new(ffmpeg).args(["-hide_banner", "-i"]).arg(file).stdin(Stdio::null()).output();
    let Ok(out) = out else { return 0.0 };
    String::from_utf8_lossy(&out.stderr)
        .lines()
        .find_map(|l| l.trim().strip_prefix("Duration:").and_then(|r| r.split(',').next()).and_then(crate::clips::parse_time))
        .unwrap_or(0.0)
}

pub fn load(kind_folder: &Path) -> Vec<Example> {
    let mut out = Vec::new();
    for name in ["examples.shared.jsonl", "examples.jsonl"] {
        for line in fs::read_to_string(kind_folder.join(name)).unwrap_or_default().lines() {
            if let Ok(e) = serde_json::from_str::<Example>(line) {
                out.push(e);
            }
        }
    }
    out
}

/// Turn a built clip into an example of `kind`.
pub fn example_from_build(kind: &str, story: Story, record: &BuildRecord, verdict: &str, correction: Option<Story>, notes: &str) -> Result<Example> {
    if !VERDICTS.contains(&verdict) {
        return fail(format!("verdict must be one of {}", VERDICTS.join(", ")));
    }
    if verdict == "corrected" && correction.is_none() {
        return fail("a corrected example needs the plan you wanted (correction=<plan json or file>)");
    }
    let source = read_source(Path::new(&record.source));
    let plans = std::iter::once(&story).chain(correction.iter());
    let lo = plans.clone().flat_map(|s| s.segments.iter().map(|x| x.start)).fold(f64::MAX, f64::min);
    let hi = plans.flat_map(|s| s.segments.iter().map(|x| x.end)).fold(0.0, f64::max);
    if lo >= hi {
        return fail("the plan has no segments");
    }
    // what a model looking at the source should see: the plan's span and some context
    let window = ((lo - 20.0).max(0.0), if source.duration > 0.0 { (hi + 10.0).min(source.duration) } else { hi + 10.0 });
    Ok(Example {
        id: short_id(),
        kind: kind.to_string(),
        created: now_iso(),
        tool: record.tool.clone(),
        source,
        window,
        plan: story,
        cut: record.cut.clone(),
        events: record.events.clone(),
        verdict: verdict.to_string(),
        correction,
        notes: notes.to_string(),
        checks: record.checks.clone(),
    })
}

pub fn append(kind_folder: &Path, e: &Example) -> Result<()> {
    fs::create_dir_all(kind_folder)?;
    use std::io::Write;
    let mut f = fs::OpenOptions::new().create(true).append(true).open(kind_folder.join("examples.jsonl"))?;
    writeln!(f, "{}", serde_json::to_string(e)?)?;
    Ok(())
}

#[derive(Serialize, Debug, Default)]
pub struct Stats {
    pub examples: usize,
    pub approved: usize,
    pub corrected: usize,
    pub rejected: usize,
    pub pending: usize,
    pub with_link: usize,
    pub sources: usize,
}

pub fn stats(examples: &[Example]) -> Stats {
    let count = |v: &str| examples.iter().filter(|e| e.verdict == v).count();
    let mut sources: Vec<&str> = examples.iter().map(|e| e.source.sha256_head.as_str()).collect();
    sources.sort();
    sources.dedup();
    Stats {
        examples: examples.len(),
        approved: count("approved"),
        corrected: count("corrected"),
        rejected: count("rejected"),
        pending: count("pending"),
        with_link: examples.iter().filter(|e| e.source.url.is_some()).count(),
        sources: sources.len(),
    }
}

/// The instruction a model is trained to follow, the same one it gets when planning.
pub fn task_text(kind_rules: &[String], window: (f64, f64), times: &[f64]) -> String {
    let rules = kind_rules.iter().map(|r| format!("- {r}")).collect::<Vec<_>>().join("\n");
    format!(
        "You are editing a clip of this kind. Rules:\n{rules}\n\nThe frames below were taken from {:.1}s to {:.1}s of the recording at these times (seconds): {}.\nPlan the clip and answer with JSON only: {{\"name\",\"title\",\"end\":\"natural|punchline\",\"segments\":[{{\"start\",\"end\",\"label\",\"hits\":[...],\"why\"}}]}}. Times are seconds in the recording.",
        window.0,
        window.1,
        times.iter().map(|t| format!("{t:.1}")).collect::<Vec<_>>().join(", ")
    )
}

/// Sample times inside a window, plus every event (the goals, the impacts), so a
/// model always sees the moments the edit turns on.
pub fn sample_times(window: (f64, f64), fps: f64, events: &[Event], max: usize) -> Vec<f64> {
    let step = 1.0 / fps.max(0.01);
    let mut t: Vec<f64> = Vec::new();
    let mut x = window.0;
    while x < window.1 {
        t.push((x * 10.0).round() / 10.0);
        x += step;
    }
    if t.len() > max {
        let n = t.len();
        t = (0..max).map(|i| t[i * (n - 1) / (max - 1).max(1)]).collect();
    }
    for e in events.iter().filter(|e| e.t >= window.0 && e.t <= window.1) {
        let at = (e.t * 10.0).round() / 10.0;
        if !t.iter().any(|x| (x - at).abs() < 0.3) {
            t.push(at);
        }
    }
    t.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    t
}

#[derive(Serialize, Debug, Default)]
pub struct Exported {
    pub dir: String,
    /// Supervised examples (approved and corrected): frames in, plan out.
    pub sft: usize,
    /// Preference pairs (corrected: the wanted plan over the built one; rejected: no clip over this clip).
    pub preference: usize,
    pub frames: usize,
    /// Examples whose source could not be found on this machine (frames skipped).
    pub missing_sources: Vec<String>,
}

/// Write a training set: `sft.jsonl` (messages with images, the format most
/// vision-language fine-tuning tools read), `preference.jsonl` (chosen/rejected
/// pairs) and, with `sources`, the frames themselves taken from the local files.
pub fn export(examples: &[Example], rules: &[String], out: &Path, fps: f64, max_frames: usize, sources: &[(String, PathBuf)]) -> Result<Exported> {
    fs::create_dir_all(out)?;
    let ffmpeg = crate::motion::render::find_ffmpeg();
    let mut sft = String::new();
    let mut pref = String::new();
    let mut r = Exported { dir: path_str(out), ..Default::default() };
    for e in examples.iter().filter(|e| e.verdict != "pending") {
        let times = sample_times(e.window, fps, &e.events, max_frames);
        let local = sources.iter().find(|(h, _)| *h == e.source.sha256_head).map(|(_, p)| p.clone());
        let mut images: Vec<String> = Vec::new();
        match (&local, &ffmpeg) {
            (Some(file), Some(ff)) => {
                let dir = out.join("frames").join(&e.id);
                fs::create_dir_all(&dir)?;
                for t in &times {
                    let name = format!("{:07.1}.jpg", t);
                    let path = dir.join(&name);
                    if !path.exists() {
                        let ok = Command::new(ff)
                            .args(["-hide_banner", "-loglevel", "error", "-y", "-ss", &format!("{t:.2}"), "-i"])
                            .arg(file)
                            .args(["-frames:v", "1", "-vf", "scale=448:-2", "-q:v", "4"])
                            .arg(&path)
                            .stdin(Stdio::null())
                            .status()
                            .map(|s| s.success())
                            .unwrap_or(false);
                        if !ok {
                            continue;
                        }
                        r.frames += 1;
                    }
                    images.push(format!("frames/{}/{}", e.id, name));
                }
            }
            _ => r.missing_sources.push(e.source.url.clone().unwrap_or_else(|| e.source.sha256_head.clone())),
        }
        let user = task_text(rules, e.window, &times);
        let built = serde_json::to_string(&e.plan)?;
        match e.verdict.as_str() {
            "approved" | "corrected" => {
                let target = e.correction.as_ref().map(serde_json::to_string).transpose()?.unwrap_or_else(|| built.clone());
                let line = serde_json::json!({
                    "id": e.id, "kind": e.kind, "source": e.source,
                    "messages": [
                        {"role": "user", "content": format!("{}{}", "<image>".repeat(images.len()), user)},
                        {"role": "assistant", "content": target}
                    ],
                    "images": images,
                });
                sft.push_str(&serde_json::to_string(&line)?);
                sft.push('\n');
                r.sft += 1;
                if e.verdict == "corrected" {
                    let p = serde_json::json!({"id": e.id, "kind": e.kind, "prompt": user, "images": images, "chosen": target, "rejected": built});
                    pref.push_str(&serde_json::to_string(&p)?);
                    pref.push('\n');
                    r.preference += 1;
                }
            }
            "rejected" => {
                let p = serde_json::json!({"id": e.id, "kind": e.kind, "prompt": user, "images": images,
                    "chosen": "{\"clips\":[],\"why\":\"nothing here is worth a clip of this kind\"}", "rejected": built, "notes": e.notes});
                pref.push_str(&serde_json::to_string(&p)?);
                pref.push('\n');
                r.preference += 1;
            }
            _ => {}
        }
    }
    write_atomic(&out.join("sft.jsonl"), sft.as_bytes())?;
    write_atomic(&out.join("preference.jsonl"), pref.as_bytes())?;
    write_atomic(
        &out.join("README.md"),
        format!(
            "# editemos training set\n\n{} supervised examples (`sft.jsonl`) and {} preference pairs (`preference.jsonl`).\n\n\
             Each line of `sft.jsonl` has `messages` (one `<image>` per frame, then the task; the answer is the plan) and `images` (paths under `frames/`).\n\
             Each line of `preference.jsonl` has `prompt`, `images`, `chosen` and `rejected`.\n\n\
             Frames were taken from the local recordings; the recordings are not part of this set and must not be shared with it.\n",
            r.sft, r.preference
        )
        .as_bytes(),
    )?;
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::story::Segment;

    fn plan(segs: &[(f64, f64)]) -> Story {
        Story { name: "x".into(), segments: segs.iter().map(|(s, e)| Segment { start: *s, end: *e, ..Default::default() }).collect(), ..Default::default() }
    }

    #[test]
    fn sections_become_offsets() {
        assert_eq!(section_offset("01:40:00-02:00:00"), 6000.0);
        assert_eq!(section_offset("*00:10:30-00:12:00"), 630.0);
        assert_eq!(section_offset("600-1500"), 600.0);
    }

    #[test]
    fn examples_carry_their_window_and_refuse_bad_verdicts() {
        let rec =
            BuildRecord { source: "nowhere.mp4".into(), cut: vec![(100.0, 110.0)], events: vec![Event { t: 109.0, what: "goal".into(), team: Some(0) }], ..Default::default() };
        let e = example_from_build("gameplay-highlights", plan(&[(100.0, 110.0), (200.0, 210.0)]), &rec, "approved", None, "").unwrap();
        assert_eq!(e.window, (80.0, 220.0));
        assert!(example_from_build("k", plan(&[(1.0, 2.0)]), &rec, "great", None, "").is_err());
        assert!(example_from_build("k", plan(&[(1.0, 2.0)]), &rec, "corrected", None, "").is_err(), "a correction needs the wanted plan");
        // the frames a model sees always include the goal, even between samples
        let t = sample_times((100.0, 120.0), 0.5, &e.events, 50);
        assert!(t.contains(&109.0) && t.contains(&100.0) && t.len() == 11, "{t:?}");
        assert_eq!(sample_times((0.0, 1000.0), 1.0, &[], 64).len(), 64);
    }

    #[test]
    fn export_pairs_corrections_and_rejections() {
        let rec = BuildRecord { source: "nowhere.mp4".into(), cut: vec![(10.0, 20.0)], ..Default::default() };
        let ok = example_from_build("k", plan(&[(10.0, 20.0)]), &rec, "approved", None, "").unwrap();
        let fixed = example_from_build("k", plan(&[(10.0, 20.0)]), &rec, "corrected", Some(plan(&[(10.0, 26.0)])), "ended before the goal").unwrap();
        let bad = example_from_build("k", plan(&[(10.0, 20.0)]), &rec, "rejected", None, "nothing happens").unwrap();
        let waiting = example_from_build("k", plan(&[(10.0, 20.0)]), &rec, "pending", None, "").unwrap();
        let out = std::env::temp_dir().join(format!("editemos-export-{}", std::process::id()));
        let r = export(&[ok, fixed, bad, waiting], &["Show the outcome.".into()], &out, 0.5, 32, &[]).unwrap();
        assert_eq!((r.sft, r.preference), (2, 2), "approved + corrected train; corrected + rejected pair");
        assert_eq!(r.missing_sources.len(), 3, "no local footage: frames skipped, said so");
        let sft = fs::read_to_string(out.join("sft.jsonl")).unwrap();
        assert!(sft.contains("26.0") && sft.contains("Show the outcome."));
        let pref = fs::read_to_string(out.join("preference.jsonl")).unwrap();
        assert!(pref.contains("nothing here is worth a clip"));
        let _ = fs::remove_dir_all(&out);
    }
}
