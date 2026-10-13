//! Needs a Chromium-based browser and ffmpeg; skipped (with a note) without them.
//! Renders a real scene and checks that the video keeps its alpha channel.

mod common;

use serde_json::json;
use std::process::{Command, Stdio};

#[test]
fn title_renders_with_alpha() {
    let (Some(_), Some(ffmpeg)) = (editemos::cdp::find_browser(), editemos::motion::render::find_ffmpeg()) else {
        eprintln!("skipped: no browser or ffmpeg on this machine");
        return;
    };
    let root = common::temp_home("render");
    let ctx = common::ctx_in(&root);
    ctx.project_op(&json!({"action": "create", "name": "Alpha Test", "activate": true})).unwrap();

    let scene = ctx.motion_op(&json!({"action": "new", "kind": "title", "text": "Quiebre", "params": {"kicker": "SEC.01"}})).unwrap();
    let scene_path = scene["scene"].as_str().unwrap().to_string();

    // the project's own tokens drive the scene: lint must run and not error
    let lint = ctx.motion_op(&json!({"action": "lint", "scene": scene_path})).unwrap();
    assert_eq!(lint["ok"], true, "lint: {lint}");

    let out = root.join("title.mov");
    let r = ctx.motion_op(&json!({"action": "render", "scene": scene_path, "out": out.to_string_lossy(), "duration": 1.0, "subframes": 2, "jobs": 2})).unwrap();
    assert_eq!(r["ok"], true);
    assert_eq!(r["frames"], 30);
    assert!(out.exists() && std::fs::metadata(&out).unwrap().len() > 10_000);
    assert!(std::path::Path::new(r["contact"].as_str().unwrap()).exists(), "contact sheet missing");

    // decode one frame back out and look for transparent AND opaque pixels
    let frame = root.join("frame.png");
    let st = Command::new(&ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-ss", "0.8", "-i"])
        .arg(&out)
        .args(["-frames:v", "1", "-pix_fmt", "rgba"])
        .arg(&frame)
        .stdin(Stdio::null())
        .status()
        .unwrap();
    assert!(st.success());
    let img = image::open(&frame).unwrap().to_rgba8();
    let transparent = img.pixels().filter(|p| p.0[3] == 0).count();
    let opaque = img.pixels().filter(|p| p.0[3] > 200).count();
    assert!(transparent > img.pixels().len() / 2, "background is not transparent ({transparent} px)");
    assert!(opaque > 500, "no visible graphics in the frame ({opaque} px)");

    let _ = std::fs::remove_dir_all(&root);
}
