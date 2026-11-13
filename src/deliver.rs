//! Final masters for a platform, checked after encoding instead of assumed.
//!
//! A cut is built and reviewed at its working size (1080p60, say). Delivery is the
//! last step: scale once with lanczos, draw the subtitles at the output size (libass
//! draws at the resolution it is given, so they stay sharp), set the loudness if asked,
//! encode with settings the platform handles well, then read the result back with
//! ffprobe and compare it with what was asked for: size, frame rate, duration, audio.

use crate::error::{fail, Result};
use crate::util::{path_str, short_id};
use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

#[derive(Serialize, Clone, Copy, Debug)]
pub struct Preset {
    pub id: &'static str,
    pub width: u32,
    pub height: u32,
    pub crf: u32,
    /// Peak video bitrate in Mbit/s (the buffer is twice this).
    pub maxrate_mbps: u32,
    pub audio_kbps: u32,
    /// Where the mix should sit for this destination (LUFS integrated).
    pub loudness: f64,
    pub about: &'static str,
}

pub const PRESETS: &[Preset] = &[
    Preset {
        id: "youtube-4k",
        width: 3840,
        height: 2160,
        crf: 16,
        maxrate_mbps: 68,
        audio_kbps: 384,
        loudness: -16.0,
        about: "YouTube master: built at 1080p, scaled to 3840x2160 with lanczos, subtitles drawn at 4K (YouTube gives 4K uploads a better encode)",
    },
    Preset { id: "youtube-1080", width: 1920, height: 1080, crf: 17, maxrate_mbps: 24, audio_kbps: 384, loudness: -16.0, about: "YouTube or Vimeo at 1080p" },
    Preset { id: "vertical", width: 1080, height: 1920, crf: 18, maxrate_mbps: 20, audio_kbps: 320, loudness: -14.0, about: "Shorts, Reels and TikTok at 1080x1920" },
];

const SPEEDS: &[&str] = &["ultrafast", "superfast", "veryfast", "faster", "fast", "medium", "slow", "slower", "veryslow"];
/// -1.5 dBFS as a linear sample limit.
const PEAK_LIMIT: f64 = 0.841;

pub fn preset(id: &str) -> Option<Preset> {
    PRESETS.iter().copied().find(|p| p.id == id)
}

pub struct Options {
    pub input: PathBuf,
    pub out: PathBuf,
    pub preset: Preset,
    /// A .srt or .ass file, drawn after scaling.
    pub subtitles: Option<PathBuf>,
    /// Bring the mix to this many LUFS (gain from a measurement, then a limiter at -1.5 dBFS).
    pub loudness: Option<f64>,
    /// Output frame rate; the source's when absent. Frames are repeated or dropped, never invented.
    pub fps: Option<f64>,
    /// x264 preset.
    pub speed: String,
}

#[derive(Serialize, Debug, Default, Clone)]
pub struct Probe {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub fps_rational: String,
    pub duration: f64,
    pub video_codec: String,
    pub pix_fmt: String,
    pub audio: bool,
    pub audio_codec: Option<String>,
    pub sample_rate: Option<u32>,
}

#[derive(Serialize, Debug, Default)]
pub struct Check {
    pub ok: bool,
    pub checked_with: String,
    pub probe: Option<Probe>,
    pub problems: Vec<String>,
}

#[derive(Serialize, Debug, Default)]
pub struct Loudness {
    pub target: f64,
    pub applied_gain_db: Option<f64>,
    pub before_lufs: Option<f64>,
    pub after_lufs: Option<f64>,
    pub true_peak_dbtp: Option<f64>,
}

#[derive(Serialize, Debug)]
pub struct Delivered {
    pub file: String,
    pub preset: Preset,
    pub settings: Vec<String>,
    pub subtitles_burned: Option<String>,
    /// A copy of the .srt next to the master: burned captions cannot be edited.
    pub srt: Option<String>,
    pub loudness: Option<Loudness>,
    pub check: Check,
    pub seconds: f64,
    pub notes: Vec<String>,
}

/// ffprobe from the same folder as ffmpeg (builds ship them together), EDITEMOS_FFPROBE,
/// or the PATH.
pub fn find_ffprobe(ffmpeg: &Path) -> Option<PathBuf> {
    if let Ok(p) = std::env::var("EDITEMOS_FFPROBE") {
        if Path::new(&p).exists() {
            return Some(PathBuf::from(p));
        }
    }
    let name = if cfg!(windows) { "ffprobe.exe" } else { "ffprobe" };
    if let Some(dir) = ffmpeg.parent().filter(|d| !d.as_os_str().is_empty()) {
        let c = dir.join(name);
        if c.exists() {
            return Some(c);
        }
    }
    let ok = Command::new("ffprobe").arg("-version").stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false);
    ok.then(|| PathBuf::from("ffprobe"))
}

fn rational(s: &str) -> f64 {
    match s.split_once('/') {
        Some((a, b)) => {
            let (a, b) = (a.trim().parse::<f64>().unwrap_or(0.0), b.trim().parse::<f64>().unwrap_or(0.0));
            if b > 0.0 {
                a / b
            } else {
                0.0
            }
        }
        None => s.trim().parse().unwrap_or(0.0),
    }
}

/// Read a file's first video and audio streams, as ffprobe sees them.
pub fn probe(ffprobe: &Path, file: &Path) -> Result<Probe> {
    let out = Command::new(ffprobe)
        .args(["-v", "error", "-show_entries", "stream=codec_type,codec_name,width,height,r_frame_rate,pix_fmt,sample_rate:format=duration", "-of", "json"])
        .arg(file)
        .stdin(Stdio::null())
        .output()?;
    if !out.status.success() {
        return fail(format!("ffprobe could not read {}: {}", path_str(file), crate::util::truncate_chars(String::from_utf8_lossy(&out.stderr).trim(), 300)));
    }
    let v: Value = serde_json::from_slice(&out.stdout)?;
    let streams = v["streams"].as_array().cloned().unwrap_or_default();
    let video = streams.iter().find(|s| s["codec_type"] == "video");
    let audio = streams.iter().find(|s| s["codec_type"] == "audio");
    let Some(video) = video else { return fail(format!("{} has no video stream", path_str(file))) };
    let fps_rational = video["r_frame_rate"].as_str().unwrap_or("0/1").to_string();
    Ok(Probe {
        width: video["width"].as_u64().unwrap_or(0) as u32,
        height: video["height"].as_u64().unwrap_or(0) as u32,
        fps: rational(&fps_rational),
        fps_rational,
        duration: v["format"]["duration"].as_str().and_then(|d| d.parse().ok()).unwrap_or(0.0),
        video_codec: video["codec_name"].as_str().unwrap_or("").to_string(),
        pix_fmt: video["pix_fmt"].as_str().unwrap_or("").to_string(),
        audio: audio.is_some(),
        audio_codec: audio.and_then(|a| a["codec_name"].as_str()).map(String::from),
        sample_rate: audio.and_then(|a| a["sample_rate"].as_str()).and_then(|r| r.parse().ok()),
    })
}

/// Integrated loudness and true peak (EBU R128), read from ffmpeg's summary.
pub fn measure(ffmpeg: &Path, file: &Path) -> (Option<f64>, Option<f64>) {
    let out = Command::new(ffmpeg).args(["-hide_banner", "-nostats", "-i"]).arg(file).args(["-vn", "-af", "ebur128=peak=true", "-f", "null", "-"]).stdin(Stdio::null()).output();
    let Ok(out) = out else { return (None, None) };
    let text = String::from_utf8_lossy(&out.stderr);
    let summary = text.rsplit("Summary:").next().unwrap_or("");
    let grab = |key: &str| summary.lines().find_map(|l| l.trim().strip_prefix(key).and_then(|v| v.split_whitespace().next()).and_then(|v| v.parse::<f64>().ok()));
    let integrated = grab("I:").filter(|i| i.is_finite() && *i > -69.0);
    // the true-peak block prints "Peak:" after "True peak:"
    let peak = summary
        .split("True peak:")
        .nth(1)
        .and_then(|t| t.lines().find_map(|l| l.trim().strip_prefix("Peak:").and_then(|v| v.split_whitespace().next()).and_then(|v| v.parse::<f64>().ok())));
    (integrated, peak)
}

/// What the master must be, compared with what it is.
pub fn compare(got: &Probe, want_w: u32, want_h: u32, want_fps: f64, want_duration: f64, want_audio: bool) -> Vec<String> {
    let mut problems = Vec::new();
    if got.width != want_w || got.height != want_h {
        problems.push(format!("size is {}x{}, expected {want_w}x{want_h}", got.width, got.height));
    }
    if want_fps > 0.0 && (got.fps - want_fps).abs() > 0.01 {
        problems.push(format!("frame rate is {} ({:.3}), expected {:.3}", got.fps_rational, got.fps, want_fps));
    }
    let frame = if want_fps > 0.0 { 1.0 / want_fps } else { 0.04 };
    if want_duration > 0.0 && (got.duration - want_duration).abs() > frame * 2.0 + 0.05 {
        problems.push(format!("duration is {:.3} s, expected {:.3} s", got.duration, want_duration));
    }
    if want_audio && !got.audio {
        problems.push("no audio stream".into());
    }
    if got.pix_fmt != "yuv420p" {
        problems.push(format!("pixel format is {}, platforms expect yuv420p", got.pix_fmt));
    }
    problems
}

fn absolute(p: &Path) -> PathBuf {
    std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf())
}

pub fn run(ffmpeg: &Path, o: &Options) -> Result<Delivered> {
    let t0 = Instant::now();
    if !o.input.exists() {
        return fail(format!("no such file: {}", path_str(&o.input)));
    }
    let input = absolute(&o.input);
    let out = absolute(&o.out);
    if out == input {
        return fail("the master would overwrite its source; choose another out");
    }
    if !SPEEDS.contains(&o.speed.as_str()) {
        return fail(format!("speed must be one of {}", SPEEDS.join(", ")));
    }
    let out_dir = out.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
    std::fs::create_dir_all(&out_dir)?;
    let ffprobe = find_ffprobe(ffmpeg);
    let mut notes = Vec::new();

    let source = ffprobe.as_deref().and_then(|fp| probe(fp, &input).ok());
    let src_fps = source.as_ref().map(|s| s.fps).filter(|f| *f > 0.0);
    let fps = o.fps.filter(|f| *f > 0.0).or(src_fps).unwrap_or(30.0);
    let p = o.preset;

    // picture: one lanczos scale (letterboxed if the shape differs), then subtitles at that size
    let mut vf = vec![format!(
        "scale={w}:{h}:flags=lanczos:force_original_aspect_ratio=decrease:force_divisible_by=2:out_color_matrix=bt709,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2:color=black,setsar=1",
        w = p.width,
        h = p.height
    )];
    if let Some(s) = &source {
        let (sa, pa) = (s.width as f64 / s.height.max(1) as f64, p.width as f64 / p.height as f64);
        if (sa - pa).abs() > 0.01 {
            notes.push(format!("the source is {}x{}, a different shape from {}x{}: it was letterboxed, not cropped", s.width, s.height, p.width, p.height));
        }
        if s.width > p.width || s.height > p.height {
            notes.push("the source is larger than the preset: it was scaled down".into());
        }
    }
    let mut temp_subs: Option<PathBuf> = None;
    let mut srt_copy = None;
    if let Some(subs) = &o.subtitles {
        if !subs.exists() {
            return fail(format!("no such subtitle file: {}", path_str(subs)));
        }
        if !crate::story::has_filter(ffmpeg, "subtitles") {
            return fail("this ffmpeg has no libass (no subtitles filter): install an ffmpeg built with libass, or deliver without subtitles and hand over the .srt");
        }
        let ext = subs.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        if !["srt", "ass", "ssa", "vtt"].contains(&ext.as_str()) {
            return fail("subtitles must be .srt, .ass, .ssa or .vtt");
        }
        // the filter parser dislikes drive letters and quotes: copy next to the output under a plain name
        let tmp = out_dir.join(format!("editemos-subs-{}.{ext}", short_id()));
        std::fs::copy(subs, &tmp)?;
        vf.push(format!("subtitles={}", tmp.file_name().unwrap_or_default().to_string_lossy()));
        temp_subs = Some(tmp);
        if ext == "srt" {
            let copy = out.with_extension("srt");
            if absolute(subs) != copy {
                std::fs::copy(subs, &copy)?;
            }
            srt_copy = Some(path_str(&copy));
        }
    }

    // sound: measured first, so one encode sets the level
    let has_audio = source.as_ref().map(|s| s.audio).unwrap_or(true);
    let mut loud = None;
    let mut af: Option<String> = None;
    if has_audio {
        let (before, _) = measure(ffmpeg, &input);
        if let Some(target) = o.loudness {
            match before {
                Some(b) => {
                    let gain = target - b;
                    af = Some(format!("volume={gain:.2}dB,alimiter=limit={PEAK_LIMIT}:level=0"));
                    loud = Some(Loudness { target, applied_gain_db: Some((gain * 100.0).round() / 100.0), before_lufs: Some(b), ..Default::default() });
                }
                None => {
                    notes.push("the audio is silent or could not be measured: its level was left as it is".into());
                    loud = Some(Loudness { target, ..Default::default() });
                }
            }
        } else {
            loud = Some(Loudness { target: p.loudness, before_lufs: before, ..Default::default() });
        }
    }

    let gop = ((fps / 2.0).round() as u32).max(1);
    let maxrate = format!("{}M", p.maxrate_mbps);
    let bufsize = format!("{}M", p.maxrate_mbps * 2);
    let mut args: Vec<String> = vec!["-hide_banner".into(), "-loglevel".into(), "error".into(), "-y".into(), "-i".into(), path_str(&input), "-map".into(), "0:v:0".into()];
    if has_audio {
        args.extend(["-map".into(), "0:a:0?".into()]);
    }
    args.extend(["-vf".into(), vf.join(",")]);
    if o.fps.is_some() {
        args.extend(["-r".into(), crate::motion::render::fps_arg(fps)]);
    }
    let settings: Vec<String> = [
        "-c:v",
        "libx264",
        "-preset",
        &o.speed,
        "-profile:v",
        "high",
        "-crf",
        &p.crf.to_string(),
        "-maxrate",
        &maxrate,
        "-bufsize",
        &bufsize,
        "-g",
        &gop.to_string(),
        "-bf",
        "2",
        "-pix_fmt",
        "yuv420p",
        "-color_primaries",
        "bt709",
        "-color_trc",
        "bt709",
        "-colorspace",
        "bt709",
    ]
    .iter()
    .map(|x| x.to_string())
    .collect();
    args.extend(settings.iter().cloned());
    let mut audio_settings: Vec<String> = Vec::new();
    if has_audio {
        if let Some(f) = &af {
            args.extend(["-af".into(), f.clone()]);
        }
        audio_settings = ["-c:a", "aac", "-b:a", &format!("{}k", p.audio_kbps), "-ar", "48000"].iter().map(|x| x.to_string()).collect();
        args.extend(audio_settings.iter().cloned());
    }
    args.extend(["-movflags".into(), "+faststart".into(), path_str(&out)]);

    let r = Command::new(ffmpeg).args(&args).current_dir(&out_dir).stdin(Stdio::null()).output();
    if let Some(t) = &temp_subs {
        let _ = std::fs::remove_file(t);
    }
    let r = r?;
    if !r.status.success() {
        return fail(format!("ffmpeg could not encode the master: {}", crate::util::truncate_chars(String::from_utf8_lossy(&r.stderr).trim(), 600)));
    }

    if let Some(l) = loud.as_mut() {
        let (after, peak) = measure(ffmpeg, &out);
        l.after_lufs = after;
        l.true_peak_dbtp = peak;
        if let Some(a) = after {
            if (a - l.target).abs() > 1.5 {
                notes.push(format!("the master sits at {a:.1} LUFS; this destination expects about {:.0} (pass loudness={:.0} to set it)", l.target, l.target));
            }
        }
        if peak.map(|pk| pk > -1.0).unwrap_or(false) {
            notes.push(format!("true peak {:.1} dBTP: above -1 dBTP, the platform's encode may clip", peak.unwrap_or(0.0)));
        }
    }

    let check = match &ffprobe {
        Some(fp) => match probe(fp, &out) {
            Ok(got) => {
                let want_duration = source.as_ref().map(|s| s.duration).unwrap_or(0.0);
                let want_fps = o.fps.filter(|f| *f > 0.0).or(src_fps).unwrap_or(0.0);
                let problems = compare(&got, p.width, p.height, want_fps, want_duration, has_audio);
                Check { ok: problems.is_empty(), checked_with: path_str(fp), probe: Some(got), problems }
            }
            Err(e) => Check { ok: false, checked_with: path_str(fp), probe: None, problems: vec![e.to_string()] },
        },
        None => Check { ok: false, checked_with: String::new(), probe: None, problems: vec!["ffprobe not found (set EDITEMOS_FFPROBE): the master was NOT checked".into()] },
    };

    let mut all_settings = settings;
    all_settings.extend(audio_settings);
    Ok(Delivered {
        file: path_str(&out),
        preset: p,
        settings: all_settings,
        subtitles_burned: o.subtitles.as_ref().map(|s| path_str(s)),
        srt: srt_copy,
        loudness: loud,
        check,
        seconds: (t0.elapsed().as_secs_f64() * 10.0).round() / 10.0,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_are_sane() {
        for p in PRESETS {
            assert!(p.width % 2 == 0 && p.height % 2 == 0 && p.crf <= 23 && p.loudness < -10.0, "{}", p.id);
        }
        assert_eq!(preset("youtube-4k").map(|p| (p.width, p.height)), Some((3840, 2160)));
        assert!(preset("nope").is_none());
    }

    #[test]
    fn rationals_and_checks() {
        assert!((rational("60000/1001") - 59.94).abs() < 0.01);
        assert_eq!(rational("60/1"), 60.0);
        assert_eq!(rational("0/0"), 0.0);
        let good = Probe {
            width: 3840,
            height: 2160,
            fps: 60.0,
            fps_rational: "60/1".into(),
            duration: 168.0,
            video_codec: "h264".into(),
            pix_fmt: "yuv420p".into(),
            audio: true,
            ..Default::default()
        };
        assert!(compare(&good, 3840, 2160, 60.0, 168.0, true).is_empty());
        let bad = Probe { width: 1920, height: 1080, fps: 30.0, fps_rational: "30/1".into(), duration: 160.0, pix_fmt: "yuv444p".into(), audio: false, ..Default::default() };
        let p = compare(&bad, 3840, 2160, 60.0, 168.0, true);
        assert_eq!(p.len(), 5, "{p:?}");
    }
}
