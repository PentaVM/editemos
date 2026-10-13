//! The anti-generic linter. It reads the tweens a scene actually declared (the
//! scene reports them through `window.__motion.report()`) and flags the defaults
//! every tool and every model reaches for.

use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Issue {
    pub level: String,
    pub code: String,
    pub message: String,
}

const BANNED_FONTS: &[&str] = &["inter", "roboto", "arial", "helvetica", "helvetica neue", "system-ui", "segoe ui", "sans-serif", "open sans", "lato", "montserrat", "poppins"];

fn get_f(v: &Value, k: &str) -> f64 {
    v.get(k).and_then(|x| x.as_f64()).unwrap_or(0.0)
}

pub fn lint(report: &Value, fps: f64) -> Vec<Issue> {
    let mut issues: Vec<Issue> = Vec::new();
    let allow: HashMap<String, String> =
        report.get("allow").and_then(|a| a.as_object()).map(|o| o.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string())).collect()).unwrap_or_default();
    let mut add = |level: &str, code: &str, msg: String| {
        if let Some(why) = allow.get(code) {
            issues.push(Issue { level: "info".into(), code: code.into(), message: format!("{msg} (allowed: {why})") });
        } else {
            issues.push(Issue { level: level.into(), code: code.into(), message: msg });
        }
    };

    let all: Vec<&Value> = report.get("tweens").and_then(|t| t.as_array()).map(|a| a.iter().filter(|t| t["kind"] != "set").collect()).unwrap_or_default();
    if all.is_empty() {
        add("warn", "NO_MOTION", "the scene declares no tweens".into());
    }
    // Hard cuts (zero duration or the `hold` ease) are deliberate, not motion: they
    // must not count towards "everything fades" or "everything lasts the same".
    let tweens: Vec<&Value> = all.into_iter().filter(|t| get_f(t, "dur") > 0.0 && t["ease"] != "hold").collect();
    let n = tweens.len();
    if n >= 4 {
        let generic = tweens.iter().filter(|t| t["generic"] == true).count();
        let ratio = generic as f64 / n as f64;
        if ratio >= 0.5 {
            add(
                if ratio >= 0.8 { "error" } else { "warn" },
                "GENERIC_EASE",
                format!("{generic}/{n} tweens use stock easing (linear, ease, sine, quad, cubic). Use signature curves."),
            );
        }
        let mut eases: Vec<String> = tweens.iter().filter_map(|t| t["ease"].as_str().map(String::from)).collect();
        eases.sort();
        eases.dedup();
        if n >= 6 && eases.len() < 3 {
            add("warn", "FEW_EASES", format!("only {} distinct easing(s) across {n} tweens", eases.len()));
        }
        let mut counts: HashMap<i64, usize> = HashMap::new();
        for t in &tweens {
            *counts.entry((get_f(t, "dur") * fps).round() as i64).or_insert(0) += 1;
        }
        if let Some((frames, cnt)) = counts.iter().max_by_key(|(_, c)| **c) {
            if n >= 5 && *cnt as f64 / n as f64 >= 0.6 {
                add("warn", "UNIFORM_DUR", format!("{cnt}/{n} tweens last the same {frames} frames. Vary durations (slowest about 3x the fastest)."));
            }
        }
        let zero = tweens.iter().filter(|t| get_f(t, "at") == 0.0).count();
        if n >= 5 && zero as f64 / n as f64 > 0.5 {
            add("warn", "START_ZERO", format!("{zero}/{n} tweens start at t=0; offset the first move 0.1 to 0.3 s so it does not read as a jump cut"));
        }
        let fade_slide = tweens
            .iter()
            .filter(|t| {
                let props: Vec<&str> = t["props"].as_array().map(|a| a.iter().filter_map(|p| p.as_str()).collect()).unwrap_or_default();
                props.contains(&"opacity")
                    && ["x", "y", "xPercent", "yPercent"].iter().any(|k| {
                        props.contains(k) && {
                            let d = t["deltas"].get(*k).and_then(|d| d.as_f64()).unwrap_or(0.0).abs();
                            (6.0..=90.0).contains(&d)
                        }
                    })
            })
            .count();
        if fade_slide as f64 / n as f64 >= 0.4 {
            add(
                "error",
                "FADE_SLIDE_DEFAULT",
                format!("{fade_slide}/{n} tweens are the fade + small slide entrance every tool ships. Use masks, scale, rotation, clip reveals or cuts on motion."),
            );
        }
        let fade_only = tweens.iter().filter(|t| t["props"].as_array().map(|p| p.len() == 1 && p[0] == "opacity").unwrap_or(false)).count();
        if fade_only as f64 / n as f64 >= 0.5 {
            add("warn", "FADE_ONLY", format!("{fade_only}/{n} tweens are plain opacity fades"));
        }
        let bouncy = tweens.iter().filter(|t| t["ease"].as_str().map(|e| e.to_lowercase()).map(|e| e.contains("bounce") || e.contains("elastic")).unwrap_or(false)).count();
        if bouncy > 0 {
            add("warn", "BOUNCY_EASE", format!("{bouncy} tweens use bounce/elastic eases"));
        }
    }
    let fonts: Vec<String> = report["audit"]["fonts"].as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default();
    let flagged: Vec<&String> = fonts.iter().filter(|f| BANNED_FONTS.contains(&f.to_lowercase().as_str())).collect();
    if !flagged.is_empty() {
        add("warn", "DEFAULT_FONT", format!("default/web-safe font in use: {}", flagged.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")));
    }
    if fonts.len() > 2 {
        add("warn", "TOO_MANY_FONTS", format!("{} font families: {}", fonts.len(), fonts.join(", ")));
    }
    issues
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tw(ease: &str, generic: bool, dur: f64, at: f64, props: &[&str], dy: f64) -> Value {
        json!({"kind": "to", "ease": ease, "generic": generic, "dur": dur, "at": at, "props": props, "deltas": {"y": dy}})
    }

    #[test]
    fn flags_the_default_look() {
        let tweens: Vec<Value> = (0..6).map(|_| tw("easeOut", true, 0.4, 0.0, &["opacity", "y"], 30.0)).collect();
        let r = json!({"tweens": tweens, "allow": {}, "audit": {"fonts": {"Inter": 3}}});
        let codes: Vec<String> = lint(&r, 30.0).into_iter().map(|i| i.code).collect();
        for c in ["GENERIC_EASE", "FEW_EASES", "UNIFORM_DUR", "START_ZERO", "FADE_SLIDE_DEFAULT", "DEFAULT_FONT"] {
            assert!(codes.contains(&c.to_string()), "missing {c}: {codes:?}");
        }
    }

    #[test]
    fn varied_motion_is_clean_and_allow_is_reported() {
        let mut tweens = vec![
            tw("bezier(0.1,1,0.3,1)", false, 0.18, 0.12, &["yPercent"], 100.0),
            tw("bezier(0.7,0,1,0.5)", false, 0.6, 0.5, &["scaleX"], 1.0),
            tw("spring", false, 0.9, 0.9, &["rotate"], 0.0),
            tw("hold", false, 0.0, 1.3, &["opacity"], 0.0),
            tw("bezier(0.6,0,0.2,1)", false, 1.4, 1.6, &["x"], 600.0),
        ];
        tweens.push(tw("bezier(0.3,0,0.2,1)", false, 0.33, 2.2, &["clipR"], 100.0));
        let r = json!({"tweens": tweens, "allow": {}, "audit": {"fonts": {"Bahnschrift": 2}}});
        assert!(lint(&r, 30.0).is_empty());
        let r2 = json!({"tweens": (0..6).map(|_| tw("linear", true, 0.4, 0.1, &["rotate"], 0.0)).collect::<Vec<_>>(), "allow": {"GENERIC_EASE": "continuous rotation must be linear"}, "audit": {"fonts": {}}});
        let out = lint(&r2, 30.0);
        let g = out.iter().find(|i| i.code == "GENERIC_EASE").unwrap();
        assert_eq!(g.level, "info");
        assert!(g.message.contains("continuous rotation"));
    }
}
