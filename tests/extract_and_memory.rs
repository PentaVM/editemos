//! End to end without a browser: extract a brand from a local site, then check
//! that the project holds its own edit brand, memory and context.

mod common;

use serde_json::json;

#[test]
fn site_to_brand_to_context() {
    let base = common::serve(common::site_routes());
    let root = common::temp_home("extract");
    let ctx = common::ctx_in(&root);

    let created = ctx.project_op(&json!({"action": "create", "name": "Estudio Brasa", "site": base, "activate": true})).unwrap();
    assert_eq!(created["created"], "estudio-brasa");

    let report = ctx.extract(&json!({"project": "estudio-brasa", "url": base, "backend": "native"})).unwrap();
    let palette: Vec<String> = report["palette"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_lowercase()).collect();
    assert!(palette.contains(&"primary #ff3d1f".to_string()), "brand colour missing from {palette:?}");
    // colours defined through CSS variables are resolved, not replaced by defaults
    assert!(palette.contains(&"background #f6efe4".to_string()), "var() background not resolved: {palette:?}");
    assert!(palette.contains(&"text #14110f".to_string()), "var() text colour not resolved: {palette:?}");
    let fonts: Vec<String> = report["fonts"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
    assert!(fonts.iter().any(|f| f.contains("Fraunces")) && fonts.iter().any(|f| f.contains("Space Grotesk")), "fonts: {fonts:?}");
    assert!(report["logos"].as_u64().unwrap() >= 1, "no logo found: {report}");
    assert!(!report["assets_added"].as_array().unwrap().is_empty());

    // the brand is on disk and carries an edit brand of its own
    let brand = ctx.brand_op(&json!({"project": "estudio-brasa"})).unwrap();
    assert_eq!(brand["name"].as_str().map(|s| s.contains("Brasa")), Some(true), "name: {}", brand["name"]);
    assert!(brand["social"].as_array().unwrap().iter().any(|s| s["network"] == "instagram"));
    assert!(["calm", "steady", "punchy"].contains(&brand["edit"]["tempo"].as_str().unwrap()));
    assert_eq!(brand["edit"]["easings"]["enter"].as_array().unwrap().len(), 4);

    // memory: add, find, supersede
    let first = ctx
        .memory_add(&json!({"project": "estudio-brasa", "text": "Los titulos entran con mascara, nunca con fundido", "kind": "decision", "confidence": "high", "pinned": true}))
        .unwrap();
    let id = first["id"].as_str().unwrap().to_string();
    let hits = ctx.memory_search(&json!({"project": "estudio-brasa", "query": "titulos mascara"})).unwrap();
    assert_eq!(hits["memories"][0]["id"], id.as_str());
    ctx.memory_add(&json!({"project": "estudio-brasa", "text": "Los titulos entran con corte seco sobre el beat", "kind": "decision", "supersedes": id})).unwrap();
    let all = ctx.memory_search(&json!({"project": "estudio-brasa", "k": 50})).unwrap();
    let texts: Vec<String> = all["memories"].as_array().unwrap().iter().map(|m| m["text"].as_str().unwrap().to_string()).collect();
    assert!(texts.iter().any(|t| t.contains("corte seco")));
    assert!(!texts.iter().any(|t| t.contains("mascara, nunca")), "superseded memory is still active: {texts:?}");

    // context pack: brand, edit brand, memory and the rules, within budget
    let pack = ctx.context_pack(&json!({"project": "estudio-brasa", "task": "titulos de apertura"})).unwrap();
    let md = pack["context"].as_str().unwrap();
    for needle in ["Estudio Brasa", "#ff3d1f", "corte seco", "How to work in this project"] {
        assert!(md.contains(needle), "context lacks {needle:?}:\n{md}");
    }
    assert!(root.join("projects/estudio-brasa/context/CONTEXT.md").exists(), "context file is regenerated on every change");

    // a second project never inherits the first one's style
    ctx.project_op(&json!({"action": "create", "name": "Taller Norte", "site": "https://example.org"})).unwrap();
    let other = ctx.brand_op(&json!({"project": "taller-norte"})).unwrap();
    assert_ne!(other["edit"]["easings"], brand["edit"]["easings"], "two projects must not share easings");
    let other_pack = ctx.context_pack(&json!({"project": "taller-norte"})).unwrap();
    assert!(!other_pack["context"].as_str().unwrap().contains("corte seco"), "memory leaked across projects");

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn robots_txt_is_obeyed() {
    let mut routes = common::site_routes();
    routes.push(("/robots.txt", "text/plain", b"User-agent: *\nDisallow: /\n".to_vec()));
    let base = common::serve(routes);
    let root = common::temp_home("robots");
    let ctx = common::ctx_in(&root);
    ctx.project_op(&json!({"action": "create", "name": "Blocked", "activate": true})).unwrap();

    let err = ctx.extract(&json!({"url": base, "backend": "native"})).unwrap_err();
    assert!(err.to_string().to_lowercase().contains("robots"), "unexpected error: {err}");
    // explicit override is honoured
    let ok = ctx.extract(&json!({"url": base, "backend": "native", "ignore_robots": true}));
    assert!(ok.is_ok(), "{:?}", ok.err());

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn generate_builds_an_on_brand_prompt_without_calling_anything() {
    let root = common::temp_home("generate");
    let ctx = common::ctx_in(&root);
    ctx.project_op(&json!({"action": "create", "name": "Prompted", "activate": true})).unwrap();
    ctx.memory_add(&json!({"text": "no stock photos of people laughing at salads", "kind": "rejection"})).unwrap();
    let r = ctx.generate(&json!({"prompt": "thumbnail background for episode 4", "dry_run": true, "adapter": "echo"})).unwrap();
    let prompt = r["prompt"].as_str().unwrap();
    assert!(prompt.contains("Prompted") && prompt.contains("episode 4") && prompt.contains("no stock photos"), "{prompt}");
    let _ = std::fs::remove_dir_all(&root);
}
