//! Clips mode: a long recording in, the best moments out, already cut and framed.
//!
//! One audio-only pass over the file (hundreds of times faster than real time)
//! measures loudness every 100 ms and finds the silences. Moments are scored by
//! how much louder and livelier they are than the recording as a whole, snapped to
//! silences so no clip starts or ends mid-word, and exported in parallel. With a
//! transcript (.srt or .vtt) the words count too and each clip gets its own
//! subtitle file. Everything lands in a manifest a person can correct and re-export.

use crate::error::{fail, Result};
use crate::motion::render::find_ffmpeg;
use crate::util::{path_str, write_atomic};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

#[derive(Clone, Debug)]
pub struct Options {
    pub input: PathBuf,
    pub out_dir: PathBuf,
    pub count: usize,
    pub min_s: f64,
    pub max_s: f64,
    /// "9:16", "1:1", "4:5", "16:9" or "source"
    pub aspect: String,
    /// Output height in pixels (width follows the aspect).
    pub height: u32,
    pub transcript: Option<PathBuf>,
    /// Stream copy: instant, cuts on keyframes, keeps the source framing.
    pub copy: bool,
    /// Burn the subtitles into the picture (needs a transcript).
    pub burn: bool,
    /// Only analyse and write the manifest.
    pub plan_only: bool,
    pub jobs: usize,
    /// "crop" fills the frame from the centre; "fit" keeps the whole picture over a blurred copy
    /// of itself (gameplay, screen recordings, anything where the edges matter).
    pub layout: String,
    /// libass style for burned captions (built from the project's edit brand).
    pub caption_style: Option<String>,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            input: PathBuf::new(),
            out_dir: PathBuf::new(),
            count: 5,
            min_s: 15.0,
            max_s: 60.0,
            aspect: "9:16".into(),
            height: 1920,
            transcript: None,
            copy: false,
            burn: false,
            plan_only: false,
            jobs: 2,
            layout: "crop".into(),
            caption_style: None,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Clip {
    pub index: usize,
    pub start: f64,
    pub end: f64,
    pub duration: f64,
    pub score: f64,
    pub why: Vec<String>,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default)]
    pub subtitles: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Manifest {
    pub input: String,
    pub duration: f64,
    pub aspect: String,
    pub clips: Vec<Clip>,
    pub analysis_seconds: f64,
    pub export_seconds: f64,
    /// How many times faster than real time the analysis ran.
    pub analysis_speed: f64,
    pub manifest: String,
    pub edl: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Cue {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

pub(crate) fn parse_time(t: &str) -> Option<f64> {
    let t = t.trim().replace(',', ".");
    let parts: Vec<&str> = t.split(':').collect();
    let mut secs = 0.0;
    for p in &parts {
        secs = secs * 60.0 + p.parse::<f64>().ok()?;
    }
    Some(secs)
}

/// Read .srt or .vtt cues.
pub fn parse_transcript(text: &str) -> Vec<Cue> {
    let mut cues = Vec::new();
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        if let Some((a, b)) = lines[i].split_once("-->") {
            let end_tok = b.split_whitespace().next().unwrap_or("");
            if let (Some(start), Some(end)) = (parse_time(a), parse_time(end_tok)) {
                let mut body = Vec::new();
                i += 1;
                while i < lines.len() && !lines[i].trim().is_empty() {
                    body.push(lines[i].trim());
                    i += 1;
                }
                let raw = body.join(" ");
                // drop inline tags such as <i> or <00:00:01.000>
                let mut clean = String::new();
                let mut depth = 0;
                for ch in raw.chars() {
                    match ch {
                        '<' => depth += 1,
                        '>' if depth > 0 => depth -= 1,
                        c if depth == 0 => clean.push(c),
                        _ => {}
                    }
                }
                cues.push(Cue { start, end, text: clean.trim().to_string() });
                continue;
            }
        }
        i += 1;
    }
    cues
}

fn srt_time(t: f64) -> String {
    let ms = (t.max(0.0) * 1000.0).round() as u64;
    format!("{:02}:{:02}:{:02},{:03}", ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000)
}

/// Automatic captions (YouTube and others) "roll": every cue repeats the line
/// before it, with a 10 ms copy in between. Left alone that doubles every word on
/// screen and in the scoring. Keep each word once, on the cue where it first appears.
pub fn dedupe_rolling(cues: Vec<Cue>) -> Vec<Cue> {
    let mut out: Vec<Cue> = Vec::new();
    let mut prev_words: Vec<String> = Vec::new();
    for c in cues {
        // speaker-change markers are not words
        let text = c.text.replace(">>", " ");
        let words: Vec<String> = text.split_whitespace().map(String::from).collect();
        if words.is_empty() {
            continue;
        }
        // the longest tail of what was already said that this cue starts with
        let max = prev_words.len().min(words.len());
        let overlap = (1..=max).rev().find(|n| prev_words[prev_words.len() - n..] == words[..*n]).unwrap_or(0);
        let fresh = &words[overlap..];
        if c.end - c.start >= 0.05 && !fresh.is_empty() {
            out.push(Cue { start: c.start, end: c.end, text: fresh.join(" ") });
        }
        prev_words = words;
    }
    out
}

/// The cues inside a clip, re-timed to start at zero.
pub fn slice_srt(cues: &[Cue], start: f64, end: f64) -> String {
    let mut out = String::new();
    let mut n = 0;
    for c in cues.iter().filter(|c| c.end > start && c.start < end) {
        n += 1;
        out.push_str(&format!("{n}\n{} --> {}\n{}\n\n", srt_time(c.start.max(start) - start), srt_time(c.end.min(end) - start), c.text));
    }
    out
}

fn ytdlp() -> Option<Command> {
    if let Ok(p) = std::env::var("EDITEMOS_YTDLP") {
        if Path::new(&p).exists() {
            return Some(Command::new(p));
        }
    }
    let ok = |c: &mut Command| c.arg("--version").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false);
    if ok(&mut Command::new("yt-dlp")) {
        return Some(Command::new("yt-dlp"));
    }
    for py in ["python", "python3"] {
        if ok(Command::new(py).args(["-m", "yt_dlp"])) {
            let mut c = Command::new(py);
            c.args(["-m", "yt_dlp"]);
            return Some(c);
        }
    }
    None
}

pub fn is_url(s: &str) -> bool {
    s.starts_with("http://") || s.starts_with("https://")
}

/// Fetch a recording (and its subtitles, when the site has them) with yt-dlp, which
/// must already be installed. `section` is "START-END" in seconds or hh:mm:ss to take
/// only part of a long stream. Only use this for footage you may edit.
pub fn fetch(url: &str, dir: &Path, max_height: u32, section: Option<&str>) -> Result<(PathBuf, Option<PathBuf>)> {
    let Some(mut cmd) = ytdlp() else {
        return fail("a URL needs yt-dlp, which is not installed (pip install yt-dlp, or set EDITEMOS_YTDLP). You can also download the recording yourself and pass the file");
    };
    std::fs::create_dir_all(dir)?;
    let before: Vec<PathBuf> = std::fs::read_dir(dir)?.flatten().map(|e| e.path()).collect();
    cmd.args(["--no-playlist", "--no-progress", "--restrict-filenames", "-f", &format!("bv*[height<={max_height}]+ba/b[height<={max_height}]/b"), "--merge-output-format", "mp4"]);
    cmd.args(["--write-subs", "--write-auto-subs", "--sub-langs", "en.*,es.*", "--convert-subs", "srt", "--ignore-errors"]);
    if let Some(ff) = find_ffmpeg().filter(|f| f.is_absolute()) {
        cmd.arg("--ffmpeg-location").arg(ff);
    }
    if let Some(s) = section {
        cmd.args(["--download-sections", &format!("*{s}")]);
    }
    cmd.arg("-o").arg(dir.join("%(id)s.%(ext)s")).arg(url);
    let out = cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).output()?;
    let new: Vec<PathBuf> = std::fs::read_dir(dir)?.flatten().map(|e| e.path()).filter(|p| !before.contains(p)).collect();
    let video = new.iter().find(|p| p.extension().map(|e| e == "mp4" || e == "mkv" || e == "webm").unwrap_or(false)).cloned().or_else(|| {
        // already downloaded on an earlier run
        before.iter().filter(|p| p.extension().map(|e| e == "mp4").unwrap_or(false)).max_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok()).cloned()
    });
    // a site that refuses the captions must not cost the recording: try once more without them
    let video = match video {
        Some(v) => Some(v),
        None => {
            let Some(mut retry) = ytdlp() else { return fail("yt-dlp disappeared") };
            retry.args([
                "--no-playlist",
                "--no-progress",
                "--restrict-filenames",
                "-f",
                &format!("bv*[height<={max_height}]+ba/b[height<={max_height}]/b"),
                "--merge-output-format",
                "mp4",
            ]);
            if let Some(ff) = find_ffmpeg().filter(|f| f.is_absolute()) {
                retry.arg("--ffmpeg-location").arg(ff);
            }
            if let Some(s) = section {
                retry.args(["--download-sections", &format!("*{s}")]);
            }
            let _ = retry.arg("-o").arg(dir.join("%(id)s.%(ext)s")).arg(url).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status();
            std::fs::read_dir(dir)?
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().map(|e| e == "mp4").unwrap_or(false))
                .max_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
        }
    };
    let Some(video) = video else {
        return fail(format!("yt-dlp could not fetch {url}: {}", crate::util::truncate_chars(String::from_utf8_lossy(&out.stderr).trim(), 300)));
    };
    let stem = video.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    // the language that was actually spoken ("-orig") before any machine translation
    let mut subs: Vec<PathBuf> = std::fs::read_dir(dir)?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().map(|e| e == "srt").unwrap_or(false) && p.file_name().map(|n| n.to_string_lossy().starts_with(&stem)).unwrap_or(false))
        .collect();
    subs.sort_by_key(|p| (!p.to_string_lossy().contains("-orig"), p.to_string_lossy().len()));
    Ok((video, subs.into_iter().next()))
}

pub struct Audio {
    pub duration: f64,
    /// Momentary loudness (LUFS) every 100 ms.
    pub loud: Vec<f32>,
    /// (start, end) of each silence.
    pub silences: Vec<(f64, f64)>,
}

/// One audio-only ffmpeg pass: loudness curve + silences + duration.
pub fn analyse_audio(ffmpeg: &Path, input: &Path) -> Result<Audio> {
    let out = Command::new(ffmpeg)
        .args(["-hide_banner", "-nostats", "-vn", "-sn", "-dn", "-i"])
        .arg(input)
        .args(["-af", "silencedetect=noise=-34dB:d=0.3,ebur128=metadata=1,ametadata=print:key=lavfi.r128.M:file=-", "-f", "null", "-"])
        .stdin(Stdio::null())
        .output()?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() {
        return fail(format!("ffmpeg could not read {}: {}", input.display(), crate::util::truncate_chars(stderr.trim(), 300)));
    }
    let mut duration = 0.0;
    let mut silences = Vec::new();
    let mut open: Option<f64> = None;
    for line in stderr.lines() {
        if let Some(rest) = line.trim().strip_prefix("Duration:") {
            if let Some(d) = parse_time(rest.split(',').next().unwrap_or("")) {
                duration = d;
            }
        } else if let Some(i) = line.find("silence_start:") {
            open = line[i + 14..].split_whitespace().next().and_then(|v| v.parse().ok());
        } else if let Some(i) = line.find("silence_end:") {
            let end: Option<f64> = line[i + 12..].split_whitespace().next().and_then(|v| v.parse().ok());
            if let (Some(s), Some(e)) = (open.take(), end) {
                silences.push((s.max(0.0), e));
            }
        }
    }
    let mut loud = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        if let Some(v) = line.strip_prefix("lavfi.r128.M=") {
            loud.push(v.trim().parse::<f32>().unwrap_or(-70.0).max(-70.0));
        }
    }
    if loud.is_empty() {
        return fail("the file has no audio track to analyse; clips mode listens for the moments");
    }
    if duration <= 0.0 {
        duration = loud.len() as f64 * 0.1;
    }
    Ok(Audio { duration, loud, silences })
}

/* ------------------------------ understanding without words ------------------------------ */

/// Something happened here, judged by sound alone: a crowd, a caster, a reaction.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Event {
    /// Loudest instant.
    pub peak: f64,
    /// How far above the recording's normal level, in LU.
    pub over: f64,
    /// Where the build-up began (the level left its baseline).
    pub rise: f64,
    /// Where it calmed down again.
    pub settle: f64,
    /// Picture cuts in the 20 seconds after the peak: a replay or a new camera usually starts at the first.
    #[serde(default)]
    pub cuts_after: Vec<f64>,
}

/// Below this momentary loudness a stretch counts as silence (no microphone, muted
/// game, a pause screen), not as the recording's normal level.
const SILENT_LU: f64 = -60.0;

/// Find the moments where the sound says something happened. Works on footage
/// where nobody speaks (or everybody shouts): matches, gameplay, concerts.
#[allow(clippy::needless_range_loop)]
pub fn events(audio: &Audio, max: usize) -> Vec<Event> {
    let n = audio.loud.len();
    if n < 50 {
        return Vec::new();
    }
    // one-second moving average, so a single click is not an event
    let mut smooth = vec![0f64; n];
    let mut acc = 0f64;
    for i in 0..n {
        acc += audio.loud[i] as f64;
        if i >= 10 {
            acc -= audio.loud[i - 10] as f64;
        }
        smooth[i] = acc / (i.min(9) + 1) as f64;
    }
    // The normal level is measured only where there is sound. A stream with no
    // microphone is silent for long stretches; counting those would make the
    // silence "normal" and every audible second an event.
    let mut sorted: Vec<f64> = smooth.iter().copied().filter(|v| *v > SILENT_LU).collect();
    if sorted.len() < 50 {
        return Vec::new();
    }
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let m = sorted.len();
    let base = sorted[m / 2];
    let p90 = sorted[m * 9 / 10];
    // an event stands clearly above the ordinary: at least 2.5 LU, more on dynamic recordings
    let threshold = base + ((p90 - base) * 0.9).max(2.5);
    let mut peaks: Vec<(f64, usize)> = Vec::new();
    for i in 30..n - 1 {
        let lo = i.saturating_sub(30);
        let hi = (i + 30).min(n - 1);
        if smooth[i] >= threshold && (lo..=hi).all(|j| smooth[j] <= smooth[i]) {
            peaks.push((smooth[i], i));
        }
    }
    peaks.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut kept: Vec<(f64, usize)> = Vec::new();
    for p in peaks {
        // one event per twelve seconds: the loudest wins
        if kept.iter().all(|k| (k.1 as i64 - p.1 as i64).abs() > 120) {
            kept.push(p);
        }
        if kept.len() >= max {
            break;
        }
    }
    kept.sort_by_key(|k| k.1);
    kept.into_iter()
        .map(|(level, i)| {
            // walk back to where the level last sat at its baseline for a second
            let mut r = i;
            let mut calm = 0;
            while r > 0 && i - r < 300 {
                r -= 1;
                calm = if smooth[r] <= base + 0.8 { calm + 1 } else { 0 };
                if calm >= 10 {
                    r += 10;
                    break;
                }
            }
            let mut s = i;
            calm = 0;
            while s + 1 < n && s - i < 250 {
                s += 1;
                calm = if smooth[s] <= base + 1.2 { calm + 1 } else { 0 };
                if calm >= 15 {
                    s -= 15;
                    break;
                }
            }
            Event { peak: i as f64 / 10.0, over: ((level - base) * 10.0).round() / 10.0, rise: r as f64 / 10.0, settle: s as f64 / 10.0, cuts_after: Vec::new() }
        })
        .collect()
}

/// Times at which the picture cuts between `from` and `to` (decoded small, so it is cheap).
pub fn scene_cuts(ffmpeg: &Path, src: &Path, from: f64, to: f64) -> Vec<f64> {
    let out = Command::new(ffmpeg)
        .args(["-hide_banner", "-nostats", "-ss", &format!("{from:.2}"), "-t", &format!("{:.2}", (to - from).max(0.5)), "-an", "-i"])
        .arg(src)
        .args(["-vf", "scale=160:-2,select='gt(scene,0.32)',metadata=print:file=-", "-f", "null", "-"])
        .stdin(Stdio::null())
        .output();
    let Ok(out) = out else { return Vec::new() };
    let mut v: Vec<f64> = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        if let Some(i) = line.find("pts_time:") {
            if let Some(t) = line[i + 9..].split_whitespace().next().and_then(|x| x.parse::<f64>().ok()) {
                let t = ((from + t) * 10.0).round() / 10.0;
                if v.last().map(|l| t - l > 0.4).unwrap_or(true) {
                    v.push(t);
                }
            }
        }
    }
    v
}

/// A grid of frames between two times, for a person or a model to *see* what
/// happens. Returns the time of each frame, row by row.
pub fn storyboard(ffmpeg: &Path, src: &Path, from: f64, to: f64, cols: u32, rows: u32, out: &Path) -> Result<Vec<f64>> {
    let n = (cols * rows).max(1);
    let dur = (to - from).max(0.5);
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let r = Command::new(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-ss", &format!("{from:.2}"), "-t", &format!("{dur:.2}"), "-an", "-i"])
        .arg(src)
        .args(["-vf", &format!("fps={:.5},scale=384:-2,tile={cols}x{rows}:padding=4:margin=4", n as f64 / dur), "-frames:v", "1"])
        .arg(out)
        .stdin(Stdio::null())
        .output()?;
    if !r.status.success() || !out.exists() {
        return fail(format!("could not make the storyboard: {}", crate::util::truncate_chars(String::from_utf8_lossy(&r.stderr).trim(), 200)));
    }
    // the fps filter samples the middle of each interval
    Ok((0..n).map(|i| ((from + dur * (i as f64 + 0.5) / n as f64) * 10.0).round() / 10.0).collect())
}

/// A part of the frame as fractions of its size: x, y, width, height.
pub type Region = (f64, f64, f64, f64);

/// Several regions separated by ';' (one per team's score).
pub fn parse_regions(s: &str) -> Option<Vec<Region>> {
    let v: Vec<Region> = s.split(';').filter(|x| !x.trim().is_empty()).map(parse_region).collect::<Option<Vec<_>>>()?;
    (!v.is_empty()).then_some(v)
}

pub fn parse_region(s: &str) -> Option<Region> {
    let v: Vec<f64> = s.split(',').filter_map(|x| x.trim().parse().ok()).collect();
    (v.len() == 4 && v.iter().all(|x| (0.0..=1.0).contains(x)) && v[2] > 0.0 && v[3] > 0.0 && v[0] + v[2] <= 1.0001 && v[1] + v[3] <= 1.0001).then(|| (v[0], v[1], v[2], v[3]))
}

/// A small grey frame with its brightness and contrast taken out: what is left is the
/// shape (a digit), so a pause menu dimming the screen or a glow is not a change.
fn normalise(f: &[u8]) -> Vec<f32> {
    let n = f.len().max(1) as f32;
    let mean = f.iter().map(|v| *v as f32).sum::<f32>() / n;
    let sd = (f.iter().map(|v| (*v as f32 - mean).powi(2)).sum::<f32>() / n).sqrt();
    let sd = if sd < 1.0 { 1.0 } else { sd };
    f.iter().map(|v| (*v as f32 - mean) / sd).collect()
}

fn shape_diff(a: &[f32], b: &[f32]) -> f64 {
    a.iter().zip(b).map(|(x, y)| (x - y).abs() as f64).sum::<f64>() / a.len().max(1) as f64
}

fn mean_frame(frames: &[Vec<f32>]) -> Vec<f32> {
    let mut out = vec![0f32; frames.first().map(|f| f.len()).unwrap_or(0)];
    for f in frames {
        for (o, v) in out.iter_mut().zip(f) {
            *o += v / frames.len() as f32;
        }
    }
    out
}

/// Where a region that normally holds still changes and then holds still again: a
/// scoreboard taking a new score, a kill counter, a round number. `frames` are small
/// grey crops sampled `fps` times a second starting at `from`. Only the shape counts
/// (each crop is normalised), so a pause menu or a glow is not a change. Scoreboards
/// animate a new score for a few seconds, so the 2 s before a change are compared with
/// the 2 s that start 3 s after it, and both must have been calm: a flash or a replay
/// wipe that returns to the old picture is not a goal.
pub fn stable_changes(frames: &[Vec<u8>], fps: f64, from: f64) -> Vec<f64> {
    let z: Vec<Vec<f32>> = frames.iter().map(|f| normalise(f)).collect();
    let w = (2.0 * fps).round().max(2.0) as usize;
    let settle = (3.0 * fps).round().max(2.0) as usize;
    if z.len() < 2 * w + settle + 1 {
        return Vec::new();
    }
    let diffs: Vec<f64> = z.windows(2).map(|p| shape_diff(&p[0], &p[1])).collect();
    let mut sorted = diffs.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let noise = sorted[sorted.len() / 2];
    let threshold = (noise * 4.0).max(0.2);
    let calm = |m: &[f32], range: std::ops::Range<usize>| range.into_iter().all(|j| shape_diff(&z[j], m) <= 1.5 * threshold);
    let mut out: Vec<f64> = Vec::new();
    let mut k = w - 1;
    while k + 1 + settle + w <= z.len() {
        // frame k+1 is the first that looks different
        if diffs[k] < threshold {
            k += 1;
            continue;
        }
        let before = mean_frame(&z[k + 1 - w..k + 1]);
        let after = mean_frame(&z[k + 1 + settle..k + 1 + settle + w]);
        if shape_diff(&before, &after) >= threshold && calm(&before, k + 1 - w..k + 1) && calm(&after, k + 1 + settle..k + 1 + settle + w) {
            let t = ((from + (k + 1) as f64 / fps) * 10.0).round() / 10.0;
            if out.last().map(|l| t - l > 3.0).unwrap_or(true) {
                out.push(t);
            }
            k += 1 + settle;
        } else {
            k += 1;
        }
    }
    out
}

/// Changes in several regions at once (one per team's score). A change that shows up in
/// two regions within 1.5 s is the whole screen changing (a menu, a replay, the end of
/// the match), not a goal: those are returned apart. Returns (region index, time) pairs
/// in time order, and the times dropped as screen-wide.
pub fn score_changes(per_region: &[Vec<f64>]) -> (Vec<(usize, f64)>, Vec<f64>) {
    let mut kept = Vec::new();
    let mut together = Vec::new();
    for (i, list) in per_region.iter().enumerate() {
        for t in list {
            let elsewhere = per_region.iter().enumerate().any(|(j, other)| j != i && other.iter().any(|o| (o - t).abs() <= 1.5));
            if elsewhere {
                if !together.iter().any(|x: &f64| (x - t).abs() <= 1.5) {
                    together.push(*t);
                }
            } else {
                kept.push((i, *t));
            }
        }
    }
    kept.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    together.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    (kept, together)
}

/// Sample a region of the picture between two times and find where it changes for good.
pub fn region_changes(ffmpeg: &Path, src: &Path, from: f64, to: f64, r: Region) -> Result<Vec<f64>> {
    let fps = 2.0;
    let (gw, gh) = (48usize, 24usize);
    let out = Command::new(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-ss", &format!("{from:.2}"), "-t", &format!("{:.2}", (to - from).max(1.0)), "-an", "-i"])
        .arg(src)
        .args(["-vf", &format!("fps={fps},crop=iw*{:.4}:ih*{:.4}:iw*{:.4}:ih*{:.4},scale={gw}:{gh}:flags=area,format=gray", r.2, r.3, r.0, r.1), "-f", "rawvideo", "-"])
        .stdin(Stdio::null())
        .output()?;
    if !out.status.success() {
        return fail(format!("could not read the region: {}", crate::util::truncate_chars(String::from_utf8_lossy(&out.stderr).trim(), 200)));
    }
    let frames: Vec<Vec<u8>> = out.stdout.chunks_exact(gw * gh).map(|c| c.to_vec()).collect();
    Ok(stable_changes(&frames, fps, from))
}

/// One frame with the region drawn on it, so whoever chose the region can check it.
pub fn region_preview(ffmpeg: &Path, src: &Path, at: f64, regions: &[Region], out: &Path) -> Result<()> {
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let boxes: Vec<String> = regions.iter().map(|r| format!("drawbox=x=iw*{:.4}:y=ih*{:.4}:w=iw*{:.4}:h=ih*{:.4}:color=red@0.9:t=3", r.0, r.1, r.2, r.3)).collect();
    let ok = Command::new(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-ss", &format!("{at:.2}"), "-i"])
        .arg(src)
        .args(["-frames:v", "1", "-vf", &format!("{},scale=960:-2", boxes.join(","))])
        .arg(out)
        .stdin(Stdio::null())
        .status()?
        .success();
    if !ok {
        return fail("could not draw the region preview");
    }
    Ok(())
}

const HYPE: &[&str] = &[
    "wow",
    "omg",
    "no way",
    "insane",
    "crazy",
    "let's go",
    "lets go",
    "what",
    "holy",
    "unbelievable",
    "clip",
    "increible",
    "increíble",
    "no puede ser",
    "vamos",
    "brutal",
    "locura",
    "mira",
    "dios",
    "jajaja",
    "haha",
    "lol",
];

fn words_score(cues: &[Cue], start: f64, end: f64) -> (f64, String, Vec<String>) {
    let inside: Vec<&Cue> = cues.iter().filter(|c| c.end > start && c.start < end).collect();
    let text = inside.iter().map(|c| c.text.as_str()).collect::<Vec<_>>().join(" ");
    let low = text.to_lowercase();
    let mut score = 0.0;
    let mut why = Vec::new();
    let hits = HYPE.iter().filter(|k| low.contains(*k)).count();
    if hits > 0 {
        score += (hits as f64).min(4.0) * 0.5;
        why.push(format!("{hits} reaction word(s)"));
    }
    let marks = text.matches(['!', '?', '¡', '¿']).count();
    if marks > 0 {
        score += (marks as f64).min(6.0) * 0.15;
        why.push(format!("{marks} question/exclamation"));
    }
    let words = text.split_whitespace().count() as f64;
    let rate = words / (end - start).max(1.0);
    if rate > 2.6 {
        score += 0.6;
        why.push(format!("fast speech ({rate:.1} words/s)"));
    } else if words < 3.0 && !cues.is_empty() {
        score -= 1.0;
    }
    (score, text, why)
}

/// Pick the best non-overlapping moments.
pub fn plan(audio: &Audio, cues: &[Cue], o: &Options) -> Vec<Clip> {
    let n = audio.loud.len();
    if n == 0 {
        return Vec::new();
    }
    // prefix sums for O(1) window statistics
    let mut sum = vec![0f64; n + 1];
    let mut sq = vec![0f64; n + 1];
    for (i, v) in audio.loud.iter().enumerate() {
        sum[i + 1] = sum[i] + *v as f64;
        sq[i + 1] = sq[i] + (*v as f64) * (*v as f64);
    }
    let stats = |a: usize, b: usize| {
        let len = (b - a).max(1) as f64;
        let mean = (sum[b] - sum[a]) / len;
        let var = ((sq[b] - sq[a]) / len - mean * mean).max(0.0);
        (mean, var.sqrt())
    };
    let (global_mean, global_sd) = stats(0, n);
    let mut sorted: Vec<f32> = audio.loud.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let p90 = sorted[(n as f64 * 0.9) as usize % n] as f64;

    // candidate edges: where a silence ends (speech starts) and where one begins
    let mut starts: Vec<f64> = audio.silences.iter().map(|s| (s.1 - 0.12).max(0.0)).collect();
    let mut ends: Vec<f64> = audio.silences.iter().map(|s| (s.0 + 0.18).min(audio.duration)).collect();
    starts.insert(0, 0.0);
    ends.push(audio.duration);
    if starts.len() < 8 {
        // wall-to-wall sound (music, crowd): fall back to a regular grid
        let step = (o.min_s / 3.0).max(2.0);
        let mut t = 0.0;
        while t < audio.duration {
            starts.push(t);
            ends.push(t);
            t += step;
        }
        starts.sort_by(|a, b| a.partial_cmp(b).unwrap());
        ends.sort_by(|a, b| a.partial_cmp(b).unwrap());
    }
    let target = ((o.min_s + o.max_s) / 2.0).min(o.min_s * 2.2);

    let mut cands: Vec<Clip> = Vec::new();
    for &s in &starts {
        // the natural ending closest to the target length
        let Some(&e) = ends.iter().filter(|&&e| e - s >= o.min_s && e - s <= o.max_s).min_by(|a, b| ((**a - s) - target).abs().partial_cmp(&((**b - s) - target).abs()).unwrap())
        else {
            continue;
        };
        let (a, b) = (((s * 10.0) as usize).min(n - 1), ((e * 10.0) as usize).clamp(1, n));
        if b <= a {
            continue;
        }
        let (mean, sd) = stats(a, b);
        let peaks = audio.loud[a..b].iter().filter(|v| **v as f64 >= p90).count() as f64 / (b - a) as f64;
        let hook = stats(a, (a + 30).min(b)).0; // the first three seconds must not be dead air
        let mut score =
            (mean - global_mean) / global_sd.max(1.0) + (sd / global_sd.max(1.0)) * 0.5 + peaks * 3.0 + ((hook - global_mean) / global_sd.max(1.0)).clamp(-1.0, 1.0) * 0.6;
        let mut why = vec![format!("{:+.1} LU vs the recording", mean - global_mean)];
        if peaks > 0.2 {
            why.push(format!("{:.0}% of it in the loudest tenth", peaks * 100.0));
        }
        let (ws, text, wwhy) = words_score(cues, s, e);
        score += ws;
        why.extend(wwhy);
        cands.push(Clip {
            index: 0,
            start: (s * 100.0).round() / 100.0,
            end: (e * 100.0).round() / 100.0,
            duration: ((e - s) * 100.0).round() / 100.0,
            score: (score * 100.0).round() / 100.0,
            why,
            text,
            ..Default::default()
        });
    }
    cands.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    let mut picked: Vec<Clip> = Vec::new();
    for c in cands {
        if picked.len() >= o.count {
            break;
        }
        if picked.iter().all(|p| c.end <= p.start || c.start >= p.end) {
            picked.push(c);
        }
    }
    picked.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());
    for (i, c) in picked.iter_mut().enumerate() {
        c.index = i + 1;
    }
    picked
}

fn aspect_ratio(aspect: &str) -> Option<(u32, u32)> {
    let (a, b) = aspect.split_once(':')?;
    Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
}

fn export_one(ffmpeg: &Path, o: &Options, c: &Clip, out: &Path, srt: Option<&Path>) -> Result<()> {
    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-hide_banner", "-loglevel", "error", "-y", "-ss", &format!("{:.3}", c.start), "-i"]).arg(&o.input).args(["-t", &format!("{:.3}", c.duration)]);
    if o.copy {
        cmd.args(["-c", "copy", "-avoid_negative_ts", "make_zero"]);
    } else {
        let mut vf: Vec<String> = Vec::new();
        if let Some((aw, ah)) = aspect_ratio(&o.aspect) {
            let h = o.height - o.height % 2;
            let w = (h as u64 * aw as u64 / ah as u64) as u32;
            let w = w - w % 2;
            if o.layout == "fit" {
                // the whole picture, centred, over a blurred and darkened copy that fills the frame
                vf.push(format!("split[a][b];[a]scale={w}:{h}:force_original_aspect_ratio=increase,crop={w}:{h},boxblur=24:4,eq=brightness=-0.12[bg];[b]scale={w}:-2[fg];[bg][fg]overlay=0:(H-h)/2,setsar=1"));
            } else {
                // fill the frame: scale until both sides cover it, then crop the centre
                vf.push(format!("scale={w}:{h}:force_original_aspect_ratio=increase,crop={w}:{h},setsar=1"));
            }
        }
        if let (true, Some(s)) = (o.burn, srt) {
            // the filter parser dislikes drive letters; run next to the file instead
            let name = s.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let style = o.caption_style.clone().unwrap_or_else(|| "Fontsize=13,Outline=1.2,MarginV=60,Alignment=2".into());
            vf.push(format!("subtitles={name}:force_style='{style}'"));
            if let Some(dir) = s.parent() {
                cmd.current_dir(dir);
            }
        }
        if !vf.is_empty() {
            cmd.args(["-vf", &vf.join(",")]);
        }
        cmd.args(["-c:v", "libx264", "-preset", "veryfast", "-crf", "20", "-pix_fmt", "yuv420p", "-c:a", "aac", "-b:a", "160k", "-movflags", "+faststart"]);
    }
    let abs_out = std::fs::canonicalize(out.parent().unwrap_or(Path::new(".")))
        .unwrap_or_else(|_| out.parent().unwrap_or(Path::new(".")).to_path_buf())
        .join(out.file_name().unwrap_or_default());
    let r = cmd.arg(&abs_out).stdin(Stdio::null()).output()?;
    if !r.status.success() {
        return fail(format!("ffmpeg failed on clip {}: {}", c.index, crate::util::truncate_chars(String::from_utf8_lossy(&r.stderr).trim(), 300)));
    }
    Ok(())
}

fn edl(clips: &[Clip], fps: f64, reel: &str) -> String {
    let tc = |t: f64| {
        let f = (t * fps).round() as u64;
        let r = fps.round() as u64;
        format!("{:02}:{:02}:{:02}:{:02}", f / (r * 3600), f / (r * 60) % 60, f / r % 60, f % r)
    };
    let mut s = String::from("TITLE: editemos clips\nFCM: NON-DROP FRAME\n\n");
    let mut rec = 0.0;
    for c in clips {
        s.push_str(&format!(
            "{:03}  AX       AA/V  C        {} {} {} {}\n* FROM CLIP NAME: {reel}\n* SCORE {} {}\n\n",
            c.index,
            tc(c.start),
            tc(c.end),
            tc(rec),
            tc(rec + c.duration),
            c.score,
            c.why.join("; ")
        ));
        rec += c.duration;
    }
    s
}

/// Analyse, choose and export.
pub fn run(o: &Options) -> Result<Manifest> {
    let Some(ffmpeg) = find_ffmpeg() else { return fail("ffmpeg not found (set EDITEMOS_FFMPEG)") };
    if !o.input.exists() {
        return fail(format!("{} not found", o.input.display()));
    }
    if o.min_s <= 0.0 || o.max_s < o.min_s {
        return fail("clip length: need 0 < min <= max");
    }
    std::fs::create_dir_all(&o.out_dir)?;
    let t0 = Instant::now();
    let audio = analyse_audio(&ffmpeg, &o.input)?;
    let cues = match &o.transcript {
        Some(t) => dedupe_rolling(parse_transcript(&std::fs::read_to_string(t)?)),
        None => Vec::new(),
    };
    let mut clips = plan(&audio, &cues, o);
    let analysis_seconds = t0.elapsed().as_secs_f64();
    if clips.is_empty() {
        return fail(format!("no moment between {} and {} seconds stood out; try a wider range", o.min_s, o.max_s));
    }

    let stem = o.input.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "clip".into());
    let t1 = Instant::now();
    if !o.plan_only {
        for c in clips.iter_mut() {
            let base = o.out_dir.join(format!("{stem}-{:02}-{}s", c.index, c.start.round() as u64));
            if !cues.is_empty() {
                let srt = base.with_extension("srt");
                write_atomic(&srt, slice_srt(&cues, c.start, c.end).as_bytes())?;
                c.subtitles = Some(path_str(&srt));
            }
            c.file = Some(path_str(&base.with_extension("mp4")));
        }
        let next = std::sync::atomic::AtomicUsize::new(0);
        let errors: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
        std::thread::scope(|s| {
            for _ in 0..o.jobs.clamp(1, 6).min(clips.len()) {
                s.spawn(|| loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(c) = clips.get(i) else { break };
                    let out = PathBuf::from(c.file.clone().unwrap_or_default());
                    let srt = c.subtitles.as_ref().map(PathBuf::from);
                    if let Err(e) = export_one(&ffmpeg, o, c, &out, srt.as_deref()) {
                        errors.lock().unwrap().push(e.0);
                    }
                });
            }
        });
        let errors = errors.into_inner().unwrap();
        if !errors.is_empty() {
            return fail(errors.join(" | "));
        }
    }
    let export_seconds = t1.elapsed().as_secs_f64();

    let manifest_path = o.out_dir.join(format!("{stem}.clips.json"));
    let edl_path = o.out_dir.join(format!("{stem}.clips.edl"));
    write_atomic(&edl_path, edl(&clips, 30.0, &stem).as_bytes())?;
    let m = Manifest {
        input: path_str(&o.input),
        duration: audio.duration,
        aspect: o.aspect.clone(),
        clips,
        analysis_seconds: (analysis_seconds * 100.0).round() / 100.0,
        export_seconds: (export_seconds * 100.0).round() / 100.0,
        analysis_speed: (audio.duration / analysis_seconds.max(0.001)).round(),
        manifest: path_str(&manifest_path),
        edl: path_str(&edl_path),
    };
    write_atomic(&manifest_path, serde_json::to_string_pretty(&m)?.as_bytes())?;
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_srt_and_vtt() {
        let srt = "1\n00:00:01,000 --> 00:00:03,500\nHola <i>mundo</i>\n\n2\n00:01:00,000 --> 00:01:02,000\nNo puede ser!\n";
        let c = parse_transcript(srt);
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].text, "Hola mundo");
        assert!((c[1].start - 60.0).abs() < 1e-6);
        let vtt = "WEBVTT\n\n00:02.000 --> 00:04.000 align:start\nwow\n";
        assert_eq!(parse_transcript(vtt)[0].end, 4.0);
        let cut = slice_srt(&c, 59.0, 70.0);
        assert!(cut.contains("00:00:01,000 --> 00:00:03,000") && cut.contains("No puede ser"));
    }

    #[test]
    fn a_score_change_is_found_and_a_flash_is_not() {
        // two "digits": bright left half, and bright top half; `gain` dims or brightens
        let zero = |gain: f32| (0..48 * 24).map(|i| if i % 48 < 24 { (200.0 * gain) as u8 } else { (40.0 * gain) as u8 }).collect::<Vec<u8>>();
        let one = |gain: f32| (0..48 * 24).map(|i| if i / 48 < 12 { (200.0 * gain) as u8 } else { (40.0 * gain) as u8 }).collect::<Vec<u8>>();
        let mut frames: Vec<Vec<u8>> = Vec::new();
        // "0", a one-sample flash, a pause menu dimming everything for 4 s, "0" again,
        // then "1" that pops and glows for a second before it settles
        frames.extend((0..20).map(|_| zero(1.0)));
        frames.push(one(1.25));
        frames.extend((0..8).map(|_| zero(0.3)));
        frames.extend((0..10).map(|_| zero(1.0)));
        frames.push(zero(1.2));
        frames.push(one(1.2));
        frames.push(one(0.8));
        frames.extend((0..14).map(|_| one(1.0)));
        let ch = stable_changes(&frames, 2.0, 100.0);
        assert_eq!(ch, vec![120.0], "only the change of digit: {ch:?}");
        assert!(stable_changes(&(0..30).map(|_| zero(1.0)).collect::<Vec<_>>(), 2.0, 0.0).is_empty());
        // both teams' boxes changing together is the screen, not a goal
        let (kept, together) = score_changes(&[vec![120.0, 300.0], vec![200.0, 300.8]]);
        assert_eq!(kept, vec![(0, 120.0), (1, 200.0)]);
        assert_eq!(together.len(), 1);
        assert_eq!(parse_region("0.45,0,0.1,0.05"), Some((0.45, 0.0, 0.1, 0.05)));
        assert_eq!(parse_region("0.95,0,0.1,0.05"), None, "outside the frame");
    }

    #[test]
    fn silence_is_not_the_normal_level() {
        // a stream with no microphone: 60 s of nothing, 2 min of steady game sound
        // with one loud moment at 1:30, then silence again
        let mut loud = vec![-70f32; 2400];
        for v in loud.iter_mut().take(1800).skip(600) {
            *v = -24.0;
        }
        for v in loud.iter_mut().take(910).skip(890) {
            *v = -12.0;
        }
        let audio = Audio { duration: 240.0, loud, silences: vec![(0.0, 60.0), (180.0, 240.0)] };
        let ev = events(&audio, 10);
        assert_eq!(ev.len(), 1, "only the loud moment, not every second of game sound: {ev:?}");
        assert!((ev[0].peak - 90.0).abs() < 1.5 && ev[0].over > 8.0 && ev[0].over < 14.0);
        // nothing but silence: nothing happened
        assert!(events(&Audio { duration: 120.0, loud: vec![-70.0; 1200], silences: vec![] }, 10).is_empty());
    }

    #[test]
    fn picks_the_loud_part_and_snaps_to_silence() {
        // 5 minutes of quiet talk with one loud 20 s stretch at 2:00, silences every 10 s
        let mut loud = vec![-30f32; 3000];
        for v in loud.iter_mut().take(1400).skip(1200) {
            *v = -12.0;
        }
        let silences: Vec<(f64, f64)> = (1..30).map(|i| (i as f64 * 10.0 - 0.4, i as f64 * 10.0)).collect();
        let audio = Audio { duration: 300.0, loud, silences };
        let o = Options { count: 2, min_s: 15.0, max_s: 40.0, ..Default::default() };
        let clips = plan(&audio, &[], &o);
        assert_eq!(clips.len(), 2);
        let best = clips.iter().max_by(|a, b| a.score.partial_cmp(&b.score).unwrap()).unwrap();
        assert!(best.start <= 121.0 && best.end >= 139.0, "the loud stretch is inside the best clip: {best:?}");
        assert!(clips[0].end <= clips[1].start, "clips do not overlap");
        // every edge sits on a silence edge (10 s grid), not mid-sentence
        assert!(((best.start + 0.12) % 10.0) < 0.5 || best.start == 0.0, "start {} is not on a pause", best.start);
    }

    #[test]
    fn rolling_captions_say_each_word_once() {
        let srt = "1\n00:00:00,000 --> 00:00:01,149\n>> Oh no. Are you good?\n\n2\n00:00:01,149 --> 00:00:01,159\nOh no. Are you good?\n\n3\n00:00:01,159 --> 00:00:02,350\nOh no. Are you good? >> It's all good. The\n\n4\n00:00:02,350 --> 00:00:02,360\nIt's all good. The\n\n5\n00:00:02,360 --> 00:00:03,500\nIt's all good. The game is back\n";
        let c = dedupe_rolling(parse_transcript(srt));
        let all = c.iter().map(|x| x.text.as_str()).collect::<Vec<_>>().join(" | ");
        assert_eq!(all, "Oh no. Are you good? | It's all good. The | game is back");
        // ordinary subtitles pass through untouched
        let plain = dedupe_rolling(parse_transcript("1\n00:00:01,000 --> 00:00:02,000\nHola\n\n2\n00:00:02,000 --> 00:00:03,000\nmundo\n"));
        assert_eq!(plain.len(), 2);
    }

    #[test]
    fn sound_alone_finds_what_happened() {
        // five minutes of steady noise, with a build-up to a roar at 2:00 and a shorter one at 4:00
        let mut loud = vec![-28f32; 3000];
        for (i, v) in loud.iter_mut().enumerate() {
            let t = i as f32 / 10.0;
            if (112.0..120.0).contains(&t) {
                *v = -28.0 + (t - 112.0) * 1.5; // climbing
            } else if (120.0..126.0).contains(&t) {
                *v = -12.0; // the roar
            } else if (240.0..243.0).contains(&t) {
                *v = -16.0;
            }
        }
        let ev = events(&Audio { duration: 300.0, loud, silences: Vec::new() }, 5);
        assert_eq!(ev.len(), 2, "{ev:?}");
        assert!((120.0..127.5).contains(&ev[0].peak), "peak at the roar: {}", ev[0].peak);
        assert!(ev[0].rise < 115.0 && ev[0].rise > 105.0, "the build-up is part of the story: rise {}", ev[0].rise);
        assert!(ev[0].settle >= 125.0 && ev[0].settle < 135.0, "settle {}", ev[0].settle);
        assert!(ev[0].over > 10.0 && ev[1].peak > 239.0);
        // a flat recording has no events
        assert!(events(&Audio { duration: 300.0, loud: vec![-25.0; 3000], silences: Vec::new() }, 5).is_empty());
    }

    #[test]
    fn words_raise_the_score() {
        let cues = vec![Cue { start: 10.0, end: 14.0, text: "wow no way!! what?".into() }];
        let (hot, _, why) = words_score(&cues, 5.0, 25.0);
        let (cold, _, _) = words_score(&cues, 100.0, 120.0);
        assert!(hot > cold && !why.is_empty());
    }
}
