//! Adapters: how editemos talks to AI. Nothing here is tied to one vendor.
//!
//! * `exec` adapters run any command-line agent (Claude Code, Codex, Gemini CLI,
//!   Ollama, your own script) with the prompt on stdin or in an argument.
//! * `openai-image` and `gemini-image` call image APIs with a key from the
//!   environment (never stored on disk).
//! * `echo` returns the prompt, for tests and dry runs.
//!
//! Override or add adapters in `~/.editemos/adapters.toml`.

use crate::error::{fail, Error, Result};
use crate::http;
use crate::model::Brand;
use crate::store::Home;
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Deserialize, Serialize, Clone, Debug, Default)]
pub struct Adapter {
    pub kind: String,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub stdin: bool,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub env_key: String,
    #[serde(default)]
    pub timeout_s: u64,
    #[serde(default)]
    pub note: String,
    /// Command line for an interactive session in its own window. `{model}` and
    /// `{effort}` are filled in; an argument whose placeholder is empty is dropped
    /// together with the flag before it.
    #[serde(default)]
    pub interactive: Vec<String>,
    /// Environment variable that carries the reasoning effort, if the CLI reads one.
    #[serde(default)]
    pub effort_env: String,
}

fn chat(url: &str, model: &str, env_key: &str, note: &str) -> Adapter {
    Adapter { kind: "openai-chat".into(), url: url.into(), model: model.into(), env_key: env_key.into(), timeout_s: 300, note: note.into(), ..Default::default() }
}

#[derive(Deserialize, Default)]
struct AdaptersFile {
    #[serde(default)]
    adapters: BTreeMap<String, Adapter>,
}

fn exec(command: &str, args: &[&str], stdin: bool, note: &str) -> Adapter {
    Adapter { kind: "exec".into(), command: command.into(), args: args.iter().map(|s| s.to_string()).collect(), stdin, timeout_s: 900, note: note.into(), ..Default::default() }
}

pub fn builtin() -> BTreeMap<String, Adapter> {
    let mut m = BTreeMap::new();
    let with = |mut a: Adapter, interactive: &[&str], effort_env: &str| {
        a.interactive = interactive.iter().map(|s| s.to_string()).collect();
        a.effort_env = effort_env.into();
        a
    };
    m.insert(
        "claude".into(),
        with(exec("claude", &["-p"], true, "Claude Code, non-interactive (`claude -p`), prompt on stdin"), &["claude", "--model", "{model}"], "CLAUDE_CODE_EFFORT_LEVEL"),
    );
    m.insert(
        "codex".into(),
        with(exec("codex", &["exec", "-"], true, "OpenAI Codex CLI (`codex exec -`), prompt on stdin"), &["codex", "-m", "{model}", "-c", "model_reasoning_effort={effort}"], ""),
    );
    m.insert("gemini".into(), with(exec("gemini", &["-p", "{prompt}"], false, "Gemini CLI (`gemini -p`)"), &["gemini", "-m", "{model}"], ""));
    // Anything that speaks the OpenAI chat API, hosted or on this machine. These run
    // inside `editemos chat`, which gives the model the editemos tools directly.
    m.insert("deepseek".into(), chat("https://api.deepseek.com/chat/completions", "deepseek-chat", "DEEPSEEK_API_KEY", "DeepSeek API"));
    m.insert("openai".into(), chat("https://api.openai.com/v1/chat/completions", "gpt-5", "OPENAI_API_KEY", "OpenAI chat API"));
    m.insert("openrouter".into(), chat("https://openrouter.ai/api/v1/chat/completions", "openrouter/auto", "OPENROUTER_API_KEY", "OpenRouter: any hosted model"));
    m.insert("local".into(), chat("http://127.0.0.1:11434/v1/chat/completions", "llama3.1", "", "Local model through Ollama's OpenAI endpoint (no key)"));
    m.insert("lmstudio".into(), chat("http://127.0.0.1:1234/v1/chat/completions", "local-model", "", "Local model through LM Studio or llama.cpp server (no key)"));
    m.insert("ollama".into(), exec("ollama", &["run", "llama3.1"], true, "Local model through Ollama"));
    m.insert("echo".into(), Adapter { kind: "echo".into(), note: "returns the prompt; for tests and dry runs".into(), ..Default::default() });
    m.insert(
        "openai-image".into(),
        Adapter {
            kind: "openai-image".into(),
            url: "https://api.openai.com/v1/images/generations".into(),
            model: "gpt-image-1".into(),
            env_key: "OPENAI_API_KEY".into(),
            timeout_s: 180,
            note: "OpenAI Images API".into(),
            ..Default::default()
        },
    );
    m.insert(
        "gemini-image".into(),
        Adapter {
            kind: "gemini-image".into(),
            url: "https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent".into(),
            model: "gemini-2.5-flash-image".into(),
            env_key: "GEMINI_API_KEY".into(),
            timeout_s: 180,
            note: "Gemini image generation".into(),
            ..Default::default()
        },
    );
    m
}

pub fn load(home: &Home) -> BTreeMap<String, Adapter> {
    let mut all = builtin();
    if let Ok(text) = std::fs::read_to_string(home.adapters_path()) {
        if let Ok(f) = toml::from_str::<AdaptersFile>(&text) {
            for (k, v) in f.adapters {
                all.insert(k, v);
            }
        }
    }
    all
}

pub fn get(home: &Home, name: &str) -> Result<Adapter> {
    let all = load(home);
    match all.get(name) {
        Some(a) => Ok(a.clone()),
        None => fail(format!("unknown adapter '{name}'. Available: {}", all.keys().cloned().collect::<Vec<_>>().join(", "))),
    }
}

/// Run a text adapter and return its stdout.
pub fn run_text(a: &Adapter, prompt: &str) -> Result<String> {
    match a.kind.as_str() {
        "echo" => Ok(prompt.to_string()),
        "exec" => {
            if a.command.is_empty() {
                return fail("exec adapter without a command");
            }
            let args: Vec<String> = a.args.iter().map(|s| s.replace("{prompt}", prompt)).collect();
            let mut cmd = Command::new(&a.command);
            cmd.args(&args).stdin(if a.stdin { Stdio::piped() } else { Stdio::null() }).stdout(Stdio::piped()).stderr(Stdio::piped());
            let mut child = cmd.spawn().map_err(|e| Error(format!("could not start `{}`: {e}", a.command)))?;
            if a.stdin {
                if let Some(mut si) = child.stdin.take() {
                    let _ = si.write_all(prompt.as_bytes());
                }
            }
            let limit = Duration::from_secs(if a.timeout_s == 0 { 900 } else { a.timeout_s });
            let t0 = Instant::now();
            loop {
                match child.try_wait()? {
                    Some(_) => break,
                    None if t0.elapsed() > limit => {
                        let _ = child.kill();
                        return fail(format!("`{}` timed out after {}s", a.command, limit.as_secs()));
                    }
                    None => std::thread::sleep(Duration::from_millis(100)),
                }
            }
            let out = child.wait_with_output()?;
            if !out.status.success() {
                return fail(format!("`{}` exited with {}: {}", a.command, out.status, crate::util::truncate_chars(String::from_utf8_lossy(&out.stderr).trim(), 400)));
            }
            Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
        }
        "openai-chat" => {
            let v = chat_completion(a, &json!([{"role": "user", "content": prompt}]), None)?;
            Ok(v["choices"][0]["message"]["content"].as_str().unwrap_or("").trim().to_string())
        }
        other => fail(format!("adapter kind '{other}' does not generate text")),
    }
}

/// One call to an OpenAI-compatible chat endpoint. Returns the raw response.
pub fn chat_completion(a: &Adapter, messages: &Value, tools: Option<&Value>) -> Result<Value> {
    if a.kind != "openai-chat" {
        return fail(format!("adapter kind '{}' has no chat API; use an openai-chat adapter (deepseek, openai, openrouter, local, lmstudio)", a.kind));
    }
    let mut body = json!({"model": a.model, "messages": messages});
    if let Some(t) = tools {
        body["tools"] = t.clone();
    }
    let key = if a.env_key.is_empty() { String::new() } else { api_key(a)? };
    let auth = format!("Bearer {key}");
    let headers: Vec<(&str, &str)> = if key.is_empty() { Vec::new() } else { vec![("Authorization", auth.as_str())] };
    let text = http::post_json(&a.url, &body, &headers, if a.timeout_s == 0 { 300 } else { a.timeout_s })?;
    Ok(serde_json::from_str(&text)?)
}

/// The command for an interactive session, with model and effort filled in.
pub fn interactive_command(a: &Adapter, model: &str, effort: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for arg in &a.interactive {
        let needs_model = arg.contains("{model}");
        let needs_effort = arg.contains("{effort}");
        if (needs_model && model.is_empty()) || (needs_effort && effort.is_empty()) {
            // drop the flag that introduced this value too (`--model`, `-m`, `-c`)
            if out.last().map(|l| l.starts_with('-')).unwrap_or(false) {
                out.pop();
            }
            continue;
        }
        out.push(arg.replace("{model}", model).replace("{effort}", effort));
    }
    out
}

fn api_key(a: &Adapter) -> Result<String> {
    let name = if a.env_key.is_empty() { "API_KEY" } else { &a.env_key };
    match std::env::var(name) {
        Ok(k) if !k.trim().is_empty() => Ok(k),
        _ => fail(format!("set the {name} environment variable to use this adapter (keys are never stored)")),
    }
}

/// Ask an image adapter for images. Returns raw image bytes.
pub fn generate_images(a: &Adapter, prompt: &str, size: &str, n: u32) -> Result<Vec<Vec<u8>>> {
    let b64 = base64::engine::general_purpose::STANDARD;
    match a.kind.as_str() {
        "openai-image" => {
            let key = api_key(a)?;
            let body = json!({"model": a.model, "prompt": prompt, "n": n.max(1), "size": size});
            let text = http::post_json(&a.url, &body, &[("Authorization", &format!("Bearer {key}"))], if a.timeout_s == 0 { 180 } else { a.timeout_s })?;
            let v: Value = serde_json::from_str(&text)?;
            let mut out = Vec::new();
            for item in v["data"].as_array().cloned().unwrap_or_default() {
                if let Some(s) = item["b64_json"].as_str() {
                    out.push(b64.decode(s)?);
                } else if let Some(u) = item["url"].as_str() {
                    out.push(http::get(u, 20 * 1024 * 1024, &[])?.bytes);
                }
            }
            if out.is_empty() {
                return fail(format!("no images in the response: {}", crate::util::truncate_chars(&text, 300)));
            }
            Ok(out)
        }
        "gemini-image" => {
            let key = api_key(a)?;
            let url = a.url.replace("{model}", &a.model);
            let body = json!({"contents": [{"parts": [{"text": prompt}]}], "generationConfig": {"responseModalities": ["IMAGE"]}});
            let text = http::post_json(&url, &body, &[("x-goog-api-key", &key)], if a.timeout_s == 0 { 180 } else { a.timeout_s })?;
            let v: Value = serde_json::from_str(&text)?;
            let mut out = Vec::new();
            for cand in v["candidates"].as_array().cloned().unwrap_or_default() {
                for part in cand["content"]["parts"].as_array().cloned().unwrap_or_default() {
                    let data = part["inlineData"]["data"].as_str().or(part["inline_data"]["data"].as_str());
                    if let Some(s) = data {
                        out.push(b64.decode(s)?);
                    }
                }
            }
            if out.is_empty() {
                return fail(format!("no images in the response: {}", crate::util::truncate_chars(&text, 300)));
            }
            Ok(out)
        }
        other => fail(format!("adapter kind '{other}' does not generate images")),
    }
}

/// The on-brand prompt: the project's identity wrapped around the user's request.
pub fn brand_prompt(brand: &Brand, avoid: &[String], kind: &str, request: &str) -> String {
    let mut p = String::new();
    p.push_str(&format!("Create {kind} for the brand \"{}\".\n", brand.name));
    if let Some(t) = &brand.tagline {
        p.push_str(&format!("Tagline: {t}\n"));
    }
    if let Some(d) = &brand.description {
        p.push_str(&format!("About the brand: {}\n", crate::util::truncate_chars(d, 240)));
    }
    if !brand.voice.descriptors.is_empty() {
        p.push_str(&format!("Voice and attitude: {}.\n", brand.voice.descriptors.join(", ")));
    }
    if !brand.palette.is_empty() {
        let pal: Vec<String> = brand.palette.iter().filter(|c| c.role != "neutral").map(|c| format!("{} {}", c.role, c.hex)).collect();
        p.push_str(&format!("Palette (use these colours, no others): {}.\n", pal.join(", ")));
    }
    let heading = &brand.edit.type_tokens.heading;
    if !heading.is_empty() {
        p.push_str(&format!("Typographic character: {heading}-like headlines. Do not render any text unless the request asks for it.\n"));
    }
    if !brand.edit.tempo.is_empty() {
        p.push_str(&format!("Energy: {} pacing and composition.\n", brand.edit.tempo));
    }
    p.push_str("Avoid: generic stock-photo look, neon gradients, glassmorphism, cluttered layouts, watermarks");
    for a in avoid.iter().take(6) {
        p.push_str(&format!("; {a}"));
    }
    p.push_str(".\n");
    p.push_str(&format!("\nRequest: {request}\n"));
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ColorToken, Voice};

    #[test]
    fn echo_and_unknown() {
        let a = &builtin()["echo"];
        assert_eq!(run_text(a, "hola").unwrap(), "hola");
        assert!(generate_images(a, "x", "1024x1024", 1).is_err());
    }

    #[test]
    fn exec_runs_a_command() {
        let a = if cfg!(windows) {
            Adapter { kind: "exec".into(), command: "cmd".into(), args: vec!["/c".into(), "echo".into(), "{prompt}".into()], timeout_s: 20, ..Default::default() }
        } else {
            Adapter { kind: "exec".into(), command: "echo".into(), args: vec!["{prompt}".into()], timeout_s: 20, ..Default::default() }
        };
        assert_eq!(run_text(&a, "pong").unwrap(), "pong");
        let bad = Adapter { kind: "exec".into(), command: "definitely-not-a-real-binary-xyz".into(), ..Default::default() };
        assert!(run_text(&bad, "x").is_err());
    }

    #[test]
    fn prompt_carries_the_brand() {
        let b = Brand {
            name: "Acme".into(),
            palette: vec![ColorToken { role: "primary".into(), hex: "#ff3d1f".into(), weight: 1.0, source: "t".into() }],
            voice: Voice { descriptors: vec!["short, direct sentences".into()], ..Default::default() },
            ..Default::default()
        };
        let p = brand_prompt(&b, &["no stock people".into()], "a 16:9 poster", "launch day");
        assert!(p.contains("Acme") && p.contains("#ff3d1f") && p.contains("no stock people") && p.contains("launch day"));
    }
}
