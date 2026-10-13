//! Typefaces: a curated catalogue of open fonts, a choice that fits the brand,
//! and preferences that are remembered.
//!
//! A system fallback is never a design decision. When a brand has no typeface of
//! its own, one is picked from the catalogue by what the brand is (gaming, comedy,
//! sport, editorial, calm), downloaded once into `~/.editemos/fonts`, and written
//! down: per project in the brand, and per user in `prefs.json` when the user says
//! "always" or "never". A rejected typeface is not offered again.

use crate::error::{fail, Result};
use crate::http;
use crate::model::Brand;
use crate::store::Home;
use crate::util::write_atomic;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub struct Face {
    pub family: &'static str,
    /// Query for the Google Fonts CSS API (family and, for multi-weight families, the weight).
    pub query: &'static str,
    /// Name the font file reports, which the caption renderer must ask for.
    pub ass_name: &'static str,
    pub tags: &'static [&'static str],
    pub note: &'static str,
}

/// Open-licence display faces that hold up as on-screen text. All static files:
/// one weight, one unambiguous name.
pub const CATALOG: &[Face] = &[
    Face {
        family: "Anton",
        query: "Anton",
        ass_name: "Anton",
        tags: &["gaming", "sport", "impact", "condensed", "loud"],
        note: "tall, heavy, condensed; the standard for loud short-form captions",
    },
    Face {
        family: "Bebas Neue",
        query: "Bebas+Neue",
        ass_name: "Bebas Neue",
        tags: &["gaming", "sport", "impact", "condensed", "clean"],
        note: "condensed caps, cleaner than Anton",
    },
    Face {
        family: "Archivo Black",
        query: "Archivo+Black",
        ass_name: "Archivo Black",
        tags: &["gaming", "impact", "wide", "serious", "tech"],
        note: "wide and heavy; reads at small sizes",
    },
    Face { family: "Russo One", query: "Russo+One", ass_name: "Russo One", tags: &["gaming", "tech", "esports", "wide"], note: "squared, technical; esports overlays" },
    Face { family: "Black Ops One", query: "Black+Ops+One", ass_name: "Black Ops One", tags: &["gaming", "military", "stencil"], note: "stencil; shooters" },
    Face {
        family: "Racing Sans One",
        query: "Racing+Sans+One",
        ass_name: "Racing Sans One",
        tags: &["gaming", "sport", "speed", "cars", "italic"],
        note: "slanted and fast; racing and car games",
    },
    Face { family: "Chakra Petch", query: "Chakra+Petch:wght@700", ass_name: "Chakra Petch", tags: &["gaming", "tech", "esports", "clean"], note: "angular tech sans" },
    Face { family: "Bangers", query: "Bangers", ass_name: "Bangers", tags: &["funny", "comic", "gaming", "loud"], note: "comic-book caps; jokes and reactions" },
    Face { family: "Luckiest Guy", query: "Luckiest+Guy", ass_name: "Luckiest Guy", tags: &["funny", "comic", "kids", "loud"], note: "round and bouncy; comedy" },
    Face { family: "Titan One", query: "Titan+One", ass_name: "Titan One", tags: &["funny", "friendly", "round", "loud"], note: "fat, friendly, rounded" },
    Face { family: "Lilita One", query: "Lilita+One", ass_name: "Lilita One", tags: &["funny", "friendly", "gaming", "round"], note: "rounded display; casual gaming" },
    Face { family: "Paytone One", query: "Paytone+One", ass_name: "Paytone One", tags: &["friendly", "round", "loud", "lifestyle"], note: "soft heavy sans" },
    Face { family: "Passion One", query: "Passion+One:wght@700", ass_name: "Passion One", tags: &["sport", "impact", "loud"], note: "poster weight" },
    Face { family: "Bowlby One", query: "Bowlby+One", ass_name: "Bowlby One", tags: &["impact", "retro", "loud"], note: "very wide, very heavy" },
    Face { family: "Alfa Slab One", query: "Alfa+Slab+One", ass_name: "Alfa Slab One", tags: &["impact", "slab", "food", "retro", "serious"], note: "heavy slab serif" },
    Face { family: "Staatliches", query: "Staatliches", ass_name: "Staatliches", tags: &["clean", "condensed", "design", "serious"], note: "geometric caps" },
    Face {
        family: "Poppins",
        query: "Poppins:wght@800",
        ass_name: "Poppins ExtraBold",
        tags: &["clean", "friendly", "lifestyle", "business", "tech"],
        note: "geometric sans; safe and modern",
    },
    Face { family: "Kanit", query: "Kanit:wght@800", ass_name: "Kanit ExtraBold", tags: &["clean", "sport", "tech", "gaming"], note: "sturdy sans with some attitude" },
    Face {
        family: "Barlow Condensed",
        query: "Barlow+Condensed:wght@800",
        ass_name: "Barlow Condensed ExtraBold",
        tags: &["clean", "condensed", "sport", "cars", "serious"],
        note: "condensed grotesque; signage feel",
    },
    Face {
        family: "Saira Condensed",
        query: "Saira+Condensed:wght@800",
        ass_name: "Saira Condensed ExtraBold",
        tags: &["gaming", "tech", "esports", "condensed"],
        note: "condensed tech sans",
    },
    Face {
        family: "DM Serif Display",
        query: "DM+Serif+Display",
        ass_name: "DM Serif Display",
        tags: &["editorial", "calm", "luxury", "food", "serious"],
        note: "high-contrast serif",
    },
    Face { family: "Abril Fatface", query: "Abril+Fatface", ass_name: "Abril Fatface", tags: &["editorial", "luxury", "fashion", "retro"], note: "didone display" },
    Face {
        family: "Libre Baskerville",
        query: "Libre+Baskerville:wght@700",
        ass_name: "Libre Baskerville",
        tags: &["editorial", "calm", "documentary", "serious"],
        note: "bookish serif",
    },
];

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Prefs {
    /// Used for every project that has not chosen its own.
    #[serde(default)]
    pub caption_font: Option<String>,
    /// Never offered again, in any project.
    #[serde(default)]
    pub rejected_fonts: Vec<String>,
    /// Captions in capitals (the short-form default).
    #[serde(default)]
    pub caption_caps: Option<bool>,
    /// Words per caption line.
    #[serde(default)]
    pub caption_words: Option<usize>,
    /// Seconds a cut waits after the last word of a segment.
    #[serde(default)]
    pub cut_breath: Option<f64>,
}

pub fn prefs_path(home: &Home) -> PathBuf {
    home.root.join("prefs.json")
}

pub fn load_prefs(home: &Home) -> Prefs {
    std::fs::read_to_string(prefs_path(home)).ok().and_then(|s| serde_json::from_str(s.trim_start_matches('\u{feff}')).ok()).unwrap_or_default()
}

pub fn save_prefs(home: &Home, p: &Prefs) -> Result<()> {
    write_atomic(&prefs_path(home), serde_json::to_string_pretty(p)?.as_bytes())
}

pub fn dir(home: &Home) -> PathBuf {
    home.root.join("fonts")
}

pub fn find(family: &str) -> Option<&'static Face> {
    CATALOG.iter().find(|f| f.family.eq_ignore_ascii_case(family.trim()))
}

/// What a brand is about, as catalogue tags.
pub fn vibes(b: &Brand) -> Vec<&'static str> {
    let mut text = format!("{} {} {} {}", b.name, b.tagline.clone().unwrap_or_default(), b.description.clone().unwrap_or_default(), b.voice.descriptors.join(" ")).to_lowercase();
    for s in &b.social {
        text.push(' ');
        text.push_str(&s.network);
    }
    let mut v: Vec<&'static str> = Vec::new();
    let mut has = |tag: &'static str, words: &[&str]| {
        if words.iter().any(|w| text.contains(w)) && !v.contains(&tag) {
            v.push(tag);
        }
    };
    has(
        "gaming",
        &[
            "gaming",
            "gamer",
            "game",
            "rocket league",
            "fortnite",
            "valorant",
            "minecraft",
            "stream",
            "twitch",
            "kick",
            "esport",
            "ranked",
            "gameplay",
            "clips",
            "ssl",
            "fps",
            "speedrun",
        ],
    );
    has("esports", &["esport", "pro player", "world champion", "rlcs", "tournament", "competitive"]);
    has("cars", &["car", "coche", "auto", "motor", "racing", "rocket league", "drift"]);
    has("funny", &["funny", "comedy", "meme", "playful", "humor", "humour", "chist", "gracios", "jokes", "troll", "banter"]);
    has("sport", &["sport", "football", "soccer", "futbol", "fitness", "gym", "basket", "training"]);
    has("tech", &["tech", "software", "code", "developer", "ai ", "startup", "saas", "app"]);
    has("food", &["food", "cocina", "restaurant", "chef", "recipe", "receta", "cafe", "coffee"]);
    has("luxury", &["luxury", "lujo", "fashion", "moda", "jewel", "premium", "boutique"]);
    has("editorial", &["documentary", "documental", "essay", "journal", "podcast", "interview", "history", "historia"]);
    has("business", &["business", "finance", "consult", "agency", "marketing", "b2b"]);
    match b.edit.tempo.as_str() {
        "punchy" => v.push("loud"),
        "calm" => v.push("calm"),
        _ => v.push("clean"),
    }
    v
}

pub struct Choice {
    pub face: &'static Face,
    pub why: String,
}

/// The typeface for on-screen text: the project's own choice, else the user's
/// standing preference, else the best catalogue match that was never rejected.
pub fn pick(home: &Home, b: &Brand, rejected_here: &[String]) -> Choice {
    let prefs = load_prefs(home);
    let rejected = |fam: &str| prefs.rejected_fonts.iter().chain(rejected_here.iter()).any(|r| r.eq_ignore_ascii_case(fam));
    if let Some(f) = b.fonts.iter().find(|f| f.role == "caption").and_then(|f| find(&f.family)).filter(|f| !rejected(f.family)) {
        return Choice { face: f, why: "chosen for this project".into() };
    }
    if let Some(f) = prefs.caption_font.as_deref().and_then(find).filter(|f| !rejected(f.family)) {
        return Choice { face: f, why: "your standing preference".into() };
    }
    let want = vibes(b);
    let mut scored: Vec<(i32, &'static Face)> = CATALOG
        .iter()
        .filter(|f| !rejected(f.family))
        .map(|f| {
            // the first vibes are the most specific; later ones break ties
            let s: i32 = want.iter().enumerate().filter(|(_, t)| f.tags.contains(t)).map(|(i, _)| 6 - (i as i32).min(4)).sum();
            (s, f)
        })
        .collect();
    scored.sort_by_key(|(s, f)| (std::cmp::Reverse(*s), f.family));
    let best = scored.first().map(|x| x.0).unwrap_or(0);
    // among the equally good, the project's seed decides: two channels of the same kind still differ
    let top: Vec<&'static Face> = scored.iter().filter(|(s, _)| *s >= best - 1).map(|x| x.1).take(3).collect();
    let face = top.get((b.edit.seed % top.len().max(1) as u64) as usize).copied().unwrap_or(&CATALOG[0]);
    Choice { face, why: format!("fits {} ({})", if want.is_empty() { "a general brand".to_string() } else { want.join(", ") }, face.note) }
}

/// Download a catalogue face into the shared font folder (once). Returns the file.
pub fn install(home: &Home, family: &str) -> Result<PathBuf> {
    let Some(face) = find(family) else {
        return fail(format!("'{family}' is not in the catalogue. Known: {}", CATALOG.iter().map(|f| f.family).collect::<Vec<_>>().join(", ")));
    };
    let d = dir(home);
    std::fs::create_dir_all(&d)?;
    let file = d.join(format!("{}.ttf", face.family.replace(' ', "")));
    if file.exists() && std::fs::metadata(&file).map(|m| m.len() > 4096).unwrap_or(false) {
        return Ok(file);
    }
    // a non-browser client is served TrueType, which is what the renderer needs
    let css = http::get(&format!("https://fonts.googleapis.com/css2?family={}", face.query), 256 * 1024, &[])?;
    let css = String::from_utf8_lossy(&css.bytes).to_string();
    let Some(url) = css.split("url(").nth(1).and_then(|s| s.split(')').next()).map(|s| s.trim_matches(|c| c == '"' || c == '\'').to_string()) else {
        return fail(format!("the font service returned no file for {}", face.family));
    };
    let bytes = http::get(&url, 8 * 1024 * 1024, &[])?.bytes;
    if bytes.len() < 4096 {
        return fail(format!("{} downloaded as an empty file", face.family));
    }
    write_atomic(&file, &bytes)?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{SocialLink, Voice};

    fn home() -> Home {
        let root = std::env::temp_dir().join(format!("editemos-fonts-{}", crate::util::short_id()));
        std::fs::create_dir_all(&root).unwrap();
        Home { root }
    }

    #[test]
    fn a_gaming_channel_does_not_get_a_bookish_serif() {
        let h = home();
        let b = Brand {
            name: "Zed".into(),
            description: Some("Rocket League ranked gameplay and funny clips".into()),
            social: vec![SocialLink { network: "youtube".into(), url: String::new() }],
            ..Default::default()
        };
        let c = pick(&h, &b, &[]);
        assert!(c.face.tags.contains(&"gaming"), "picked {}", c.face.family);
        let calm = Brand { name: "Atlas".into(), description: Some("A documentary podcast about history".into()), voice: Voice::default(), ..Default::default() };
        let c2 = pick(&h, &calm, &[]);
        assert!(c2.face.tags.contains(&"editorial"), "picked {}", c2.face.family);
        let _ = std::fs::remove_dir_all(&h.root);
    }

    #[test]
    fn rejections_and_preferences_are_obeyed() {
        let h = home();
        let b = Brand { name: "Zed".into(), description: Some("gaming".into()), ..Default::default() };
        let first = pick(&h, &b, &[]).face.family;
        // rejected here: something else
        assert_ne!(pick(&h, &b, &[first.to_string()]).face.family, first);
        // rejected for the user: never again, in any project
        save_prefs(&h, &Prefs { rejected_fonts: vec![first.to_string()], ..Default::default() }).unwrap();
        assert_ne!(pick(&h, &b, &[]).face.family, first);
        // a standing preference wins over the catalogue match
        save_prefs(&h, &Prefs { caption_font: Some("DM Serif Display".into()), ..Default::default() }).unwrap();
        assert_eq!(pick(&h, &b, &[]).face.family, "DM Serif Display");
        let _ = std::fs::remove_dir_all(&h.root);
    }

    #[test]
    fn catalogue_is_consistent() {
        for f in CATALOG {
            assert!(!f.tags.is_empty() && !f.query.contains(' ') && f.ass_name.starts_with(f.family), "{}", f.family);
        }
    }
}
