//! Minimal colour maths: parsing, HSL, contrast, quantisation of images.

use image::GenericImageView;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub fn hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }

    /// (h in 0..360, s in 0..1, l in 0..1)
    pub fn hsl(self) -> (f64, f64, f64) {
        let (r, g, b) = (self.0 as f64 / 255.0, self.1 as f64 / 255.0, self.2 as f64 / 255.0);
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let l = (max + min) / 2.0;
        if (max - min).abs() < 1e-9 {
            return (0.0, 0.0, l);
        }
        let d = max - min;
        let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
        let h = if (max - r).abs() < 1e-9 {
            ((g - b) / d) + if g < b { 6.0 } else { 0.0 }
        } else if (max - g).abs() < 1e-9 {
            (b - r) / d + 2.0
        } else {
            (r - g) / d + 4.0
        };
        (h * 60.0, s, l)
    }

    pub fn from_hsl(h: f64, s: f64, l: f64) -> Rgb {
        let h = h.rem_euclid(360.0) / 360.0;
        if s <= 0.0 {
            let v = (l * 255.0).round() as u8;
            return Rgb(v, v, v);
        }
        let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
        let p = 2.0 * l - q;
        let f = |mut t: f64| {
            if t < 0.0 {
                t += 1.0;
            }
            if t > 1.0 {
                t -= 1.0;
            }
            let v = if t < 1.0 / 6.0 {
                p + (q - p) * 6.0 * t
            } else if t < 0.5 {
                q
            } else if t < 2.0 / 3.0 {
                p + (q - p) * (2.0 / 3.0 - t) * 6.0
            } else {
                p
            };
            (v * 255.0).round().clamp(0.0, 255.0) as u8
        };
        Rgb(f(h + 1.0 / 3.0), f(h), f(h - 1.0 / 3.0))
    }

    pub fn luminance(self) -> f64 {
        let ch = |c: u8| {
            let v = c as f64 / 255.0;
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * ch(self.0) + 0.7152 * ch(self.1) + 0.0722 * ch(self.2)
    }

    pub fn dist(self, o: Rgb) -> f64 {
        let dr = self.0 as f64 - o.0 as f64;
        let dg = self.1 as f64 - o.1 as f64;
        let db = self.2 as f64 - o.2 as f64;
        (dr * dr + dg * dg + db * db).sqrt()
    }
}

pub fn contrast(a: Rgb, b: Rgb) -> f64 {
    let (la, lb) = (a.luminance(), b.luminance());
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

const NAMED: &[(&str, (u8, u8, u8))] = &[
    ("black", (0, 0, 0)),
    ("white", (255, 255, 255)),
    ("red", (255, 0, 0)),
    ("green", (0, 128, 0)),
    ("blue", (0, 0, 255)),
    ("yellow", (255, 255, 0)),
    ("orange", (255, 165, 0)),
    ("purple", (128, 0, 128)),
    ("gray", (128, 128, 128)),
    ("grey", (128, 128, 128)),
];

/// Parse `#rgb`, `#rrggbb`, `#rrggbbaa`, `rgb()/rgba()`, `hsl()/hsla()` and a few names.
/// Returns None for fully transparent colours.
pub fn parse_color(input: &str) -> Option<Rgb> {
    let s = input.trim().to_ascii_lowercase();
    if let Some(h) = s.strip_prefix('#') {
        let h: String = h.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
        let expand = |c: char| format!("{c}{c}");
        let full = match h.len() {
            3 | 4 => h.chars().map(expand).collect::<String>(),
            6 | 8 => h,
            _ => return None,
        };
        let p = |i: usize| u8::from_str_radix(&full[i..i + 2], 16).ok();
        if full.len() == 8 && p(6)? < 8 {
            return None;
        }
        return Some(Rgb(p(0)?, p(2)?, p(4)?));
    }
    let inner = |name: &str| -> Option<Vec<f64>> {
        let rest = s.strip_prefix(name)?;
        let rest = rest.strip_prefix('(')?;
        let end = rest.find(')')?;
        let nums: Vec<f64> = rest[..end].split([',', ' ', '/']).filter(|t| !t.is_empty()).filter_map(|t| t.trim_end_matches(['%', 'd', 'e', 'g']).parse::<f64>().ok()).collect();
        Some(nums)
    };
    for name in ["rgba", "rgb"] {
        if let Some(n) = inner(name) {
            if n.len() >= 3 {
                if n.len() >= 4 && n[3] < 0.03 {
                    return None;
                }
                let c = |v: f64| v.round().clamp(0.0, 255.0) as u8;
                return Some(Rgb(c(n[0]), c(n[1]), c(n[2])));
            }
        }
    }
    for name in ["hsla", "hsl"] {
        if let Some(n) = inner(name) {
            if n.len() >= 3 {
                if n.len() >= 4 && n[3] < 0.03 {
                    return None;
                }
                return Some(Rgb::from_hsl(n[0], (n[1] / 100.0).clamp(0.0, 1.0), (n[2] / 100.0).clamp(0.0, 1.0)));
            }
        }
    }
    NAMED.iter().find(|(n, _)| *n == s).map(|(_, (r, g, b))| Rgb(*r, *g, *b))
}

/// Median-cut palette of an image: up to `k` colours with their share of pixels.
/// Near-transparent pixels are ignored; the image is downscaled first.
pub fn palette_of(img: &image::DynamicImage, k: usize) -> Vec<(Rgb, f64)> {
    let (w, h) = img.dimensions();
    let small = if w.max(h) > 96 { img.thumbnail(96, 96) } else { img.clone() };
    let rgba = small.to_rgba8();
    let mut px: Vec<[u8; 3]> = rgba.pixels().filter(|p| p.0[3] > 40).map(|p| [p.0[0], p.0[1], p.0[2]]).collect();
    if px.is_empty() {
        return Vec::new();
    }
    let total = px.len() as f64;
    let mut boxes: Vec<Vec<[u8; 3]>> = vec![std::mem::take(&mut px)];
    while boxes.len() < k {
        // split the box with the widest channel range and most pixels
        let mut best = None;
        let mut best_score = 0.0;
        for (i, b) in boxes.iter().enumerate() {
            if b.len() < 2 {
                continue;
            }
            let (ch, range) = widest(b);
            let score = range as f64 * (b.len() as f64).sqrt();
            if score > best_score {
                best_score = score;
                best = Some((i, ch));
            }
        }
        let Some((i, ch)) = best else { break };
        let mut b = boxes.swap_remove(i);
        b.sort_by_key(|p| p[ch]);
        let mid = b.len() / 2;
        let right = b.split_off(mid);
        boxes.push(b);
        boxes.push(right);
    }
    let mut out: Vec<(Rgb, f64)> = boxes
        .iter()
        .filter(|b| !b.is_empty())
        .map(|b| {
            let n = b.len() as f64;
            let sum = b.iter().fold([0u64; 3], |a, p| [a[0] + p[0] as u64, a[1] + p[1] as u64, a[2] + p[2] as u64]);
            (Rgb((sum[0] as f64 / n) as u8, (sum[1] as f64 / n) as u8, (sum[2] as f64 / n) as u8), n / total)
        })
        .collect();
    out.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    out
}

fn widest(b: &[[u8; 3]]) -> (usize, u8) {
    let mut best = (0, 0u8);
    for ch in 0..3 {
        let (lo, hi) = b.iter().fold((255u8, 0u8), |(lo, hi), p| (lo.min(p[ch]), hi.max(p[ch])));
        if hi - lo >= best.1 {
            best = (ch, hi - lo);
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_forms() {
        assert_eq!(parse_color("#FF3D1F"), Some(Rgb(255, 61, 31)));
        assert_eq!(parse_color("#f00"), Some(Rgb(255, 0, 0)));
        assert_eq!(parse_color("rgb(16, 16, 96)"), Some(Rgb(16, 16, 96)));
        assert_eq!(parse_color("rgba(0,0,0,0)"), None);
        assert_eq!(parse_color("hsl(0, 100%, 50%)"), Some(Rgb(255, 0, 0)));
        assert_eq!(parse_color("nonsense"), None);
    }

    #[test]
    fn contrast_black_white() {
        assert!((contrast(Rgb(0, 0, 0), Rgb(255, 255, 255)) - 21.0).abs() < 0.01);
    }

    #[test]
    fn palette_finds_two_colours() {
        let mut img = image::RgbaImage::new(20, 20);
        for (x, _, p) in img.enumerate_pixels_mut() {
            *p = if x < 10 { image::Rgba([255, 0, 0, 255]) } else { image::Rgba([0, 0, 255, 255]) };
        }
        let pal = palette_of(&image::DynamicImage::ImageRgba8(img), 2);
        assert_eq!(pal.len(), 2);
        assert!((pal[0].1 - 0.5).abs() < 0.05);
    }
}
