//! Derives the project's "edit brand" (how it moves, cuts and looks in motion)
//! from the extracted brand. Deterministic: same brand + seed, same tokens, and
//! different brands land on measurably different curves, durations and rules.

use crate::color::{contrast, parse_color, Rgb};
use crate::model::{Brand, ColorRoles, Easings, EditBrand, TypeTokens};

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(seed | 1)
    }
    fn next(&mut self) -> u64 {
        // xorshift64*
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn jitter(&mut self, amp: f64) -> f64 {
        (self.unit() * 2.0 - 1.0) * amp
    }
}

fn fnv1a(s: &str) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

fn r2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

fn bez(rng: &mut Rng, base: [f64; 4], amp: f64) -> [f64; 4] {
    [r2((base[0] + rng.jitter(amp)).clamp(0.0, 1.0)), r2(base[1] + rng.jitter(amp * 0.4)), r2((base[2] + rng.jitter(amp)).clamp(0.0, 1.0)), r2(base[3] + rng.jitter(amp * 0.4))]
}

fn is_serif(family: &str) -> bool {
    let f = family.to_ascii_lowercase();
    ["serif", "garamond", "times", "playfair", "georgia", "merriweather", "lora", "baskerville", "didot", "sitka", "cormorant"].iter().any(|k| f.contains(k)) && !f.contains("sans")
}

/// Energy in 0..1 from voice, palette and type. Explained in `rationale`.
fn energy(b: &Brand, why: &mut Vec<String>) -> f64 {
    let mut e: f64 = 0.5;
    let accent = b.palette.iter().filter_map(|c| parse_color(&c.hex)).map(|c| c.hsl()).fold((0.0, 0.0, 0.0), |a, h| if h.1 > a.1 { h } else { a });
    if accent.1 > 0.0 {
        let d = (accent.1 - 0.45) * 0.5;
        e += d;
        why.push(format!("accent saturation {:.2} moves energy {:+.2}", accent.1, d));
    }
    let v = &b.voice;
    if v.avg_sentence_words > 0.0 {
        let d = if v.avg_sentence_words < 9.0 {
            0.1
        } else if v.avg_sentence_words > 18.0 {
            -0.12
        } else {
            0.0
        };
        if d != 0.0 {
            e += d;
            why.push(format!("average sentence {:.0} words moves energy {:+.2}", v.avg_sentence_words, d));
        }
    }
    if v.exclamations > 0.08 {
        e += 0.1;
        why.push("frequent exclamations: +0.10".into());
    }
    if v.uppercase_ratio > 0.25 {
        e += 0.06;
        why.push("heavy uppercase: +0.06".into());
    }
    if let Some(f) = b.fonts.iter().find(|f| f.role == "heading") {
        if is_serif(&f.family) {
            e -= 0.12;
            why.push(format!("serif heading ({}): -0.12", f.family));
        }
    }
    let dark = b.palette.iter().find(|c| c.role == "background").and_then(|c| parse_color(&c.hex)).map(|c| c.luminance() < 0.12).unwrap_or(false);
    if dark {
        e -= 0.05;
        why.push("dark background: -0.05".into());
    }
    e.clamp(0.0, 1.0)
}

pub fn derive_edit_brand(b: &Brand) -> EditBrand {
    let host = b.source_url.as_deref().and_then(|u| url::Url::parse(u).ok()).and_then(|u| u.host_str().map(String::from)).unwrap_or_default();
    // 52 bits: the scene runtime reads it as a JS number, which is exact only below 2^53.
    let seed = fnv1a(&format!("{}|{}", b.name.to_lowercase(), host)) & ((1u64 << 52) - 1);
    let mut rng = Rng::new(seed);
    let mut why: Vec<String> = Vec::new();

    let e = energy(b, &mut why);
    let tempo = if e > 0.62 {
        "punchy"
    } else if e < 0.38 {
        "calm"
    } else {
        "steady"
    };
    why.push(format!("energy {:.2} -> tempo {tempo}", e));

    // easings: a family per tempo, then numerically unique per brand
    let (enter, exit, travel, impact): ([f64; 4], [f64; 4], [f64; 4], [f64; 4]) = match tempo {
        "punchy" => ([0.12, 1.0, 0.28, 1.0], [0.76, 0.0, 1.0, 0.5], [0.62, 0.0, 0.2, 1.0], [0.3, 1.5, 0.5, 1.0]),
        "calm" => ([0.16, 0.84, 0.28, 1.0], [0.42, 0.0, 0.3, 1.0], [0.45, 0.05, 0.2, 1.0], [0.25, 1.15, 0.45, 1.0]),
        _ => ([0.22, 0.9, 0.34, 1.0], [0.6, 0.0, 0.9, 0.45], [0.55, 0.0, 0.2, 1.0], [0.28, 1.3, 0.48, 1.0]),
    };
    let easings = Easings { enter: bez(&mut rng, enter, 0.06), exit: bez(&mut rng, exit, 0.06), travel: bez(&mut rng, travel, 0.06), impact: bez(&mut rng, impact, 0.05) };

    let (base_d, stagger, shrink, hold): (Vec<u32>, u32, f64, u32) = match tempo {
        "punchy" => (vec![4, 8, 14, 26], 28, 0.82, 6),
        "calm" => (vec![9, 16, 28, 48], 58, 0.9, 16),
        _ => (vec![6, 11, 19, 34], 40, 0.86, 10),
    };
    let durations: Vec<u32> = base_d.iter().map(|d| (*d as i64 + (rng.jitter(1.6)).round() as i64).max(2) as u32).collect();
    let stagger_ms = (stagger as f64 + rng.jitter(6.0)).round().max(12.0) as u32;
    let stagger_shrink = r2(shrink + rng.jitter(0.03));

    let step_fps = if rng.unit() < 0.3 { Some(if tempo == "punchy" { 15 } else { 12 }) } else { None };
    let motion_blur = tempo == "punchy" || rng.unit() < 0.2;

    let lib: [&str; 9] = ["cut-on-motion", "whip-pan", "slide-push", "mask-wipe", "scale-through", "match-cut", "hard-cut", "luma-wipe", "light-leak"];
    let picks: [usize; 3] = match tempo {
        "punchy" => [1, 6, 4],
        "calm" => [3, 5, 7],
        _ => [0, 3, 2],
    };
    let rot = (seed % 3) as usize;
    let mut transitions: Vec<String> = picks.iter().map(|i| lib[*i].to_string()).collect();
    transitions.rotate_left(rot);
    if rng.unit() < 0.35 {
        transitions[2] = lib[8].to_string();
    }

    // colours
    let tok = |role: &str| b.palette.iter().find(|c| c.role == role).and_then(|c| parse_color(&c.hex));
    let bg = tok("background").unwrap_or(Rgb(14, 14, 16));
    let fg = tok("text").unwrap_or_else(|| if bg.luminance() < 0.4 { Rgb(240, 236, 228) } else { Rgb(18, 18, 20) });
    // Graphics sit on top of footage, not on the website's page: the light colour of
    // the pair carries the type and the dark one is the plate behind it.
    let (bg, fg) = if fg.luminance() < bg.luminance() { (fg, bg) } else { (bg, fg) };
    let mut accent_pick: Option<Rgb> = None;
    let mut best = 0.0;
    for c in b.palette.iter().filter_map(|c| parse_color(&c.hex)) {
        let (_, s, l) = c.hsl();
        let score = s * (1.0 - (l - 0.5).abs()) * if contrast(c, bg) >= 3.0 { 1.0 } else { 0.4 };
        if score > best {
            best = score;
            accent_pick = Some(c);
        }
    }
    // a stated primary is the accent, even when it is white or grey: a monochrome
    // identity stays monochrome instead of being handed an invented colour
    let accent = match (tok("primary"), accent_pick) {
        (Some(p), _) if contrast(p, bg) >= 2.0 => p,
        (Some(_), Some(c)) => c,
        (Some(p), None) => p,
        (None, Some(c)) => c,
        (None, None) => {
            why.push("no usable accent in the palette: derived one from the seed".into());
            Rgb::from_hsl((seed % 360) as f64, 0.78, 0.55)
        }
    };
    let (ah, as_, al) = accent.hsl();
    let monochrome = as_ < 0.12;
    if monochrome {
        why.push("the brand is monochrome: no second hue is invented".into());
    }
    let accent2 = tok("secondary").filter(|c| c.dist(accent) > 30.0).unwrap_or_else(|| {
        if monochrome {
            return Rgb(((fg.0 as u16 + bg.0 as u16) / 2) as u8, ((fg.1 as u16 + bg.1 as u16) / 2) as u8, ((fg.2 as u16 + bg.2 as u16) / 2) as u8);
        }
        b.palette
            .iter()
            .filter_map(|c| parse_color(&c.hex))
            .find(|c| {
                let (h, s, _) = c.hsl();
                s > 0.2 && ((h - ah).abs().min(360.0 - (h - ah).abs())) > 50.0
            })
            .unwrap_or_else(|| Rgb::from_hsl(ah + 150.0, as_.min(0.7), al.clamp(0.4, 0.62)))
    });
    let muted = Rgb(((fg.0 as u16 + bg.0 as u16 * 2) / 3) as u8, ((fg.1 as u16 + bg.1 as u16 * 2) / 3) as u8, ((fg.2 as u16 + bg.2 as u16 * 2) / 3) as u8);

    // type
    let heading_brand = b.fonts.iter().find(|f| f.role == "heading").map(|f| f.family.clone());
    let body_brand = b.fonts.iter().find(|f| f.role == "body").map(|f| f.family.clone());
    // a typeface the machine really has, so the look does not silently fall back
    let fallback = match (tempo, std::env::consts::OS) {
        ("punchy", "windows") => "Bahnschrift",
        ("calm", "windows") => "Sitka",
        (_, "windows") => "Segoe UI Variable",
        ("punchy", "macos") => "Avenir Next Condensed",
        ("calm", "macos") => "New York",
        (_, "macos") => "Avenir Next",
        ("punchy", _) => "DejaVu Sans Condensed",
        ("calm", _) => "DejaVu Serif",
        _ => "DejaVu Sans",
    };
    let heading = heading_brand.clone().unwrap_or_else(|| fallback.to_string());
    if heading_brand.is_none() {
        why.push(format!("no heading font found: {fallback} for tempo {tempo}"));
    }
    let body = body_brand.unwrap_or_else(|| heading.clone());
    let type_tokens = TypeTokens {
        heading: heading.clone(),
        body: body.clone(),
        mono: match std::env::consts::OS {
            "windows" => "Cascadia Mono",
            "macos" => "Menlo",
            _ => "DejaVu Sans Mono",
        }
        .into(),
        weight_range: if tempo == "punchy" {
            [300, 800]
        } else if tempo == "calm" {
            [300, 600]
        } else {
            [350, 700]
        },
    };

    let transitions_str = transitions.join(", ");
    let mut rules = vec![
        "Entrances are mask reveals, scale or hard cuts. Fade plus slide-up is not part of this brand.".to_string(),
        format!("Durations come from the palette {durations:?} frames at 30 fps. A new value needs a reason in memory."),
        format!("Accent {} appears on one element per shot.", accent.hex()),
        format!("Hold {hold} frames of stillness before any climax.", hold = hold),
        format!("Transitions are limited to: {transitions_str}."),
        format!("Type: {heading} for headlines, {body} for everything else; two families at most."),
    ];
    if let Some(s) = step_fps {
        rules.push(format!("Graphic elements animate on {s} fps steps; footage and camera never do."));
    }
    if motion_blur {
        rules.push("Use sub-frame motion blur only on slams, whips and spins; never on text that must be read.".to_string());
    }

    EditBrand {
        seed,
        tempo: tempo.to_string(),
        easings,
        durations,
        stagger_ms,
        stagger_shrink,
        hold_frames: hold,
        step_fps,
        motion_blur,
        transitions,
        type_tokens,
        colors: ColorRoles { bg: bg.hex(), fg: fg.hex(), accent: accent.hex(), accent2: accent2.hex(), muted: muted.hex() },
        rules,
        rationale: why,
        locked: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ColorToken, FontToken, Voice};

    fn brand(name: &str, hex: &str, sentence: f64, serif: bool) -> Brand {
        Brand {
            name: name.into(),
            palette: vec![
                ColorToken { role: "background".into(), hex: "#ffffff".into(), weight: 1.0, source: "css".into() },
                ColorToken { role: "accent".into(), hex: hex.into(), weight: 1.0, source: "css".into() },
            ],
            fonts: vec![FontToken { family: if serif { "Playfair Display".into() } else { "Inter".into() }, role: "heading".into(), ..Default::default() }],
            voice: Voice { avg_sentence_words: sentence, ..Default::default() },
            ..Default::default()
        }
    }

    #[test]
    fn deterministic() {
        let b = brand("Acme", "#ff3d1f", 7.0, false);
        let a1 = derive_edit_brand(&b);
        let a2 = derive_edit_brand(&b);
        assert_eq!(serde_json::to_string(&a1).unwrap(), serde_json::to_string(&a2).unwrap());
    }

    #[test]
    fn different_brands_diverge() {
        let loud = derive_edit_brand(&brand("Loud Co", "#ff2a00", 6.0, false));
        let quiet = derive_edit_brand(&brand("Quiet House", "#6b7a8f", 24.0, true));
        assert_eq!(loud.tempo, "punchy");
        assert_eq!(quiet.tempo, "calm");
        assert_ne!(loud.easings.enter, quiet.easings.enter);
        assert_ne!(loud.durations, quiet.durations);
    }
}
