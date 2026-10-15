//! Edit kinds: an open description of a type of edit (gameplay highlights, a
//! talking story clip, a calm documentary...). A kind is a folder anyone can
//! write and share:
//!
//! ```text
//! kinds/<id>/kind.toml       what it is, its defaults, the rules a model follows
//! kinds/<id>/examples.jsonl  edits people approved, corrected or rejected (see dataset.rs)
//! kinds/<id>/adapter/        a LoRA trained on those examples, downloaded on demand
//! ```
//!
//! A kind works with any model straight away (its rules and examples go into
//! the context); when it ships an adapter and the planner is a local model, the
//! adapter is what the model is based on. Folders hold text only: never footage.

use crate::error::{fail, Result};
use crate::store::Home;
use crate::util::{sha256_hex, slugify, write_atomic};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// The kinds that ship with editemos (the same files as `kinds/` in the repository).
pub const BUILTIN: &[(&str, &str)] = &[
    ("gameplay-highlights", include_str!("../kinds/gameplay-highlights/kind.toml")),
    ("talking-story", include_str!("../kinds/talking-story/kind.toml")),
    ("calm-documentary", include_str!("../kinds/calm-documentary/kind.toml")),
];

/// Where community kinds are published: this repository's `kinds/` folder.
pub const REGISTRY: &str = "PentaVM/editemos";

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Defaults {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aspect: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transition: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub breath: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_pause: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub captions: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sfx: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip_seconds: Option<f64>,
    /// Scoreboard regions when every recording of this kind shares a layout.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score_region: Option<String>,
}

/// What a good edit of this kind measures like (`editemos reference`).
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Target {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub median_shot: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loudness_lufs: Option<f64>,
}

/// A LoRA trained for this kind. The weights are not in the folder: they are
/// downloaded from `url` and checked against `sha256`.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Adapter {
    /// The model the adapter was trained on, e.g. "Qwen/Qwen2.5-VL-7B-Instruct".
    pub base: String,
    /// "gguf" (llama.cpp, Ollama) or "safetensors" (PEFT).
    pub format: String,
    pub url: String,
    pub sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    /// How many examples it was trained on, and anything a user should know.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Kind {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub rules: Vec<String>,
    #[serde(default)]
    pub defaults: Defaults,
    #[serde(default)]
    pub target: Target,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter: Option<Adapter>,
}

pub fn dir(home: &Home) -> PathBuf {
    home.root.join("kinds")
}

pub fn parse(text: &str) -> Result<Kind> {
    toml::from_str::<Kind>(text.trim_start_matches('\u{feff}')).map_err(|e| crate::error::Error(format!("kind.toml: {e}")))
}

/// Licences a shared kind may use: its rules and examples must be free to train on.
const LICENSES: &[&str] = &["CC0-1.0", "CC-BY-4.0", "MIT", "Apache-2.0"];
const MEDIA: &[&str] = &["mp4", "mov", "mkv", "webm", "avi", "m4a", "mp3", "wav", "aac", "flac", "png", "jpg", "jpeg", "webp", "gif"];

/// Everything wrong with a kind folder before it can be shared.
pub fn validate(folder: &Path) -> Vec<String> {
    let mut problems = Vec::new();
    let text = match fs::read_to_string(folder.join("kind.toml")) {
        Ok(t) => t,
        Err(_) => return vec!["no kind.toml in the folder".into()],
    };
    let k = match parse(&text) {
        Ok(k) => k,
        Err(e) => return vec![e.0],
    };
    if k.id != slugify(&k.id) || k.id.is_empty() {
        problems.push(format!("id \"{}\" must be lower-case words joined by dashes", k.id));
    }
    if folder.file_name().map(|n| n.to_string_lossy() != k.id).unwrap_or(true) {
        problems.push(format!("the folder must be named after the id ({})", k.id));
    }
    if k.name.trim().is_empty() || k.description.trim().len() < 20 {
        problems.push("name and a description of at least a sentence are required".into());
    }
    if !LICENSES.contains(&k.license.as_str()) {
        problems.push(format!("license must be one of {} (rules and examples are training data)", LICENSES.join(", ")));
    }
    if k.rules.len() < 2 {
        problems.push("give at least two rules: they are what a model follows".into());
    }
    let flags: Vec<String> = k.rules.iter().chain(std::iter::once(&k.description)).flat_map(|r| crate::skills::scan(r)).collect();
    if !flags.is_empty() {
        problems.push(format!("the text reads like instructions to an agent rather than editing rules: {}", flags.join(", ")));
    }
    if let Some(a) = &k.adapter {
        if a.sha256.len() != 64 || !a.url.starts_with("https://") || a.base.is_empty() || !["gguf", "safetensors"].contains(&a.format.as_str()) {
            problems.push("adapter needs base, format (gguf or safetensors), an https url and its sha256".into());
        }
    }
    let mut stack = vec![folder.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                // downloaded adapter weights live here and are never shared
                if p.file_name().map(|n| n != "adapter").unwrap_or(true) {
                    stack.push(p);
                }
                continue;
            }
            let ext = p.extension().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default();
            if MEDIA.contains(&ext.as_str()) {
                problems.push(format!("{} is media: a kind holds text only (examples point at sources by link)", p.display()));
            }
        }
    }
    if folder.join("examples.jsonl").exists() {
        for (i, line) in fs::read_to_string(folder.join("examples.jsonl")).unwrap_or_default().lines().enumerate().filter(|(_, l)| !l.trim().is_empty()) {
            match serde_json::from_str::<crate::dataset::Example>(line) {
                Ok(ex) if ex.kind != k.id => problems.push(format!("examples.jsonl line {}: kind is {}, not {}", i + 1, ex.kind, k.id)),
                Ok(_) => {}
                Err(e) => problems.push(format!("examples.jsonl line {}: {e}", i + 1)),
            }
        }
    }
    problems
}

/// Put the built-in kinds where every other kind lives. A kind.toml the user has
/// edited is left alone; examples are never touched.
pub fn install_builtin(home: &Home) -> Result<()> {
    for (id, text) in BUILTIN {
        let folder = dir(home).join(id);
        let file = folder.join("kind.toml");
        let stamp = folder.join(".builtin");
        let ours = sha256_hex(text.as_bytes());
        let current = fs::read_to_string(&file).ok();
        let untouched = match (&current, fs::read_to_string(&stamp).ok()) {
            (None, _) => true,
            (Some(c), Some(s)) => sha256_hex(c.as_bytes()) == s.trim(),
            (Some(_), None) => false,
        };
        if untouched && current.as_deref() != Some(text) {
            write_atomic(&file, text.as_bytes())?;
            write_atomic(&stamp, ours.as_bytes())?;
        }
    }
    Ok(())
}

pub struct Loaded {
    pub kind: Kind,
    pub folder: PathBuf,
    pub builtin: bool,
    pub examples: usize,
}

pub fn list(home: &Home) -> Vec<Loaded> {
    let _ = install_builtin(home);
    let mut out = Vec::new();
    for e in fs::read_dir(dir(home)).into_iter().flatten().flatten() {
        let folder = e.path();
        let Ok(text) = fs::read_to_string(folder.join("kind.toml")) else { continue };
        let Ok(kind) = parse(&text) else { continue };
        let examples = fs::read_to_string(folder.join("examples.jsonl")).map(|t| t.lines().filter(|l| !l.trim().is_empty()).count()).unwrap_or(0);
        let builtin = BUILTIN.iter().any(|(id, _)| *id == kind.id);
        out.push(Loaded { kind, folder, builtin, examples });
    }
    out.sort_by(|a, b| a.kind.id.cmp(&b.kind.id));
    out
}

pub fn find(home: &Home, id: &str) -> Result<Loaded> {
    let want = slugify(id);
    list(home).into_iter().find(|k| k.kind.id == want).ok_or_else(|| crate::error::Error(format!("no kind '{id}' (kinds list shows what exists; kinds sync fetches shared ones)")))
}

/// A new kind folder to fill in, based on an existing one or on nothing.
pub fn scaffold(home: &Home, name: &str, from: Option<&Kind>) -> Result<PathBuf> {
    let id = slugify(name);
    if id.is_empty() {
        return fail("give the kind a name");
    }
    let folder = dir(home).join(&id);
    if folder.join("kind.toml").exists() {
        return fail(format!("{} already exists", folder.display()));
    }
    let mut k = from.cloned().unwrap_or_else(|| Kind {
        rules: vec!["Describe the first thing an editor of this kind always does.".into(), "And the thing they never do.".into()],
        ..Default::default()
    });
    k.id = id.clone();
    k.name = name.trim().to_string();
    if from.is_none() || k.description.is_empty() {
        k.description = "What this type of edit is, who watches it and what makes one good.".into();
    }
    k.version = "1".into();
    k.license = "CC0-1.0".into();
    k.authors = Vec::new();
    k.adapter = None;
    let text = format!("# A kind is shared as this folder (text only). See docs/KINDS.md.\n{}", toml::to_string_pretty(&k).map_err(|e| crate::error::Error(e.to_string()))?);
    write_atomic(&folder.join("kind.toml"), text.as_bytes())?;
    Ok(folder)
}

/// Fetch the shared kinds published in the registry's `kinds/` folder (text only,
/// scanned like skills). Local edits to a kind with the same id are kept.
pub fn sync(home: &Home) -> Result<Vec<String>> {
    let api = format!("https://api.github.com/repos/{REGISTRY}/contents/kinds");
    let f = crate::http::get(&api, 2 * 1024 * 1024, &[("Accept", "application/vnd.github+json")])?;
    let entries: Vec<serde_json::Value> = serde_json::from_slice(&f.bytes)?;
    let mut got = Vec::new();
    for e in entries.iter().filter(|e| e["type"] == "dir") {
        let Some(id) = e["name"].as_str() else { continue };
        let folder = dir(home).join(id);
        if folder.join("kind.toml").exists() && !folder.join(".builtin").exists() && !folder.join(".synced").exists() {
            continue; // the user's own kind of the same name
        }
        let raw = |file: &str| format!("https://raw.githubusercontent.com/{REGISTRY}/main/kinds/{id}/{file}");
        let Ok(toml_file) = crate::http::get(&raw("kind.toml"), 256 * 1024, &[]) else { continue };
        let text = String::from_utf8_lossy(&toml_file.bytes).to_string();
        if parse(&text).is_err() {
            continue;
        }
        write_atomic(&folder.join("kind.toml"), text.as_bytes())?;
        if let Ok(ex) = crate::http::get(&raw("examples.jsonl"), 16 * 1024 * 1024, &[]) {
            if ex.status == 200 {
                write_atomic(&folder.join("examples.shared.jsonl"), &ex.bytes)?;
            }
        }
        write_atomic(&folder.join(".synced"), b"from the registry\n")?;
        got.push(id.to_string());
    }
    Ok(got)
}

/// Download a kind's adapter and check it. Returns the file and how to load it.
pub fn fetch_adapter(home: &Home, id: &str) -> Result<(PathBuf, String)> {
    let k = find(home, id)?;
    let Some(a) = k.kind.adapter.clone() else { return fail(format!("{} has no adapter yet: it works through its rules and examples", k.kind.id)) };
    let name = a.url.rsplit('/').next().filter(|n| !n.is_empty()).unwrap_or("adapter.bin").to_string();
    let file = k.folder.join("adapter").join(&name);
    let ok = |p: &Path| fs::read(p).map(|b| sha256_hex(&b) == a.sha256.to_lowercase()).unwrap_or(false);
    if !ok(&file) {
        fs::create_dir_all(file.parent().unwrap_or(Path::new(".")))?;
        crate::http::download(&a.url, &file, a.bytes.unwrap_or(4 * 1024 * 1024 * 1024) + 1024, 1800)?;
        if !ok(&file) {
            let _ = fs::remove_file(&file);
            return fail("the downloaded adapter does not match its sha256; it was deleted");
        }
    }
    let how = match a.format.as_str() {
        "gguf" => format!(
            "llama.cpp: llama-server -m <{base} gguf> --lora {f}\nOllama: a Modelfile with `FROM <{base}>` and `ADAPTER {f}`, then `ollama create {id} -f Modelfile` and use adapter local with model {id}",
            base = a.base,
            f = file.display(),
            id = k.kind.id
        ),
        _ => format!("PEFT: PeftModel.from_pretrained(<{}>, \"{}\")", a.base, file.parent().unwrap_or(Path::new(".")).display()),
    };
    Ok((file, how))
}

/// The kind as a model should read it: what it is, its rules, and a few edits
/// people approved, as examples to follow.
pub fn context(home: &Home, id: &str, examples: usize) -> Option<String> {
    let k = find(home, id).ok()?;
    let mut s = format!("## Edit kind: {} ({})\n\n{}\n\n", k.kind.name, k.kind.id, k.kind.description);
    for r in &k.kind.rules {
        s.push_str(&format!("- {r}\n"));
    }
    let d = &k.kind.defaults;
    let mut defs = Vec::new();
    if let Some(v) = &d.aspect {
        defs.push(format!("aspect {v}"));
    }
    if let Some(v) = &d.layout {
        defs.push(format!("layout {v}"));
    }
    if let Some(v) = &d.transition {
        defs.push(format!("transition {v}"));
    }
    if let Some(v) = d.clip_seconds {
        defs.push(format!("about {v:.0} s a clip"));
    }
    if !defs.is_empty() {
        s.push_str(&format!("\nDefaults (clips build uses them unless told otherwise): {}.\n", defs.join(", ")));
    }
    let good: Vec<crate::dataset::Example> =
        crate::dataset::load(&k.folder).into_iter().filter(|e| e.verdict == "approved" || e.verdict == "corrected").rev().take(examples).collect();
    for e in good {
        let plan = e.correction.as_ref().unwrap_or(&e.plan);
        s.push_str(&format!("\nAn approved edit of this kind ({}):\n```json\n{}\n```\n", e.verdict, serde_json::to_string(plan).unwrap_or_default()));
    }
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_kinds_parse_and_pass_their_own_checks() {
        for (id, text) in BUILTIN {
            let k = parse(text).unwrap();
            assert_eq!(k.id, *id);
            assert!(k.rules.len() >= 2 && LICENSES.contains(&k.license.as_str()));
            assert!(k.defaults.transition.is_some(), "{id} has defaults");
        }
        let root = std::env::temp_dir().join(format!("editemos-kinds-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let home = Home { root: root.clone() };
        let all = list(&home);
        assert_eq!(all.len(), BUILTIN.len());
        for k in &all {
            assert!(validate(&k.folder).is_empty(), "{}: {:?}", k.kind.id, validate(&k.folder));
        }
        // a new kind starts from one that exists, and a video dropped in it is refused
        let base = find(&home, "gameplay-highlights").unwrap().kind;
        let f = scaffold(&home, "Fighting game combos", Some(&base)).unwrap();
        assert!(f.ends_with("fighting-game-combos"));
        assert!(validate(&f).is_empty(), "{:?}", validate(&f));
        fs::write(f.join("clip.mp4"), b"x").unwrap();
        assert!(validate(&f).iter().any(|p| p.contains("media")));
        // a user's edit of a built-in kind survives the next start
        let file = dir(&home).join("talking-story").join("kind.toml");
        let edited = fs::read_to_string(&file).unwrap().replace("Skip sponsor reads", "Keep sponsor reads");
        fs::write(&file, &edited).unwrap();
        install_builtin(&home).unwrap();
        assert!(fs::read_to_string(&file).unwrap().contains("Keep sponsor reads"));
        assert!(context(&home, "gameplay-highlights", 2).unwrap().contains("Never end a segment"));
        let _ = fs::remove_dir_all(&root);
    }
}
