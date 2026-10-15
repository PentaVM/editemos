//! Data model persisted per project. Everything is plain JSON/TOML/JSONL on disk.

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct ProjectConfig {
    pub name: String,
    pub slug: String,
    #[serde(default)]
    pub created: String,
    #[serde(default)]
    pub description: String,
    /// Website the brand was (or will be) extracted from.
    #[serde(default)]
    pub site: Option<String>,
    /// Default frame size for motion renders, e.g. "1920x1080".
    #[serde(default)]
    pub size: Option<String>,
    #[serde(default)]
    pub fps: Option<f64>,
    /// Skill ids enabled for this project (see `editemos skills`).
    #[serde(default)]
    pub skills: Vec<String>,
    /// Preferred agent adapter for `editemos agent run`.
    #[serde(default)]
    pub adapter: Option<String>,
    /// The edit kind this project makes (see `editemos kinds`): its rules go into the
    /// context, its defaults into `clips build`, and its examples are recorded under it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct ColorToken {
    pub role: String,
    pub hex: String,
    #[serde(default)]
    pub weight: f64,
    #[serde(default)]
    pub source: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct FontToken {
    pub family: String,
    pub role: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub weights: Vec<u32>,
    #[serde(default)]
    pub count: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Voice {
    #[serde(default)]
    pub descriptors: Vec<String>,
    #[serde(default)]
    pub samples: Vec<String>,
    #[serde(default)]
    pub exclamations: f64,
    #[serde(default)]
    pub avg_sentence_words: f64,
    #[serde(default)]
    pub second_person: f64,
    #[serde(default)]
    pub uppercase_ratio: f64,
    #[serde(default)]
    pub language: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct SocialLink {
    pub network: String,
    pub url: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Easings {
    pub enter: [f64; 4],
    pub exit: [f64; 4],
    pub travel: [f64; 4],
    pub impact: [f64; 4],
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct TypeTokens {
    pub heading: String,
    pub body: String,
    pub mono: String,
    /// Weight range used for variable-axis animation, e.g. [300, 700].
    pub weight_range: [u32; 2],
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct ColorRoles {
    pub bg: String,
    pub fg: String,
    pub accent: String,
    pub accent2: String,
    pub muted: String,
}

/// The "edit brand": how this project moves, cuts and sounds. It is derived from the
/// extracted brand plus a seed, so two projects never end up with the same motion.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct EditBrand {
    #[serde(default)]
    pub seed: u64,
    /// calm | steady | punchy
    #[serde(default)]
    pub tempo: String,
    #[serde(default)]
    pub easings: Easings,
    /// Duration palette in frames at 30 fps (rescale for other fps).
    #[serde(default)]
    pub durations: Vec<u32>,
    #[serde(default)]
    pub stagger_ms: u32,
    #[serde(default)]
    pub stagger_shrink: f64,
    /// Stillness before a climax, in frames at 30 fps.
    #[serde(default)]
    pub hold_frames: u32,
    /// Some projects animate graphics "on twos"; None = every frame.
    #[serde(default)]
    pub step_fps: Option<u32>,
    #[serde(default)]
    pub motion_blur: bool,
    #[serde(default)]
    pub transitions: Vec<String>,
    #[serde(default)]
    pub type_tokens: TypeTokens,
    #[serde(default)]
    pub colors: ColorRoles,
    #[serde(default)]
    pub rules: Vec<String>,
    #[serde(default)]
    pub rationale: Vec<String>,
    /// When true, re-extraction never overwrites the edit brand (hand-tuned).
    #[serde(default)]
    pub locked: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Brand {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub source_url: Option<String>,
    #[serde(default)]
    pub extracted_at: Option<String>,
    #[serde(default)]
    pub tagline: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub palette: Vec<ColorToken>,
    #[serde(default)]
    pub fonts: Vec<FontToken>,
    /// Asset ids of the logo candidates, best first.
    #[serde(default)]
    pub logos: Vec<String>,
    #[serde(default)]
    pub voice: Voice,
    #[serde(default)]
    pub social: Vec<SocialLink>,
    #[serde(default)]
    pub edit: EditBrand,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Memory {
    pub id: String,
    pub ts: String,
    /// decision | preference | rejection | fact | note | run | tombstone
    pub kind: String,
    pub text: String,
    #[serde(default)]
    pub tags: Vec<String>,
    /// low | medium | high
    #[serde(default)]
    pub confidence: String,
    /// user | agent | extract | system
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub pinned: bool,
    /// Id of the memory this one replaces (or deletes, for tombstones).
    #[serde(default)]
    pub supersedes: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Asset {
    pub id: String,
    /// logo | image | icon | svg | font | video | audio | other
    pub kind: String,
    /// Path relative to the project's assets directory.
    pub file: String,
    #[serde(default)]
    pub source_url: Option<String>,
    /// extracted | generated | imported
    #[serde(default)]
    pub origin: String,
    pub sha256: String,
    #[serde(default)]
    pub bytes: u64,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    #[serde(default)]
    pub mime: String,
    #[serde(default)]
    pub colors: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub adapter: Option<String>,
    #[serde(default)]
    pub added: String,
}
