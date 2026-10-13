//! Sessions: put an agent in its own window next to the edit, and a built-in chat
//! for models that have no command-line agent of their own.
//!
//! `launch` opens a new terminal window in the project's folder running the agent
//! you picked (Claude Code, Codex, Gemini CLI, or `editemos chat` for DeepSeek and
//! local models). The window is independent of whatever started it, so the model,
//! the reasoning effort or the whole vendor can change between sessions while the
//! project's memory stays the same.

use crate::adapters::{self, Adapter};
use crate::context;
use crate::error::{fail, Result};
use crate::mcp;
use crate::ops::Ctx;
use crate::store::Project;
use crate::util::{path_str, unix_secs, write_atomic};
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn quote(arg: &str) -> String {
    if !arg.is_empty() && arg.chars().all(|c| c.is_ascii_alphanumeric() || "-_=./:\\".contains(c)) {
        arg.to_string()
    } else {
        format!("\"{}\"", arg.replace('"', "\\\""))
    }
}

/// Write the launcher script for this platform and return its path.
fn write_script(dir: &Path, workdir: &Path, env: &[(String, String)], cmd: &[String], title: &str) -> Result<PathBuf> {
    let line = cmd.iter().map(|a| quote(a)).collect::<Vec<_>>().join(" ");
    if cfg!(windows) {
        let mut s = format!("@echo off\r\ntitle {title}\r\ncd /d \"{}\"\r\n", workdir.display());
        for (k, v) in env {
            s.push_str(&format!("set \"{k}={v}\"\r\n"));
        }
        s.push_str(&format!("{line}\r\n"));
        let p = dir.join(format!("session-{}.cmd", unix_secs()));
        write_atomic(&p, s.as_bytes())?;
        Ok(p)
    } else {
        let mut s = format!("#!/bin/sh\nprintf '\\033]0;{title}\\007'\ncd \"{}\" || exit 1\n", workdir.display());
        for (k, v) in env {
            s.push_str(&format!("export {k}=\"{v}\"\n"));
        }
        s.push_str(&format!("exec {line}\n"));
        let p = dir.join(format!("session-{}.sh", unix_secs()));
        write_atomic(&p, s.as_bytes())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755))?;
        }
        Ok(p)
    }
}

/// Open `script` in a new terminal window that outlives this process.
fn open_window(script: &Path) -> Result<String> {
    let quiet = |c: &mut Command| c.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().is_ok();
    if cfg!(windows) {
        // Windows Terminal when present, the classic console otherwise
        if quiet(Command::new("wt").args(["-w", "new", "cmd", "/k"]).arg(script)) {
            return Ok("Windows Terminal".into());
        }
        if quiet(Command::new("cmd").args(["/C", "start", "editemos", "cmd", "/K"]).arg(script)) {
            return Ok("cmd".into());
        }
        return fail("could not open a console window");
    }
    if cfg!(target_os = "macos") {
        if quiet(Command::new("open").args(["-a", "Terminal"]).arg(script)) {
            return Ok("Terminal".into());
        }
        return fail("could not open Terminal");
    }
    let sp = path_str(script);
    let candidates: [(&str, Vec<&str>); 8] = [
        ("x-terminal-emulator", vec!["-e", &sp]),
        ("gnome-terminal", vec!["--", &sp]),
        ("konsole", vec!["-e", &sp]),
        ("kitty", vec![&sp]),
        ("alacritty", vec!["-e", &sp]),
        ("wezterm", vec!["start", "--", &sp]),
        ("foot", vec![&sp]),
        ("xterm", vec!["-e", &sp]),
    ];
    for (term, args) in candidates {
        if quiet(Command::new(term).args(args)) {
            return Ok(term.to_string());
        }
    }
    fail(format!("no terminal emulator found; run the session yourself: {sp}"))
}

pub struct Launch {
    pub adapter: String,
    pub model: String,
    pub effort: String,
    pub task: Option<String>,
    /// Folder the agent works in (default: the project folder).
    pub dir: Option<PathBuf>,
    /// Only write the script and say what would run.
    pub dry_run: bool,
}

pub fn launch(ctx: &Ctx, p: &Project, l: &Launch) -> Result<Value> {
    let ad: Adapter = adapters::get(&ctx.home, &l.adapter)?;
    let workdir = l.dir.clone().unwrap_or_else(|| p.dir.clone());
    if !workdir.is_dir() {
        return fail(format!("{} is not a folder", workdir.display()));
    }
    // every agent finds the project's context under the name it looks for
    let written = context::write_agent_files(&ctx.home, p, &workdir, &context::PackOptions { task: l.task.clone(), ..Default::default() })?;

    let exe = path_str(&std::env::current_exe()?);
    let cmd: Vec<String> = match ad.kind.as_str() {
        "openai-chat" => {
            let mut c = vec![exe, "chat".into(), "--adapter".into(), l.adapter.clone(), "--project".into(), p.config.slug.clone()];
            if !l.model.is_empty() {
                c.extend(["--model".into(), l.model.clone()]);
            }
            c
        }
        _ if !ad.interactive.is_empty() => adapters::interactive_command(&ad, &l.model, &l.effort),
        _ => return fail(format!("adapter '{}' has no interactive mode; set `interactive = [...]` for it in adapters.toml", l.adapter)),
    };
    let mut env = vec![("EDITEMOS_PROJECT".to_string(), p.config.slug.clone())];
    if !ad.effort_env.is_empty() && !l.effort.is_empty() {
        env.push((ad.effort_env.clone(), l.effort.clone()));
    }
    let title = format!("editemos - {} - {}", p.config.slug, l.adapter);
    let script = write_script(&p.runs_dir(), &workdir, &env, &cmd, &title)?;
    let terminal = if l.dry_run { "(dry run)".to_string() } else { open_window(&script)? };
    Ok(json!({
        "project": p.config.slug, "adapter": l.adapter, "model": l.model, "effort": l.effort,
        "command": cmd, "dir": path_str(&workdir), "script": path_str(&script), "terminal": terminal, "context_files": written,
    }))
}

/// The editemos tools in the shape the OpenAI chat API expects.
pub fn openai_tools() -> Value {
    Value::Array(
        mcp::tools().into_iter().map(|t| json!({"type": "function", "function": {"name": t["name"], "description": t["description"], "parameters": t["inputSchema"]}})).collect(),
    )
}

/// One user turn: let the model call tools until it answers in text.
pub fn turn(ctx: &Ctx, ad: &Adapter, messages: &mut Vec<Value>, tools: &Value, mut on_tool: impl FnMut(&str, &Value)) -> Result<String> {
    for _ in 0..16 {
        let resp = adapters::chat_completion(ad, &Value::Array(messages.clone()), Some(tools))?;
        let msg = resp["choices"][0]["message"].clone();
        if msg.is_null() {
            return fail(format!("the model returned no message: {}", crate::util::truncate_chars(&resp.to_string(), 300)));
        }
        messages.push(msg.clone());
        let calls = msg["tool_calls"].as_array().cloned().unwrap_or_default();
        if calls.is_empty() {
            return Ok(msg["content"].as_str().unwrap_or("").to_string());
        }
        for c in calls {
            let name = c["function"]["name"].as_str().unwrap_or("").to_string();
            let args: Value = c["function"]["arguments"].as_str().and_then(|s| serde_json::from_str(s).ok()).unwrap_or_else(|| json!({}));
            on_tool(&name, &args);
            let result = mcp::call_tool(ctx, &name, &args);
            let text = result["content"][0]["text"].as_str().unwrap_or("").to_string();
            messages.push(json!({"role": "tool", "tool_call_id": c["id"], "content": crate::util::truncate_chars(&text, 16_000)}));
        }
    }
    fail("the model kept calling tools without answering (16 rounds)")
}

/// Interactive chat on the terminal with any OpenAI-compatible model.
pub fn chat(ctx: &Ctx, p: &Project, adapter: &str, model: Option<&str>) -> Result<()> {
    let mut ad = adapters::get(&ctx.home, adapter)?;
    if let Some(m) = model {
        ad.model = m.to_string();
    }
    let pack = context::build(&ctx.home, p, &context::PackOptions::default());
    let system = format!("{}\n\n{pack}\n\nYou are working in the project '{}'. Pass project=\"{}\" to the tools.", mcp::INSTRUCTIONS, p.config.name, p.config.slug);
    let mut messages = vec![json!({"role": "system", "content": system})];
    let tools = openai_tools();
    println!("editemos chat - project {} - {} ({})\ntype /exit to leave\n", p.config.slug, adapter, ad.model);
    let stdin = std::io::stdin();
    loop {
        print!("> ");
        std::io::stdout().flush()?;
        let mut line = String::new();
        if stdin.lock().read_line(&mut line)? == 0 {
            break;
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line == "/exit" || line == "/quit" {
            break;
        }
        messages.push(json!({"role": "user", "content": line}));
        match turn(ctx, &ad, &mut messages, &tools, |name, args| println!("  [{name} {}]", crate::util::truncate_chars(&args.to_string(), 120))) {
            Ok(answer) => println!("\n{answer}\n"),
            Err(e) => {
                println!("\nerror: {e}\n");
                messages.pop();
            }
        }
    }
    Ok(())
}
