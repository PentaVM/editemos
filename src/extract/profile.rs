//! Brand from a social profile (YouTube channel, Instagram, TikTok, Twitch, X, Kick).
//!
//! A profile page is dressed in the platform's own colours and typeface: YouTube
//! red, Twitch purple, the Instagram gradient. None of that is the creator's brand.
//! So this backend ignores the page's CSS entirely and reads only what the creator
//! uploaded: the avatar, the banner, the thumbnails, the name and the description.
//! Every colour is kept with where it came from, so a person or a model can still
//! overrule the choice (`editemos brand evidence`, `brand refine`, `brand set`).

use super::analyze::{self, Analysis};
use super::{Options, Report};
use crate::assets;
use crate::brand::derive_edit_brand;
use crate::color::{parse_color, Rgb};
use crate::context;
use crate::error::{fail, Result};
use crate::http;
use crate::memory;
use crate::model::Brand;
use crate::store::{Home, Project};
use crate::util::{now_iso, path_str, truncate_chars, write_atomic};
use regex::Regex;
use serde_json::json;
use std::time::Instant;
use url::Url;

pub struct Platform {
    pub name: &'static str,
    /// The platform's own interface colours: never the creator's.
    pub colors: &'static [&'static str],
    suffixes: &'static [&'static str],
}

const PLATFORMS: &[(&[&str], Platform)] = &[
    (&["youtube.com", "youtu.be"], Platform { name: "youtube", colors: &["#ff0000", "#ff0033", "#0f0f0f", "#ffffff", "#065fd4", "#606060"], suffixes: &[" - YouTube"] }),
    (
        &["instagram.com"],
        Platform {
            name: "instagram",
            colors: &["#e1306c", "#c13584", "#833ab4", "#fd1d1d", "#f77737", "#fcaf45", "#0095f6", "#ffffff", "#000000"],
            suffixes: &[" • Instagram photos and videos", " | Instagram"],
        },
    ),
    (&["tiktok.com"], Platform { name: "tiktok", colors: &["#fe2c55", "#25f4ee", "#000000", "#ffffff"], suffixes: &[" | TikTok"] }),
    (&["twitch.tv"], Platform { name: "twitch", colors: &["#9146ff", "#772ce8", "#0e0e10", "#18181b", "#efeff1"], suffixes: &[" - Twitch"] }),
    (&["x.com", "twitter.com"], Platform { name: "x", colors: &["#1d9bf0", "#000000", "#ffffff", "#0f1419"], suffixes: &[" / X", " / Twitter"] }),
    (&["kick.com"], Platform { name: "kick", colors: &["#53fc18", "#0b0e0f", "#191b1f"], suffixes: &[" | Kick"] }),
    (&["facebook.com"], Platform { name: "facebook", colors: &["#1877f2", "#0866ff", "#ffffff", "#f0f2f5"], suffixes: &[" | Facebook"] }),
];

pub fn platform_of(url: &Url) -> Option<&'static Platform> {
    let host = url.host_str()?.trim_start_matches("www.").trim_start_matches("m.").to_ascii_lowercase();
    PLATFORMS.iter().find(|(hosts, _)| hosts.iter().any(|h| host == *h || host.ends_with(&format!(".{h}")))).map(|(_, p)| p)
}

/// True when `c` is one of the platform's interface colours (or very close).
pub fn is_platform_color(p: &Platform, c: Rgb) -> bool {
    p.colors.iter().filter_map(|h| parse_color(h)).any(|k| k.dist(c) < 34.0)
}

fn unescape(s: &str) -> String {
    s.replace("\\u0026", "&").replace("\\/", "/").replace("&amp;", "&")
}

fn meta(html: &str, key: &str) -> Option<String> {
    let re = Regex::new(&format!(r#"<meta[^>]+(?:property|name)=["']{}["'][^>]*content=["']([^"']+)["']"#, regex::escape(key))).ok()?;
    let re2 = Regex::new(&format!(r#"<meta[^>]+content=["']([^"']+)["'][^>]*(?:property|name)=["']{}["']"#, regex::escape(key))).ok()?;
    re.captures(html).or_else(|| re2.captures(html)).map(|c| unescape(&c[1])).filter(|s| !s.trim().is_empty())
}

/// What the creator uploaded, as (kind, url) pairs, most identifying first.
pub fn creator_images(platform: &Platform, html: &str) -> Vec<(&'static str, String)> {
    let mut out: Vec<(&'static str, String)> = Vec::new();
    if let Some(a) = meta(html, "og:image") {
        out.push(("avatar", a));
    }
    if platform.name == "youtube" {
        for pat in [r#""banner":\{"imageBannerViewModel":\{"image":\{"sources":\[\{"url":"([^"]+)""#, r#""banner":\{"thumbnails":\[\{"url":"([^"]+)""#] {
            if let Some(c) = Regex::new(pat).ok().and_then(|r| r.captures(html)) {
                let mut u = unescape(&c[1]);
                if u.starts_with("//") {
                    u = format!("https:{u}");
                }
                out.push(("banner", u));
                break;
            }
        }
        let mut seen: Vec<String> = Vec::new();
        if let Ok(re) = Regex::new(r#""videoId":"([A-Za-z0-9_-]{11})""#) {
            for c in re.captures_iter(html) {
                let id = c[1].to_string();
                if !seen.contains(&id) {
                    seen.push(id.clone());
                    out.push(("thumbnail", format!("https://i.ytimg.com/vi/{id}/hqdefault.jpg")));
                }
                if seen.len() >= 6 {
                    break;
                }
            }
        }
    }
    out
}

fn clean_name(platform: &Platform, raw: &str) -> String {
    let mut s = raw.trim().to_string();
    for suf in platform.suffixes {
        if let Some(i) = s.find(suf) {
            s.truncate(i);
        }
    }
    // "Name (@handle)" -> "Name"
    if let Some(i) = s.find(" (@") {
        s.truncate(i);
    }
    truncate_chars(s.trim(), 60)
}

pub fn run(home: &Home, project: &Project, platform: &'static Platform, url: &Url, opts: &Options, started: Instant) -> Result<Report> {
    let mut warnings: Vec<String> = Vec::new();
    // the initial HTML carries the Open Graph tags; a browser is only the fallback
    let (html, final_url) = match http::get_text(url.as_str(), 8 * 1024 * 1024) {
        Ok((h, u)) if meta(&h, "og:image").is_some() || meta(&h, "og:title").is_some() => (h, u),
        first => {
            warnings.push(format!("plain HTTP did not return the profile ({}); asking the browser", first.err().map(|e| e.0).unwrap_or_else(|| "no Open Graph tags".into())));
            let p = super::fetch_browser(url.as_str())?;
            (p.html, p.final_url)
        }
    };
    let images = creator_images(platform, &html);
    if images.is_empty() && meta(&html, "og:title").is_none() {
        return fail(format!(
            "{} did not show this profile to an anonymous visitor. Give the brand by hand instead: `editemos brand set --primary #... --heading-font ...`, `editemos brand describe \"...\"`, or add the avatar with `editemos assets add` and run `editemos brand from-images`.",
            platform.name
        ));
    }

    // download what the creator uploaded and read colours out of the pictures
    let jobs: Vec<(String, usize)> = images.iter().take(2 + opts.max_images.min(6)).map(|(_, u)| (u.clone(), 6 * 1024 * 1024)).collect();
    let mut evidence: Vec<(Rgb, f64, String)> = Vec::new();
    let mut added: Vec<String> = Vec::new();
    let mut logo_ids: Vec<String> = Vec::new();
    let mut files: Vec<String> = Vec::new();
    for ((kind, src), got) in images.iter().zip(http::get_many(&jobs, 15)) {
        let Some(f) = got else {
            warnings.push(format!("{kind} not downloaded"));
            continue;
        };
        let asset_kind = if *kind == "avatar" { "logo" } else { "image" };
        let Ok(asset) = assets::add_bytes(project, &f.bytes, asset_kind, src, "extracted", Some(src), None, None, vec![kind.to_string(), platform.name.to_string()]) else {
            continue;
        };
        // the avatar and the banner are the identity; thumbnails only support it
        let source_weight = match *kind {
            "avatar" => 3.0,
            "banner" => 3.0,
            _ => 0.6,
        };
        for (i, hex) in asset.colors.iter().enumerate() {
            if let Some(c) = parse_color(hex) {
                evidence.push((c, source_weight * [5.0, 3.0, 2.0, 1.0, 1.0][i.min(4)], kind.to_string()));
            }
        }
        if *kind == "avatar" {
            logo_ids.push(asset.id.clone());
        }
        files.push(path_str(&project.assets_dir().join(&asset.file)));
        added.push(asset.file);
    }

    let base = Url::parse(&final_url).unwrap_or(url.clone());
    let parsed = analyze::parse_html(&html, &base);
    let name = clean_name(platform, &meta(&html, "og:title").or(parsed.title.clone()).unwrap_or_else(|| project.config.name.clone()));
    let description = meta(&html, "og:description").or(parsed.description.clone());

    // graphics go over footage: a dark plate and light type unless the pictures say otherwise
    // The identity is what the avatar and the banner show. Thumbnails change every
    // week and are full of game footage: they may support a colour, never choose it.
    let identity: Vec<&(Rgb, f64, String)> = evidence.iter().filter(|e| e.2 != "thumbnail").collect();
    let total: f64 = identity.iter().map(|e| e.1).sum::<f64>().max(1.0);
    let chroma: f64 = identity.iter().filter(|e| e.0.hsl().1 >= 0.22 && (0.12..0.92).contains(&e.0.hsl().2)).map(|e| e.1).sum();
    let dark = identity.iter().filter(|e| e.0.luminance() < 0.08).max_by(|x, y| x.1.partial_cmp(&y.1).unwrap()).map(|e| e.0);
    let light = identity.iter().filter(|e| e.0.luminance() > 0.6 && e.0.hsl().1 < 0.2).max_by(|x, y| x.1.partial_cmp(&y.1).unwrap()).map(|e| e.0);
    let (bg, text) = (dark.unwrap_or(Rgb(14, 14, 16)), light.unwrap_or(Rgb(240, 236, 228)));
    let mut a = Analysis { bg_hint: Some(bg), text_hint: Some(text), ..Default::default() };
    a.texts = description.iter().cloned().collect();
    let monochrome = !identity.is_empty() && chroma / total < 0.08;
    let mut palette = if monochrome {
        // black-and-white identities are common and deliberate: keep them that way
        warnings.push("the avatar and banner are monochrome: the brand is kept black and white, and colours seen only in thumbnails were not promoted".into());
        analyze::build_palette(&a, &[])
    } else {
        a.colors = evidence.clone();
        analyze::build_palette(&a, &[])
    };
    if monochrome {
        palette.push(crate::model::ColorToken { role: "primary".into(), hex: text.hex(), weight: 1.0, source: "avatar (monochrome identity)".into() });
    }
    for c in palette.iter_mut().filter(|c| c.role == "background" || c.role == "text") {
        c.source = if dark.is_some() || light.is_some() { "avatar/banner".into() } else { "default for overlays".into() };
    }
    let voice = analyze::analyze_voice(&a, parsed.language.as_deref());

    // keep the evidence: which colour came from where, and which look like the platform's
    let mut merged: Vec<(Rgb, f64, Vec<String>)> = Vec::new();
    for (c, w, src) in &evidence {
        match merged.iter_mut().find(|m| m.0.dist(*c) < 18.0) {
            Some(m) => {
                m.1 += w;
                if !m.2.contains(src) {
                    m.2.push(src.clone());
                }
            }
            None => merged.push((*c, *w, vec![src.clone()])),
        }
    }
    merged.sort_by(|x, y| y.1.partial_cmp(&x.1).unwrap_or(std::cmp::Ordering::Equal));
    let ev = json!({
        "platform": platform.name,
        "platform_colors": platform.colors,
        "note": "Colours come only from pictures the creator uploaded; the page's own styling was ignored. `near_platform` marks colours that also match the platform's interface and deserve a second look.",
        "candidates": merged.iter().take(14).map(|(c, w, src)| json!({"hex": c.hex(), "weight": (w * 10.0).round() / 10.0, "from": src, "near_platform": is_platform_color(platform, *c)})).collect::<Vec<_>>(),
        "images": files,
        "name": name,
        "description": description,
    });
    write_atomic(&project.dir.join("brand-evidence.json"), serde_json::to_string_pretty(&ev)?.as_bytes())?;

    let existing = project.load_brand();
    let mut brand = Brand {
        version: 1,
        name,
        source_url: Some(final_url.clone()),
        extracted_at: Some(now_iso()),
        tagline: description.as_ref().map(|d| truncate_chars(d.lines().next().unwrap_or(""), 90)).filter(|t| !t.is_empty()),
        description,
        language: parsed.language.clone().or(Some(voice.language.clone())),
        palette,
        fonts: Vec::new(),
        logos: logo_ids,
        voice,
        social: vec![crate::model::SocialLink { network: platform.name.into(), url: final_url.clone() }],
        edit: existing.edit.clone(),
    };
    if !existing.edit.locked || existing.edit.tempo.is_empty() {
        brand.edit = derive_edit_brand(&brand);
    }
    project.save_brand(&brand)?;
    warnings.push(format!("{} profile: its interface colours and typeface were left out; typefaces cannot be read from pictures, so set them with `editemos brand set --heading-font ...` or let a model look: `editemos brand refine`", platform.name));

    let report = Report {
        url: url.to_string(),
        final_url: final_url.clone(),
        backend: format!("profile:{}", platform.name),
        title: Some(brand.name.clone()),
        palette: brand.palette.iter().map(|c| format!("{} {}", c.role, c.hex)).collect(),
        fonts: Vec::new(),
        logos: brand.logos.len(),
        images: added.len().saturating_sub(brand.logos.len()),
        assets_added: added,
        tempo: brand.edit.tempo.clone(),
        warnings,
        elapsed_ms: started.elapsed().as_millis(),
    };
    write_atomic(&project.dir.join("extract-report.json"), serde_json::to_string_pretty(&report)?.as_bytes())?;
    memory::add(
        project,
        &format!("Brand read from the {} profile {}: palette {} (platform colours excluded).", platform.name, final_url, report.palette.join(", ")),
        "fact",
        vec!["brand".into(), "extract".into()],
        "medium",
        "extract",
        false,
        None,
    )?;
    let mut p2 = project.clone();
    p2.config.site = Some(final_url);
    p2.save_config()?;
    context::refresh(home, &p2)?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knows_platforms_and_their_colours() {
        let yt = platform_of(&Url::parse("https://www.youtube.com/@someone").unwrap()).unwrap();
        assert_eq!(yt.name, "youtube");
        assert!(is_platform_color(yt, Rgb(250, 8, 6)), "YouTube red is the platform's");
        assert!(!is_platform_color(yt, Rgb(20, 200, 140)));
        assert_eq!(platform_of(&Url::parse("https://m.twitch.tv/x").unwrap()).unwrap().name, "twitch");
        assert!(platform_of(&Url::parse("https://example.com/youtube.com").unwrap()).is_none());
    }

    #[test]
    fn reads_only_what_the_creator_uploaded() {
        let yt = platform_of(&Url::parse("https://youtube.com/@x").unwrap()).unwrap();
        let html = r#"<meta property="og:image" content="https://yt3.example/avatar=s900"><meta property="og:title" content="Zed Plays - YouTube">
            <style>.red{color:#ff0000}</style>
            "banner":{"imageBannerViewModel":{"image":{"sources":[{"url":"https://yt3.example/banner=w1060&x=1","width":1060}]}}}
            "videoId":"abcdefghijk" "videoId":"abcdefghijk" "videoId":"ABCDEFGHIJK""#;
        let imgs = creator_images(yt, html);
        assert_eq!(imgs[0], ("avatar", "https://yt3.example/avatar=s900".to_string()));
        assert_eq!(imgs[1], ("banner", "https://yt3.example/banner=w1060&x=1".to_string()));
        assert_eq!(imgs.iter().filter(|i| i.0 == "thumbnail").count(), 2, "video ids are deduplicated");
        assert_eq!(clean_name(yt, "Zed Plays - YouTube"), "Zed Plays");
        let ig = platform_of(&Url::parse("https://instagram.com/x").unwrap()).unwrap();
        assert_eq!(clean_name(ig, "Ana (@ana.cocina) • Instagram photos and videos"), "Ana");
    }
}
