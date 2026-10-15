//! Needs ffmpeg; skipped (with a note) without it. A project makes a kind of
//! edit, a clip is built and judged, and the judgements become a training set.

mod common;

use serde_json::json;
use std::path::Path;

#[test]
fn judged_clips_become_a_training_set_without_footage() {
    let Some(ffmpeg) = editemos::motion::render::find_ffmpeg() else {
        eprintln!("skipped: no ffmpeg on this machine");
        return;
    };
    let root = common::temp_home("dataset");
    let ctx = common::ctx_in(&root);
    ctx.project_op(&json!({"action": "create", "name": "Match Night", "activate": true})).unwrap();

    // the project makes gameplay highlights: the kind's rules reach the context
    let used = ctx.kinds_op(&json!({"action": "use", "kind": "gameplay-highlights"})).unwrap();
    assert_eq!(used["defaults"]["layout"], "fit");
    let pack = ctx.context_pack(&json!({})).unwrap();
    assert!(pack.to_string().contains("Never end a segment"), "kind rules in the context");

    // a recording fetched "from a link", built with the kind's defaults
    let sources = root.join("projects").join("match-night").join("sources");
    std::fs::create_dir_all(&sources).unwrap();
    let video = sources.join("vod.mp4");
    common::make_video(&ffmpeg, &video, &[("0x203040", 6.0), ("0x604020", 6.0), ("0x206040", 6.0)], 0.3);
    let src = ctx.dataset_op(&json!({"action": "source", "file": video.to_string_lossy(), "url": "https://example.com/videos/1", "section": "01:40:00-01:40:18"})).unwrap();
    assert_eq!(src["source"]["offset"], 6000.0);

    let spec = json!({"clips": [{"name": "the-play", "segments": [{"start": 1.0, "end": 5.0, "hits": [4.0]}, {"start": 7.0, "end": 11.0}]}]});
    let out = root.join("clips");
    let built = ctx.clips_op(&json!({"action": "build", "file": video.to_string_lossy(), "spec": spec, "out": out.to_string_lossy()})).unwrap();
    let clip = built["clips"][0]["out"].as_str().unwrap().to_string();
    assert!(Path::new(&clip).with_extension("build.json").exists(), "the build leaves what an example needs");

    // judged: approved once, corrected once
    let a = ctx.dataset_op(&json!({"action": "add", "clip": clip, "verdict": "approved", "notes": "the impact lands"})).unwrap();
    assert_eq!(a["kind"], "gameplay-highlights");
    assert!(a["warning"].is_null(), "the source has a link");
    let fixed = json!({"clips": [{"name": "the-play", "segments": [{"start": 1.0, "end": 6.0, "hits": [4.0]}, {"start": 7.0, "end": 13.0}]}]});
    ctx.dataset_op(&json!({"action": "add", "clip": clip, "verdict": "corrected", "correction": fixed, "notes": "end after the outcome"})).unwrap();
    let stats = ctx.dataset_op(&json!({"action": "stats"})).unwrap();
    assert_eq!(stats["stats"]["approved"], 1);
    assert_eq!(stats["stats"]["corrected"], 1);
    assert_eq!(stats["stats"]["with_link"], 2);

    // the set: frames taken from the local recording, plans as answers, a preference pair
    let train = root.join("train");
    let e = ctx.dataset_op(&json!({"action": "export", "out": train.to_string_lossy(), "fps": 1.0})).unwrap();
    assert_eq!(e["export"]["sft"], 2, "{e}");
    assert_eq!(e["export"]["preference"], 1);
    assert!(e["export"]["frames"].as_u64().unwrap() >= 10, "{e}");
    assert!(e["export"]["missing_sources"].as_array().unwrap().is_empty());
    let sft = std::fs::read_to_string(train.join("sft.jsonl")).unwrap();
    let first: serde_json::Value = serde_json::from_str(sft.lines().next().unwrap()).unwrap();
    let images = first["images"].as_array().unwrap();
    assert!(!images.is_empty() && train.join(images[0].as_str().unwrap()).exists());
    assert_eq!(first["messages"][0]["content"].as_str().unwrap().matches("<image>").count(), images.len());
    assert!(first["messages"][1]["content"].as_str().unwrap().contains("\"segments\""));

    // the kind folder is still shareable: text only
    let kind = ctx.kinds_op(&json!({"action": "validate", "kind": "gameplay-highlights"})).unwrap();
    assert_eq!(kind["ok"], true, "{kind}");
    let shared = std::fs::read_to_string(root.join("kinds").join("gameplay-highlights").join("examples.jsonl")).unwrap();
    assert!(shared.contains("https://example.com/videos/1") && !shared.contains("vod.mp4\""), "examples point at the link, not at a local file");

    let _ = std::fs::remove_dir_all(&root);
}
