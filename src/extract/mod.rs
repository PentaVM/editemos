//! Brand and asset extraction: the "business DNA" of a website.
//!
//! One site in; a brand (palette with roles, fonts, voice, logos, social links),
//! a catalog of downloaded assets and a derived edit brand out. Three interchangeable
//! backends fetch the page: `native` (plain HTTP), `browser` (the machine's own
//! Edge/Chrome, for JavaScript sites and true computed styles) and `scrapling`
//! (the Scrapling CLI, for sites that fight scrapers). `auto` picks.

pub mod analyze;
pub mod profile;
pub mod scrapling;

use crate::assets;
use crate::brand::derive_edit_brand;
use crate::cdp::Browser;
use crate::color::{parse_color, Rgb};
use crate::context;
use crate::error::{fail, Result};
use crate::http;
use crate::memory;
use crate::model::Brand;
use crate::store::{Home, Project};
use crate::util::{now_iso, truncate_chars, write_atomic};
use analyze::{Analysis, Cand};
use serde::Serialize;
use serde_json::Value;
use std::time::{Duration, Instant};
use url::Url;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Backend {
    Auto,
    Native,
    Browser,
    Scrapling,
}

impl Backend {
    pub fn parse(s: &str) -> Backend {
        match s {
            "native" => Backend::Native,
            "browser" => Backend::Browser,
            "scrapling" => Backend::Scrapling,
            _ => Backend::Auto,
        }
    }
}

pub struct Options {
    pub backend: Backend,
    pub max_images: usize,
    pub fonts: bool,
    pub ignore_robots: bool,
    pub scrapling_mode: String,
}

impl Default for Options {
    fn default() -> Self {
        Options { backend: Backend::Auto, max_images: 12, fonts: false, ignore_robots: false, scrapling_mode: "get".into() }
    }
}

#[derive(Serialize, Default, Debug)]
pub struct Report {
    pub url: String,
    pub final_url: String,
    pub backend: String,
    pub title: Option<String>,
    pub palette: Vec<String>,
    pub fonts: Vec<String>,
    pub logos: usize,
    pub images: usize,
    pub assets_added: Vec<String>,
    pub tempo: String,
    pub warnings: Vec<String>,
    pub elapsed_ms: u128,
}

struct Page {
    html: String,
    final_url: String,
    computed: Option<Value>,
    screenshot: Option<Vec<u8>>,
    backend: &'static str,
}

/// Minimal robots.txt check for `*` and `editemos` groups. Fetch failures allow.
pub fn robots_allows(url: &Url) -> bool {
    // `join` keeps the port: dev servers and intranets rarely listen on 80/443
    let Ok(robots) = url.join("/robots.txt") else { return true };
    let Ok(text) = http::get_text(robots.as_str(), 256 * 1024).map(|t| t.0) else { return true };
    let path = url.path();
    let mut applies = false;
    let mut in_group_header = false;
    let mut rules: Vec<(bool, String)> = Vec::new();
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        let Some((k, v)) = line.split_once(':') else { continue };
        let (k, v) = (k.trim().to_ascii_lowercase(), v.trim());
        match k.as_str() {
            "user-agent" => {
                if !in_group_header {
                    applies = false;
                }
                in_group_header = true;
                if v == "*" || v.eq_ignore_ascii_case("editemos") {
                    applies = true;
                }
            }
            "disallow" | "allow" => {
                in_group_header = false;
                if applies && !v.is_empty() {
                    rules.push((k == "allow", v.to_string()));
                }
            }
            _ => {}
        }
    }
    // longest matching rule wins; allow beats disallow on ties
    let best = rules.iter().filter(|(_, p)| path.starts_with(p.trim_end_matches('*'))).max_by(|a, b| a.1.len().cmp(&b.1.len()).then(a.0.cmp(&b.0)));
    best.map(|(allow, _)| *allow).unwrap_or(true)
}

const COMPUTED_JS: &str = r#"(() => {
  const q = s => document.querySelector(s);
  const cs = (el, p) => el ? getComputedStyle(el)[p] : null;
  const fam = el => { const f = cs(el, 'fontFamily'); return f ? f.split(',')[0].replace(/["']/g, '').trim() : null; };
  const btn = q('button, a.btn, a.button, [class*="cta"], [class*="btn"], input[type=submit]');
  const a = q('main a[href], a[href]');
  const h1 = q('h1') || q('h2');
  return {
    bodyBg: cs(document.body, 'backgroundColor'), htmlBg: cs(document.documentElement, 'backgroundColor'),
    bodyColor: cs(document.body, 'color'), bodyFont: fam(document.body), h1Font: fam(h1), h1Color: cs(h1, 'color'),
    buttonBg: cs(btn, 'backgroundColor'), buttonColor: cs(btn, 'color'), buttonFont: fam(btn), linkColor: cs(a, 'color'),
    images: [...document.images].slice(0, 80).map(i => ({ src: i.currentSrc || i.src, w: i.naturalWidth, h: i.naturalHeight })),
    html: document.documentElement.outerHTML, url: location.href
  };
})()"#;

fn fetch_native(url: &str) -> Result<Page> {
    let (html, final_url) = http::get_text(url, 6 * 1024 * 1024)?;
    Ok(Page { html, final_url, computed: None, screenshot: None, backend: "native" })
}

fn fetch_browser(url: &str) -> Result<Page> {
    let b = Browser::launch(1366, 900)?;
    let mut page = b.page()?;
    page.navigate(url, Duration::from_secs(40))?;
    std::thread::sleep(Duration::from_millis(1800));
    let v = page.eval(COMPUTED_JS, false)?;
    let html = v["html"].as_str().unwrap_or("").to_string();
    let final_url = v["url"].as_str().unwrap_or(url).to_string();
    let shot = page.screenshot(false).ok();
    Ok(Page { html, final_url, computed: Some(v), screenshot: shot, backend: "browser" })
}

fn fetch_scrapling(home: &Home, url: &str, mode: &str) -> Result<Page> {
    let Some(exe) = scrapling::locate(home) else {
        return fail("scrapling is not installed. Run `editemos tools install-scrapling` or set EDITEMOS_SCRAPLING");
    };
    let html = scrapling::fetch_html(&exe, url, mode)?;
    Ok(Page { html, final_url: url.to_string(), computed: None, screenshot: None, backend: "scrapling" })
}

fn looks_empty(html: &str) -> bool {
    let body = html.split("<body").nth(1).unwrap_or(html);
    let text: String = body.split('<').filter_map(|s| s.split_once('>').map(|x| x.1)).collect::<Vec<_>>().join(" ");
    text.split_whitespace().count() < 25
}

fn fetch_page(home: &Home, url: &str, opts: &Options, warnings: &mut Vec<String>) -> Result<Page> {
    match opts.backend {
        Backend::Native => fetch_native(url),
        Backend::Browser => fetch_browser(url),
        Backend::Scrapling => fetch_scrapling(home, url, &opts.scrapling_mode),
        Backend::Auto => match fetch_native(url) {
            Ok(p) if !looks_empty(&p.html) => Ok(p),
            first => {
                let why = match &first {
                    Ok(_) => "page looks client-rendered".to_string(),
                    Err(e) => e.0.clone(),
                };
                // refused outright (403, TLS fingerprinting): that is Scrapling's job
                if first.is_err() && scrapling::locate(home).is_some() {
                    if let Ok(p) = fetch_scrapling(home, url, "get") {
                        if !looks_empty(&p.html) {
                            warnings.push(format!("native fetch refused ({why}); Scrapling got through"));
                            return Ok(p);
                        }
                    }
                }
                warnings.push(format!("native fetch not enough ({why}); trying the browser backend"));
                match fetch_browser(url) {
                    Ok(p) => Ok(p),
                    Err(be) => {
                        warnings.push(format!("browser backend failed: {be}"));
                        if scrapling::locate(home).is_some() {
                            fetch_scrapling(home, url, "get")
                        } else {
                            first
                        }
                    }
                }
            }
        },
    }
}

fn rgb_from_computed(v: &Value, key: &str) -> Option<Rgb> {
    v.get(key).and_then(|x| x.as_str()).and_then(parse_color)
}

fn apply_computed(a: &mut Analysis, v: &Value) {
    let bg = rgb_from_computed(v, "bodyBg").or_else(|| rgb_from_computed(v, "htmlBg"));
    if let Some(bg) = bg {
        a.bg_hint = Some(bg);
    }
    if let Some(t) = rgb_from_computed(v, "bodyColor") {
        a.text_hint = Some(t);
    }
    for (k, w) in [("buttonBg", 8.0), ("linkColor", 4.0), ("h1Color", 3.0)] {
        if let Some(c) = rgb_from_computed(v, k) {
            a.colors.push((c, w, format!("computed {k}")));
        }
    }
    for (k, role) in [("h1Font", "heading"), ("bodyFont", "body"), ("buttonFont", "other")] {
        if let Some(f) = v.get(k).and_then(|x| x.as_str()).filter(|f| !f.is_empty()) {
            let e = a.fonts.entry(f.to_string()).or_default();
            e.source = if e.source.is_empty() || e.source == "css" { "computed".into() } else { e.source.clone() };
            match role {
                "heading" => e.heading += 20,
                "body" => e.body += 20,
                _ => e.other += 5,
            }
        }
    }
    if let Some(imgs) = v.get("images").and_then(|x| x.as_array()) {
        for i in imgs {
            let (w, h) = (i["w"].as_u64().unwrap_or(0), i["h"].as_u64().unwrap_or(0));
            if let Some(src) = i["src"].as_str() {
                if w >= 300 && h >= 160 && !src.starts_with("data:") {
                    a.images.push(Cand { url: src.to_string(), score: 2 + (w.min(2000) / 500) as i32, why: format!("rendered {w}x{h}"), ..Default::default() });
                }
            }
        }
    }
}

fn apply_manifest(a: &mut Analysis, base: &Url, warnings: &mut Vec<String>) {
    let Some(mu) = a.manifest.clone() else { return };
    match http::get_text(&mu, 128 * 1024) {
        Ok((text, _)) => {
            if let Ok(v) = serde_json::from_str::<Value>(&text) {
                for (k, w) in [("theme_color", 4.0), ("background_color", 2.0)] {
                    if let Some(c) = v[k].as_str().and_then(parse_color) {
                        a.colors.push((c, w, format!("manifest {k}")));
                        if k == "background_color" && a.bg_hint.is_none() {
                            a.bg_hint = Some(c);
                        }
                    }
                }
                if a.site_name.is_none() {
                    a.site_name = v["name"].as_str().or(v["short_name"].as_str()).map(String::from);
                }
                if let Some(icons) = v["icons"].as_array() {
                    for i in icons {
                        if let (Some(src), Ok(u)) = (i["src"].as_str(), Url::parse(&mu)) {
                            let size = i["sizes"].as_str().and_then(|s| s.split('x').next()).and_then(|n| n.parse::<i32>().ok()).unwrap_or(0);
                            if let Ok(abs) = u.join(src) {
                                a.icons.push(Cand { url: abs.to_string(), score: size / 16 + 2, why: "manifest icon".into(), ..Default::default() });
                            }
                        }
                    }
                }
            }
        }
        Err(e) => warnings.push(format!("manifest not readable: {e}")),
    }
    let _ = base;
}

fn best_title(a: &Analysis, fallback: &str) -> String {
    let raw = a.site_name.clone().or_else(|| a.title.clone()).unwrap_or_else(|| fallback.to_string());
    let first = raw.split(['|', '–', '—', '·']).next().unwrap_or(&raw).trim().to_string();
    let first = if first.contains(" - ") && first.chars().count() > 28 { first.split(" - ").next().unwrap_or(&first).trim().to_string() } else { first };
    truncate_chars(&first, 60)
}

/// HTML + linked stylesheets (fetched together) + manifest + computed styles.
fn analyse_page(page: &Page, base: &Url, warnings: &mut Vec<String>) -> Analysis {
    let mut a = analyze::parse_html(&page.html, base);
    let sheets: Vec<(String, usize)> = a.stylesheets.iter().take(8).map(|u| (u.clone(), 900 * 1024)).collect();
    for ((css_url, _), got) in sheets.iter().zip(http::get_many(&sheets, 12)) {
        match got {
            Some(f) => {
                let sheet_base = Url::parse(css_url).unwrap_or(base.clone());
                analyze::parse_css(&String::from_utf8_lossy(&f.bytes), &sheet_base, &mut a);
            }
            None => warnings.push(format!("stylesheet skipped: {css_url}")),
        }
    }
    apply_manifest(&mut a, base, warnings);
    if let Some(c) = &page.computed {
        apply_computed(&mut a, c);
    }
    a
}

pub fn run(home: &Home, project: &Project, url_in: &str, opts: &Options) -> Result<Report> {
    let started = Instant::now();
    let mut warnings: Vec<String> = Vec::new();
    let url = Url::parse(url_in).or_else(|_| Url::parse(&format!("https://{url_in}")))?;
    if url.scheme() != "http" && url.scheme() != "https" {
        return fail("only http and https URLs can be extracted");
    }
    if !opts.ignore_robots && !robots_allows(&url) {
        return fail("robots.txt disallows this path for generic crawlers. Re-run with --ignore-robots only for sites you own or have permission to analyse");
    }

    // a social profile wears the platform's colours, not the creator's: read it differently
    if let Some(platform) = profile::platform_of(&url) {
        return profile::run(home, project, platform, &url, opts, started);
    }

    let mut page = fetch_page(home, url.as_str(), opts, &mut warnings)?;
    let mut base = Url::parse(&page.final_url).unwrap_or(url.clone());
    let mut a = analyse_page(&page, &base, &mut warnings);

    // A page can have plenty of text and still keep its whole look in JavaScript.
    // If plain HTTP found no typeface or no accent colour, ask a real browser.
    if opts.backend == Backend::Auto && page.backend == "native" && (analyze::build_fonts(&a).is_empty() || !analyze::build_palette(&a, &[]).iter().any(|c| c.role == "primary")) {
        match fetch_browser(url.as_str()) {
            Ok(p) => {
                warnings.push("plain HTTP showed no typeface or accent colour; read the rendered page with the browser instead".into());
                page = p;
                base = Url::parse(&page.final_url).unwrap_or(url.clone());
                a = analyse_page(&page, &base, &mut warnings);
            }
            Err(e) => warnings.push(format!("browser backend unavailable for a second look: {e}")),
        }
    }

    // download assets: everything is fetched at once, then catalogued in order
    let mut added: Vec<String> = Vec::new();
    let mut logo_ids: Vec<String> = Vec::new();
    let mut logo_colors: Vec<(Rgb, f64)> = Vec::new();
    let logo_cands: Vec<&Cand> = a.logos.iter().take(3).collect();
    let icon_cand: Option<&Cand> = a.icons.iter().max_by_key(|c| c.score);
    let mut image_cands: Vec<&Cand> = Vec::new();
    for c in a.images.iter() {
        if !c.url.starts_with("data:") && !image_cands.iter().any(|x| x.url == c.url) && image_cands.len() < opts.max_images * 2 {
            image_cands.push(c);
        }
    }
    let font_files: Vec<(String, String)> = if opts.fonts {
        a.fonts.iter().filter_map(|(family, acc)| acc.files.iter().find(|f| f.contains("woff2")).or_else(|| acc.files.first()).map(|f| (family.clone(), f.clone()))).collect()
    } else {
        Vec::new()
    };
    let mut jobs: Vec<(String, usize)> = Vec::new();
    jobs.extend(logo_cands.iter().filter(|c| c.inline_svg.is_none()).map(|c| (c.url.clone(), 4 * 1024 * 1024)));
    jobs.extend(icon_cand.iter().map(|c| (c.url.clone(), 2 * 1024 * 1024)));
    jobs.extend(image_cands.iter().map(|c| (c.url.clone(), 8 * 1024 * 1024)));
    jobs.extend(font_files.iter().map(|(_, f)| (f.clone(), 4 * 1024 * 1024)));
    let mut fetched = http::get_many(&jobs, 15).into_iter();
    let mut next_bytes = || fetched.next().flatten().map(|f| f.bytes);

    for cand in logo_cands {
        let bytes = match &cand.inline_svg {
            Some(s) => Some(s.as_bytes().to_vec()),
            None => next_bytes(),
        };
        if let Some(b) = bytes {
            if let Ok(asset) = assets::add_bytes(project, &b, "logo", &cand.url, "extracted", Some(&cand.url), None, None, vec!["logo".into(), cand.why.clone()]) {
                if logo_ids.is_empty() {
                    for c in &asset.colors {
                        if let Some(rgb) = parse_color(c) {
                            logo_colors.push((rgb, 0.5));
                        }
                    }
                }
                logo_ids.push(asset.id.clone());
                added.push(asset.file);
            }
        }
    }
    if let Some(cand) = icon_cand {
        if let Some(b) = next_bytes() {
            if let Ok(asset) = assets::add_bytes(project, &b, "icon", &cand.url, "extracted", Some(&cand.url), None, None, vec!["icon".into()]) {
                added.push(asset.file);
            }
        }
    }
    let mut n_images = 0;
    for cand in image_cands {
        let Some(b) = next_bytes() else { continue };
        if n_images >= opts.max_images || b.len() < 2048 {
            continue;
        }
        // tiny pictures (tracking pixels, bullets) are not brand imagery
        if image::load_from_memory(&b).map(|i| i.width() < 64 || i.height() < 64).unwrap_or(false) {
            continue;
        }
        if let Ok(asset) = assets::add_bytes(project, &b, "image", &cand.url, "extracted", Some(&cand.url), None, None, vec![cand.why.clone()]) {
            n_images += 1;
            added.push(asset.file);
        }
    }
    for (family, file) in &font_files {
        if let Some(b) = next_bytes() {
            if let Ok(asset) = assets::add_bytes(project, &b, "font", file, "extracted", Some(file), None, None, vec![family.clone()]) {
                added.push(asset.file);
            }
        }
    }
    if let Some(shot) = &page.screenshot {
        if let Ok(asset) = assets::add_bytes(project, shot, "image", "screenshot.png", "extracted", Some(&page.final_url), None, None, vec!["screenshot".into()]) {
            added.push(asset.file);
        }
    }
    added.sort();
    added.dedup();

    // brand
    let existing = project.load_brand();
    let name = best_title(&a, &project.config.name);
    let palette = analyze::build_palette(&a, &logo_colors);
    let fonts = analyze::build_fonts(&a);
    let voice = analyze::analyze_voice(&a, a.language.as_deref());
    let tagline = a.texts.iter().find(|t| (8..=90).contains(&t.chars().count())).cloned();
    let mut brand = Brand {
        version: 1,
        name,
        source_url: Some(page.final_url.clone()),
        extracted_at: Some(now_iso()),
        tagline,
        description: a.description.clone(),
        language: a.language.clone().or(Some(voice.language.clone())),
        palette,
        fonts,
        logos: logo_ids,
        voice,
        social: a.social.clone(),
        edit: existing.edit.clone(),
    };
    if !existing.edit.locked || existing.edit.tempo.is_empty() {
        brand.edit = derive_edit_brand(&brand);
    }
    project.save_brand(&brand)?;

    let report = Report {
        url: url_in.to_string(),
        final_url: page.final_url.clone(),
        backend: page.backend.to_string(),
        title: a.title.clone(),
        palette: brand.palette.iter().map(|c| format!("{} {}", c.role, c.hex)).collect(),
        fonts: brand.fonts.iter().map(|f| format!("{} ({})", f.family, f.role)).collect(),
        logos: a.logos.len(),
        images: n_images,
        assets_added: added,
        tempo: brand.edit.tempo.clone(),
        warnings,
        elapsed_ms: started.elapsed().as_millis(),
    };
    write_atomic(&project.dir.join("extract-report.json"), serde_json::to_string_pretty(&report)?.as_bytes())?;
    memory::add(
        project,
        &format!(
            "Brand extracted from {} via {}: tempo {}, palette {}, heading font {}.",
            report.final_url,
            report.backend,
            report.tempo,
            report.palette.join(", "),
            brand.edit.type_tokens.heading
        ),
        "fact",
        vec!["brand".into(), "extract".into()],
        "medium",
        "extract",
        false,
        None,
    )?;
    // keep the project's recorded site up to date
    let mut p2 = project.clone();
    p2.config.site = Some(page.final_url);
    p2.save_config()?;
    context::refresh(home, &p2)?;
    Ok(report)
}
