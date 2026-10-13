//! The co-pilot surface: reuse instead of duplication, clips mode, the live studio,
//! detached sessions and OpenAI-compatible models.

mod common;

use serde_json::{json, Value};
use std::io::{Read, Write};
use std::process::{Command, Stdio};

#[test]
fn the_same_graphic_is_built_once() {
    let root = common::temp_home("reuse");
    let ctx = common::ctx_in(&root);
    ctx.project_op(&json!({"action": "create", "name": "Reuse", "activate": true})).unwrap();
    let first = ctx.motion_op(&json!({"action": "new", "kind": "lower-third", "text": "Ana Salazar", "params": {"role": "Directora"}})).unwrap();
    let second = ctx.motion_op(&json!({"action": "new", "kind": "lower-third", "text": "Ana Salazar", "params": {"role": "Directora"}})).unwrap();
    assert_eq!(first["reused"], false);
    assert_eq!(second["reused"], true);
    assert_eq!(first["scene"], second["scene"], "no second copy of the same graphic");
    // a different text is a different piece; force builds again on purpose
    let other = ctx.motion_op(&json!({"action": "new", "kind": "lower-third", "text": "Luis Vera"})).unwrap();
    assert_ne!(other["scene"], first["scene"]);
    let scenes = std::fs::read_dir(root.join("projects/reuse/motion/scenes")).unwrap().count();
    assert_eq!(scenes, 2, "two graphics asked for three times leave two files");
    let lib = ctx.motion_op(&json!({"action": "library"})).unwrap();
    assert_eq!(lib["count"], 2);
    assert!(lib["pieces"].as_array().unwrap().iter().any(|p| p["reused"] == 1));
    // the agent is told what exists before it designs anything
    let pack = ctx.context_pack(&json!({})).unwrap();
    assert!(pack["context"].as_str().unwrap().contains("Motion library"), "context lists reusable graphics");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn clips_finds_the_loud_moments_and_cuts_them() {
    let Some(ffmpeg) = editemos::motion::render::find_ffmpeg() else {
        eprintln!("skipped: no ffmpeg");
        return;
    };
    let root = common::temp_home("clips");
    let ctx = common::ctx_in(&root);
    ctx.project_op(&json!({"action": "create", "name": "Stream", "activate": true})).unwrap();
    // 90 s: quiet tone with a pause every 6 s, loud from 30 s to 48 s
    let src = root.join("stream.mp4");
    let st = Command::new(&ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "testsrc2=s=640x360:r=25:d=90", "-f", "lavfi", "-i"])
        .arg("aevalsrc='sin(2*PI*220*t)*(if(lt(mod(t,6),0.4),0,if(between(t,30,48),0.8,0.05)))':d=90")
        .args(["-c:v", "libx264", "-preset", "ultrafast", "-c:a", "aac", "-shortest"])
        .arg(&src)
        .stdin(Stdio::null())
        .status()
        .unwrap();
    assert!(st.success());
    let srt = root.join("stream.srt");
    std::fs::write(&srt, "1\n00:00:31,000 --> 00:00:34,000\nwow no way!\n\n2\n00:01:10,000 --> 00:01:12,000\nok\n").unwrap();

    let m = ctx.clips_op(&json!({"file": src.to_string_lossy(), "count": 1, "min": 10, "max": 20, "aspect": "9:16", "height": 640, "transcript": srt.to_string_lossy()})).unwrap();
    let c = &m["clips"][0];
    let (start, end) = (c["start"].as_f64().unwrap(), c["end"].as_f64().unwrap());
    assert!(start >= 29.0 && end <= 49.5, "the clip sits inside the loud stretch: {start}-{end}");
    assert!(c["why"].to_string().contains("reaction word"), "the transcript counted: {}", c["why"]);
    let file = std::path::PathBuf::from(c["file"].as_str().unwrap());
    assert!(file.exists() && std::fs::metadata(&file).unwrap().len() > 5_000);
    assert!(std::fs::read_to_string(c["subtitles"].as_str().unwrap()).unwrap().contains("wow no way"));
    assert!(std::path::Path::new(m["edl"].as_str().unwrap()).exists());
    // vertical framing: decode one frame and measure it
    let frame = root.join("f.png");
    Command::new(&ffmpeg).args(["-hide_banner", "-loglevel", "error", "-y", "-i"]).arg(&file).args(["-frames:v", "1"]).arg(&frame).stdin(Stdio::null()).status().unwrap();
    let img = image::open(&frame).unwrap();
    assert_eq!((img.width(), img.height()), (360, 640));
    // clips are project assets and the run is remembered
    let assets = ctx.assets_op(&json!({})).unwrap();
    assert!(assets["assets"].as_array().unwrap().iter().any(|a| a["kind"] == "clip"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_brand_can_be_stated_described_or_shown_without_any_website() {
    let root = common::temp_home("manual");
    let ctx = common::ctx_in(&root);
    ctx.project_op(&json!({"action": "create", "name": "Canal Uno", "activate": true})).unwrap();

    // in the user's own words
    let r = ctx.brand_op(&json!({"action": "set", "description": "Canal de coches. Oscuro #0d0d12, texto #f4f1ea, acento #19e3a1. Fuente: Russo One"})).unwrap();
    assert!(r["changed"].to_string().contains("primary"));
    let b = ctx.brand_op(&json!({})).unwrap();
    assert_eq!(b["edit"]["colors"]["accent"], "#19e3a1");
    assert_eq!(b["edit"]["colors"]["bg"], "#0d0d12");
    assert_eq!(b["edit"]["type_tokens"]["heading"], "Russo One");

    // stated field by field; `text` is a colour here, not a description
    ctx.brand_op(&json!({"action": "set", "primary": "#ff5a36", "text": "#ffffff", "voice": ["dry", "fast"]})).unwrap();
    let b = ctx.brand_op(&json!({})).unwrap();
    assert_eq!(b["edit"]["colors"]["accent"], "#ff5a36");
    assert_eq!(b["edit"]["colors"]["fg"], "#ffffff");
    assert_eq!(b["voice"]["descriptors"], json!(["dry", "fast"]));
    assert!(ctx.brand_op(&json!({"action": "set", "primary": "not-a-colour"})).is_err());

    // a black-and-white identity stays black and white: no invented hue
    ctx.brand_op(&json!({"action": "set", "primary": "#fafafa", "background": "#000000", "text": "#fafafa"})).unwrap();
    let b = ctx.brand_op(&json!({})).unwrap();
    assert_eq!(b["edit"]["colors"]["accent"], "#fafafa");
    let a2 = b["edit"]["colors"]["accent2"].as_str().unwrap();
    assert!(a2[1..3] == a2[3..5] && a2[3..5] == a2[5..7], "second colour of a monochrome brand is a grey, got {a2}");

    // from a picture, and with a model judging the evidence
    let logo = root.join("logo.png");
    let mut img = image::RgbaImage::from_pixel(64, 64, image::Rgba([12, 12, 16, 255]));
    for (x, _, px) in img.enumerate_pixels_mut() {
        if x > 20 {
            *px = image::Rgba([25, 227, 161, 255]);
        }
    }
    img.save(&logo).unwrap();
    let r = ctx.brand_op(&json!({"action": "from-images", "files": [logo.to_string_lossy()]})).unwrap();
    assert!(r["palette"].to_string().contains("#19e3a1"), "{r}");
    let reply = json!({"choices": [{"message": {"role": "assistant", "content": "Here:\n{\"primary\":\"#19e3a1\",\"drop\":[\"#ff0000\"],\"voice\":[\"calm\"],\"reason\":\"the red is the platform's\"}"}}]}).to_string();
    let base = common::serve(vec![("/v1/chat/completions", "application/json", reply.into_bytes())]);
    std::fs::write(root.join("adapters.toml"), format!("[adapters.judge]\nkind = \"openai-chat\"\nurl = \"{base}/v1/chat/completions\"\nmodel = \"tiny\"\n")).unwrap();
    let ev = ctx.brand_op(&json!({"action": "evidence"})).unwrap();
    assert!(!ev["images"].as_array().unwrap().is_empty(), "the judge is shown the pictures");
    let r = ctx.brand_op(&json!({"action": "refine", "adapter": "judge"})).unwrap();
    assert!(r["reason"].as_str().unwrap().contains("platform"));
    assert_eq!(ctx.brand_op(&json!({})).unwrap()["voice"]["descriptors"], json!(["calm"]));
    let _ = std::fs::remove_dir_all(&root);
}

fn http(port: u16, req: &str) -> (String, Vec<u8>) {
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.write_all(req.as_bytes()).unwrap();
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).unwrap();
    let split = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap_or(buf.len());
    (String::from_utf8_lossy(&buf[..split]).to_string(), buf[(split + 4).min(buf.len())..].to_vec())
}

#[test]
fn studio_shows_changes_and_stays_local() {
    let root = common::temp_home("studio");
    let ctx = common::ctx_in(&root);
    ctx.project_op(&json!({"action": "create", "name": "Live", "activate": true})).unwrap();
    let (listener, url) = editemos::studio::bind(0).unwrap();
    let port: u16 = url.trim_end_matches('/').rsplit(':').next().unwrap().parse().unwrap();
    let ctx2 = common::ctx_in(&root);
    std::thread::spawn(move || editemos::studio::serve(&ctx2, listener));

    let get = |path: &str| http(port, &format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"));
    let (head, body) = get("/");
    assert!(head.starts_with("HTTP/1.1 200") && String::from_utf8_lossy(&body).contains("editemos"));
    let v1: Value = serde_json::from_slice(&get("/api/version?project=live").1).unwrap();

    // something changes on disk -> the version the page polls changes
    std::thread::sleep(std::time::Duration::from_millis(30));
    ctx.motion_op(&json!({"action": "new", "kind": "title", "text": "En vivo"})).unwrap();
    let body = r#"{"text":"Nunca usar fundidos entre planos","kind":"rejection"}"#;
    let (head, _) = http(port, &format!("POST /api/memory?project=live HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: http://127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()));
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    let v2: Value = serde_json::from_slice(&get("/api/version?project=live").1).unwrap();
    assert_ne!(v1["version"], v2["version"]);
    let state: Value = serde_json::from_slice(&get("/api/state?project=live").1).unwrap();
    assert!(state["items"].as_array().unwrap().iter().any(|i| i["kind"] == "motion"));
    assert!(state["memories"].as_array().unwrap().iter().any(|m| m["text"].as_str().unwrap().contains("fundidos") && m["source"] == "studio"));

    // only this project's files; only this machine's pages
    let outside = if cfg!(windows) { "C:%5CWindows%5Cwin.ini" } else { "%2Fetc%2Fhostname" };
    assert!(get(&format!("/media?project=live&path={outside}")).0.starts_with("HTTP/1.1 403"));
    let (head, _) = http(
        port,
        &format!(
            "POST /api/memory?project=live HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: http://evil.example\r\nContent-Length: 12\r\nConnection: close\r\n\r\n{{\"text\":\"x\"}}"
        ),
    );
    assert!(head.starts_with("HTTP/1.1 403"), "a foreign page must not write memory: {head}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn sessions_are_built_per_agent_and_any_openai_endpoint_works() {
    let root = common::temp_home("session");
    let ctx = common::ctx_in(&root);
    ctx.project_op(&json!({"action": "create", "name": "Sesion", "activate": true})).unwrap();

    let codex = ctx.session_op(&json!({"adapter": "codex", "model": "gpt-5", "effort": "high", "dry_run": true})).unwrap();
    assert_eq!(codex["command"], json!(["codex", "-m", "gpt-5", "-c", "model_reasoning_effort=high"]));
    // no model and no effort: the flags disappear instead of being passed empty
    let plain = ctx.session_op(&json!({"adapter": "claude", "dry_run": true})).unwrap();
    assert_eq!(plain["command"], json!(["claude"]));
    let script = std::fs::read_to_string(plain["script"].as_str().unwrap()).unwrap();
    assert!(script.contains("EDITEMOS_PROJECT") && script.contains("sesion"), "the session is pinned to its project");
    assert!(root.join("projects/sesion/AGENTS.md").exists() && root.join("projects/sesion/CLAUDE.md").exists());
    // API-only models get the built-in chat
    let ds = ctx.session_op(&json!({"adapter": "deepseek", "dry_run": true})).unwrap();
    assert!(ds["command"].to_string().contains("chat"));

    // a local OpenAI-compatible server (the mock answers every POST the same way)
    let reply = json!({"choices": [{"message": {"role": "assistant", "content": "Listo: corte en 00:12."}}]}).to_string();
    let base = common::serve(vec![("/v1/chat/completions", "application/json", reply.into_bytes())]);
    std::fs::write(root.join("adapters.toml"), format!("[adapters.mock]\nkind = \"openai-chat\"\nurl = \"{base}/v1/chat/completions\"\nmodel = \"tiny\"\n")).unwrap();
    let ad = editemos::adapters::get(&ctx.home, "mock").unwrap();
    assert_eq!(editemos::adapters::run_text(&ad, "hola").unwrap(), "Listo: corte en 00:12.");
    let mut msgs = vec![json!({"role": "user", "content": "corta"})];
    let tools = editemos::session::openai_tools();
    assert!(tools.as_array().unwrap().iter().any(|t| t["function"]["name"] == "editemos_clips"));
    let answer = editemos::session::turn(&ctx, &ad, &mut msgs, &tools, |_, _| {}).unwrap();
    assert!(answer.contains("00:12") && msgs.len() == 2);
    let _ = std::fs::remove_dir_all(&root);
}
