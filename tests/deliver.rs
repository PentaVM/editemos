//! Needs ffmpeg and ffprobe; skipped (with a note) without them. A small cut is
//! delivered as a 1080p master with burned subtitles and a loudness target, and
//! the master is read back and checked.

mod common;

use serde_json::json;
use std::path::Path;

#[test]
fn a_cut_is_delivered_and_checked() {
    let Some(ffmpeg) = editemos::motion::render::find_ffmpeg() else {
        eprintln!("skipped: no ffmpeg on this machine");
        return;
    };
    if editemos::deliver::find_ffprobe(&ffmpeg).is_none() {
        eprintln!("skipped: no ffprobe next to ffmpeg");
        return;
    }
    let root = common::temp_home("deliver");
    let ctx = common::ctx_in(&root);
    ctx.project_op(&json!({"action": "create", "name": "Deliver Test", "activate": true})).unwrap();

    // 320x180 at 25 fps, 2 s, a quiet tone
    let cut = root.join("cut.mp4");
    common::make_video(&ffmpeg, &cut, &[("0x203040", 1.0), ("0xc08040", 1.0)], 0.05);
    let srt = root.join("cut.srt");
    std::fs::write(&srt, "1\n00:00:00,200 --> 00:00:01,600\nA caption drawn at the output size\n").unwrap();
    let burn = editemos::story::has_filter(&ffmpeg, "subtitles");

    let mut args = json!({"file": cut.to_string_lossy(), "preset": "youtube-1080", "loudness": -16.0, "speed": "ultrafast"});
    if burn {
        args["subtitles"] = json!(srt.to_string_lossy());
    }
    let d = ctx.deliver_op(&args).unwrap();
    assert_eq!(d["check"]["ok"], true, "{d}");
    let p = &d["check"]["probe"];
    assert_eq!((p["width"].as_u64(), p["height"].as_u64()), (Some(1920), Some(1080)), "{d}");
    assert!((p["fps"].as_f64().unwrap() - 25.0).abs() < 0.01, "the source's frame rate is kept: {d}");
    assert!((p["duration"].as_f64().unwrap() - 2.0).abs() < 0.2, "{d}");
    assert_eq!(p["audio"], true);
    assert_eq!(p["sample_rate"], 48000);
    let file = d["file"].as_str().unwrap();
    assert!(Path::new(file).exists() && file.contains("exports"), "{d}");
    let after = d["loudness"]["after_lufs"].as_f64().expect("measured after");
    assert!((after + 16.0).abs() < 1.5, "brought to -16 LUFS: {d}");
    assert!(d["loudness"]["true_peak_dbtp"].as_f64().unwrap() <= -0.5, "peaks held down: {d}");
    if burn {
        assert!(Path::new(d["srt"].as_str().unwrap()).exists(), "the .srt travels with the master");
    } else {
        eprintln!("note: this ffmpeg has no libass; subtitles were not tested");
    }
    // nothing temporary left behind
    let leftovers: Vec<_> =
        std::fs::read_dir(Path::new(file).parent().unwrap()).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().starts_with("editemos-subs-")).collect();
    assert!(leftovers.is_empty());

    // the source is never overwritten, and unknown presets are refused
    assert!(ctx.deliver_op(&json!({"file": cut.to_string_lossy(), "out": cut.to_string_lossy()})).is_err());
    assert!(ctx.deliver_op(&json!({"file": cut.to_string_lossy(), "preset": "tiktok-8k"})).is_err());
    assert_eq!(ctx.deliver_op(&json!({"action": "presets"})).unwrap()["presets"].as_array().unwrap().len(), 3);

    let _ = std::fs::remove_dir_all(&root);
}
