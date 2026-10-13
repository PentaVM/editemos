//! `editemos bench`: the numbers in the README, measured on your machine.
//! Everything runs in a throwaway home; nothing touches real projects.

use crate::error::Result;
use crate::ops::Ctx;
use crate::store::Home;
use crate::{clips, context, memory, motion};
use serde_json::{json, Value};
use std::process::{Command, Stdio};
use std::time::Instant;

fn ms(t: Instant) -> f64 {
    (t.elapsed().as_secs_f64() * 100_000.0).round() / 100.0
}

const PAGE: &str = r##"<html lang="en"><head><title>Acme Studio</title><meta name="theme-color" content="#ff3d1f">
<style>:root{--brand:#ff3d1f;--ink:#14110f;--paper:#f6efe4}body{background:var(--paper);color:var(--ink);font-family:'Space Grotesk',sans-serif}
h1{font-family:'Fraunces',serif;color:var(--brand)}.cta{background:#ff3d1f;color:#fff}</style></head>
<body><header><img class="logo" src="/logo.svg" alt="Acme logo"></header><h1>Real fire. No shortcuts.</h1><p>Book a table today. We cook for you.</p>
<a class="cta" href="/book">Book</a><a href="https://instagram.com/acme">Instagram</a></body></html>"##;

pub fn run(quick: bool) -> Result<Value> {
    let root = std::env::temp_dir().join(format!("editemos-bench-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("projects"))?;
    let ctx = Ctx { home: Home { root: root.clone() } };
    let mut rows: Vec<Value> = Vec::new();
    let mut row = |name: &str, value: f64, unit: &str, note: String| rows.push(json!({"name": name, "value": value, "unit": unit, "note": note}));

    // project + memory
    let t = Instant::now();
    ctx.project_op(&json!({"action": "create", "name": "Bench", "activate": true}))?;
    row("create a project (with its edit brand)", ms(t), "ms", String::new());
    let p = ctx.home.open_project("bench")?;
    let n_mem = if quick { 1000 } else { 5000 };
    let topics = [
        "cuts land on the word",
        "titles enter through a mask",
        "no crossfades between interviews",
        "grain stays under four percent",
        "music ducks six decibels under speech",
        "captions never cover the eyes",
    ];
    let t = Instant::now();
    for i in 0..n_mem {
        memory::add(
            &p,
            &format!("{} (take {i})", topics[i % topics.len()]),
            if i % 5 == 0 { "rejection" } else { "decision" },
            vec![format!("t{}", i % 7)],
            "medium",
            "bench",
            false,
            None,
        )?;
    }
    row("write a memory", (ms(t) / n_mem as f64 * 1000.0).round() / 1000.0, "ms", format!("average over {n_mem} appends"));
    let t = Instant::now();
    let all = memory::active(&p);
    let loaded = ms(t);
    let t = Instant::now();
    let hits = memory::search(&all, "interview crossfades", 8);
    row("search memory (BM25)", ms(t), "ms", format!("{} memories, {} hits; load {loaded} ms", all.len(), hits.len()));
    let t = Instant::now();
    let pack = context::build(&ctx.home, &p, &context::PackOptions { task: Some("opening titles".into()), ..Default::default() });
    row("build the context pack", ms(t), "ms", format!("~{} tokens from {} memories", crate::util::approx_tokens(&pack), all.len()));

    // brand analysis (parsing only: no network in a benchmark)
    let base = url::Url::parse("https://acme.example/").unwrap();
    let t = Instant::now();
    let reps = 200;
    for _ in 0..reps {
        let a = crate::extract::analyze::parse_html(PAGE, &base);
        std::hint::black_box(crate::extract::analyze::build_palette(&a, &[]));
    }
    row("brand analysis of a page", (ms(t) / reps as f64 * 100.0).round() / 100.0, "ms", "HTML + CSS -> palette, fonts, logo, voice (network excluded)".into());

    // motion
    if crate::cdp::find_browser().is_some() && motion::render::find_ffmpeg().is_some() {
        let scene = ctx.motion_op(&json!({"action": "new", "kind": "title", "text": "Benchmark"}))?;
        let path = scene["scene"].as_str().unwrap_or("").to_string();
        let secs = if quick { 1.0 } else { 2.0 };
        for (label, jobs) in [("render 1080p ProRes 4444 alpha, 1 window", 1), ("render 1080p ProRes 4444 alpha, 3 windows", 3)] {
            let t = Instant::now();
            let r = ctx.motion_op(&json!({"action": "render", "scene": path, "duration": secs, "jobs": jobs, "force": true, "no_contact": true, "out": root.join(format!("b{jobs}.mov")).to_string_lossy()}))?;
            let frames = r["frames"].as_f64().unwrap_or(1.0);
            let cap = r["capture_seconds"].as_f64().unwrap_or(0.0);
            row(
                label,
                (frames / cap.max(0.01) * 10.0).round() / 10.0,
                "frames/s captured",
                format!("{frames} frames, {:.1} s wall including browser start and encode", t.elapsed().as_secs_f64()),
            );
        }
        let t = Instant::now();
        let again = ctx.motion_op(&json!({"action": "new", "kind": "title", "text": "Benchmark"}))?;
        row("ask for a graphic that already exists", ms(t), "ms", format!("reused={}: returned from the motion library, nothing rebuilt", again["reused"]));
    } else {
        row("motion render", 0.0, "skipped", "needs a Chromium-based browser and ffmpeg".into());
    }

    // clips
    if let Some(ffmpeg) = motion::render::find_ffmpeg() {
        let minutes = if quick { 10 } else { 60 };
        let wav = root.join("stream.wav");
        let ok = Command::new(&ffmpeg)
            .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i"])
            .arg(format!("aevalsrc='sin(2*PI*220*t)*(if(lt(mod(t,7),0.4),0,if(lt(mod(t,300),25),0.8,0.05)))':d={}:s=44100", minutes * 60))
            .args(["-c:a", "pcm_s16le"])
            .arg(&wav)
            .stdin(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            let t = Instant::now();
            let audio = clips::analyse_audio(&ffmpeg, &wav)?;
            let analysed = t.elapsed().as_secs_f64();
            let t = Instant::now();
            let picked = clips::plan(&audio, &[], &clips::Options { count: 8, ..Default::default() });
            row("clips: listen to a recording", (audio.duration / analysed.max(0.001)).round(), "x real time", format!("{minutes} min of audio in {analysed:.2} s"));
            row("clips: choose the moments", ms(t), "ms", format!("{} clips from {} loudness samples and {} pauses", picked.len(), audio.loud.len(), audio.silences.len()));
        }
    }
    let _ = std::fs::remove_dir_all(&root);
    Ok(json!({
        "machine": format!("{} {} ({} logical cores)", std::env::consts::OS, std::env::consts::ARCH, std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1)),
        "version": env!("CARGO_PKG_VERSION"),
        "build": if cfg!(debug_assertions) { "debug" } else { "release" },
        "rows": rows,
    }))
}

pub fn markdown(v: &Value) -> String {
    let mut s = format!(
        "editemos {} ({}) on {}\n\n| What | Result | Detail |\n| --- | ---: | --- |\n",
        v["version"].as_str().unwrap_or(""),
        v["build"].as_str().unwrap_or(""),
        v["machine"].as_str().unwrap_or("")
    );
    for r in v["rows"].as_array().cloned().unwrap_or_default() {
        s.push_str(&format!("| {} | {} {} | {} |\n", r["name"].as_str().unwrap_or(""), r["value"], r["unit"].as_str().unwrap_or(""), r["note"].as_str().unwrap_or("")));
    }
    s
}
