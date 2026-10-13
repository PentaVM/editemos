//! Scene templates. Each one is a self-contained HTML file driven by the project's
//! edit brand (easings, durations, stagger, hold, type, colours), so the same
//! template looks and moves differently in every project.

use super::fonts;
use crate::error::{fail, Result};
use crate::model::EditBrand;
use crate::store::Project;
use serde_json::Value;

pub const KINDS: &[&str] = &["title", "lower-third", "quote", "chapter", "callout", "caption"];

const PRELUDE: &str = r#"
const $ = (s) => document.querySelector(s);
const css = (el, o) => { for (const k in o) { if (k.startsWith('--')) el.style.setProperty(k, o[k]); else el.style[k] = o[k]; } return el; };
const P = /*PARAMS*/{};
const T = Object.assign({}, Motion.tokens, /*TOKENS*/{});
const W = innerWidth, H = innerHeight, FPS = P.fps || 30;
const S = H / 1080;                                   // everything is designed at 1080p
const sec = (f) => f / 30;                            // token durations are frames at 30 fps
const D = (T.durations && T.durations.length ? T.durations : [6, 11, 19, 34]).map(sec);
// a token set that exists but is empty (a brand that was never derived) falls back too
const flat = (e) => !e || !e.enter || e.enter.every((v) => v === 0);
const E = flat(T.easings) ? { enter: [.22, .9, .34, 1], exit: [.6, 0, .9, .45], travel: [.55, 0, .2, 1], impact: [.28, 1.3, .48, 1] } : T.easings;
const HOLD = sec(T.hold_frames || 10);
const C = (T.colors && T.colors.fg) ? T.colors : { bg: '#0e0e10', fg: '#efe7d8', accent: '#ff3d1f', accent2: '#6bd0ff', muted: '#8a867d' };
const TY = (T.type_tokens && T.type_tokens.heading) ? T.type_tokens : { heading: 'Bahnschrift', body: 'Bahnschrift', mono: 'Cascadia Mono', weight_range: [300, 700] };
const STEP = T.step_fps ? 1 / T.step_fps : 0;
const ST = (T.stagger_ms || 40) / 1000, SHRINK = T.stagger_shrink || 0.86;
const SEED = Number((T.seed || 1) % 100000), V = SEED % 3, R = Motion.rng(SEED + 7);
const TEMPO = T.tempo || 'steady';
const MX = 0.08 * W, MY = 0.09 * H;                   // safe margins
const rs = document.documentElement.style;
rs.setProperty('--fg', C.fg); rs.setProperty('--accent', C.accent); rs.setProperty('--accent2', C.accent2);
rs.setProperty('--muted', C.muted); rs.setProperty('--bg', C.bg);
rs.setProperty('--h', `'${TY.heading}', 'Bahnschrift', sans-serif`);
rs.setProperty('--b', `'${TY.body}', 'Bahnschrift', sans-serif`);
rs.setProperty('--m', `'${TY.mono}', 'Cascadia Mono', Consolas, monospace`);
const W0 = TY.weight_range[0], W1 = TY.weight_range[1];
"#;

const HEAD: &str = r#"<!doctype html><html><head><meta charset="utf-8"><title>editemos scene</title><style>
/*FONTS*/
html,body{margin:0;width:__W__px;height:__H__px;overflow:hidden;background:transparent}
#stage{position:absolute;inset:0;color:var(--fg);font-family:var(--b)}
.h{font-family:var(--h);font-variation-settings:'wght' var(--wght,500),'wdth' var(--wdth,100);white-space:nowrap}
.m{font-family:var(--m);letter-spacing:.14em;text-transform:uppercase;white-space:nowrap}
.mask{position:absolute;overflow:hidden}
.abs{position:absolute}
</style></head><body><div id="stage">"#;

fn tpl(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "title" => include_str!("../../templates/title.html"),
        "lower-third" => include_str!("../../templates/lower-third.html"),
        "quote" => include_str!("../../templates/quote.html"),
        "chapter" => include_str!("../../templates/chapter.html"),
        "callout" => include_str!("../../templates/callout.html"),
        "caption" => include_str!("../../templates/caption.html"),
        _ => return None,
    })
}

/// Build a scene file for `kind`. Params are template-specific JSON
/// (e.g. `{"text": "Quiebre", "kicker": "SEC.02"}`); `width`/`height` default to 1920x1080.
pub fn build(kind: &str, params: &Value, edit: &EditBrand, project: Option<&Project>, width: u32, height: u32) -> Result<String> {
    let Some(body) = tpl(kind) else {
        return fail(format!("unknown template '{kind}'. Available: {}", KINDS.join(", ")));
    };
    let families = [edit.type_tokens.heading.as_str(), edit.type_tokens.body.as_str(), edit.type_tokens.mono.as_str()];
    let font_css = fonts::face_css(project, &families);
    let tokens = serde_json::to_string(edit)?;
    let params_json = serde_json::to_string(params)?;
    let prelude = PRELUDE.replace("/*PARAMS*/{}", &params_json).replace("/*TOKENS*/{}", &tokens);
    let html = format!("{HEAD}{body}").replace("/*FONTS*/", &font_css).replace("/*PRELUDE*/", &prelude).replace("__W__", &width.to_string()).replace("__H__", &height.to_string());
    Ok(html)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Brand, ColorToken};

    #[test]
    fn builds_every_template() {
        let brand = Brand { name: "T".into(), palette: vec![ColorToken { role: "primary".into(), hex: "#ff3d1f".into(), weight: 1.0, source: "t".into() }], ..Default::default() };
        let edit = crate::brand::derive_edit_brand(&brand);
        for k in KINDS {
            let html = build(k, &serde_json::json!({"text": "Hola mundo", "name": "Ana", "role": "Director"}), &edit, None, 1920, 1080).unwrap();
            assert!(html.contains("Motion.mount"), "{k} must mount");
            assert!(!html.contains("/*PRELUDE*/") && !html.contains("__W__") && !html.contains("/*FONTS*/"), "{k} left placeholders");
            assert!(html.contains("\"tempo\""), "{k} must embed tokens");
        }
        assert!(build("nope", &serde_json::json!({}), &edit, None, 1920, 1080).is_err());
    }
}
