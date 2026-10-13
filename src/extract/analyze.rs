//! Pure analysis of a web page: HTML + CSS in, brand evidence out. No network.
//! The same code serves the native, browser and Scrapling backends.

use crate::color::{contrast, parse_color, Rgb};
use crate::model::{ColorToken, FontToken, SocialLink, Voice};
use regex::Regex;
use scraper::{ElementRef, Html, Selector};
use std::collections::HashMap;
use url::Url;

#[derive(Clone, Debug, Default)]
pub struct Cand {
    pub url: String,
    pub inline_svg: Option<String>,
    pub score: i32,
    pub why: String,
    pub alt: String,
}

#[derive(Clone, Debug, Default)]
pub struct Analysis {
    pub title: Option<String>,
    pub description: Option<String>,
    pub site_name: Option<String>,
    pub language: Option<String>,
    pub theme_color: Option<String>,
    pub manifest: Option<String>,
    pub logos: Vec<Cand>,
    pub icons: Vec<Cand>,
    pub images: Vec<Cand>,
    pub stylesheets: Vec<String>,
    pub colors: Vec<(Rgb, f64, String)>,
    pub bg_hint: Option<Rgb>,
    pub text_hint: Option<Rgb>,
    pub fonts: HashMap<String, FontAcc>,
    pub texts: Vec<String>,
    pub ctas: Vec<String>,
    pub social: Vec<SocialLink>,
    pub custom_props: HashMap<String, String>,
}

#[derive(Clone, Debug, Default)]
pub struct FontAcc {
    pub heading: u32,
    pub body: u32,
    pub other: u32,
    pub files: Vec<String>,
    pub weights: Vec<u32>,
    pub source: String,
}

fn sel(s: &str) -> Selector {
    Selector::parse(s).expect("static selector")
}

fn abs(base: &Url, href: &str) -> Option<String> {
    let h = href.trim();
    if h.is_empty() || h.starts_with("data:") || h.starts_with("javascript:") || h.starts_with('#') {
        return None;
    }
    base.join(h).ok().map(|u| u.to_string())
}

fn first_srcset(s: &str) -> &str {
    s.split(',').next().unwrap_or("").split_whitespace().next().unwrap_or("")
}

fn text_of(el: &ElementRef) -> String {
    el.text().collect::<Vec<_>>().join(" ").split_whitespace().collect::<Vec<_>>().join(" ")
}

fn in_header(el: &ElementRef) -> bool {
    el.ancestors().filter_map(ElementRef::wrap).any(|a| {
        let n = a.value().name();
        let class = a.value().attr("class").unwrap_or("").to_ascii_lowercase();
        n == "header" || n == "nav" || class.contains("header") || class.contains("navbar") || class.contains("masthead")
    })
}

const GENERIC_FONTS: &[&str] = &[
    "serif",
    "sans-serif",
    "monospace",
    "cursive",
    "fantasy",
    "system-ui",
    "ui-sans-serif",
    "ui-serif",
    "ui-monospace",
    "ui-rounded",
    "-apple-system",
    "blinkmacsystemfont",
    "inherit",
    "initial",
    "unset",
    "emoji",
    "math",
    "fangsong",
];

pub fn parse_html(html: &str, base: &Url) -> Analysis {
    let doc = Html::parse_document(html);
    let mut a = Analysis::default();

    let meta = |attr: &str, value: &str| -> Option<String> {
        let s = Selector::parse(&format!("meta[{attr}=\"{value}\"]")).ok()?;
        doc.select(&s).next().and_then(|e| e.value().attr("content")).map(|c| c.trim().to_string()).filter(|c| !c.is_empty())
    };
    a.title = meta("property", "og:title").or_else(|| doc.select(&sel("title")).next().map(|t| text_of(&t)).filter(|t| !t.is_empty()));
    a.description = meta("property", "og:description").or_else(|| meta("name", "description"));
    a.site_name = meta("property", "og:site_name");
    a.theme_color = meta("name", "theme-color");
    a.language = doc.select(&sel("html")).next().and_then(|h| h.value().attr("lang")).map(|l| l.split('-').next().unwrap_or(l).to_lowercase());

    for l in doc.select(&sel("link[rel~=\"manifest\"]")) {
        if let Some(h) = l.value().attr("href").and_then(|h| abs(base, h)) {
            a.manifest = Some(h);
        }
    }
    for l in doc.select(&sel("link[rel~=\"stylesheet\"]")) {
        if let Some(h) = l.value().attr("href") {
            if h.contains("fonts.googleapis.com") {
                google_fonts(h, &mut a);
            } else if let Some(u) = abs(base, h) {
                if !a.stylesheets.contains(&u) {
                    a.stylesheets.push(u);
                }
            }
        }
    }

    // icons
    for l in doc.select(&sel("link[rel*=\"icon\"]")) {
        if let Some(u) = l.value().attr("href").and_then(|h| abs(base, h)) {
            let rel = l.value().attr("rel").unwrap_or("").to_ascii_lowercase();
            let size = l.value().attr("sizes").and_then(|s| s.split('x').next().and_then(|n| n.parse::<i32>().ok())).unwrap_or(0);
            let score = size / 16 + if rel.contains("apple") { 4 } else { 1 };
            a.icons.push(Cand { url: u, score, why: format!("link rel={rel}"), ..Default::default() });
        }
    }

    // logos and images
    let mut dom_order = 0;
    for img in doc.select(&sel("img")) {
        dom_order += 1;
        let v = img.value();
        let src = v
            .attr("src")
            .or_else(|| v.attr("data-src"))
            .or_else(|| v.attr("data-lazy-src"))
            .map(|s| s.to_string())
            .or_else(|| v.attr("srcset").map(|s| first_srcset(s).to_string()));
        let Some(src) = src.and_then(|s| abs(base, &s)) else { continue };
        let alt = v.attr("alt").unwrap_or("").to_string();
        let hay = format!("{} {} {} {}", src, alt, v.attr("class").unwrap_or(""), v.attr("id").unwrap_or("")).to_ascii_lowercase();
        let mut score = 0;
        let mut why = Vec::new();
        if hay.contains("logo") {
            score += 6;
            why.push("name contains logo");
        }
        if hay.contains("brand") || hay.contains("wordmark") {
            score += 2;
            why.push("brand/wordmark");
        }
        if in_header(&img) {
            score += 3;
            why.push("in header/nav");
        }
        if dom_order <= 2 {
            score += 1;
        }
        if src.to_ascii_lowercase().contains(".svg") {
            score += 1;
        }
        if hay.contains("favicon") || hay.contains("sprite") || hay.contains("pixel") || hay.contains("tracking") {
            score -= 4;
        }
        let small = |k: &str| v.attr(k).and_then(|n| n.trim_end_matches("px").parse::<i32>().ok()).map(|n| n <= 2).unwrap_or(false);
        if small("width") || small("height") {
            continue;
        }
        let cand = Cand { url: src, score, why: why.join(", "), alt, ..Default::default() };
        if score >= 6 {
            a.logos.push(cand);
        } else {
            a.images.push(cand);
        }
    }
    for svg in doc.select(&sel("header svg, nav svg, a[href=\"/\"] svg, .logo svg, [class*=\"logo\"] svg")) {
        let outer = svg.html();
        if outer.len() < 20_000 && (outer.contains("<path") || outer.contains("<text") || outer.contains("<g")) {
            let mut s = outer;
            if !s.contains("xmlns=") {
                s = s.replacen("<svg", "<svg xmlns=\"http://www.w3.org/2000/svg\"", 1);
            }
            a.logos.push(Cand { url: format!("inline:svg:{}", a.logos.len()), inline_svg: Some(s), score: 5, why: "inline svg in header".into(), alt: String::new() });
        }
    }
    for m in ["og:image", "twitter:image"] {
        let found = meta("property", m).or_else(|| meta("name", m));
        if let Some(u) = found.and_then(|u| abs(base, &u)) {
            a.images.insert(0, Cand { url: u, score: 4, why: m.to_string(), alt: String::new(), inline_svg: None });
        }
    }
    // CSS background-image in inline style attributes
    let bg_re = Regex::new(r#"url\(\s*['"]?([^'")]+)['"]?\s*\)"#).unwrap();
    for el in doc.select(&sel("[style]")) {
        let st = el.value().attr("style").unwrap_or("");
        if st.contains("background") {
            for c in bg_re.captures_iter(st) {
                if let Some(u) = abs(base, &c[1]) {
                    a.images.push(Cand { url: u, score: 1, why: "inline background-image".into(), ..Default::default() });
                }
            }
        }
    }

    // inline <style> blocks are CSS too
    let mut inline_css = String::new();
    for st in doc.select(&sel("style")) {
        inline_css.push_str(&st.text().collect::<String>());
        inline_css.push('\n');
    }
    if !inline_css.is_empty() {
        parse_css(&inline_css, base, &mut a);
    }

    // text for voice
    for h in doc.select(&sel("h1, h2, h3")) {
        let t = text_of(&h);
        if (8..=140).contains(&t.chars().count()) {
            a.texts.push(t);
        }
    }
    for p in doc.select(&sel("p")).take(14) {
        let t = text_of(&p);
        if (30..=260).contains(&t.chars().count()) {
            a.texts.push(t);
        }
    }
    if let Some(d) = &a.description {
        a.texts.push(d.clone());
    }
    for b in doc.select(&sel("button, a.btn, a.button, a[class*=\"cta\"], [role=\"button\"]")).take(16) {
        let t = text_of(&b);
        if (2..=40).contains(&t.chars().count()) {
            a.ctas.push(t);
        }
    }

    // social
    let nets = [
        ("twitter.com", "x"),
        ("x.com", "x"),
        ("instagram.com", "instagram"),
        ("facebook.com", "facebook"),
        ("linkedin.com", "linkedin"),
        ("youtube.com", "youtube"),
        ("tiktok.com", "tiktok"),
        ("github.com", "github"),
        ("vimeo.com", "vimeo"),
    ];
    for l in doc.select(&sel("a[href]")) {
        if let Some(h) = l.value().attr("href") {
            if let Ok(u) = Url::parse(h) {
                if let Some(host) = u.host_str() {
                    for (dom, name) in nets {
                        if host.ends_with(dom) && u.path().len() > 1 && !a.social.iter().any(|s| s.network == name) {
                            a.social.push(SocialLink { network: name.to_string(), url: h.to_string() });
                        }
                    }
                }
            }
        }
    }

    if let Some(tc) = a.theme_color.clone().and_then(|c| parse_color(&c)) {
        a.colors.push((tc, 4.0, "meta theme-color".into()));
    }
    a.logos.sort_by_key(|x| std::cmp::Reverse(x.score));
    a.images.sort_by_key(|x| std::cmp::Reverse(x.score));
    a
}

fn google_fonts(href: &str, a: &mut Analysis) {
    // css?family=Open+Sans:400,700|Roboto  or  css2?family=Inter:wght@400;700&family=Lora
    let rest = href.trim_start_matches("https:").trim_start_matches("http:").trim_start_matches("//");
    let Ok(u) = Url::parse(&format!("https://{rest}")) else { return };
    for (k, v) in u.query_pairs() {
        if k != "family" {
            continue;
        }
        for fam in v.split('|') {
            let (name, rest) = fam.split_once(':').unwrap_or((fam, ""));
            let name = name.replace('+', " ");
            let weights: Vec<u32> = rest.split(|c: char| !c.is_ascii_digit()).filter_map(|n| n.parse::<u32>().ok()).filter(|w| (100..=900).contains(w)).collect();
            let e = a.fonts.entry(name).or_default();
            e.source = "google-fonts".into();
            e.other += 1;
            for w in weights {
                if !e.weights.contains(&w) {
                    e.weights.push(w);
                }
            }
        }
    }
}

fn selector_role(selector: &str) -> &'static str {
    let s = selector.to_ascii_lowercase();
    if Regex::new(r"(^|[\s,>.#\[-])(h[1-3]|title|heading|headline|display|hero)").unwrap().is_match(&s) {
        "heading"
    } else if Regex::new(r"(^|[\s,>])(body|html|p|:root|\.text|\.body|li)\b").unwrap().is_match(&s) {
        "body"
    } else {
        "other"
    }
}

fn color_weight_for(name_or_selector: &str, prop: &str) -> f64 {
    let n = name_or_selector.to_ascii_lowercase();
    let strong = ["primary", "brand", "accent", "main", "theme", "highlight", "cta", "key"];
    let on_button = (n.contains("button") || n.contains(".btn") || n.contains("cta") || n.starts_with("a:") || n.starts_with("a ") || n == "a") && prop.starts_with("background");
    if strong.iter().any(|k| n.contains(k)) {
        5.0
    } else if n.contains("secondary") || n.contains("tertiary") || on_button {
        3.0
    } else {
        1.0
    }
}

/// First family of a `font-family` value, without quotes or `!important`.
fn clean_family(value: &str) -> String {
    let v = value.replace("!important", "");
    v.split(',').next().unwrap_or("").trim().trim_matches(|c| c == '"' || c == '\'').trim().to_string()
}

/// Icon fonts are not typography: they must never become the brand's typeface.
fn is_icon_font(family: &str) -> bool {
    let f = family.to_ascii_lowercase();
    const ICON: &[&str] = &[
        "glyph",
        "icon",
        "awesome",
        "dashicons",
        "symbols",
        "icomoon",
        "fontello",
        "ionicons",
        "feather",
        "swiper",
        "slick",
        "videojs",
        "vjs",
        "genericons",
        "eleganticons",
        "linearicons",
        "octicons",
        "codicon",
        "emoji",
    ];
    ICON.iter().any(|k| f.contains(k)) || f.chars().any(|c| matches!(c, '\'' | '"' | '(' | ')' | '{' | '}' | ';'))
}

/// Replace `var(--x)` / `var(--x, fallback)` with the custom property's value.
fn resolve_vars(val: &str, props: &HashMap<String, String>, depth: u8) -> String {
    if depth > 4 || !val.contains("var(") {
        return val.to_string();
    }
    let re = Regex::new(r"var\(\s*(--[\w-]+)\s*(?:,\s*([^()]*))?\)").unwrap();
    let out = re.replace_all(val, |c: &regex::Captures| props.get(&c[1]).cloned().or_else(|| c.get(2).map(|m| m.as_str().trim().to_string())).unwrap_or_default());
    if out.contains("var(") {
        resolve_vars(&out, props, depth + 1)
    } else {
        out.into_owned()
    }
}

pub fn parse_css(css: &str, base: &Url, a: &mut Analysis) {
    let rule_re = Regex::new(r"([^{}]+)\{([^{}]*)\}").unwrap();
    let color_re = Regex::new(r"(?i)#[0-9a-f]{3,8}\b|rgba?\([^)]*\)|hsla?\([^)]*\)").unwrap();
    let url_re = Regex::new(r#"url\(\s*['"]?([^'")]+)['"]?\s*\)"#).unwrap();
    // custom properties first, so var(--x) in font-family and colours can resolve;
    // the first definition (normally :root) wins over dark-mode overrides
    let prop_re = Regex::new(r"(--[\w-]+)\s*:\s*([^;}{]+)").unwrap();
    for c in prop_re.captures_iter(css) {
        a.custom_props.entry(c[1].to_string()).or_insert_with(|| c[2].trim().to_string());
    }
    for cap in rule_re.captures_iter(css) {
        let selector = cap[1].trim().to_string();
        let decls = &cap[2];
        let is_face = selector.starts_with("@font-face");
        let role = selector_role(&selector);
        let sel_l = selector.to_ascii_lowercase();
        let on_root = sel_l.split(',').any(|s| matches!(s.trim(), "body" | "html" | ":root" | "html, body" | "main"));
        let mut face_family = String::new();
        let mut face_files: Vec<String> = Vec::new();
        let mut face_weight: Option<u32> = None;
        for d in decls.split(';') {
            let Some((prop, val)) = d.split_once(':') else { continue };
            let prop = prop.trim().to_ascii_lowercase();
            let val = val.trim();
            if prop == "font-family" {
                let mut v = val.to_string();
                if let Some(inner) = v.strip_prefix("var(").and_then(|s| s.strip_suffix(')')) {
                    v = a.custom_props.get(inner.split(',').next().unwrap_or("").trim()).cloned().unwrap_or_default();
                }
                let fam = clean_family(&v);
                if fam.is_empty() || GENERIC_FONTS.contains(&fam.to_ascii_lowercase().as_str()) || fam.starts_with("var(") || is_icon_font(&fam) {
                    continue;
                }
                if is_face {
                    face_family = fam;
                } else {
                    let e = a.fonts.entry(fam).or_default();
                    e.source = if e.source.is_empty() { "css".into() } else { e.source.clone() };
                    match role {
                        "heading" => e.heading += 1,
                        "body" => e.body += 1,
                        _ => e.other += 1,
                    }
                }
            } else if is_face && prop == "src" {
                for c in url_re.captures_iter(val) {
                    if let Some(u) = abs(base, &c[1]) {
                        face_files.push(u);
                    }
                }
            } else if is_face && prop == "font-weight" {
                face_weight = val.split_whitespace().next().and_then(|n| n.parse::<u32>().ok());
            } else if matches!(prop.as_str(), "color" | "background" | "background-color" | "border-color" | "border" | "fill" | "stroke" | "outline-color")
                || prop.starts_with("--")
            {
                let w = if prop.starts_with("--") { color_weight_for(&prop, "") } else { color_weight_for(&sel_l, &prop) };
                let resolved = resolve_vars(val, &a.custom_props, 0);
                for m in color_re.find_iter(&resolved) {
                    if let Some(c) = parse_color(m.as_str()) {
                        a.colors.push((c, w, format!("css {prop} on {}", crate::util::truncate_chars(&selector, 30))));
                        if on_root && (prop == "background" || prop == "background-color") && a.bg_hint.is_none() {
                            a.bg_hint = Some(c);
                        }
                        if on_root && prop == "color" && a.text_hint.is_none() {
                            a.text_hint = Some(c);
                        }
                        if prop.starts_with("--") {
                            let n = prop.to_ascii_lowercase();
                            if (n.contains("bg") || n.contains("background")) && a.bg_hint.is_none() && !n.contains("accent") && !n.contains("primary") {
                                a.bg_hint = Some(c);
                            }
                            if (n.contains("text") || n.contains("foreground") || n.contains("fg")) && a.text_hint.is_none() {
                                a.text_hint = Some(c);
                            }
                        }
                    }
                }
            } else if matches!(prop.as_str(), "background-image") {
                for c in url_re.captures_iter(val) {
                    if let Some(u) = abs(base, &c[1]) {
                        a.images.push(Cand { url: u, score: 1, why: "css background-image".into(), ..Default::default() });
                    }
                }
            }
        }
        if is_face && !face_family.is_empty() {
            let e = a.fonts.entry(face_family).or_default();
            e.source = "font-face".into();
            e.files.extend(face_files);
            if let Some(w) = face_weight {
                if !e.weights.contains(&w) {
                    e.weights.push(w);
                }
            }
        }
    }
}

/// Default palettes of Bootstrap 5 and Tailwind's most used utility colours.
const FRAMEWORK_DEFAULTS: &[&str] = &[
    "#0d6efd", "#6610f2", "#6f42c1", "#d63384", "#dc3545", "#fd7e14", "#ffc107", "#198754", "#20c997", "#0dcaf0", "#0a58ca", "#0b5ed7", "#6c757d", "#212529", "#f8f9fa", "#3b82f6",
    "#ef4444", "#22c55e", "#f59e0b", "#6366f1", "#007bff", "#28a745", "#17a2b8",
];

fn near_gray(c: Rgb) -> bool {
    c.hsl().1 < 0.12
}

/// Turn weighted colour evidence into role-tagged palette tokens.
pub fn build_palette(a: &Analysis, logo_colors: &[(Rgb, f64)]) -> Vec<ColorToken> {
    // merge near-identical colours
    let mut merged: Vec<(Rgb, f64, String)> = Vec::new();
    let mut all = a.colors.clone();
    for (c, w) in logo_colors {
        all.push((*c, 2.0 * (0.5 + w), "logo".into()));
    }
    for (c, w, src) in all {
        // colours a CSS framework ships by default say nothing about the brand
        let w = if FRAMEWORK_DEFAULTS.contains(&c.hex().as_str()) && !src.starts_with("computed") && src != "logo" { w * 0.12 } else { w };
        if let Some(m) = merged.iter_mut().find(|m| m.0.dist(c) < 14.0) {
            m.1 += w;
        } else {
            merged.push((c, w, src));
        }
    }
    let bg = a.bg_hint.unwrap_or(Rgb(255, 255, 255));
    // body text is near-neutral and readable; a saturated or faint "text" hint is a link or a mistake
    let text_hint = a.text_hint.filter(|t| t.hsl().1 < 0.4 && contrast(*t, bg) >= 3.0);
    let text = text_hint.unwrap_or_else(|| if bg.luminance() > 0.4 { Rgb(17, 17, 17) } else { Rgb(240, 240, 240) });
    let mut out: Vec<ColorToken> = vec![
        ColorToken { role: "background".into(), hex: bg.hex(), weight: 1.0, source: if a.bg_hint.is_some() { "css".into() } else { "default".into() } },
        ColorToken { role: "text".into(), hex: text.hex(), weight: 1.0, source: if text_hint.is_some() { "css".into() } else { "default".into() } },
    ];
    // accents: saturated, readable against the background
    // (the text colour itself is not an accent, and neither is the browser's default link blue)
    let mut accents: Vec<&(Rgb, f64, String)> = merged
        .iter()
        .filter(|(c, _, _)| !near_gray(*c) && contrast(*c, bg) > 1.6 && c.dist(text) > 36.0 && c.dist(Rgb(0, 0, 238)) > 8.0 && c.dist(Rgb(85, 26, 139)) > 8.0)
        .collect();
    accents.sort_by(|x, y| y.1.partial_cmp(&x.1).unwrap_or(std::cmp::Ordering::Equal));
    let total: f64 = accents.iter().map(|x| x.1).sum::<f64>().max(1.0);
    let mut picked: Vec<Rgb> = Vec::new();
    for (c, w, src) in accents {
        if picked.iter().any(|p| p.dist(*c) < 60.0) {
            continue;
        }
        let role = match picked.len() {
            0 => "primary",
            1 => "secondary",
            2 => "tertiary",
            _ => "extra",
        };
        out.push(ColorToken { role: role.into(), hex: c.hex(), weight: ((w / total) * 100.0).round() / 100.0, source: src.clone() });
        picked.push(*c);
        if picked.len() >= 6 {
            break;
        }
    }
    // neutrals worth keeping (surface, border)
    for (c, _, src) in merged.iter().filter(|(c, w, _)| near_gray(*c) && *w >= 2.0 && c.dist(bg) > 12.0 && c.dist(text) > 12.0).take(2) {
        out.push(ColorToken { role: "neutral".into(), hex: c.hex(), weight: 0.0, source: src.clone() });
    }
    out
}

/// Fonts every machine has: evidence of a fallback stack, not of a brand.
const SYSTEM_FONTS: &[&str] = &[
    "arial",
    "helvetica",
    "helvetica neue",
    "times",
    "times new roman",
    "georgia",
    "verdana",
    "tahoma",
    "trebuchet ms",
    "courier",
    "courier new",
    "consolas",
    "menlo",
    "monaco",
    "sfmono-regular",
    "sf mono",
    "liberation mono",
    "segoe ui",
    "roboto mono",
    "lucida console",
    "apple color emoji",
    "segoe ui emoji",
];

pub fn build_fonts(a: &Analysis) -> Vec<FontToken> {
    // "aaargh" and "Aaargh" are one family: merge spellings, keep the best-cased name
    let mut merged: Vec<(String, FontAcc)> = Vec::new();
    for (name, acc) in &a.fonts {
        match merged.iter_mut().find(|(n, _)| n.eq_ignore_ascii_case(name)) {
            Some((n, m)) => {
                if name.chars().any(|c| c.is_uppercase()) && !n.chars().any(|c| c.is_uppercase()) {
                    *n = name.clone();
                }
                m.heading += acc.heading;
                m.body += acc.body;
                m.other += acc.other;
                m.files.extend(acc.files.clone());
                m.weights.extend(acc.weights.clone());
                m.weights.dedup();
                if m.source.is_empty() || acc.source == "font-face" || acc.source == "computed" {
                    m.source = acc.source.clone();
                }
            }
            None => merged.push((name.clone(), acc.clone())),
        }
    }
    // a system font only counts when the site actually sets text in it
    merged.retain(|(n, acc)| !SYSTEM_FONTS.contains(&n.to_ascii_lowercase().as_str()) || acc.heading + acc.body > 0);
    let mut fam: Vec<(&String, &FontAcc)> = merged.iter().map(|(n, acc)| (n, acc)).collect();
    fam.sort_by(|x, y| (y.1.heading + y.1.body + y.1.other).cmp(&(x.1.heading + x.1.body + x.1.other)).then(x.0.cmp(y.0)));
    let heading = fam.iter().max_by_key(|f| f.1.heading).filter(|f| f.1.heading > 0).map(|f| f.0.clone());
    let body = fam.iter().max_by_key(|f| f.1.body).filter(|f| f.1.body > 0).map(|f| f.0.clone());
    let mut out = Vec::new();
    for (name, acc) in fam.iter().take(6) {
        let role = if Some((*name).clone()) == heading {
            "heading"
        } else if Some((*name).clone()) == body {
            "body"
        } else {
            "other"
        };
        let mut weights = acc.weights.clone();
        weights.sort();
        out.push(FontToken {
            family: (*name).clone(),
            role: role.into(),
            source: acc.source.clone(),
            files: acc.files.clone(),
            weights,
            count: acc.heading + acc.body + acc.other,
        });
    }
    // if no explicit roles were found, the most used family is both
    if !out.is_empty() && !out.iter().any(|f| f.role == "heading") {
        out[0].role = "heading".into();
    }
    if out.len() == 1 && out[0].role == "heading" {
        let mut b = out[0].clone();
        b.role = "body".into();
        out.push(b);
    }
    out
}

const EN: &[&str] = &["the", "and", "of", "to", "in", "is", "for", "with", "that", "you", "your", "are", "on"];
const ES: &[&str] = &["el", "la", "de", "que", "y", "en", "los", "las", "un", "una", "para", "con", "por", "tu", "es"];
const PT: &[&str] = &["o", "a", "de", "que", "e", "do", "da", "em", "um", "uma", "para", "com", "não", "você"];
const FR: &[&str] = &["le", "la", "de", "et", "les", "des", "un", "une", "pour", "vous", "est", "dans", "avec"];
const DE: &[&str] = &["der", "die", "das", "und", "ist", "nicht", "mit", "für", "ein", "eine", "sie", "wir"];

pub fn detect_language(text: &str) -> String {
    let words: Vec<String> = text.split(|c: char| !c.is_alphabetic()).map(|w| w.to_lowercase()).filter(|w| !w.is_empty()).collect();
    let score = |set: &[&str]| words.iter().filter(|w| set.contains(&w.as_str())).count();
    let mut best = ("en", score(EN));
    for (l, s) in [("es", score(ES)), ("pt", score(PT)), ("fr", score(FR)), ("de", score(DE))] {
        if s > best.1 {
            best = (l, s);
        }
    }
    best.0.to_string()
}

pub fn analyze_voice(a: &Analysis, lang_hint: Option<&str>) -> Voice {
    let mut sentences = 0usize;
    let mut words = 0usize;
    let mut excl = 0usize;
    let mut quest = 0usize;
    let mut second = 0usize;
    let mut caps = 0usize;
    let second_set: &[&str] = &["you", "your", "tu", "tus", "usted", "vous", "votre", "você", "voce", "sie", "ihr", "vos"];
    let joined = a.texts.join(" ");
    for t in &a.texts {
        for s in t.split(['.', '!', '?']).map(|s| s.trim()).filter(|s| s.split_whitespace().count() >= 2) {
            sentences += 1;
            for w in s.split_whitespace() {
                words += 1;
                let wl = w.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase();
                if second_set.contains(&wl.as_str()) {
                    second += 1;
                }
                if wl.chars().count() >= 3 && w.trim_matches(|c: char| !c.is_alphanumeric()).chars().all(|c| c.is_uppercase() || !c.is_alphabetic()) {
                    caps += 1;
                }
            }
        }
        excl += t.matches('!').count();
        quest += t.matches('?').count();
    }
    let lang = lang_hint.map(String::from).filter(|l| !l.is_empty()).unwrap_or_else(|| detect_language(&joined));
    let sn = sentences.max(1) as f64;
    let wn = words.max(1) as f64;
    let avg = words as f64 / sn;
    let exclamations = excl as f64 / sn;
    let question_ratio = quest as f64 / sn;
    let second_person = second as f64 / wn;
    let uppercase_ratio = caps as f64 / wn;
    let mut d = Vec::new();
    if sentences > 0 {
        d.push(
            if avg <= 8.0 {
                "short, direct sentences"
            } else if avg >= 18.0 {
                "long, explanatory sentences"
            } else {
                "medium-length sentences"
            }
            .to_string(),
        );
    }
    if exclamations > 0.1 {
        d.push("enthusiastic (frequent exclamations)".into());
    }
    if question_ratio > 0.1 {
        d.push("asks questions".into());
    }
    if second_person > 0.04 {
        d.push("speaks to the reader directly".into());
    }
    if uppercase_ratio > 0.2 {
        d.push("uses capitals for emphasis".into());
    }
    let imperative = [
        "get", "start", "try", "join", "download", "discover", "learn", "buy", "shop", "book", "contact", "sign", "see", "explore", "empieza", "descubre", "compra", "únete",
        "contacta", "reserva", "prueba",
    ];
    if a.ctas.iter().filter(|c| c.split_whitespace().next().map(|w| imperative.contains(&w.to_lowercase().as_str())).unwrap_or(false)).count() >= 2 {
        d.push("imperative calls to action".into());
    }
    let mut samples: Vec<String> = a.texts.iter().filter(|t| (20..=130).contains(&t.chars().count())).cloned().collect();
    samples.dedup();
    samples.truncate(6);
    Voice {
        descriptors: d,
        samples,
        exclamations: (exclamations * 100.0).round() / 100.0,
        avg_sentence_words: (avg * 10.0).round() / 10.0,
        second_person: (second_person * 1000.0).round() / 1000.0,
        uppercase_ratio: (uppercase_ratio * 100.0).round() / 100.0,
        language: lang,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HTML: &str = r##"<html lang="es-MX"><head><title>Acme Studio</title>
<meta name="description" content="Hacemos video con carácter. Empieza hoy!">
<meta name="theme-color" content="#ff3d1f">
<link rel="stylesheet" href="/site.css">
<link href="https://fonts.googleapis.com/css2?family=Inter:wght@400;700&family=Lora" rel="stylesheet">
<style>:root{--brand-primary:#ff3d1f;--bg:#0e0e10} body{background:#0e0e10;color:#efe7d8;font-family:'Space Grotesk',sans-serif} h1{font-family:"Bahnschrift"} .btn{background:#ff3d1f}</style>
</head><body><header><a href="/"><img src="/img/logo.svg" alt="Acme logo" class="site-logo"></a></header>
<h1>Video con carácter propio</h1><p>Tu marca merece una edición que no se parezca a ninguna otra, y lo sabemos.</p>
<a class="btn" href="/contacto">Empieza hoy</a><a class="btn" href="/demo">Descubre más</a>
<img src="/img/hero.jpg" alt="hero"><a href="https://www.instagram.com/acme">ig</a></body></html>"##;

    #[test]
    fn extracts_meta_logo_and_social() {
        let base = Url::parse("https://acme.test/").unwrap();
        let a = parse_html(HTML, &base);
        assert_eq!(a.title.as_deref(), Some("Acme Studio"));
        assert_eq!(a.language.as_deref(), Some("es"));
        assert_eq!(a.logos[0].url, "https://acme.test/img/logo.svg");
        assert!(a.images.iter().any(|i| i.url.ends_with("hero.jpg")));
        assert!(a.social.iter().any(|s| s.network == "instagram"));
        assert!(a.stylesheets.contains(&"https://acme.test/site.css".to_string()));
    }

    #[test]
    fn extracts_fonts_and_palette() {
        let base = Url::parse("https://acme.test/").unwrap();
        let a = parse_html(HTML, &base);
        let fonts = build_fonts(&a);
        assert!(fonts.iter().any(|f| f.family == "Bahnschrift" && f.role == "heading"));
        assert!(fonts.iter().any(|f| f.family == "Inter" && f.source == "google-fonts"));
        let pal = build_palette(&a, &[]);
        assert_eq!(pal.iter().find(|c| c.role == "background").unwrap().hex, "#0e0e10");
        assert_eq!(pal.iter().find(|c| c.role == "primary").unwrap().hex, "#ff3d1f");
    }

    #[test]
    fn voice_detects_language_and_ctas() {
        let base = Url::parse("https://acme.test/").unwrap();
        let a = parse_html(HTML, &base);
        let v = analyze_voice(&a, None);
        assert_eq!(v.language, "es");
        assert!(v.descriptors.iter().any(|d| d.contains("imperative")));
    }
}
