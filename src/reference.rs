//! Reverse-engineer a reference edit. Hand it a video whose editing you want
//! (a trailer, a creator's short, a brand film) and it measures what can be
//! measured: where the picture cuts and how long shots last, the colour of
//! each shot, how bright, saturated and warm the grade is, how loud the mix is.
//! Every number says how it was obtained (`evidence`) and what could not be
//! measured is listed (`unknowns`) for whoever looks at the sheet: type, motion
//! curves, music tempo. Then `compare` measures one of our cuts the same way
//! and says where it matches the reference and where it does not, so imitation
//! is checked, not assumed.

use crate::clips;
use crate::color::{palette_of, Rgb};
use crate::error::{fail, Result};
use crate::util::path_str;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::{Command, Stdio};

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Shot {
    pub start: f64,
    pub end: f64,
    /// Main colours of the shot's middle frame, most common first.
    pub colors: Vec<String>,
    /// 0..1
    pub brightness: f64,
    pub saturation: f64,
    /// Mean red minus mean blue, -1..1: positive is warm.
    pub warmth: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Rhythm {
    pub shots: usize,
    pub cuts_per_minute: f64,
    pub median_shot: f64,
    pub mean_shot: f64,
    /// 10th and 90th percentile shot length: the fast and the slow end of the edit.
    pub p10_shot: f64,
    pub p90_shot: f64,
    /// How much shot length changes from one shot to the next (0 = metronome).
    pub variation: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Grade {
    pub brightness: f64,
    pub saturation: f64,
    pub warmth: f64,
    /// Spread of brightness inside the frame, 0..0.5: higher is punchier.
    pub contrast: f64,
    pub palette: Vec<String>,
    pub words: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Loudness {
    pub integrated_lufs: Option<f64>,
    pub range_lu: Option<f64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Reference {
    pub name: String,
    pub source: String,
    pub duration: f64,
    pub rhythm: Rhythm,
    pub grade: Grade,
    pub loudness: Loudness,
    pub shots: Vec<Shot>,
    /// One frame per shot (up to 24, spread over the film), for a person or a model to read.
    #[serde(default)]
    pub sheet: Option<String>,
    pub evidence: Vec<String>,
    pub unknowns: Vec<String>,
}

/// Small frames of the whole video in one decoding pass: (time, rgb pixels).
const PW: usize = 64;
const PH: usize = 36;

fn sample_frames(ffmpeg: &Path, src: &Path, duration: f64) -> Result<(f64, Vec<Vec<u8>>)> {
    // about two frames a second, fewer on long films so memory stays small
    let fps = (2.0f64).min(2400.0 / duration.max(1.0)).max(0.05);
    let out = Command::new(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(src)
        .args(["-an", "-vf", &format!("fps={fps:.5},scale={PW}:{PH}:flags=area,format=rgb24"), "-f", "rawvideo", "-"])
        .stdin(Stdio::null())
        .output()?;
    if !out.status.success() {
        return fail(format!("could not decode the video: {}", crate::util::truncate_chars(String::from_utf8_lossy(&out.stderr).trim(), 300)));
    }
    let size = PW * PH * 3;
    Ok((fps, out.stdout.chunks_exact(size).map(|c| c.to_vec()).collect()))
}

/// Brightness, saturation, warmth and in-frame contrast of one small frame.
pub fn frame_stats(px: &[u8]) -> (f64, f64, f64, f64) {
    let n = (px.len() / 3).max(1) as f64;
    let (mut l, mut s, mut w) = (0.0, 0.0, 0.0);
    let mut ls = Vec::with_capacity(px.len() / 3);
    for p in px.chunks_exact(3) {
        let (r, g, b) = (p[0] as f64 / 255.0, p[1] as f64 / 255.0, p[2] as f64 / 255.0);
        let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        let (mx, mn) = (r.max(g).max(b), r.min(g).min(b));
        l += y;
        s += if mx > 0.0 { (mx - mn) / mx } else { 0.0 };
        w += r - b;
        ls.push(y);
    }
    let mean = l / n;
    let sd = (ls.iter().map(|y| (y - mean) * (y - mean)).sum::<f64>() / n).sqrt();
    (mean, s / n, w / n, sd)
}

fn round(v: f64, d: i32) -> f64 {
    let k = 10f64.powi(d);
    (v * k).round() / k
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let i = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[i.min(sorted.len() - 1)]
}

/// Shot lengths in, the edit's rhythm out.
pub fn rhythm(bounds: &[(f64, f64)], duration: f64) -> Rhythm {
    let lens: Vec<f64> = bounds.iter().map(|(a, b)| b - a).filter(|l| *l > 0.0).collect();
    if lens.is_empty() {
        return Rhythm::default();
    }
    let mut sorted = lens.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mean = lens.iter().sum::<f64>() / lens.len() as f64;
    let variation = if lens.len() > 1 { lens.windows(2).map(|w| (w[1].ln() - w[0].ln()).abs()).sum::<f64>() / (lens.len() - 1) as f64 } else { 0.0 };
    Rhythm {
        shots: lens.len(),
        cuts_per_minute: round((lens.len() as f64 - 1.0).max(0.0) / duration.max(0.1) * 60.0, 1),
        median_shot: round(percentile(&sorted, 0.5), 2),
        mean_shot: round(mean, 2),
        p10_shot: round(percentile(&sorted, 0.1), 2),
        p90_shot: round(percentile(&sorted, 0.9), 2),
        variation: round(variation, 2),
    }
}

fn grade_words(g: &Grade) -> Vec<String> {
    let mut w = Vec::new();
    w.push(
        if g.brightness < 0.28 {
            "dark, low-key"
        } else if g.brightness > 0.55 {
            "bright, high-key"
        } else {
            "mid-key"
        }
        .to_string(),
    );
    w.push(
        if g.saturation < 0.18 {
            "desaturated"
        } else if g.saturation > 0.45 {
            "saturated"
        } else {
            "natural saturation"
        }
        .to_string(),
    );
    w.push(
        if g.warmth > 0.06 {
            "warm"
        } else if g.warmth < -0.04 {
            "cool"
        } else {
            "neutral white balance"
        }
        .to_string(),
    );
    w.push(
        if g.contrast > 0.24 {
            "high contrast"
        } else if g.contrast < 0.13 {
            "flat, low contrast"
        } else {
            "moderate contrast"
        }
        .to_string(),
    );
    w
}

fn tempo_word(r: &Rhythm) -> &'static str {
    if r.median_shot <= 0.0 {
        "unknown"
    } else if r.median_shot < 1.6 {
        "punchy"
    } else if r.median_shot < 3.5 {
        "steady"
    } else {
        "calm"
    }
}

fn loudness(ffmpeg: &Path, src: &Path) -> Loudness {
    let out = Command::new(ffmpeg).args(["-hide_banner", "-nostats", "-i"]).arg(src).args(["-vn", "-af", "ebur128", "-f", "null", "-"]).stdin(Stdio::null()).output();
    let Ok(out) = out else { return Loudness::default() };
    let text = String::from_utf8_lossy(&out.stderr);
    // the summary is printed last
    let summary = text.rsplit("Summary:").next().unwrap_or("");
    let grab = |key: &str| summary.lines().find_map(|l| l.trim().strip_prefix(key).and_then(|v| v.split_whitespace().next()).and_then(|v| v.parse::<f64>().ok()));
    Loudness { integrated_lufs: grab("I:"), range_lu: grab("LRA:") }
}

fn duration_of(ffmpeg: &Path, src: &Path) -> f64 {
    let out = Command::new(ffmpeg).args(["-hide_banner", "-i"]).arg(src).stdin(Stdio::null()).output();
    let Ok(out) = out else { return 0.0 };
    String::from_utf8_lossy(&out.stderr).lines().find_map(|l| l.trim().strip_prefix("Duration:").and_then(|r| r.split(',').next()).and_then(clips::parse_time)).unwrap_or(0.0)
}

fn shot_sheet(ffmpeg: &Path, src: &Path, shots: &[Shot], out: &Path) -> Result<()> {
    let n = shots.len().min(24);
    if n == 0 {
        return fail("no shots");
    }
    let picks: Vec<&Shot> = (0..n).map(|j| &shots[j * shots.len() / n]).collect();
    let (cw, ch) = (320u32, 180u32);
    let cols = n.min(6) as u32;
    let rows = n.div_ceil(6) as u32;
    let mut sheet = image::RgbImage::from_pixel(cols * (cw + 6) + 6, rows * (ch + 6) + 6, image::Rgb([24, 24, 24]));
    let tmp = std::env::temp_dir().join(format!("editemos-ref-{}.png", crate::util::short_id()));
    for (j, s) in picks.iter().enumerate() {
        let t = (s.start + s.end) / 2.0;
        let ok = Command::new(ffmpeg)
            .args(["-hide_banner", "-loglevel", "error", "-y", "-ss", &format!("{t:.3}"), "-i"])
            .arg(src)
            .args(["-frames:v", "1", "-vf", &format!("scale={cw}:{ch}:force_original_aspect_ratio=decrease,pad={cw}:{ch}:(ow-iw)/2:(oh-ih)/2")])
            .arg(&tmp)
            .stdin(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            continue;
        }
        if let Ok(img) = image::open(&tmp) {
            let (x, y) = ((j as u32 % cols) * (cw + 6) + 6, (j as u32 / cols) * (ch + 6) + 6);
            image::imageops::overlay(&mut sheet, &img.to_rgb8(), x as i64, y as i64);
        }
    }
    let _ = std::fs::remove_file(&tmp);
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    sheet.save(out).map_err(|e| crate::error::Error(format!("could not save the sheet: {e}")))?;
    Ok(())
}

/// Measure a video. `sheet` = where to draw one frame per shot (None: skip it).
pub fn analyse(ffmpeg: &Path, src: &Path, name: &str, sheet: Option<&Path>) -> Result<Reference> {
    let duration = duration_of(ffmpeg, src);
    if duration <= 0.0 {
        return fail(format!("could not read the length of {}", src.display()));
    }
    let cuts: Vec<f64> = clips::scene_cuts(ffmpeg, src, 0.0, duration).into_iter().filter(|c| *c > 0.05 && *c < duration - 0.05).collect();
    let mut bounds: Vec<(f64, f64)> = Vec::new();
    let mut prev = 0.0;
    for c in cuts.iter().chain(std::iter::once(&duration)) {
        bounds.push((prev, *c));
        prev = *c;
    }
    let (fps, frames) = sample_frames(ffmpeg, src, duration)?;
    let at = |t: f64| -> Option<&Vec<u8>> { frames.get(((t * fps).floor() as usize).min(frames.len().saturating_sub(1))) };
    let mut shots = Vec::new();
    let (mut sum_b, mut sum_s, mut sum_w, mut sum_c, mut weight) = (0.0, 0.0, 0.0, 0.0, 0.0);
    let mut all_px: Vec<u8> = Vec::new();
    for (a, b) in &bounds {
        let mid = (a + b) / 2.0;
        let Some(px) = at(mid) else { continue };
        let (br, sa, wa, co) = frame_stats(px);
        let img = image::DynamicImage::ImageRgb8(image::RgbImage::from_raw(PW as u32, PH as u32, px.clone()).unwrap_or_else(|| image::RgbImage::new(1, 1)));
        let colors: Vec<String> = palette_of(&img, 4).into_iter().filter(|(_, share)| *share > 0.08).map(|(c, _)| c.hex()).collect();
        let len = b - a;
        sum_b += br * len;
        sum_s += sa * len;
        sum_w += wa * len;
        sum_c += co * len;
        weight += len;
        all_px.extend_from_slice(px);
        shots.push(Shot { start: round(*a, 2), end: round(*b, 2), colors, brightness: round(br, 3), saturation: round(sa, 3), warmth: round(wa, 3) });
    }
    let w = weight.max(1e-6);
    // one palette for the whole film: an even sample of every shot's pixels, at most 96x96
    let total_px = all_px.len() / 3;
    let stride = (total_px / (96 * 96)).max(1);
    let picked: Vec<u8> = all_px.chunks_exact(3).step_by(stride).take(96 * 96).flatten().copied().collect();
    let rows = (picked.len() / 3 / 96).max(1) as u32;
    let picked = picked[..(rows as usize * 96 * 3).min(picked.len())].to_vec();
    let palette_img = image::DynamicImage::ImageRgb8(image::RgbImage::from_raw(96, rows, picked).unwrap_or_else(|| image::RgbImage::new(1, 1)));
    let mut grade = Grade {
        brightness: round(sum_b / w, 3),
        saturation: round(sum_s / w, 3),
        warmth: round(sum_w / w, 3),
        contrast: round(sum_c / w, 3),
        palette: palette_of(&palette_img, 6).into_iter().filter(|(_, share)| *share > 0.04).map(|(c, _)| c.hex()).collect(),
        words: Vec::new(),
    };
    grade.words = grade_words(&grade);
    let rhythm = rhythm(&bounds, duration);
    let loud = loudness(ffmpeg, src);
    let mut sheet_path = None;
    if let Some(p) = sheet {
        if shot_sheet(ffmpeg, src, &shots, p).is_ok() {
            sheet_path = Some(path_str(p));
        }
    }
    let evidence = vec![
        format!(
            "{} shots from {} picture cuts, found where consecutive frames differ by more than a scene change (ffmpeg scene score > 0.32 on 160 px frames)",
            rhythm.shots,
            cuts.len()
        ),
        format!(
            "shot lengths: median {:.2} s, 10% of shots under {:.2} s, 10% over {:.2} s; {} cuts a minute -> tempo \"{}\"",
            rhythm.median_shot,
            rhythm.p10_shot,
            rhythm.p90_shot,
            rhythm.cuts_per_minute,
            tempo_word(&rhythm)
        ),
        format!("grade from the middle frame of every shot, weighted by shot length ({} frames sampled at {:.2} fps): {}", frames.len(), fps, grade.words.join(", ")),
        match loud.integrated_lufs {
            Some(i) => format!("loudness {i:.1} LUFS integrated, range {:.1} LU (EBU R128 over the whole file)", loud.range_lu.unwrap_or(0.0)),
            None => "no audio track, or it could not be measured".into(),
        },
    ];
    let unknowns = vec![
        "dissolves, wipes and whip-pans: only hard cuts are detected, so a soft transition can hide inside a long shot or show up as two cuts".into(),
        "typefaces, titles and on-screen graphics: read them from the sheet".into(),
        "motion curves of graphics and camera moves: not measured; look at the footage around the cuts".into(),
        "music tempo and whether cuts land on the beat: not measured".into(),
        "why a cut happens (action, reaction, line of dialogue): only someone watching can say".into(),
    ];
    Ok(Reference { name: name.into(), source: path_str(src), duration: round(duration, 2), rhythm, grade, loudness: loud, shots, sheet: sheet_path, evidence, unknowns })
}

#[derive(Serialize, Debug, Default)]
pub struct Comparison {
    pub reference: String,
    pub candidate: String,
    pub matches: Vec<String>,
    pub differs: Vec<String>,
    /// 0..1, the share of the measured traits that match.
    pub score: f64,
}

fn palette_distance(a: &[String], b: &[String]) -> Option<f64> {
    let pa: Vec<Rgb> = a.iter().filter_map(|h| crate::color::parse_color(h)).collect();
    let pb: Vec<Rgb> = b.iter().filter_map(|h| crate::color::parse_color(h)).collect();
    if pa.is_empty() || pb.is_empty() {
        return None;
    }
    // each colour of the reference to its nearest in the candidate
    Some(pa.iter().map(|x| pb.iter().map(|y| x.dist(*y)).fold(f64::MAX, f64::min)).sum::<f64>() / pa.len() as f64)
}

/// Where a cut matches the reference it was meant to imitate, trait by trait.
pub fn compare(r: &Reference, c: &Reference) -> Comparison {
    let mut out = Comparison { reference: r.name.clone(), candidate: c.source.clone(), ..Default::default() };
    let mut check = |ok: bool, good: String, bad: String| if ok { out.matches.push(good) } else { out.differs.push(bad) };
    let (rm, cm) = (r.rhythm.median_shot, c.rhythm.median_shot);
    if rm > 0.0 && cm > 0.0 {
        let ratio = cm / rm;
        check(
            (0.75..=1.33).contains(&ratio),
            format!("shot length: median {cm:.2} s against {rm:.2} s"),
            format!(
                "shot length: median {cm:.2} s against {rm:.2} s in the reference ({}); {}",
                if ratio > 1.0 { "slower" } else { "faster" },
                if ratio > 1.0 { "cut sooner" } else { "hold shots longer" }
            ),
        );
        let (rv, cv) = (r.rhythm.variation, c.rhythm.variation);
        check(
            (cv - rv).abs() <= 0.35,
            format!("rhythm variation {cv:.2} against {rv:.2}"),
            format!("rhythm variation {cv:.2} against {rv:.2}: {}", if cv < rv { "too even; mix short and long shots" } else { "more erratic than the reference" }),
        );
    }
    let (rg, cg) = (&r.grade, &c.grade);
    check(
        (cg.brightness - rg.brightness).abs() <= 0.08,
        format!("brightness {:.2} against {:.2}", cg.brightness, rg.brightness),
        format!(
            "brightness {:.2} against {:.2}: {}",
            cg.brightness,
            rg.brightness,
            if cg.brightness > rg.brightness { "darker in the reference" } else { "brighter in the reference" }
        ),
    );
    check(
        (cg.saturation - rg.saturation).abs() <= 0.1,
        format!("saturation {:.2} against {:.2}", cg.saturation, rg.saturation),
        format!(
            "saturation {:.2} against {:.2}: {}",
            cg.saturation,
            rg.saturation,
            if cg.saturation > rg.saturation { "the reference is less saturated" } else { "the reference is more saturated" }
        ),
    );
    check(
        (cg.warmth - rg.warmth).abs() <= 0.06,
        format!("white balance {:+.2} against {:+.2}", cg.warmth, rg.warmth),
        format!("white balance {:+.2} against {:+.2}: the reference is {}", cg.warmth, rg.warmth, if rg.warmth > cg.warmth { "warmer" } else { "cooler" }),
    );
    check(
        (cg.contrast - rg.contrast).abs() <= 0.05,
        format!("contrast {:.2} against {:.2}", cg.contrast, rg.contrast),
        format!("contrast {:.2} against {:.2}: the reference is {}", cg.contrast, rg.contrast, if rg.contrast > cg.contrast { "punchier" } else { "flatter" }),
    );
    if let Some(d) = palette_distance(&rg.palette, &cg.palette) {
        check(
            d <= 60.0,
            format!("palette close to the reference (distance {d:.0})"),
            format!("palette far from the reference (distance {d:.0}); its colours are {}", rg.palette.join(" ")),
        );
    }
    if let (Some(ri), Some(ci)) = (r.loudness.integrated_lufs, c.loudness.integrated_lufs) {
        check(
            (ci - ri).abs() <= 2.0,
            format!("loudness {ci:.1} LUFS against {ri:.1}"),
            format!("loudness {ci:.1} LUFS against {ri:.1}: {}", if ci < ri { "turn it up" } else { "turn it down" }),
        );
    }
    let total = out.matches.len() + out.differs.len();
    out.score = if total == 0 { 0.0 } else { round(out.matches.len() as f64 / total as f64, 2) };
    out
}

/// The edit-brand tempo a reference implies.
pub fn tempo_of(r: &Reference) -> &'static str {
    tempo_word(&r.rhythm)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rhythm_reads_fast_and_slow_edits() {
        let fast: Vec<(f64, f64)> = (0..30).map(|i| (i as f64 * 1.0, i as f64 * 1.0 + 1.0)).collect();
        let r = rhythm(&fast, 30.0);
        assert_eq!(r.shots, 30);
        assert_eq!(r.median_shot, 1.0);
        assert_eq!(r.cuts_per_minute, 58.0);
        assert_eq!(r.variation, 0.0, "a metronome");
        assert_eq!(tempo_word(&r), "punchy");
        let slow = vec![(0.0, 6.0), (6.0, 9.0), (9.0, 17.0)];
        let r = rhythm(&slow, 17.0);
        assert_eq!(r.median_shot, 6.0);
        assert!(r.variation > 0.5);
        assert_eq!(tempo_word(&r), "calm");
    }

    #[test]
    fn frame_stats_tell_warm_from_cool() {
        let warm: Vec<u8> = (0..100).flat_map(|_| [230u8, 150, 80]).collect();
        let cool: Vec<u8> = (0..100).flat_map(|_| [60u8, 110, 200]).collect();
        let (bw, sw, ww, cw) = frame_stats(&warm);
        let (_, _, wc, _) = frame_stats(&cool);
        assert!(ww > 0.4 && wc < -0.4);
        assert!(bw > 0.5 && sw > 0.6 && cw < 1e-9, "flat colour has no contrast");
        let g = Grade { brightness: 0.2, saturation: 0.1, warmth: -0.1, contrast: 0.3, ..Default::default() };
        assert_eq!(grade_words(&g), vec!["dark, low-key", "desaturated", "cool", "high contrast"]);
    }

    #[test]
    fn compare_says_what_to_change() {
        let mut r = Reference { name: "ref".into(), ..Default::default() };
        r.rhythm = Rhythm { median_shot: 1.2, variation: 0.6, ..Default::default() };
        r.grade = Grade { brightness: 0.3, saturation: 0.2, warmth: 0.1, contrast: 0.25, palette: vec!["#202830".into(), "#d08040".into()], ..Default::default() };
        r.loudness.integrated_lufs = Some(-14.0);
        let mut c = r.clone();
        c.source = "ours.mp4".into();
        let same = compare(&r, &c);
        assert_eq!(same.score, 1.0, "{:?}", same.differs);
        c.rhythm.median_shot = 3.0;
        c.rhythm.variation = 0.1;
        c.grade.saturation = 0.5;
        c.loudness.integrated_lufs = Some(-20.0);
        let d = compare(&r, &c);
        assert!(d.differs.iter().any(|x| x.contains("slower") && x.contains("cut sooner")));
        assert!(d.differs.iter().any(|x| x.contains("too even")));
        assert!(d.differs.iter().any(|x| x.contains("less saturated")));
        assert!(d.differs.iter().any(|x| x.contains("turn it up")));
        assert!(d.score < 0.6);
    }
}
