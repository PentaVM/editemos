//! editemos command line. It runs the same operations the MCP server exposes.

use editemos::error::{fail, Result};
use editemos::ops::Ctx;
use editemos::util::{path_str, write_atomic};
use editemos::{extract, mcp};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const SKILL_MD: &str = include_str!("../assets/SKILL.md");

const BOOL_FLAGS: &[&str] = &[
    "fonts",
    "ignore-robots",
    "dry-run",
    "pin",
    "pinned",
    "start",
    "strict",
    "no-lint",
    "no-contact",
    "keep-frames",
    "include-cloud",
    "allow-flagged",
    "activate",
    "write",
    "json",
    "once",
    "extract",
    "force",
    "help",
    "version",
    "serve",
    "no-open",
    "no-session",
    "copy",
    "burn",
    "plan-only",
    "prune",
    "quick",
    "global",
    "transcribe",
    "no-captions",
    "no-punch",
    "no-snap",
    "heading",
];
const NUM_KEYS: &[&str] = &[
    "fps",
    "duration",
    "subframes",
    "shutter",
    "at",
    "dur",
    "k",
    "n",
    "max_images",
    "budget",
    "timeout",
    "interval",
    "jobs",
    "count",
    "min",
    "max",
    "height",
    "port",
    "source_height",
    "from",
    "to",
    "seconds",
    "max_pause",
    "breath",
    "lead",
    "tail",
    "cols",
    "rows",
    "words",
];

const HELP: &str = "\
editemos - every video gets its own edit brand.

USAGE  editemos <command> [options]        (add --json for machine-readable output)

START HERE
  start [project] [--adapter A] [--model M] [--effort E] [--file F.kdenlive] [--no-session]
        opens Kdenlive, the live studio page and an agent session in its own window
  studio                      live page: assets, renders, clips and memory as they change
  session [--adapter A] [--model M] [--effort E] [--dir D]   agent in its own window
  chat [--adapter deepseek|openai|openrouter|local|lmstudio] [--model M]   built-in chat with tools
  clips <recording or link> [--layout crop|fit] [--section 600-1500] [--count 5] [--min 15] [--max 60] [--aspect 9:16|1:1|4:5|16:9|source]
        [--transcript F.srt] [--burn] [--copy] [--plan-only]   best moments, cut and framed

STORY CLIPS (edited, not just cut out)
  clips transcript <recording> [--from S --to S]      the words with their times, to plan from
  clips search <recording> <words>                    where something was said
  clips events <recording>                            what happened, judged by sound (no words needed)
  clips look <recording> --from S --to S              a grid of frames with their times, to see it
  clips score <recording> --region x,y,w,h            when the scoreboard changes (the goals of a quiet stream)
  clips highlights <recording> [--count 3] [--lead 10] [--tail 9]   build-up, moment, aftermath
  clips story <recording> [--adapter A] [--count 3] [--seconds 60] [--notes \"...\"]   a model plans, this builds
  clips build <recording> --spec-file plan.json       build a plan you or your agent wrote
        segments in any order, pauses removed, word-by-word captions from the clip's own audio,
        a title, context flashes found by what was said, natural or punchline endings
  reference <video or link> [--name N]                reverse-engineer an edit: cut rhythm, grade, loudness
  reference compare <name> <our cut>                  where our cut matches it and what to change
  reference apply <name> | reference list             adopt its tempo in the edit brand
  fonts [show|list] | fonts set <family> [--global] | fonts reject <family> [--global] [--reason R]
  tools install-whisper                               accurate captions, on this machine
  memory add \"...\" --global                           a preference for every project

PROJECTS
  init <name> [--site URL] [--size WxH] [--fps N] [--adapter A] [--extract]
  use <name>                  set the active project        which    show the active project
  projects                    list projects                 -p/--project <name> on any command

BRAND
  extract [URL] [--backend auto|native|browser|scrapling] [--max-images N] [--fonts] [--ignore-robots]
        a YouTube, Instagram, TikTok or Twitch profile works too: the platform's own colours are left out
  brand set [--primary #hex] [--secondary #hex] [--background #hex] [--text #hex]
            [--heading-font F] [--body-font F] [--name N] [--tagline T] [--voice \"a, b\"]
  brand describe \"dark, neon green #19e3a1, font Russo One\"      the brand in your own words
  brand from-images <logo> [banner] [frame]...                    the brand from pictures
  brand evidence | brand refine [--adapter A] [--notes \"...\"]     let a model judge the evidence
  brand [show|sheet|derive|lock|unlock]
  assets [list|search <q>|add <file>] [--kind K] [--tag T]
  generate \"prompt\" [--adapter A] [--kind K] [--size S] [-n N] [--dry-run]

MEMORY AND CONTEXT
  memory add \"text\" [--kind decision|preference|rejection|fact|note] [--tag T] [--confidence C] [--pin]
  memory search \"query\" [-k N]     memory list     memory forget <id>     memory compact
  context [--task T] [--budget N] [--write DIR] [--out FILE]

MOTION GRAPHICS (open source, no After Effects)
  motion templates            motion library [query]   (what exists is reused, not rebuilt)
  motion new <kind> [\"text\"] [--param k=v]... [--out FILE] [--size WxH]
  motion lint <scene.html>
  motion render <scene.html> [--out FILE] [--format prores4444|pngmov|vp9|pngseq] [--fps N] [--size WxH]
                [--duration S] [--subframes N] [--shutter DEG] [--over IMG] [--jobs N] [--strict]
  motion poster <scene.html> [--at S] [--out FILE]

KDENLIVE
  kdenlive status [--start] [--file F]    kdenlive inspect <f>    kdenlive diff <a> <b>
  kdenlive overlay <f> --clip C --at S [--dur S] --out F          kdenlive draft <f> --out F
  watch <f.kdenlive> [--interval S] [--once]    learn from every save you make by hand

SKILLS
  skills list [query]    skills catalog    skills read <name>
  skills sync [id...] [--dry-run] [--include-cloud] [--force]
  skills enable|disable <id...>    skills export [id...] --dest DIR [--agent claude|codex|gemini]

AGENTS
  serve                       MCP server on stdio (Claude Desktop, Claude Code, Codex, Gemini, ...)
  install [print|claude-desktop|claude-code|codex|gemini|skill] [--write]
  agent run \"task\" [--adapter claude|codex|gemini|ollama|...]
  doctor                      what is available on this machine
  bench [--quick]             measure this machine (the README table)
  tools install-scrapling [--python PATH]

Home: ~/.editemos (set EDITEMOS_HOME to move it). Pin a project per shell with EDITEMOS_PROJECT.";

struct Args {
    pos: Vec<String>,
    opt: BTreeMap<String, Vec<String>>,
    flags: HashSet<String>,
}

fn parse(argv: &[String]) -> Args {
    let mut a = Args { pos: Vec::new(), opt: BTreeMap::new(), flags: HashSet::new() };
    let mut i = 0;
    while i < argv.len() {
        let t = &argv[i];
        if t == "--" {
            a.pos.extend(argv[i + 1..].iter().cloned());
            break;
        }
        if let Some(rest) = t.strip_prefix("--") {
            if let Some((k, v)) = rest.split_once('=') {
                a.opt.entry(k.to_string()).or_default().push(v.to_string());
            } else if BOOL_FLAGS.contains(&rest) {
                a.flags.insert(rest.to_string());
            } else if i + 1 < argv.len() {
                a.opt.entry(rest.to_string()).or_default().push(argv[i + 1].clone());
                i += 1;
            } else {
                a.flags.insert(rest.to_string());
            }
        } else if t.len() == 2 && t.starts_with('-') && t.as_bytes()[1].is_ascii_alphabetic() {
            let short = &t[1..];
            match short {
                "h" => {
                    a.flags.insert("help".into());
                }
                "V" => {
                    a.flags.insert("version".into());
                }
                _ => {
                    let long = match short {
                        "o" => "out",
                        "p" => "project",
                        other => other,
                    };
                    if i + 1 < argv.len() {
                        a.opt.entry(long.to_string()).or_default().push(argv[i + 1].clone());
                        i += 1;
                    }
                }
            }
        } else {
            a.pos.push(t.clone());
        }
        i += 1;
    }
    a
}

fn scalar(v: &str) -> Value {
    if let Ok(b) = v.parse::<bool>() {
        return json!(b);
    }
    if let Ok(n) = v.parse::<f64>() {
        if v.chars().all(|c| c.is_ascii_digit() || c == '.' || c == '-') {
            return json!(n);
        }
    }
    json!(v)
}

fn to_args(a: &Args) -> Map<String, Value> {
    let mut m = Map::new();
    for (k, vals) in &a.opt {
        let key = k.replace('-', "_");
        match key.as_str() {
            "tag" => {
                m.insert("tags".into(), json!(vals));
            }
            "param" => {
                let mut o = Map::new();
                for p in vals {
                    if let Some((k, v)) = p.split_once('=') {
                        o.insert(k.to_string(), scalar(v));
                    }
                }
                m.insert("params".into(), Value::Object(o));
            }
            _ => {
                let v = vals.last().cloned().unwrap_or_default();
                if NUM_KEYS.contains(&key.as_str()) {
                    if let Ok(f) = v.parse::<f64>() {
                        m.insert(key, json!(f));
                        continue;
                    }
                }
                m.insert(key, json!(v));
            }
        }
    }
    for f in &a.flags {
        m.insert(f.replace('-', "_"), json!(true));
    }
    if a.flags.contains("pin") {
        m.insert("pinned".into(), json!(true));
    }
    m
}

fn rest(a: &Args, from: usize) -> Option<String> {
    let t = a.pos.get(from..)?.join(" ");
    if t.trim().is_empty() {
        None
    } else {
        Some(t)
    }
}

fn put(m: &mut Map<String, Value>, k: &str, v: impl Into<Value>) {
    m.insert(k.into(), v.into());
}

/* ------------------------------ output ------------------------------ */

fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string())
}

fn st(v: &Value, k: &str) -> String {
    match v.get(k) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(o) => o.to_string(),
    }
}

fn h_which(v: &Value) -> String {
    format!(
        "{}  ({})\n  dir      {}\n  site     {}\n  size     {} @ {} fps\n  tempo    {}\n  memory   {} entries\n  assets   {}\n  context  {}\n",
        st(v, "slug"),
        st(v, "name"),
        st(v, "dir"),
        if st(v, "site").is_empty() { "-".into() } else { st(v, "site") },
        st(v, "size"),
        st(v, "fps"),
        if st(v, "tempo").is_empty() { "(not derived yet; run `editemos extract`)".into() } else { st(v, "tempo") },
        st(v, "memories"),
        st(v, "assets"),
        st(v, "context")
    )
}

fn h_projects(v: &Value) -> String {
    let mut s = String::new();
    for p in v["projects"].as_array().cloned().unwrap_or_default() {
        s.push_str(&format!("{} {:<24} {}\n", if p["active"] == true { "*" } else { " " }, st(&p, "slug"), st(&p, "site")));
    }
    if s.is_empty() {
        s.push_str("no projects yet. Run: editemos init <name> --site https://example.com --extract\n");
    }
    s
}

fn h_memories(v: &Value) -> String {
    let mut s = String::new();
    for m in v["memories"].as_array().cloned().unwrap_or_default() {
        let tags = m["tags"].as_array().map(|t| t.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(",")).unwrap_or_default();
        s.push_str(&format!("{}  {:<10} {}{}\n", st(&m, "id"), st(&m, "kind"), st(&m, "text"), if tags.is_empty() { String::new() } else { format!("  [{tags}]") }));
    }
    if s.is_empty() {
        s.push_str("(no memories)\n");
    }
    s
}

fn h_skills(v: &Value) -> String {
    let mut s = String::new();
    for k in v["skills"].as_array().cloned().unwrap_or_default() {
        let flag = if k["flagged"].as_bool() == Some(true) || k["flagged"].as_array().map(|a| !a.is_empty()).unwrap_or(false) { " [flagged]" } else { "" };
        s.push_str(&format!("{:<34} {:<14} {}{}\n", st(&k, "name"), st(&k, "license"), editemos::util::truncate_chars(&st(&k, "description"), 70), flag));
    }
    if s.is_empty() {
        s.push_str("no skills downloaded yet. Run: editemos skills sync\n");
    }
    s
}

fn h_render(v: &Value) -> String {
    let mut s = format!(
        "rendered {}\n  {} @ {} fps, {} s, {} frames{}, {} bytes, captured in {} s\n",
        st(v, "out"),
        st(v, "size"),
        st(v, "fps"),
        st(v, "duration"),
        st(v, "frames"),
        if v["subframes"].as_u64().unwrap_or(1) > 1 { format!(" x{} sub-frames", st(v, "subframes")) } else { String::new() },
        st(v, "bytes"),
        st(v, "capture_seconds")
    );
    if let Some(c) = v["contact"].as_str() {
        s.push_str(&format!("  contact sheet: {c}\n"));
    }
    for i in v["lint"].as_array().cloned().unwrap_or_default() {
        s.push_str(&format!("  [{}] {}: {}\n", st(&i, "level"), st(&i, "code"), st(&i, "message")));
    }
    s
}

fn h_lint(v: &Value) -> String {
    let issues = v["issues"].as_array().cloned().unwrap_or_default();
    let mut s = String::new();
    if issues.is_empty() {
        s.push_str("no generic-motion issues found\n");
    }
    for i in issues {
        s.push_str(&format!("[{}] {}: {}\n", st(&i, "level"), st(&i, "code"), st(&i, "message")));
    }
    s
}

fn h_extract(v: &Value) -> String {
    let join = |k: &str| v[k].as_array().map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(" ")).unwrap_or_default();
    let mut s = format!(
        "extracted {} with {} in {} ms\n  title    {}\n  palette  {}\n  fonts    {}\n  logos    {}   images {}   new assets {}\n  tempo    {}\n",
        st(v, "final_url"),
        st(v, "backend"),
        st(v, "elapsed_ms"),
        st(v, "title"),
        join("palette"),
        join("fonts"),
        st(v, "logos"),
        st(v, "images"),
        v["assets_added"].as_array().map(|a| a.len()).unwrap_or(0),
        st(v, "tempo")
    );
    for w in v["warnings"].as_array().cloned().unwrap_or_default() {
        s.push_str(&format!("  warning: {}\n", w.as_str().unwrap_or("")));
    }
    s
}

fn h_doctor(v: &Value) -> String {
    let yes = |b: bool| if b { "[ok]" } else { "[--]" };
    let opt = |k: &str| v.get(k).and_then(|x| x.as_str()).map(String::from);
    let mut s = String::new();
    s.push_str(&format!("home            {}\n", st(v, "home")));
    s.push_str(&format!("active project  {}   ({} projects)\n", if st(v, "active_project").is_empty() { "none".into() } else { st(v, "active_project") }, st(v, "projects")));
    s.push_str(&format!(
        "{} browser        {}\n",
        yes(opt("browser").is_some()),
        opt("browser").unwrap_or_else(|| "none (Edge or Chrome needed for motion and the browser backend)".into())
    ));
    s.push_str(&format!("{} ffmpeg         {}\n", yes(opt("ffmpeg").is_some()), opt("ffmpeg").unwrap_or_else(|| "none (set EDITEMOS_FFMPEG)".into())));
    let k = &v["kdenlive"];
    s.push_str(&format!(
        "{} kdenlive       {}{}{}\n",
        yes(k["installed"] == true),
        k["version"].as_str().unwrap_or("not installed"),
        if k["running"] == true { ", running" } else { "" },
        k["melt"].as_str().map(|m| format!(", melt: {m}")).unwrap_or_default()
    ));
    s.push_str(&format!(
        "{} scrapling      {}\n",
        yes(opt("scrapling").is_some()),
        opt("scrapling").unwrap_or_else(|| "not installed (optional: editemos tools install-scrapling)".into())
    ));
    let ag = &v["agents_on_path"];
    for name in ["claude", "codex", "gemini", "ollama"] {
        s.push_str(&format!("{} agent {:<8}  {}\n", yes(ag[name] == true), name, if ag[name] == true { "on PATH" } else { "not found" }));
    }
    let keys = &v["image_keys"];
    for name in ["OPENAI_API_KEY", "GEMINI_API_KEY"] {
        s.push_str(&format!("{} {:<16} {}\n", yes(keys[name] == true), name, if keys[name] == true { "set" } else { "not set" }));
    }
    s.push_str(&format!("    skills        {} downloaded\n", st(v, "skills_downloaded")));
    s
}

/* ------------------------------ install ------------------------------ */

fn claude_desktop_config() -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(std::env::var("APPDATA").unwrap_or_default()).join("Claude").join("claude_desktop_config.json")
    } else if cfg!(target_os = "macos") {
        PathBuf::from(std::env::var("HOME").unwrap_or_default()).join("Library/Application Support/Claude/claude_desktop_config.json")
    } else {
        PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config/Claude/claude_desktop_config.json")
    }
}

fn user_home() -> PathBuf {
    PathBuf::from(std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap_or_else(|_| ".".into()))
}

fn merge_mcp_json(path: &Path, exe: &str) -> Result<String> {
    let mut root: Value = match std::fs::read_to_string(path) {
        Ok(t) if !t.trim().is_empty() => serde_json::from_str(&t).map_err(|e| editemos::error::Error(format!("{} is not valid JSON ({e}); not touching it", path.display())))?,
        _ => json!({}),
    };
    if path.exists() {
        let bak = path.with_extension(format!("json.bak-{}", editemos::util::unix_secs()));
        std::fs::copy(path, &bak)?;
    }
    let obj = root.as_object_mut().ok_or_else(|| editemos::error::Error("config root is not an object".into()))?;
    let servers = obj.entry("mcpServers").or_insert_with(|| json!({}));
    let Some(servers) = servers.as_object_mut() else { return fail("mcpServers is not an object") };
    let existed = servers.contains_key("editemos");
    servers.insert("editemos".into(), json!({"command": exe, "args": ["serve"]}));
    write_atomic(path, pretty(&root).as_bytes())?;
    Ok(format!("{} {} in {}", if existed { "updated" } else { "added" }, "editemos", path.display()))
}

fn install(a: &Args) -> Result<()> {
    let exe = path_str(&std::env::current_exe()?);
    let target = a.pos.get(1).map(String::as_str).unwrap_or("print");
    let write = a.flags.contains("write");
    let snippet_json = pretty(&json!({"mcpServers": {"editemos": {"command": exe, "args": ["serve"]}}}));
    let snippet_toml = format!("[mcp_servers.editemos]\ncommand = {}\nargs = [\"serve\"]\n", serde_json::to_string(&exe)?);
    match target {
        "print" => {
            println!("# Claude Desktop: {}\n{snippet_json}\n", claude_desktop_config().display());
            println!("# Claude Code (user scope)\nclaude mcp add editemos --scope user -- \"{exe}\" serve\n");
            println!("# Codex: ~/.codex/config.toml\n{snippet_toml}");
            println!("# Gemini CLI: ~/.gemini/settings.json\n{snippet_json}\n");
            println!("# Claude Code slash command /editemos\neditemos install skill --write\n");
            println!("Add --write to a target (claude-desktop, claude-code, codex, gemini, skill) to apply it. A backup of any config is kept.");
        }
        "claude-desktop" | "gemini" => {
            let path = if target == "gemini" { user_home().join(".gemini").join("settings.json") } else { claude_desktop_config() };
            if write {
                println!("{}", merge_mcp_json(&path, &exe)?);
                if target == "claude-desktop" {
                    println!("Restart Claude Desktop to load it.");
                }
            } else {
                println!("Would write to {}:\n{snippet_json}\n(run again with --write)", path.display());
            }
        }
        "claude-code" => {
            let cmd = format!("claude mcp add editemos --scope user -- \"{exe}\" serve");
            if write {
                let ok =
                    std::process::Command::new("claude").args(["mcp", "add", "editemos", "--scope", "user", "--", &exe, "serve"]).status().map(|s| s.success()).unwrap_or(false);
                if !ok {
                    return fail(format!("`claude mcp add` failed; run it yourself:\n{cmd}"));
                }
            } else {
                println!("{cmd}\n(run it yourself, or repeat with --write)");
            }
        }
        "codex" => {
            let path = user_home().join(".codex").join("config.toml");
            let current = std::fs::read_to_string(&path).unwrap_or_default();
            if current.contains("[mcp_servers.editemos]") {
                println!("{} already has [mcp_servers.editemos]", path.display());
            } else if write {
                if path.exists() {
                    std::fs::copy(&path, path.with_extension(format!("toml.bak-{}", editemos::util::unix_secs())))?;
                }
                let sep = if current.is_empty() || current.ends_with('\n') { "" } else { "\n" };
                write_atomic(&path, format!("{current}{sep}\n{snippet_toml}").as_bytes())?;
                println!("added editemos to {}", path.display());
            } else {
                println!("Would append to {}:\n{snippet_toml}(run again with --write)", path.display());
            }
        }
        "skill" => {
            let dest = user_home().join(".claude").join("skills").join("editemos").join("SKILL.md");
            if write {
                write_atomic(&dest, SKILL_MD.as_bytes())?;
                println!("installed {}\nUse it in Claude Code with /editemos", dest.display());
            } else {
                println!("Would write {}\n(run again with --write)", dest.display());
            }
        }
        other => return fail(format!("unknown install target '{other}' (print, claude-desktop, claude-code, codex, gemini, skill)")),
    }
    Ok(())
}

/* ------------------------------ watch ------------------------------ */

fn watch(ctx: &Ctx, a: &Args, base: &Map<String, Value>) -> Result<()> {
    let Some(file) = a.pos.get(1).cloned() else { return fail("watch needs a .kdenlive file") };
    let interval = base.get("interval").and_then(|v| v.as_f64()).unwrap_or(2.0).max(0.5);
    let mtime = |p: &str| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    if mtime(&file).is_none() {
        return fail(format!("{file} not found"));
    }
    let mut args = base.clone();
    put(&mut args, "file", file.clone());
    let report = |r: &Value| {
        if r["baseline"] == true {
            println!(
                "watching {file}: baseline stored ({} tracks, {:.1}s). Save in Kdenlive and I will write what you changed into memory.",
                r["tracks"],
                r["duration"].as_f64().unwrap_or(0.0)
            );
        } else if r["changed"] == true {
            println!("learned: {}", st(r, "summary"));
        } else {
            println!("saved, no editing changes");
        }
    };
    report(&ctx.kdenlive_learn(&Value::Object(args.clone()))?);
    if a.flags.contains("once") {
        return Ok(());
    }
    let mut last = mtime(&file);
    loop {
        std::thread::sleep(std::time::Duration::from_millis((interval * 1000.0) as u64));
        let now = mtime(&file);
        if now.is_some() && now != last {
            last = now;
            std::thread::sleep(std::time::Duration::from_millis(600)); // let Kdenlive finish writing
            match ctx.kdenlive_learn(&Value::Object(args.clone())) {
                Ok(r) => report(&r),
                Err(e) => eprintln!("could not read the project yet: {e}"),
            }
        }
    }
}

/* ------------------------------ dispatch ------------------------------ */

fn run(argv: &[String]) -> Result<()> {
    let a = parse(argv);
    if a.flags.contains("version") {
        println!("editemos {VERSION}");
        return Ok(());
    }
    let cmd = a.pos.first().map(String::as_str).unwrap_or("help");
    if a.flags.contains("help") || cmd == "help" {
        println!("{HELP}");
        return Ok(());
    }
    if cmd == "serve" {
        return mcp::serve();
    }
    if cmd == "__cleanup" {
        // Internal: deletes a temporary browser profile after the browser has died.
        if let Some(d) = a.pos.get(1) {
            editemos::cdp::sweep_profile(Path::new(d), 90);
        }
        return Ok(());
    }
    if cmd == "install" {
        return install(&a);
    }
    let json_out = a.flags.contains("json");
    let ctx = Ctx::open()?;
    let mut m = to_args(&a);
    let sub = a.pos.get(1).map(String::as_str).unwrap_or("");
    let show = |v: &Value, human: &dyn Fn(&Value) -> String| {
        if json_out {
            println!("{}", pretty(v))
        } else {
            print!("{}", human(v))
        }
    };

    match cmd {
        "version" => println!("editemos {VERSION}"),
        "init" | "new" => {
            let Some(name) = rest(&a, 1) else { return fail("init needs a project name") };
            put(&mut m, "action", "create");
            put(&mut m, "name", name);
            put(&mut m, "activate", true);
            let r = ctx.project_op(&Value::Object(m.clone()))?;
            println!("created {} at {}", st(&r, "created"), st(&r, "dir"));
            if a.flags.contains("extract") {
                let mut e = Map::new();
                put(&mut e, "project", st(&r, "created"));
                for k in ["backend", "max_images", "fonts", "ignore_robots", "scrapling_mode"] {
                    if let Some(v) = m.get(k) {
                        e.insert(k.into(), v.clone());
                    }
                }
                if let Some(site) = m.get("site") {
                    put(&mut e, "url", site.clone());
                } else {
                    return fail("--extract needs --site <url>");
                }
                let rep = ctx.extract(&Value::Object(e))?;
                show(&rep, &h_extract);
            }
        }
        "use" => {
            let Some(name) = rest(&a, 1) else { return fail("use needs a project name") };
            put(&mut m, "action", "use");
            put(&mut m, "name", name);
            let r = ctx.project_op(&Value::Object(m))?;
            println!("active project: {} ({})", st(&r, "name"), st(&r, "dir"));
        }
        "which" => show(&ctx.project_op(&Value::Object(m))?, &h_which),
        "projects" => {
            put(&mut m, "action", "list");
            show(&ctx.project_op(&Value::Object(m))?, &h_projects);
        }
        "extract" => {
            if let Some(u) = a.pos.get(1) {
                put(&mut m, "url", u.clone());
            }
            show(&ctx.extract(&Value::Object(m))?, &h_extract);
        }
        "brand" => {
            put(&mut m, "action", if sub.is_empty() { "show" } else { sub });
            match sub {
                "describe" => {
                    let Some(t) = rest(&a, 2) else { return fail("brand describe needs the description, e.g. \"dark, neon green #19e3a1, font Russo One\"") };
                    put(&mut m, "action", "set");
                    put(&mut m, "description", t);
                }
                "from-images" => {
                    let files: Vec<String> = a.pos.iter().skip(2).cloned().collect();
                    put(&mut m, "files", json!(files));
                }
                _ => {}
            }
            println!("{}", pretty(&ctx.brand_op(&Value::Object(m))?));
        }
        "assets" => {
            put(&mut m, "action", if sub.is_empty() { "list" } else { sub });
            match sub {
                "search" => {
                    if let Some(q) = rest(&a, 2) {
                        put(&mut m, "query", q);
                    }
                }
                "add" => {
                    let Some(f) = a.pos.get(2) else { return fail("assets add needs a file") };
                    put(&mut m, "file", f.clone());
                }
                _ => {}
            }
            println!("{}", pretty(&ctx.assets_op(&Value::Object(m))?));
        }
        "generate" => {
            let Some(p) = rest(&a, 1) else { return fail("generate needs a prompt") };
            put(&mut m, "prompt", p);
            let r = ctx.generate(&Value::Object(m))?;
            match r.get("text").and_then(|t| t.as_str()) {
                Some(t) if !json_out => println!("{t}"),
                _ => println!("{}", pretty(&r)),
            }
        }
        "memory" => match sub {
            "add" => {
                let Some(t) = rest(&a, 2) else { return fail("memory add needs the text") };
                put(&mut m, "text", t);
                let r = ctx.memory_add(&Value::Object(m))?;
                println!("remembered {} ({})", st(&r, "id"), st(&r, "kind"));
            }
            "search" | "list" => {
                if let Some(q) = rest(&a, 2) {
                    put(&mut m, "query", q);
                }
                show(&ctx.memory_search(&Value::Object(m))?, &h_memories);
            }
            "forget" | "compact" => {
                put(&mut m, "action", sub);
                if sub == "forget" {
                    let Some(id) = a.pos.get(2) else { return fail("memory forget needs an id") };
                    put(&mut m, "id", id.clone());
                }
                println!("{}", pretty(&ctx.memory_admin(&Value::Object(m))?));
            }
            _ => return fail("memory: add | search | list | forget | compact"),
        },
        "context" => {
            let r = ctx.context_pack(&Value::Object(m.clone()))?;
            let text = st(&r, "context");
            if let Some(out) = m.get("out").and_then(|v| v.as_str()) {
                write_atomic(Path::new(out), text.as_bytes())?;
                println!("wrote {out} (~{} tokens)", st(&r, "tokens"));
            } else {
                println!("{text}");
            }
            if let Some(w) = r["written"].as_array().filter(|w| !w.is_empty()) {
                eprintln!("wrote: {}", w.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(", "));
            }
        }
        "motion" => {
            put(&mut m, "action", if sub.is_empty() { "templates" } else { sub });
            match sub {
                "new" => {
                    let Some(kind) = a.pos.get(2) else { return fail("motion new needs a template kind (editemos motion templates)") };
                    put(&mut m, "kind", kind.clone());
                    if let Some(t) = rest(&a, 3) {
                        put(&mut m, "text", t);
                    }
                    let r = ctx.motion_op(&Value::Object(m))?;
                    show(&r, &|v| format!("scene: {}\nnext:  editemos motion render \"{}\"\n", st(v, "scene"), st(v, "scene")));
                }
                "lint" | "render" | "poster" => {
                    let Some(scene) = a.pos.get(2) else { return fail("this command needs a scene .html path") };
                    put(&mut m, "scene", scene.clone());
                    let r = ctx.motion_op(&Value::Object(m))?;
                    if sub == "lint" {
                        show(&r, &h_lint);
                        if r["ok"] == false {
                            std::process::exit(2);
                        }
                    } else {
                        show(&r, &h_render);
                    }
                }
                _ => println!("{}", pretty(&ctx.motion_op(&Value::Object(m))?)),
            }
        }
        "kdenlive" => {
            put(&mut m, "action", if sub.is_empty() { "status" } else { sub });
            match sub {
                "inspect" | "draft" | "overlay" | "learn" => {
                    let Some(f) = a.pos.get(2) else { return fail("this command needs a .kdenlive file") };
                    put(&mut m, "file", f.clone());
                }
                "diff" => {
                    let (Some(x), Some(y)) = (a.pos.get(2), a.pos.get(3)) else { return fail("diff needs two project files") };
                    put(&mut m, "file", x.clone());
                    put(&mut m, "other", y.clone());
                }
                _ => {}
            }
            println!("{}", pretty(&ctx.kdenlive_op(&Value::Object(m))?));
        }
        "bench" => {
            let r = editemos::bench::run(a.flags.contains("quick"))?;
            if json_out {
                println!("{}", pretty(&r))
            } else {
                print!("{}", editemos::bench::markdown(&r))
            }
        }
        "watch" => watch(&ctx, &a, &m)?,
        "clips" if ["transcript", "transcribe", "search", "build", "story", "events", "look", "highlights", "score"].contains(&sub) => {
            let Some(f) = a.pos.get(2) else { return fail(format!("clips {sub} needs the recording (a file or a link)")) };
            put(&mut m, "action", sub);
            put(&mut m, "file", f.clone());
            if sub == "search" {
                let Some(q) = rest(&a, 3) else { return fail("clips search <recording> <words to find>") };
                put(&mut m, "query", q);
            }
            let r = ctx.clips_op(&Value::Object(m))?;
            match sub {
                "transcript" | "transcribe" if !json_out => print!("{}", st(&r, "transcript")),
                "search" if !json_out => {
                    for h in r["hits"].as_array().cloned().unwrap_or_default() {
                        println!("{:>8.1}s - {:>8.1}s  {}", h["start"].as_f64().unwrap_or(0.0), h["end"].as_f64().unwrap_or(0.0), st(&h, "text"));
                    }
                }
                _ if !json_out && r.get("clips").is_some() && r["clips"][0].get("out").is_some() => {
                    println!("typeface: {}", st(&r, "typeface"));
                    for c in r["clips"].as_array().cloned().unwrap_or_default() {
                        println!(
                            "{}\n  {} s, {} cuts, {} s of pauses removed, captions: {} ({} words), ending: {}",
                            st(&c, "out"),
                            st(&c, "duration"),
                            st(&c, "pieces"),
                            st(&c, "removed_pause_seconds"),
                            st(&c, "captions"),
                            st(&c, "words"),
                            if st(&c, "end").is_empty() { "natural".into() } else { st(&c, "end") }
                        );
                    }
                    for c in r["clips"].as_array().cloned().unwrap_or_default() {
                        for x in c["notes"].as_array().into_iter().flatten().chain(c["checks"].as_array().into_iter().flatten()) {
                            println!("check ({}): {}", st(&c, "name"), x.as_str().unwrap_or(""));
                        }
                        if c.get("contact").is_some() {
                            println!("review sheet: {}", st(&c, "contact"));
                        }
                    }
                    for n in r["notes"].as_array().cloned().unwrap_or_default() {
                        println!("note: {}", n.as_str().unwrap_or(""));
                    }
                }
                _ => println!("{}", pretty(&r)),
            }
        }
        "reference" => {
            match sub {
                "list" => put(&mut m, "action", "list"),
                "compare" => {
                    let (Some(r), Some(f)) = (a.pos.get(2), a.pos.get(3)) else { return fail("reference compare <reference name> <our cut>") };
                    put(&mut m, "action", "compare");
                    put(&mut m, "reference", r.clone());
                    put(&mut m, "file", f.clone());
                }
                "apply" => {
                    let Some(r) = a.pos.get(2) else { return fail("reference apply <reference name>") };
                    put(&mut m, "action", "apply");
                    put(&mut m, "reference", r.clone());
                }
                _ => {
                    let Some(f) = a.pos.get(1) else { return fail("reference <video file or link> [--name N] [--section \"*00:10:00-00:12:00\"]") };
                    put(&mut m, "action", "analyse");
                    put(&mut m, "file", f.clone());
                }
            }
            println!("{}", pretty(&ctx.reference_op(&Value::Object(m))?));
        }
        "fonts" => {
            put(&mut m, "action", if sub.is_empty() { "show" } else { sub });
            if let Some(f) = rest(&a, 2) {
                put(&mut m, "family", f);
            }
            let r = ctx.fonts_op(&Value::Object(m))?;
            if sub == "list" && !json_out {
                for f in r["fonts"].as_array().cloned().unwrap_or_default() {
                    println!(
                        "{:<20} {:<44} {}",
                        st(&f, "family"),
                        f["tags"].as_array().map(|t| t.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(" ")).unwrap_or_default(),
                        st(&f, "note")
                    );
                }
            } else {
                println!("{}", pretty(&r));
            }
        }
        "clips" => {
            let Some(f) = a.pos.get(1) else { return fail("clips needs the recording: editemos clips <file> [--count 5] [--aspect 9:16]") };
            put(&mut m, "file", f.clone());
            let r = ctx.clips_op(&Value::Object(m))?;
            show(&r, &|v| {
                let mut s = format!(
                    "{:.1} min analysed in {} s ({}x real time), exported in {} s\n",
                    v["duration"].as_f64().unwrap_or(0.0) / 60.0,
                    st(v, "analysis_seconds"),
                    st(v, "analysis_speed"),
                    st(v, "export_seconds")
                );
                for c in v["clips"].as_array().cloned().unwrap_or_default() {
                    s.push_str(&format!(
                        "  {:>2}. {:>8.1}s -> {:>8.1}s  score {:<5} {}\n      {}\n",
                        st(&c, "index"),
                        c["start"].as_f64().unwrap_or(0.0),
                        c["end"].as_f64().unwrap_or(0.0),
                        st(&c, "score"),
                        st(&c, "file"),
                        c["why"].as_array().map(|w| w.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join("; ")).unwrap_or_default()
                    ));
                }
                s.push_str(&format!("manifest: {}\nedl:      {}\n", st(v, "manifest"), st(v, "edl")));
                s
            });
        }
        "session" => {
            let r = ctx.session_op(&Value::Object(m))?;
            show(&r, &|v| {
                format!(
                    "session for {} with {} opened in {}\n  folder:  {}\n  command: {}\n",
                    st(v, "project"),
                    st(v, "adapter"),
                    st(v, "terminal"),
                    st(v, "dir"),
                    v["command"].as_array().map(|c| c.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(" ")).unwrap_or_default()
                )
            });
        }
        "chat" => {
            let p = ctx.project(&Value::Object(m.clone()))?;
            let adapter = m.get("adapter").and_then(|v| v.as_str()).map(String::from).or_else(|| p.config.adapter.clone()).unwrap_or_else(|| "local".into());
            editemos::session::chat(&ctx, &p, &adapter, m.get("model").and_then(|v| v.as_str()))?;
        }
        "studio" => {
            if a.flags.contains("serve") {
                let port = m.get("port").and_then(|v| v.as_f64()).map(|v| v as u16).unwrap_or(4173);
                let (listener, url) = editemos::studio::bind(port)?;
                let marker = ctx.home.root.join("studio.url");
                write_atomic(&marker, url.as_bytes())?;
                eprintln!("editemos studio on {url}  (ctrl+c to stop)");
                editemos::studio::serve(&ctx, listener);
            } else {
                let r = ctx.studio_op(&Value::Object(m))?;
                println!("studio: {}", st(&r, "url"));
            }
        }
        "start" => {
            // one command from a cold machine to a working session
            if let Some(name) = a.pos.get(1) {
                if ctx.home.open_project(name).is_err() {
                    ctx.project_op(&json!({"action": "create", "name": name, "site": m.get("site"), "activate": true}))?;
                    println!("project   created {name}");
                }
                ctx.project_op(&json!({"action": "use", "name": name}))?;
                put(&mut m, "project", name.clone());
            }
            let p = ctx.project(&Value::Object(m.clone()))?;
            println!("project   {} ({})", p.config.name, p.dir.display());
            let k = ctx.kdenlive_op(&json!({"action": "status", "start": true, "file": m.get("file")}))?;
            println!(
                "kdenlive  {}",
                if k["installed"] != true {
                    "not installed (the edit still works: motion, clips and memory do not need it)".to_string()
                } else {
                    format!(
                        "{}{}",
                        k["version"].as_str().unwrap_or("installed"),
                        if k["started"] == true {
                            ", started"
                        } else if k["running"] == true {
                            ", already open"
                        } else {
                            ", could not start"
                        }
                    )
                }
            );
            match ctx.studio_op(&Value::Object(m.clone())) {
                Ok(r) => println!("studio    {}", st(&r, "url")),
                Err(e) => println!("studio    not started: {e}"),
            }
            if !a.flags.contains("no-session") {
                match ctx.session_op(&Value::Object(m)) {
                    Ok(r) => println!("session   {} in {}", st(&r, "adapter"), st(&r, "terminal")),
                    Err(e) => println!("session   not opened: {e}"),
                }
            }
        }
        "skills" => {
            put(&mut m, "action", if sub.is_empty() { "list" } else { sub });
            match sub {
                "" | "list" => {
                    if let Some(q) = rest(&a, 2) {
                        put(&mut m, "query", q);
                    }
                    show(&ctx.skills_op(&Value::Object(m))?, &h_skills);
                }
                "read" => {
                    let Some(n) = rest(&a, 2) else { return fail("skills read needs a name") };
                    put(&mut m, "name", n);
                    let r = ctx.skills_op(&Value::Object(m))?;
                    println!("{}", st(&r, "content"));
                }
                "sync" | "enable" | "disable" | "export" => {
                    let ids: Vec<String> = a.pos.iter().skip(2).cloned().collect();
                    if !ids.is_empty() {
                        put(&mut m, "ids", json!(ids));
                    }
                    println!("{}", pretty(&ctx.skills_op(&Value::Object(m))?));
                }
                _ => println!("{}", pretty(&ctx.skills_op(&Value::Object(m))?)),
            }
        }
        "agent" | "delegate" => {
            let from = if cmd == "agent" { 2 } else { 1 };
            if cmd == "agent" && sub != "run" {
                return fail("agent: run \"task\" [--adapter A]");
            }
            let Some(t) = rest(&a, from) else { return fail("needs a task") };
            put(&mut m, "task", t);
            let r = ctx.agent_run(&Value::Object(m))?;
            println!("{}", st(&r, "output"));
            eprintln!("log: {}", st(&r, "log"));
        }
        "doctor" => show(&ctx.doctor()?, &h_doctor),
        "tools" => match sub {
            "install-scrapling" => {
                let exe = extract::scrapling::install(&ctx.home, m.get("python").and_then(|v| v.as_str()))?;
                println!("scrapling ready: {}", exe.display());
            }
            "install-whisper" => println!("{}", pretty(&editemos::transcribe::install(&ctx.home)?)),
            _ => return fail("tools: install-scrapling [--python PATH] | install-whisper"),
        },
        other => return fail(format!("unknown command '{other}'. Try: editemos help")),
    }
    Ok(())
}

fn main() {
    editemos::util::trace("start");
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if let Err(e) = run(&argv) {
        eprintln!("editemos: {e}");
        std::process::exit(1);
    }
}
