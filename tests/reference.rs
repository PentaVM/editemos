//! Needs ffmpeg; skipped (with a note) without it. A video made of four shots
//! of known length and colour is reverse-engineered, and a cut is compared to it.

mod common;

use serde_json::json;
use std::path::Path;
use std::process::{Command, Stdio};

fn make_video(ffmpeg: &Path, out: &Path, shots: &[(&str, f64)]) {
    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-hide_banner", "-loglevel", "error", "-y"]);
    for (c, d) in shots {
        cmd.args(["-f", "lavfi", "-i", &format!("color=c={c}:s=320x180:d={d}:r=25")]);
    }
    let total: f64 = shots.iter().map(|s| s.1).sum();
    cmd.args(["-f", "lavfi", "-i", &format!("sine=f=440:d={total}")]);
    let inputs: String = (0..shots.len()).map(|i| format!("[{i}:v]")).collect();
    cmd.args(["-filter_complex", &format!("{inputs}concat=n={}:v=1:a=0[v]", shots.len()), "-map", "[v]", "-map", &format!("{}:a", shots.len())]);
    cmd.args(["-c:v", "mpeg4", "-q:v", "3", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"]).arg(out);
    let st = cmd.stdin(Stdio::null()).status().unwrap();
    assert!(st.success(), "could not make the test video");
}

#[test]
fn a_reference_is_measured_and_a_cut_is_checked_against_it() {
    let Some(ffmpeg) = editemos::motion::render::find_ffmpeg() else {
        eprintln!("skipped: no ffmpeg on this machine");
        return;
    };
    let root = common::temp_home("reference");
    let ctx = common::ctx_in(&root);
    ctx.project_op(&json!({"action": "create", "name": "Ref Test", "activate": true})).unwrap();

    // warm, fast: 1 s, 2 s, 1 s, 3 s
    let reference = root.join("trailer.mp4");
    make_video(&ffmpeg, &reference, &[("0xd02010", 1.0), ("0xf0d040", 2.0), ("0x401008", 1.0), ("0xf08020", 3.0)]);
    let r = ctx.reference_op(&json!({"file": reference.to_string_lossy(), "name": "Trailer"})).unwrap();
    assert_eq!(r["rhythm"]["shots"], 4, "{r}");
    assert!((r["rhythm"]["median_shot"].as_f64().unwrap() - 2.0).abs() < 0.2, "{r}");
    assert!(r["grade"]["warmth"].as_f64().unwrap() > 0.2, "orange shots read as warm: {r}");
    assert!(r["grade"]["words"].to_string().contains("warm"));
    assert!(r["loudness"]["integrated_lufs"].as_f64().is_some(), "the tone is measured");
    assert!(Path::new(r["contact"].as_str().unwrap()).exists(), "one frame per shot");
    assert!(!r["unknowns"].as_array().unwrap().is_empty() && r["evidence"].as_array().unwrap().len() >= 3);

    // the same video matches itself on every trait
    let same = ctx.reference_op(&json!({"action": "compare", "reference": "Trailer", "file": reference.to_string_lossy()})).unwrap();
    assert_eq!(same["comparison"]["score"], 1.0, "{same}");

    // a slow, cold cut is told what to change
    let ours = root.join("ours.mp4");
    make_video(&ffmpeg, &ours, &[("0x2040c0", 4.0), ("0x80c0f0", 5.0)]);
    let d = ctx.reference_op(&json!({"action": "compare", "reference": "trailer", "file": ours.to_string_lossy()})).unwrap();
    let differs = d["comparison"]["differs"].to_string();
    assert!(differs.contains("cut sooner") && differs.contains("warmer"), "{d}");

    // adopting it sets the tempo and a rule in the edit brand
    let a = ctx.reference_op(&json!({"action": "apply", "reference": "Trailer"})).unwrap();
    assert_eq!(a["tempo"], "steady");
    let b = ctx.brand_op(&json!({})).unwrap();
    assert!(b["edit"]["rules"].to_string().contains("Cut like 'Trailer'"), "{b}");
    assert_eq!(ctx.reference_op(&json!({"action": "list"})).unwrap()["references"].as_array().unwrap().len(), 1);

    let _ = std::fs::remove_dir_all(&root);
}
