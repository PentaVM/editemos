//! Speech to text with a time for every word, on this machine.
//!
//! Site captions are good enough to *find* a moment and not good enough to *show*:
//! they roll, they have no punctuation worth the name and their timing is per line.
//! What ends up on screen is transcribed here with whisper.cpp, which runs locally
//! and returns the start and end of each word. Only the finished clip's audio is
//! transcribed, so a three-hour stream costs one minute of speech, not three hours.

use crate::error::{fail, Result};
use crate::http;
use crate::motion::render::find_ffmpeg;
use crate::store::Home;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Word {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

const MODEL_FILE: &str = "ggml-small-q5_1.bin";
const MODEL_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small-q5_1.bin";

fn tool_dir(home: &Home) -> PathBuf {
    home.tools_dir().join("whisper")
}

fn find_file(dir: &Path, name: &str, depth: u32) -> Option<PathBuf> {
    if depth > 4 {
        return None;
    }
    let rd = std::fs::read_dir(dir).ok()?;
    let mut dirs = Vec::new();
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            dirs.push(p);
        } else if e.file_name().to_string_lossy().eq_ignore_ascii_case(name) {
            return Some(p);
        }
    }
    dirs.into_iter().find_map(|d| find_file(&d, name, depth + 1))
}

/// The whisper.cpp command-line program: EDITEMOS_WHISPER, the copy installed by
/// `editemos tools install-whisper`, or one on PATH (Homebrew, a distro package).
pub fn locate_cli(home: &Home) -> Option<PathBuf> {
    if let Ok(p) = std::env::var("EDITEMOS_WHISPER") {
        if Path::new(&p).exists() {
            return Some(PathBuf::from(p));
        }
    }
    for name in ["whisper-cli.exe", "whisper-cli", "main.exe"] {
        if let Some(p) = find_file(&tool_dir(home), name, 0) {
            return Some(p);
        }
    }
    let names = ["whisper-cli", "whisper-cpp", "whisper-cli.exe"];
    std::env::var_os("PATH").and_then(|paths| std::env::split_paths(&paths).flat_map(|d| names.iter().map(move |n| d.join(n))).find(|p| p.is_file()))
}

pub fn locate_model(home: &Home) -> Option<PathBuf> {
    if let Ok(p) = std::env::var("EDITEMOS_WHISPER_MODEL") {
        if Path::new(&p).exists() {
            return Some(PathBuf::from(p));
        }
    }
    let d = tool_dir(home);
    let preferred = d.join(MODEL_FILE);
    if preferred.exists() {
        return Some(preferred);
    }
    std::fs::read_dir(&d)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.file_name().map(|n| n.to_string_lossy().starts_with("ggml-") && n.to_string_lossy().ends_with(".bin")).unwrap_or(false))
}

pub fn available(home: &Home) -> bool {
    locate_cli(home).is_some() && locate_model(home).is_some()
}

/// Install whisper.cpp and a multilingual model (about 185 MB) under the editemos home.
/// The program is downloaded for Windows; elsewhere install `whisper-cpp` with your
/// package manager and only the model is fetched.
pub fn install(home: &Home) -> Result<serde_json::Value> {
    let d = tool_dir(home);
    std::fs::create_dir_all(&d)?;
    let mut notes: Vec<String> = Vec::new();
    if locate_cli(home).is_none() {
        if !cfg!(windows) {
            return fail("install whisper.cpp first (macOS: `brew install whisper-cpp`; Linux: your package manager or build https://github.com/ggml-org/whisper.cpp), then run this again for the model");
        }
        let headers: Vec<(String, String)> = http::github_token().map(|t| vec![("Authorization".to_string(), format!("Bearer {t}"))]).unwrap_or_default();
        let refs: Vec<(&str, &str)> = headers.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
        let rel = http::get("https://api.github.com/repos/ggml-org/whisper.cpp/releases/latest", 4 * 1024 * 1024, &refs)?;
        let rel: serde_json::Value = serde_json::from_slice(&rel.bytes)?;
        let asset = rel["assets"]
            .as_array()
            .and_then(|a| a.iter().find(|x| x["name"].as_str().map(|n| n.eq_ignore_ascii_case("whisper-bin-x64.zip")).unwrap_or(false)))
            .and_then(|x| x["browser_download_url"].as_str());
        let Some(url) = asset else { return fail("the whisper.cpp release has no Windows x64 build; set EDITEMOS_WHISPER to a whisper-cli you installed") };
        let zip = d.join("whisper-bin-x64.zip");
        http::download(url, &zip, 200 * 1024 * 1024, 600)?;
        // bsdtar ships with Windows 10 and later and reads zip files
        let ok = Command::new("tar")
            .arg("-xf")
            .arg(&zip)
            .arg("-C")
            .arg(&d)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        let _ = std::fs::remove_file(&zip);
        if !ok || locate_cli(home).is_none() {
            return fail("could not unpack whisper.cpp (tar failed); unpack it yourself and set EDITEMOS_WHISPER");
        }
        notes.push(format!("whisper.cpp {} installed", rel["tag_name"].as_str().unwrap_or("")));
    }
    if locate_model(home).is_none() {
        let n = http::download(MODEL_URL, &d.join(MODEL_FILE), 400 * 1024 * 1024, 1800)?;
        notes.push(format!("model {MODEL_FILE} downloaded ({} MB)", n / 1024 / 1024));
    }
    Ok(serde_json::json!({"cli": locate_cli(home).map(|p| p.to_string_lossy().to_string()), "model": locate_model(home).map(|p| p.to_string_lossy().to_string()), "notes": notes}))
}

/// Merge whisper's sub-word tokens into words. A token that starts with a space
/// starts a word; punctuation sticks to the word before it.
pub fn words_from_tokens(tokens: &[(f64, f64, String)]) -> Vec<Word> {
    let mut out: Vec<Word> = Vec::new();
    // sound descriptions such as "(upbeat music)" or "[laughter]" span several tokens
    let mut aside = false;
    for (from, to, text) in tokens {
        if text.trim().is_empty() || text.trim_start().starts_with("[_") || text.trim_start().starts_with("<|") {
            continue;
        }
        if aside || text.contains(['(', '[']) {
            aside = !text.contains([')', ']']);
            continue;
        }
        // "$", "#", an opening quote: these begin the next word ("$100"), they do not end the last one
        let opener = text.starts_with(' ') && text.trim().chars().all(|c| "$€£¥#@\"“‘".contains(c));
        let glue = !opener && (!text.starts_with(' ') || text.trim().chars().all(|c| !c.is_alphanumeric()));
        match out.last_mut() {
            Some(w) if glue => {
                w.text.push_str(text.trim());
                w.end = *to;
            }
            _ => out.push(Word { start: *from, end: *to, text: text.trim().to_string() }),
        }
    }
    // bracketed sound tags ("[Music]", "(laughs)") are not speech
    out.retain(|w| !(w.text.starts_with('[') || w.text.starts_with('(') || w.text.starts_with('*')));
    // a word cannot end before it starts or run into the next one
    for i in 0..out.len() {
        if out[i].end <= out[i].start {
            out[i].end = out[i].start + 0.12;
        }
        if i + 1 < out.len() && out[i].end > out[i + 1].start {
            out[i].end = out[i + 1].start.max(out[i].start + 0.05);
        }
    }
    out
}

/// Transcribe a media file. `language` is a code such as "en" or "es", or "auto".
pub fn words(home: &Home, media: &Path, language: &str) -> Result<Vec<Word>> {
    let (Some(cli), Some(model)) = (locate_cli(home), locate_model(home)) else {
        return fail("whisper is not installed: run `editemos tools install-whisper`");
    };
    let Some(ffmpeg) = find_ffmpeg() else { return fail("ffmpeg not found") };
    let work = std::env::temp_dir().join(format!("editemos-asr-{}", crate::util::short_id()));
    std::fs::create_dir_all(&work)?;
    let wav = work.join("a.wav");
    let conv = Command::new(&ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(media)
        .args(["-vn", "-ac", "1", "-ar", "16000", "-c:a", "pcm_s16le"])
        .arg(&wav)
        .stdin(Stdio::null())
        .output()?;
    if !conv.status.success() {
        let _ = std::fs::remove_dir_all(&work);
        return fail(format!("could not read the audio: {}", crate::util::truncate_chars(String::from_utf8_lossy(&conv.stderr).trim(), 200)));
    }
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(2, 8).to_string();
    let base = work.join("out");
    let run = Command::new(&cli)
        .arg("-m")
        .arg(&model)
        .arg("-f")
        .arg(&wav)
        .args(["-l", if language.is_empty() { "auto" } else { language }, "-t", &threads, "-ojf", "-np", "-of"])
        .arg(&base)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()?;
    let json = std::fs::read_to_string(base.with_extension("json"));
    let _ = std::fs::remove_dir_all(&work);
    let Ok(json) = json else {
        return fail(format!("whisper produced no transcript: {}", crate::util::truncate_chars(String::from_utf8_lossy(&run.stderr).trim(), 300)));
    };
    let v: serde_json::Value = serde_json::from_str(&json)?;
    let mut tokens: Vec<(f64, f64, String)> = Vec::new();
    for seg in v["transcription"].as_array().cloned().unwrap_or_default() {
        match seg["tokens"].as_array() {
            Some(toks) if !toks.is_empty() => {
                for t in toks {
                    let from = t["offsets"]["from"].as_f64().unwrap_or(0.0) / 1000.0;
                    let to = t["offsets"]["to"].as_f64().unwrap_or(0.0) / 1000.0;
                    tokens.push((from, to, t["text"].as_str().unwrap_or("").to_string()));
                }
            }
            _ => {
                // no token timing: spread the line's words over the line
                let (from, to) = (seg["offsets"]["from"].as_f64().unwrap_or(0.0) / 1000.0, seg["offsets"]["to"].as_f64().unwrap_or(0.0) / 1000.0);
                let ws: Vec<&str> = seg["text"].as_str().unwrap_or("").split_whitespace().collect();
                let step = (to - from) / ws.len().max(1) as f64;
                for (i, w) in ws.iter().enumerate() {
                    tokens.push((from + step * i as f64, from + step * (i + 1) as f64, format!(" {w}")));
                }
            }
        }
    }
    Ok(words_from_tokens(&tokens))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_currency_sign_belongs_to_its_number() {
        let t = |a: f64, b: f64, s: &str| (a, b, s.to_string());
        let w = words_from_tokens(&[t(0.0, 0.2, " offer"), t(0.2, 0.3, " him"), t(0.3, 0.4, " a"), t(0.4, 0.45, " $"), t(0.45, 0.8, "100"), t(0.8, 0.85, ","), t(0.9, 1.0, " ok")]);
        assert_eq!(w.iter().map(|x| x.text.as_str()).collect::<Vec<_>>(), vec!["offer", "him", "a", "$100,", "ok"]);
    }

    #[test]
    fn sound_descriptions_are_not_speech() {
        let t = |a: f64, b: f64, s: &str| (a, b, s.to_string());
        let w = words_from_tokens(&[
            t(0.0, 0.2, " (up"),
            t(0.2, 0.4, "beat"),
            t(0.4, 0.8, " music)"),
            t(1.0, 1.3, " Nice"),
            t(1.3, 1.4, "."),
            t(1.5, 1.9, " [laughter]"),
            t(2.0, 2.3, " Go"),
        ]);
        assert_eq!(w.iter().map(|x| x.text.as_str()).collect::<Vec<_>>(), vec!["Nice.", "Go"]);
    }

    #[test]
    fn tokens_become_words() {
        let t = |a: f64, b: f64, s: &str| (a, b, s.to_string());
        let w = words_from_tokens(&[
            t(0.0, 0.0, "[_BEG_]"),
            t(0.1, 0.4, " What"),
            t(0.4, 0.6, " is"),
            t(0.6, 0.8, " his"),
            t(0.8, 1.0, " name"),
            t(1.0, 1.05, "?"),
            t(1.3, 1.5, " Va"),
            t(1.5, 1.7, "tira"),
            t(1.7, 1.75, "."),
            t(2.0, 2.4, " [Music]"),
        ]);
        let text: Vec<&str> = w.iter().map(|x| x.text.as_str()).collect();
        assert_eq!(text, vec!["What", "is", "his", "name?", "Vatira."]);
        assert_eq!(w[4].start, 1.3);
        assert_eq!(w[4].end, 1.75);
    }
}
