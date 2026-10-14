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

/// A scene written the Remotion way, with sound tied to its motion and one word
/// pushed off the frame on purpose.
const COMPOSITION: &str = r#"<!doctype html><html><head><meta charset="utf-8"><style>
#t{position:absolute;left:40%;top:40%;font:96px Georgia,serif;color:#fff}
#edge{position:absolute;left:1850px;top:600px;font:64px Georgia,serif;color:#fff}
</style></head><body><div id="t">Hola</div><div id="edge">CORTADO</div><script>
const { interpolate, spring, Easing, Sequence, Sfx } = Remotion;
Motion.sfx(0.2, 'whoosh');
Motion.composition({ durationInFrames: 30, fps: 30, render(frame) {
  const s = spring({ frame, fps: 30, config: { damping: 14 } });
  document.getElementById('t').style.transform = `scale(${s})`;
  Sequence({ from: 15 }, (f) => {
    Sfx({ kind: 'pop' });
    document.getElementById('t').style.opacity = interpolate(f, [0, 10], [1, 0.6], { easing: Easing.out(Easing.exp), extrapolateRight: 'clamp' });
  });
} });
</script></body></html>"#;

#[test]
fn remotion_style_scene_renders_with_its_sound_and_a_review() {
    let (Some(_), Some(ffmpeg)) = (editemos::cdp::find_browser(), editemos::motion::render::find_ffmpeg()) else {
        eprintln!("skipped: no browser or ffmpeg on this machine");
        return;
    };
    let root = common::temp_home("compose");
    let ctx = common::ctx_in(&root);
    let scene = root.join("comp.html");
    std::fs::write(&scene, COMPOSITION).unwrap();
    let out = root.join("comp.mov");
    let r = ctx.motion_op(&json!({"action": "render", "scene": scene.to_string_lossy(), "out": out.to_string_lossy(), "jobs": 2})).unwrap();
    assert_eq!(r["frames"], 30, "{r}");
    assert_eq!(r["cues"], 2, "the whoosh and the pop inside the Sequence: {r}");
    let codes: Vec<&str> = r["lint"].as_array().unwrap().iter().filter_map(|i| i["code"].as_str()).collect();
    assert!(codes.contains(&"TEXT_OFFSCREEN"), "the word past the right edge is reported: {codes:?}");
    assert!(std::path::Path::new(r["review"].as_str().unwrap()).exists(), "review stills missing");
    assert!(!r["checkpoints"].as_array().unwrap().is_empty());

    // the sound is inside the video, and starts where the scene put it
    let pcm = Command::new(&ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(&out)
        .args(["-map", "0:a", "-ac", "1", "-ar", "48000", "-f", "s16le", "-"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(pcm.status.success() && !pcm.stdout.is_empty(), "no audio stream: {}", String::from_utf8_lossy(&pcm.stderr));
    let samples: Vec<i16> = pcm.stdout.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
    let first = samples.iter().position(|s| s.unsigned_abs() > 300).unwrap() as f64 / 48_000.0;
    assert!((0.19..0.32).contains(&first), "whoosh should start at 0.2 s, heard at {first:.3}");
    let pop = &samples[(0.5 * 48_000.0) as usize..(0.6 * 48_000.0) as usize];
    assert!(pop.iter().any(|s| s.unsigned_abs() > 3000), "the pop at frame 15 (0.5 s) is missing");
    assert!((samples.len() as f64 / 48_000.0 - 1.0).abs() < 0.05, "audio runs as long as the video");

    // WebM carries it as Opus
    let webm = root.join("comp.webm");
    let r = ctx.motion_op(&json!({"action": "render", "scene": scene.to_string_lossy(), "out": webm.to_string_lossy(), "no_lint": true, "no_contact": true})).unwrap();
    assert!(r["audio"].as_str().is_some(), "{r}");
    let probe = Command::new(&ffmpeg).args(["-hide_banner", "-i"]).arg(&webm).stdin(Stdio::null()).output().unwrap();
    assert!(String::from_utf8_lossy(&probe.stderr).contains("Audio: opus"), "{}", String::from_utf8_lossy(&probe.stderr));

    let _ = std::fs::remove_dir_all(&root);
}
