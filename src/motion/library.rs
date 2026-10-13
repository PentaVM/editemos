//! The project's motion library: every scene and render is recorded once and
//! found again. Asking for the same graphic twice returns the first one instead
//! of making a second copy, and an agent can browse what already exists before
//! it designs anything new.

use crate::error::Result;
use crate::store::Project;
use crate::util::{now_iso, sha256_hex, write_atomic};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Piece {
    /// Hash of what defines the graphic: template, params, size and the edit brand.
    pub key: String,
    pub kind: String,
    #[serde(default)]
    pub text: String,
    pub scene: String,
    #[serde(default)]
    pub size: String,
    /// Renders of this scene, newest last.
    #[serde(default)]
    pub renders: Vec<Render>,
    #[serde(default)]
    pub created: String,
    /// How many times it was handed out again instead of being rebuilt.
    #[serde(default)]
    pub reused: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Render {
    /// Hash of the scene file plus the options that change pixels.
    pub key: String,
    pub file: String,
    pub format: String,
    pub fps: f64,
    pub duration: f64,
    #[serde(default)]
    pub contact: Option<String>,
    #[serde(default)]
    pub created: String,
}

fn path(p: &Project) -> PathBuf {
    p.motion_dir().join("library.json")
}

pub fn load(p: &Project) -> Vec<Piece> {
    std::fs::read_to_string(path(p)).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn save(p: &Project, v: &[Piece]) -> Result<()> {
    write_atomic(&path(p), serde_json::to_string_pretty(v)?.as_bytes())
}

pub fn scene_key(kind: &str, params: &serde_json::Value, size: &str, edit_seed: u64, edit_json: &str) -> String {
    sha256_hex(format!("{kind}|{params}|{size}|{edit_seed}|{edit_json}").as_bytes())[..16].to_string()
}

pub fn render_key(scene: &Path, opts: &str) -> String {
    let body = std::fs::read(scene).unwrap_or_default();
    let mut h = body;
    h.extend_from_slice(opts.as_bytes());
    sha256_hex(&h)[..16].to_string()
}

/// A scene with this key whose file still exists.
pub fn find_scene(p: &Project, key: &str) -> Option<Piece> {
    load(p).into_iter().find(|x| x.key == key && Path::new(&x.scene).exists())
}

pub fn record_scene(p: &Project, piece: Piece) -> Result<()> {
    let mut all = load(p);
    all.retain(|x| x.key != piece.key);
    all.push(Piece { created: now_iso(), ..piece });
    save(p, &all)
}

pub fn mark_reused(p: &Project, key: &str) {
    let mut all = load(p);
    if let Some(x) = all.iter_mut().find(|x| x.key == key) {
        x.reused += 1;
        let _ = save(p, &all);
    }
}

fn same_file(a: &str, b: &Path) -> bool {
    let norm = |s: &Path| std::fs::canonicalize(s).unwrap_or_else(|_| s.to_path_buf());
    norm(Path::new(a)) == norm(b)
}

/// A finished render of `scene` made with the same options, if its file is still there.
pub fn find_render(p: &Project, scene: &Path, key: &str) -> Option<Render> {
    load(p).into_iter().filter(|x| same_file(&x.scene, scene)).flat_map(|x| x.renders).find(|r| r.key == key && Path::new(&r.file).exists())
}

pub fn record_render(p: &Project, scene: &Path, r: Render) -> Result<()> {
    let mut all = load(p);
    let r = Render { created: now_iso(), ..r };
    match all.iter_mut().find(|x| same_file(&x.scene, scene)) {
        Some(x) => {
            x.renders.retain(|y| y.key != r.key);
            x.renders.push(r);
        }
        None => {
            all.push(Piece { key: r.key.clone(), kind: "custom".into(), scene: scene.to_string_lossy().to_string(), renders: vec![r], created: now_iso(), ..Default::default() })
        }
    }
    save(p, &all)
}

/// Drop entries whose files are gone.
pub fn prune(p: &Project) -> Result<usize> {
    let mut all = load(p);
    let before = all.len();
    all.retain(|x| Path::new(&x.scene).exists());
    for x in &mut all {
        x.renders.retain(|r| Path::new(&r.file).exists());
    }
    save(p, &all)?;
    Ok(before - all.len())
}
