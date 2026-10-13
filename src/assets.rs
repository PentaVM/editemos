//! Asset catalog: content-addressed files under `assets/` plus an index with
//! provenance (where it came from, which adapter made it, colours, size).

use crate::color::palette_of;
use crate::error::Result;
use crate::model::Asset;
use crate::store::Project;
use crate::util::{ensure_dir, now_iso, sha256_hex, tokenize, write_atomic};
use regex::Regex;
use std::fs;

pub fn load(p: &Project) -> Vec<Asset> {
    fs::read_to_string(p.assets_index()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub fn save(p: &Project, assets: &[Asset]) -> Result<()> {
    write_atomic(&p.assets_index(), serde_json::to_string_pretty(assets)?.as_bytes())
}

/// (extension, mime) from magic bytes, falling back to the name hint.
pub fn sniff(bytes: &[u8], hint: &str) -> (&'static str, &'static str) {
    let starts = |m: &[u8]| bytes.len() >= m.len() && &bytes[..m.len()] == m;
    if starts(&[0x89, b'P', b'N', b'G']) {
        return ("png", "image/png");
    }
    if starts(&[0xFF, 0xD8, 0xFF]) {
        return ("jpg", "image/jpeg");
    }
    if starts(b"GIF8") {
        return ("gif", "image/gif");
    }
    if bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return ("webp", "image/webp");
    }
    if starts(&[0, 0, 1, 0]) {
        return ("ico", "image/x-icon");
    }
    if starts(b"wOF2") {
        return ("woff2", "font/woff2");
    }
    if starts(b"wOFF") {
        return ("woff", "font/woff");
    }
    if starts(&[0, 1, 0, 0]) || starts(b"true") {
        return ("ttf", "font/ttf");
    }
    if starts(b"OTTO") {
        return ("otf", "font/otf");
    }
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(512)]).to_ascii_lowercase();
    if head.contains("<svg") {
        return ("svg", "image/svg+xml");
    }
    let h = hint.to_ascii_lowercase();
    for (ext, mime) in [("mp4", "video/mp4"), ("webm", "video/webm"), ("mov", "video/quicktime"), ("mp3", "audio/mpeg"), ("wav", "audio/wav"), ("json", "application/json")] {
        if h.ends_with(ext) {
            return (ext, mime);
        }
    }
    ("bin", "application/octet-stream")
}

fn svg_dims(text: &str) -> (u32, u32) {
    let vb = Regex::new(r#"viewBox\s*=\s*["']\s*[-\d.]+[ ,]+[-\d.]+[ ,]+([\d.]+)[ ,]+([\d.]+)"#).unwrap();
    if let Some(c) = vb.captures(text) {
        let w = c[1].parse::<f64>().unwrap_or(0.0) as u32;
        let h = c[2].parse::<f64>().unwrap_or(0.0) as u32;
        return (w, h);
    }
    (0, 0)
}

fn svg_colors(text: &str) -> Vec<String> {
    let re = Regex::new(r"#[0-9a-fA-F]{6}\b").unwrap();
    let mut out: Vec<String> = Vec::new();
    for m in re.find_iter(text) {
        let c = m.as_str().to_ascii_lowercase();
        if !out.contains(&c) {
            out.push(c);
        }
        if out.len() >= 5 {
            break;
        }
    }
    out
}

/// Index a file where it already lives (renders, clips, footage) without copying
/// it. `file` holds the absolute path; the id comes from the path, so re-rendering
/// to the same place updates the record instead of adding another.
pub fn register_file(p: &Project, file: &std::path::Path, kind: &str, tags: Vec<String>) -> Result<Asset> {
    let abs = fs::canonicalize(file).unwrap_or_else(|_| file.to_path_buf());
    let shown = abs.to_string_lossy().trim_start_matches(r"\\?\").to_string();
    let meta = fs::metadata(&abs)?;
    let id = format!("f{}", &sha256_hex(shown.to_lowercase().as_bytes())[..11]);
    let (_, mime) = sniff(&[], &shown);
    let asset = Asset {
        id: id.clone(),
        kind: kind.to_string(),
        file: shown,
        origin: "generated".into(),
        bytes: meta.len(),
        mime: mime.to_string(),
        tags,
        added: now_iso(),
        ..Default::default()
    };
    let mut index = load(p);
    index.retain(|a| a.id != id);
    index.push(asset.clone());
    save(p, &index)?;
    Ok(asset)
}

/// Store bytes in the catalog (deduplicated by hash) and return its record.
#[allow(clippy::too_many_arguments)]
pub fn add_bytes(
    p: &Project,
    bytes: &[u8],
    kind: &str,
    name_hint: &str,
    origin: &str,
    source_url: Option<&str>,
    prompt: Option<&str>,
    adapter: Option<&str>,
    tags: Vec<String>,
) -> Result<Asset> {
    let sha = sha256_hex(bytes);
    let id = sha[..12].to_string();
    let mut index = load(p);
    if let Some(existing) = index.iter().find(|a| a.id == id) {
        return Ok(existing.clone());
    }
    let (ext, mime) = sniff(bytes, name_hint);
    let kind = if kind.is_empty() {
        match ext {
            "svg" => "svg",
            "woff" | "woff2" | "ttf" | "otf" => "font",
            "mp4" | "webm" | "mov" => "video",
            "mp3" | "wav" => "audio",
            _ => "image",
        }
    } else {
        kind
    };
    let rel = format!("{kind}/{id}.{ext}");
    let abs = p.assets_dir().join(&rel);
    if let Some(parent) = abs.parent() {
        ensure_dir(parent)?;
    }
    fs::write(&abs, bytes)?;

    let (mut width, mut height, mut colors) = (0u32, 0u32, Vec::<String>::new());
    if ext == "svg" {
        let text = String::from_utf8_lossy(bytes);
        let (w, h) = svg_dims(&text);
        width = w;
        height = h;
        colors = svg_colors(&text);
    } else if matches!(ext, "png" | "jpg" | "gif" | "webp" | "ico") {
        if let Ok(img) = image::load_from_memory(bytes) {
            use image::GenericImageView;
            let (w, h) = img.dimensions();
            width = w;
            height = h;
            colors = palette_of(&img, 5).into_iter().map(|(c, _)| c.hex()).collect();
        }
    }
    let asset = Asset {
        id,
        kind: kind.to_string(),
        file: rel,
        source_url: source_url.map(String::from),
        origin: origin.to_string(),
        sha256: sha,
        bytes: bytes.len() as u64,
        width,
        height,
        mime: mime.to_string(),
        colors,
        tags,
        note: String::new(),
        prompt: prompt.map(String::from),
        adapter: adapter.map(String::from),
        added: now_iso(),
    };
    index.push(asset.clone());
    save(p, &index)?;
    Ok(asset)
}

pub fn search(assets: &[Asset], query: &str, k: usize) -> Vec<Asset> {
    let q = tokenize(query);
    if q.is_empty() {
        return assets.iter().take(k).cloned().collect();
    }
    let mut scored: Vec<(usize, &Asset)> = assets
        .iter()
        .map(|a| {
            let hay = tokenize(&format!(
                "{} {} {} {} {} {}",
                a.kind,
                a.tags.join(" "),
                a.note,
                a.source_url.clone().unwrap_or_default(),
                a.prompt.clone().unwrap_or_default(),
                a.colors.join(" ")
            ));
            (q.iter().filter(|t| hay.contains(t)).count(), a)
        })
        .filter(|(s, _)| *s > 0)
        .collect();
    scored.sort_by_key(|a| std::cmp::Reverse(a.0));
    scored.into_iter().take(k).map(|(_, a)| a.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_formats() {
        assert_eq!(sniff(&[0x89, b'P', b'N', b'G', 0, 0], "x").0, "png");
        assert_eq!(sniff(b"<svg xmlns=\"x\"></svg>", "x").0, "svg");
        assert_eq!(sniff(b"hello", "clip.mp4").0, "mp4");
    }

    #[test]
    fn svg_metadata() {
        let svg = r##"<svg viewBox="0 0 120 40"><rect fill="#FF3D1F"/></svg>"##;
        assert_eq!(svg_dims(svg), (120, 40));
        assert_eq!(svg_colors(svg), vec!["#ff3d1f"]);
    }
}
