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

fn parse_time(t: &str) -> Option<f64> {
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
    fn words_raise_the_score() {
        let cues = vec![Cue { start: 10.0, end: 14.0, text: "wow no way!! what?".into() }];
        let (hot, _, why) = words_score(&cues, 5.0, 25.0);
        let (cold, _, _) = words_score(&cues, 100.0, 120.0);
        assert!(hot > cold && !why.is_empty());
    }
}
