//! Operations shared by the CLI and the MCP server. Every function takes plain
//! values and returns JSON, so both front ends stay thin and always agree.

use crate::error::{fail, Result};
use crate::model::Memory;
use crate::store::{Home, Project};
use crate::util::{path_str, slugify, truncate_chars, unix_secs, write_atomic};
use crate::{adapters, assets, brand, clips, context, extract, fonts, kdenlive, memory, motion, session, skills, story, studio, transcribe};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub struct Ctx {
    pub home: Home,
}

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(|x| x.as_str()).map(String::from).filter(|x| !x.is_empty())
}
fn n(v: &Value, k: &str) -> Option<f64> {
    v.get(k).and_then(|x| x.as_f64())
}
fn b(v: &Value, k: &str) -> bool {
    v.get(k).and_then(|x| x.as_bool()).unwrap_or(false)
}
fn list(v: &Value, k: &str) -> Vec<String> {
    match v.get(k) {
        Some(Value::Array(a)) => a.iter().filter_map(|x| x.as_str().map(String::from)).collect(),
        Some(Value::String(t)) => t.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect(),
        _ => Vec::new(),
    }
}

impl Ctx {
    pub fn open() -> Result<Ctx> {
        Ok(Ctx { home: Home::open()? })
    }

    pub fn project(&self, a: &Value) -> Result<Project> {
        match s(a, "project") {
            Some(name) => self.home.open_project(&name),
            None => self.home.active(),
        }
    }

    fn refresh(&self, p: &Project) {
        let _ = context::refresh(&self.home, p);
    }

    /* ---------------- projects ---------------- */

    pub fn project_op(&self, a: &Value) -> Result<Value> {
        match s(a, "action").as_deref().unwrap_or("info") {
            "list" => {
                let active = self.home.active_slug();
                let ps: Vec<Value> = self
                    .home
                    .list()
                    .into_iter()
                    .map(|p| json!({"slug": p.config.slug, "name": p.config.name, "dir": path_str(&p.dir), "site": p.config.site, "active": Some(&p.config.slug) == active.as_ref()}))
                    .collect();
                Ok(json!({"projects": ps, "active": active}))
            }
            "create" => {
                let Some(name) = s(a, "name") else { return fail("create needs a name") };
                let dir = s(a, "dir").map(PathBuf::from);
                let mut p = self.home.create(&name, dir.as_deref())?;
                if let Some(site) = s(a, "site") {
                    p.config.site = Some(site);
                }
                if let Some(ad) = s(a, "adapter") {
                    p.config.adapter = Some(ad);
                }
                if let Some(sz) = s(a, "size") {
                    p.config.size = Some(sz);
                }
                if let Some(f) = n(a, "fps") {
                    p.config.fps = Some(f);
                }
                p.save_config()?;
                // Even before any extraction the project gets its own edit brand,
                // seeded by its name, so no two projects start from the same motion.
                let mut br = p.load_brand();
                br.edit = brand::derive_edit_brand(&br);
                p.save_brand(&br)?;
                if b(a, "activate") {
                    self.home.set_active(&p.config.slug)?;
                }
                self.refresh(&p);
                Ok(json!({"created": p.config.slug, "dir": path_str(&p.dir)}))
            }
            "use" => {
                let Some(name) = s(a, "name") else { return fail("use needs a name") };
                let p = self.home.open_project(&name)?;
                self.home.set_active(&p.config.slug)?;
                self.refresh(&p);
                Ok(json!({"active": p.config.slug, "name": p.config.name, "dir": path_str(&p.dir)}))
            }
            _ => {
                let p = self.project(a)?;
                let brand = p.load_brand();
                let mems = memory::active(&p);
                let assets = assets::load(&p);
                Ok(json!({
                    "slug": p.config.slug, "name": p.config.name, "dir": path_str(&p.dir), "site": p.config.site,
                    "size": p.config.size, "fps": p.config.fps, "skills": p.config.skills,
                    "tempo": brand.edit.tempo, "memories": mems.len(), "assets": assets.len(),
                    "context": path_str(&p.context_dir().join("CONTEXT.md")),
                }))
            }
        }
    }

    /* ---------------- brand ---------------- */

    pub fn brand_op(&self, a: &Value) -> Result<Value> {
        let p = self.project(a)?;
        let mut br = p.load_brand();
        match s(a, "action").as_deref().unwrap_or("show") {
            "derive" => {
                br.edit = brand::derive_edit_brand(&br);
                p.save_brand(&br)?;
                self.refresh(&p);
            }
            "lock" | "unlock" => {
                br.edit.locked = s(a, "action").as_deref() == Some("lock");
                p.save_brand(&br)?;
                self.refresh(&p);
            }
            // the brand stated directly: by a person, or by the agent after looking
            "set" => {
                let mut patch = a.clone();
                // `text` is the type colour when it is a colour, a description otherwise
                let text_is_colour = s(a, "text").map(|t| crate::color::parse_color(&t).is_some()).unwrap_or(false);
                let words = s(a, "description").or_else(|| if text_is_colour { None } else { s(a, "text") });
                if let Some(t) = words {
                    // what the sentence says first, explicit fields on top of it
                    let mut from_words = crate::brandai::from_text(&t);
                    if let (Some(dst), Some(src)) = (from_words.as_object_mut(), a.as_object()) {
                        for (k, v) in src {
                            if k != "description" && (k != "text" || text_is_colour) {
                                dst.insert(k.clone(), v.clone());
                            }
                        }
                    }
                    patch = from_words;
                }
                let changed = crate::brandai::apply(&self.home, &p, &patch, s(a, "source").as_deref().unwrap_or("manual"))?;
                if changed.is_empty() {
                    return fail("nothing to set. Give colours (primary, secondary, background, text), heading_font, body_font, name, tagline, voice, or text=\"a description with #hex colours\"");
                }
                let br = p.load_brand();
                return Ok(json!({"changed": changed, "palette": br.palette, "fonts": br.fonts, "tempo": br.edit.tempo}));
            }
            "from-images" | "from_images" => {
                let files = list(a, "files");
                return crate::brandai::from_images(&self.home, &p, &files);
            }
            "evidence" => return Ok(crate::brandai::evidence(&p)),
            "refine" => {
                let adapter = s(a, "adapter").or_else(|| p.config.adapter.clone()).unwrap_or_else(|| "claude".into());
                return crate::brandai::refine(&self.home, &p, &adapter, !b(a, "no_vision"), &s(a, "notes").unwrap_or_default());
            }
            "sheet" => {
                // the brand identity on one page, drawn by the project's own renderer
                let logo = br
                    .logos
                    .first()
                    .and_then(|id| assets::load(&p).into_iter().find(|x| &x.id == id))
                    .map(|x| format!("file:///{}", path_str(&p.assets_dir().join(&x.file)).replace('\\', "/").trim_start_matches('/')));
                let fams = [br.edit.type_tokens.heading.as_str(), br.edit.type_tokens.body.as_str()];
                let html = include_str!("../templates/brand-sheet.html")
                    .replace("/*FONTS*/", &motion::fonts::face_css(Some(&p), &fams))
                    .replace("/*BRAND*/{}", &serde_json::to_string(&br)?)
                    .replace("/*LOGO*/null", &serde_json::to_string(&logo)?);
                let scene = p.context_dir().join("brand-sheet.html");
                write_atomic(&scene, html.as_bytes())?;
                let out = s(a, "out").map(PathBuf::from).unwrap_or_else(|| p.dir.join("brand-sheet.png"));
                let r = motion::render::render(&motion::render::RenderOptions {
                    scene,
                    out: out.clone(),
                    width: 1600,
                    height: 1000,
                    poster: Some(0.0),
                    lint: false,
                    contact: None,
                    ..Default::default()
                })?;
                let _ = assets::register_file(&p, &out, "brand", vec!["brand-sheet".into()]);
                self.refresh(&p);
                return Ok(serde_json::to_value(r)?);
            }
            _ => {}
        }
        Ok(serde_json::to_value(&br)?)
    }

    /* ---------------- memory ---------------- */

    pub fn memory_add(&self, a: &Value) -> Result<Value> {
        // global=true: a preference of the person, for every project they have
        let p = if b(a, "global") { self.user_memory() } else { self.project(a)? };
        let Some(text) = s(a, "text") else { return fail("memory needs text") };
        let m = memory::add(
            &p,
            &text,
            s(a, "kind").as_deref().unwrap_or("note"),
            list(a, "tags"),
            s(a, "confidence").as_deref().unwrap_or("medium"),
            s(a, "source").as_deref().unwrap_or("user"),
            b(a, "pinned"),
            s(a, "supersedes"),
        )?;
        if b(a, "global") {
            if let Ok(active) = self.home.active() {
                self.refresh(&active);
            }
        } else {
            self.refresh(&p);
        }
        Ok(serde_json::to_value(&m)?)
    }

    pub fn memory_search(&self, a: &Value) -> Result<Value> {
        let p = self.project(a)?;
        let k = n(a, "k").map(|v| v as usize).unwrap_or(8);
        let mems = memory::active(&p);
        let rows: Vec<Memory> = match s(a, "query") {
            Some(q) => memory::search(&mems, &q, k).into_iter().map(|(_, m)| m).collect(),
            None => mems.into_iter().rev().take(k).collect(),
        };
        Ok(json!({"count": rows.len(), "memories": rows}))
    }

    pub fn memory_admin(&self, a: &Value) -> Result<Value> {
        let p = self.project(a)?;
        match s(a, "action").as_deref().unwrap_or("compact") {
            "forget" => {
                let Some(id) = s(a, "id") else { return fail("forget needs an id") };
                memory::forget(&p, &id)?;
                self.refresh(&p);
                Ok(json!({"forgotten": id}))
            }
            _ => {
                let r = memory::compact(&p)?;
                self.refresh(&p);
                Ok(json!({"before": r.before, "after": r.after, "backup": r.backup}))
            }
        }
    }

    /* ---------------- assets ---------------- */

    pub fn assets_op(&self, a: &Value) -> Result<Value> {
        let p = self.project(a)?;
        let all = assets::load(&p);
        let k = n(a, "k").map(|v| v as usize).unwrap_or(20);
        match s(a, "action").as_deref().unwrap_or("list") {
            "add" => {
                let Some(file) = s(a, "file") else { return fail("add needs a file path") };
                let bytes = std::fs::read(&file)?;
                let rec = assets::add_bytes(&p, &bytes, s(a, "kind").as_deref().unwrap_or(""), &file, "imported", None, None, None, list(a, "tags"))?;
                self.refresh(&p);
                Ok(serde_json::to_value(rec)?)
            }
            "search" => {
                let hits = assets::search(&all, &s(a, "query").unwrap_or_default(), k);
                Ok(json!({"dir": path_str(&p.assets_dir()), "assets": hits}))
            }
            _ => Ok(json!({"dir": path_str(&p.assets_dir()), "count": all.len(), "assets": all.into_iter().take(k).collect::<Vec<_>>()})),
        }
    }

    /* ---------------- context ---------------- */

    pub fn context_pack(&self, a: &Value) -> Result<Value> {
        let p = self.project(a)?;
        let opts = context::PackOptions { task: s(a, "task"), budget_tokens: n(a, "budget").map(|v| v as usize).unwrap_or(6000) };
        let text = context::build(&self.home, &p, &opts);
        let mut written = Vec::new();
        if let Some(dir) = s(a, "write_dir") {
            written = context::write_agent_files(&self.home, &p, Path::new(&dir), &opts)?;
        }
        Ok(json!({"project": p.config.slug, "tokens": crate::util::approx_tokens(&text), "written": written, "context": text}))
    }

    /* ---------------- extraction & generation ---------------- */

    pub fn extract(&self, a: &Value) -> Result<Value> {
        let p = self.project(a)?;
        let url = s(a, "url").or_else(|| p.config.site.clone());
        let Some(url) = url else { return fail("extract needs a url (or set the project's site)") };
        let opts = extract::Options {
            backend: extract::Backend::parse(s(a, "backend").as_deref().unwrap_or("auto")),
            max_images: n(a, "max_images").map(|v| v as usize).unwrap_or(12),
            fonts: b(a, "fonts"),
            ignore_robots: b(a, "ignore_robots"),
            scrapling_mode: s(a, "scrapling_mode").unwrap_or_else(|| "get".into()),
        };
        let r = extract::run(&self.home, &p, &url, &opts)?;
        Ok(serde_json::to_value(r)?)
    }

    pub fn generate(&self, a: &Value) -> Result<Value> {
        let p = self.project(a)?;
        let Some(request) = s(a, "prompt") else { return fail("generate needs a prompt") };
        let adapter_name = s(a, "adapter").or_else(|| p.config.adapter.clone()).unwrap_or_else(|| "echo".into());
        let ad = adapters::get(&self.home, &adapter_name)?;
        let brand = p.load_brand();
        let avoid: Vec<String> = memory::active(&p).into_iter().filter(|m| m.kind == "rejection").map(|m| m.text).collect();
        let kind = s(a, "kind").unwrap_or_else(|| "a brand image".into());
        let prompt = adapters::brand_prompt(&brand, &avoid, &kind, &request);
        if b(a, "dry_run") {
            return Ok(json!({"adapter": adapter_name, "prompt": prompt}));
        }
        if matches!(ad.kind.as_str(), "openai-image" | "gemini-image") {
            let imgs = adapters::generate_images(&ad, &prompt, &s(a, "size").unwrap_or_else(|| "1024x1024".into()), n(a, "n").map(|v| v as u32).unwrap_or(1))?;
            let mut out = Vec::new();
            for bytes in imgs {
                let rec = assets::add_bytes(&p, &bytes, "image", "generated.png", "generated", None, Some(&request), Some(&adapter_name), vec!["generated".into()])?;
                out.push(rec);
            }
            memory::add(
                &p,
                &format!("Generated {} image(s) with {adapter_name}: {}", out.len(), truncate_chars(&request, 140)),
                "run",
                vec!["generate".into()],
                "medium",
                "agent",
                false,
                None,
            )?;
            self.refresh(&p);
            Ok(json!({"adapter": adapter_name, "prompt": prompt, "assets": out}))
        } else {
            let text = adapters::run_text(&ad, &prompt)?;
            Ok(json!({"adapter": adapter_name, "prompt": prompt, "text": text}))
        }
    }

    /* ---------------- motion ---------------- */

    fn size_of(&self, p: Option<&Project>, a: &Value) -> (u32, u32) {
        let raw = s(a, "size").or_else(|| p.and_then(|p| p.config.size.clone())).unwrap_or_else(|| "1920x1080".into());
        let mut it = raw.to_lowercase();
        it = it.replace('×', "x");
        let mut parts = it.split('x').filter_map(|x| x.trim().parse::<u32>().ok());
        (parts.next().unwrap_or(1920), parts.next().unwrap_or(1080))
    }

    pub fn motion_op(&self, a: &Value) -> Result<Value> {
        let project = self.project(a).ok();
        let action = s(a, "action").unwrap_or_else(|| "render".into());
        let (w, h) = self.size_of(project.as_ref(), a);
        let fps = n(a, "fps").or_else(|| project.as_ref().and_then(|p| p.config.fps)).unwrap_or(30.0);
        match action.as_str() {
            "templates" => Ok(json!({"templates": motion::templates::KINDS})),
            "new" => {
                let Some(kind) = s(a, "kind") else { return fail("new needs a template kind") };
                let Some(p) = project.as_ref() else { return fail("no project: templates read the project's edit brand") };
                let mut params = a.get("params").cloned().unwrap_or(json!({}));
                if let (Some(t), Some(obj)) = (s(a, "text"), params.as_object_mut()) {
                    obj.entry("text").or_insert(json!(t));
                }
                if let Some(obj) = params.as_object_mut() {
                    obj.entry("fps").or_insert(json!(fps));
                }
                let br = p.load_brand();
                let size = format!("{w}x{h}");
                // the same graphic asked for twice is the same file, not a second copy
                let key = motion::library::scene_key(&kind, &params, &size, br.edit.seed, &serde_json::to_string(&br.edit)?);
                if !b(a, "force") && s(a, "out").is_none() {
                    if let Some(old) = motion::library::find_scene(p, &key) {
                        motion::library::mark_reused(p, &key);
                        let render = old.renders.last().filter(|r| Path::new(&r.file).exists());
                        return Ok(json!({"scene": old.scene, "template": kind, "size": size, "reused": true, "render": render,
                            "note": "this graphic already exists in the project's motion library; pass force=true to build a new one"}));
                    }
                }
                let html = motion::templates::build(&kind, &params, &br.edit, Some(p), w, h)?;
                let out = match s(a, "out") {
                    Some(o) => PathBuf::from(o),
                    None => p.motion_dir().join("scenes").join(format!("{kind}-{key}.html")),
                };
                write_atomic(&out, html.as_bytes())?;
                let text = params.get("text").and_then(|t| t.as_str()).unwrap_or("").to_string();
                motion::library::record_scene(p, motion::library::Piece { key, kind: kind.clone(), text, scene: path_str(&out), size: size.clone(), ..Default::default() })?;
                self.refresh(p);
                Ok(json!({"scene": path_str(&out), "template": kind, "size": size, "reused": false}))
            }
            "library" => {
                let Some(p) = project.as_ref() else { return fail("no project") };
                if b(a, "prune") {
                    motion::library::prune(p)?;
                }
                let q = s(a, "query").map(|q| q.to_lowercase());
                let pieces: Vec<_> = motion::library::load(p)
                    .into_iter()
                    .filter(|x| q.as_ref().map(|q| x.kind.contains(q.as_str()) || x.text.to_lowercase().contains(q.as_str())).unwrap_or(true))
                    .collect();
                Ok(json!({"count": pieces.len(), "pieces": pieces}))
            }
            "lint" => {
                let Some(scene) = s(a, "scene") else { return fail("lint needs a scene path") };
                let tokens = project.as_ref().map(|p| serde_json::to_value(p.load_brand().edit)).transpose()?;
                let issues = motion::render::lint_scene(Path::new(&scene), fps, w, h, tokens.as_ref())?;
                let errors = issues.iter().filter(|i| i.level == "error").count();
                Ok(json!({"scene": scene, "ok": errors == 0, "errors": errors, "issues": issues}))
            }
            "poster" | "render" => {
                let Some(scene) = s(a, "scene") else { return fail("render needs a scene path") };
                let tokens = project.as_ref().map(|p| serde_json::to_value(p.load_brand().edit)).transpose()?;
                let scene_path = PathBuf::from(&scene);
                let default_out = project.as_ref().map(|p| p.motion_dir().join("renders")).unwrap_or_else(std::env::temp_dir);
                let stem = scene_path.file_stem().map(|x| x.to_string_lossy().to_string()).unwrap_or_else(|| "scene".into());
                let format = s(a, "format");
                let ext = match (action.as_str(), format.as_deref()) {
                    ("poster", _) => "png",
                    (_, Some("vp9")) => "webm",
                    (_, Some("pngseq")) => "",
                    _ => "mov",
                };
                let out = s(a, "out").map(PathBuf::from).unwrap_or_else(|| {
                    let name = if ext.is_empty() { stem.clone() } else { format!("{stem}.{ext}") };
                    default_out.join(name)
                });
                let opts = motion::render::RenderOptions {
                    scene: scene_path,
                    out,
                    fps,
                    width: w,
                    height: h,
                    duration: n(a, "duration"),
                    format,
                    subframes: n(a, "subframes").map(|v| v as u32).unwrap_or(1),
                    shutter_deg: n(a, "shutter").unwrap_or(180.0),
                    contact: if b(a, "no_contact") { None } else { Some((4, 3)) },
                    over: s(a, "over").map(PathBuf::from),
                    lint: !b(a, "no_lint"),
                    strict: b(a, "strict"),
                    keep_frames: b(a, "keep_frames"),
                    tokens,
                    poster: if action == "poster" { Some(n(a, "at").unwrap_or(1.0)) } else { None },
                    jobs: n(a, "jobs").map(|v| v as usize),
                };
                // same scene, same options, file still there: hand back the finished render
                let rkey =
                    motion::library::render_key(&opts.scene, &format!("{}|{}|{w}x{h}|{:?}|{:?}|{}|{}", action, fps, opts.duration, opts.format, opts.subframes, opts.shutter_deg));
                if action == "render" && !b(a, "force") && s(a, "out").is_none() {
                    if let Some(old) = project.as_ref().and_then(|p| motion::library::find_render(p, &opts.scene, &rkey)) {
                        return Ok(
                            json!({"ok": true, "reused": true, "scene": scene, "out": old.file, "format": old.format, "fps": old.fps, "duration": old.duration, "contact": old.contact,
                            "size": format!("{w}x{h}"), "frames": (old.duration * old.fps).round(), "lint": [], "note": "already rendered with these options; pass force=true to render again"}),
                        );
                    }
                }
                let r = motion::render::render(&opts)?;
                if let (Some(p), "render") = (project.as_ref(), action.as_str()) {
                    motion::library::record_render(
                        p,
                        &opts.scene,
                        motion::library::Render {
                            key: rkey,
                            file: r.out.clone(),
                            format: r.format.clone(),
                            fps: r.fps,
                            duration: r.duration,
                            contact: r.contact.clone(),
                            ..Default::default()
                        },
                    )?;
                    // a finished graphic is a project asset like any other
                    let _ = assets::register_file(p, Path::new(&r.out), "motion", vec!["motion".into(), stem.clone()]);
                    self.refresh(p);
                }
                let mut v = serde_json::to_value(r)?;
                v["reused"] = json!(false);
                if let Some(review) = v.get("review").cloned() {
                    // the stills at each settle point: look at them before calling the graphic done
                    v["images"] = json!([review]);
                    v["before_you_call_it_done"] = json!("Look at the review frames (one per moment the scene comes to rest): is every word readable against what is behind it, did each element land before the next appeared, does anything collide or touch the edge? Fix the scene and render again; two rounds at most.");
                }
                Ok(v)
            }
            other => fail(format!("unknown motion action '{other}' (templates, new, library, render, lint, poster)")),
        }
    }

    /* ---------------- reference edits ---------------- */

    /// Reverse-engineer a reference video (`analyse`), check one of our cuts
    /// against it (`compare`), list what the project has, or adopt its tempo (`apply`).
    pub fn reference_op(&self, a: &Value) -> Result<Value> {
        let p = self.project(a)?;
        let dir = p.dir.join("references");
        let action = s(a, "action").unwrap_or_else(|| "analyse".into());
        let load = |name: &str| -> Result<crate::reference::Reference> {
            let path = if name.ends_with(".json") && Path::new(name).exists() { PathBuf::from(name) } else { dir.join(format!("{}.json", slugify(name))) };
            let text = std::fs::read_to_string(&path).map_err(|_| crate::error::Error(format!("no reference '{name}' in this project (action=list)")))?;
            Ok(serde_json::from_str(&text)?)
        };
        match action.as_str() {
            "list" => {
                let mut names: Vec<Value> = Vec::new();
                if let Ok(rd) = std::fs::read_dir(&dir) {
                    for e in rd.flatten().filter(|e| e.path().extension().map(|x| x == "json").unwrap_or(false)) {
                        if let Ok(r) = serde_json::from_str::<crate::reference::Reference>(&std::fs::read_to_string(e.path()).unwrap_or_default()) {
                            names.push(json!({"name": r.name, "source": r.source, "duration": r.duration, "median_shot": r.rhythm.median_shot, "grade": r.grade.words}));
                        }
                    }
                }
                Ok(json!({"references": names}))
            }
            "analyse" | "analyze" | "compare" => {
                let Some(ffmpeg) = motion::render::find_ffmpeg() else { return fail("ffmpeg not found") };
                let (file, _) = self.source_and_cues(&p, a)?;
                let stem = file.file_stem().map(|x| x.to_string_lossy().to_string()).unwrap_or_else(|| "reference".into());
                if action == "compare" {
                    let Some(name) = s(a, "reference") else { return fail("compare needs reference=<name> (action=list) and file=<our cut>") };
                    let r = load(&name)?;
                    let c = crate::reference::analyse(&ffmpeg, &file, &stem, None)?;
                    let cmp = crate::reference::compare(&r, &c);
                    return Ok(json!({"comparison": cmp, "candidate": {"rhythm": c.rhythm, "grade": c.grade, "loudness": c.loudness},
                        "note": "Each line is a trait measured the same way on both. Fix what differs that matters for this edit, build again and compare again; matching numbers do not make a good cut on their own."}));
                }
                let name = s(a, "name").unwrap_or_else(|| stem.clone());
                let slug = slugify(&name);
                let sheet = dir.join(format!("{slug}.png"));
                let r = crate::reference::analyse(&ffmpeg, &file, &name, Some(&sheet))?;
                write_atomic(&dir.join(format!("{slug}.json")), serde_json::to_string_pretty(&r)?.as_bytes())?;
                memory::add(
                    &p,
                    &format!(
                        "Reference edit '{}': median shot {:.2} s ({} cuts a minute), {}{}",
                        name,
                        r.rhythm.median_shot,
                        r.rhythm.cuts_per_minute,
                        r.grade.words.join(", "),
                        r.loudness.integrated_lufs.map(|l| format!(", {l:.1} LUFS")).unwrap_or_default()
                    ),
                    "note",
                    vec!["reference".into()],
                    "high",
                    "measured",
                    false,
                    None,
                )?;
                self.refresh(&p);
                let mut v = serde_json::to_value(&r)?;
                // the per-shot list is in the file; the answer stays readable
                v["shots"] = json!(r.shots.len());
                v["file"] = json!(path_str(&dir.join(format!("{slug}.json"))));
                v["contact"] = json!(r.sheet);
                v["next"] = json!("Read the sheet: name the type, the graphics and how the camera moves, which the numbers cannot. action=apply adopts its tempo for this project; after building a cut, action=compare reference=<name> file=<cut> checks it against this.");
                Ok(v)
            }
            "apply" => {
                let Some(name) = s(a, "reference") else { return fail("apply needs reference=<name>") };
                let r = load(&name)?;
                let mut br = p.load_brand();
                if br.edit.locked {
                    return fail("the edit brand is locked (brand action=unlock to change it)");
                }
                let tempo = crate::reference::tempo_of(&r);
                br.edit.tempo = tempo.to_string();
                let rule = format!(
                    "Cut like '{}': median shot about {:.1} s, the fast end {:.1} s, the slow end {:.1} s; grade {}",
                    r.name,
                    r.rhythm.median_shot,
                    r.rhythm.p10_shot,
                    r.rhythm.p90_shot,
                    r.grade.words.join(", ")
                );
                br.edit.rules.retain(|x| !x.starts_with("Cut like '"));
                br.edit.rules.push(rule.clone());
                br.edit.rationale.push(format!("tempo {tempo} measured from the reference '{}'", r.name));
                p.save_brand(&br)?;
                memory::add(&p, &rule, "decision", vec!["reference".into(), "tempo".into()], "high", "user", true, None)?;
                self.refresh(&p);
                Ok(json!({"project": p.config.slug, "tempo": tempo, "rule": rule}))
            }
            other => fail(format!("unknown reference action '{other}' (analyse, compare, list, apply)")),
        }
    }

    /* ---------------- clips ---------------- */

    /// The recording to work on (a file, or a link fetched into the project) and the
    /// best transcript there is for *finding* things in it.
    fn source_and_cues(&self, p: &Project, a: &Value) -> Result<(PathBuf, Vec<clips::Cue>)> {
        let Some(input) = s(a, "file").or_else(|| s(a, "input")) else { return fail("give the recording: file=<path or link>") };
        let mut srt: Option<PathBuf> = s(a, "transcript").map(PathBuf::from);
        let file = if clips::is_url(&input) {
            let (video, found) = clips::fetch(&input, &p.dir.join("sources"), n(a, "source_height").map(|v| v as u32).unwrap_or(1080), s(a, "section").as_deref())?;
            srt = srt.or(found);
            video
        } else {
            PathBuf::from(&input)
        };
        let file = std::fs::canonicalize(&file).map_err(|_| crate::error::Error(format!("{} not found", file.display())))?;
        let stem = file.file_stem().map(|x| x.to_string_lossy().to_string()).unwrap_or_default();
        let store = p.dir.join("transcripts").join(format!("{stem}.json"));
        let cues: Vec<clips::Cue> = match &srt {
            Some(f) => clips::dedupe_rolling(clips::parse_transcript(&std::fs::read_to_string(f)?)),
            None => std::fs::read_to_string(&store).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default(),
        };
        if !cues.is_empty() && srt.is_some() {
            write_atomic(&store, serde_json::to_string(&cues)?.as_bytes())?;
        }
        Ok((file, cues))
    }

    /// Type and colours for on-screen text: the brand's colours and a typeface that
    /// was chosen, installed and remembered (never a system fallback by accident).
    fn text_style(&self, p: &Project) -> (story::Style, String) {
        let br = p.load_brand();
        let prefs = fonts::load_prefs(&self.home);
        let rejected_here: Vec<String> = br.fonts.iter().filter(|f| f.role == "rejected").map(|f| f.family.clone()).collect();
        let choice = fonts::pick(&self.home, &br, &rejected_here);
        let col = |hex: &str, d: crate::color::Rgb| crate::color::parse_color(hex).unwrap_or(d);
        let mut st = story::Style {
            fg: col(&br.edit.colors.fg, crate::color::Rgb(255, 255, 255)),
            bg: col(&br.edit.colors.bg, crate::color::Rgb(0, 0, 0)),
            accent: col(&br.edit.colors.accent, crate::color::Rgb(255, 230, 0)),
            caps: prefs.caption_caps.unwrap_or(true),
            words_per_line: prefs.caption_words.unwrap_or(3).clamp(1, 6),
            ..Default::default()
        };
        // captions must read on any footage: near-white type, near-black outline
        if st.fg.luminance() < 0.6 {
            st.fg = crate::color::Rgb(255, 255, 255);
        }
        if st.bg.luminance() > 0.12 {
            st.bg = crate::color::Rgb(8, 8, 10);
        }
        match fonts::install(&self.home, choice.face.family) {
            Ok(_) => {
                st.font = choice.face.ass_name.to_string();
                st.fonts_dir = Some(fonts::dir(&self.home));
                (st, format!("{} - {}", choice.face.family, choice.why))
            }
            Err(e) => (st, format!("{} could not be downloaded ({e}); using a system font", choice.face.family)),
        }
    }

    /// Typefaces: what exists, what fits, what was chosen, what is never to be used again.
    pub fn fonts_op(&self, a: &Value) -> Result<Value> {
        let action = s(a, "action").unwrap_or_else(|| "show".into());
        let global = b(a, "global");
        match action.as_str() {
            "list" => Ok(json!({"fonts": fonts::CATALOG.iter().map(|f| json!({"family": f.family, "tags": f.tags, "note": f.note})).collect::<Vec<_>>()})),
            "set" | "reject" => {
                let Some(family) = s(a, "family") else { return fail("give the family, e.g. family=\"Anton\" (see action=list)") };
                let Some(face) = fonts::find(&family) else { return fail(format!("'{family}' is not in the catalogue; action=list shows what is")) };
                let why = s(a, "reason").unwrap_or_default();
                let mut prefs = fonts::load_prefs(&self.home);
                if action == "set" {
                    fonts::install(&self.home, face.family)?;
                }
                if global {
                    if action == "set" {
                        prefs.caption_font = Some(face.family.to_string());
                        prefs.rejected_fonts.retain(|r| !r.eq_ignore_ascii_case(face.family));
                    } else {
                        if !prefs.rejected_fonts.iter().any(|r| r.eq_ignore_ascii_case(face.family)) {
                            prefs.rejected_fonts.push(face.family.to_string());
                        }
                        if prefs.caption_font.as_deref().map(|c| c.eq_ignore_ascii_case(face.family)).unwrap_or(false) {
                            prefs.caption_font = None;
                        }
                    }
                    fonts::save_prefs(&self.home, &prefs)?;
                    memory::add(
                        &self.user_memory(),
                        &format!(
                            "Typeface {} {} for all projects{}",
                            face.family,
                            if action == "set" { "preferred" } else { "rejected" },
                            if why.is_empty() { String::new() } else { format!(": {why}") }
                        ),
                        if action == "set" { "preference" } else { "rejection" },
                        vec!["font".into()],
                        "high",
                        "user",
                        true,
                        None,
                    )?;
                    return Ok(json!({"scope": "all projects", "action": action, "family": face.family, "prefs": prefs}));
                }
                let p = self.project(a)?;
                let mut br = p.load_brand();
                br.fonts.retain(|f| !(f.family.eq_ignore_ascii_case(face.family) && (f.role == "caption" || f.role == "rejected")));
                if action == "set" {
                    br.fonts.retain(|f| f.role != "caption");
                    br.fonts.push(crate::model::FontToken { family: face.family.into(), role: "caption".into(), source: "chosen".into(), ..Default::default() });
                    if b(a, "heading") || !br.fonts.iter().any(|f| f.role == "heading") {
                        br.fonts.retain(|f| f.role != "heading");
                        br.fonts.push(crate::model::FontToken { family: face.family.into(), role: "heading".into(), source: "chosen".into(), ..Default::default() });
                    }
                } else {
                    br.fonts.retain(|f| !f.family.eq_ignore_ascii_case(face.family));
                    br.fonts.push(crate::model::FontToken { family: face.family.into(), role: "rejected".into(), source: "user".into(), ..Default::default() });
                }
                if !br.edit.locked {
                    br.edit = brand::derive_edit_brand(&br);
                }
                p.save_brand(&br)?;
                memory::add(
                    &p,
                    &format!(
                        "Typeface {} {}{}",
                        face.family,
                        if action == "set" { "chosen for on-screen text" } else { "rejected" },
                        if why.is_empty() { String::new() } else { format!(": {why}") }
                    ),
                    if action == "set" { "decision" } else { "rejection" },
                    vec!["font".into()],
                    "high",
                    "user",
                    true,
                    None,
                )?;
                self.refresh(&p);
                let (_, now) = self.text_style(&p);
                Ok(json!({"project": p.config.slug, "action": action, "family": face.family, "now_using": now}))
            }
            "prefs" => {
                let mut prefs = fonts::load_prefs(&self.home);
                if let Some(v) = a.get("caps").and_then(|x| x.as_bool()) {
                    prefs.caption_caps = Some(v);
                }
                if let Some(v) = n(a, "breath") {
                    prefs.cut_breath = Some(v.clamp(0.0, 1.5));
                }
                if let Some(v) = n(a, "words") {
                    prefs.caption_words = Some((v as usize).clamp(1, 6));
                }
                fonts::save_prefs(&self.home, &prefs)?;
                Ok(serde_json::to_value(prefs)?)
            }
            _ => {
                // show: what this project would use right now, and why
                let p = self.project(a)?;
                let br = p.load_brand();
                let rejected_here: Vec<String> = br.fonts.iter().filter(|f| f.role == "rejected").map(|f| f.family.clone()).collect();
                let c = fonts::pick(&self.home, &br, &rejected_here);
                let alternatives: Vec<&str> = {
                    let v = fonts::vibes(&br);
                    fonts::CATALOG.iter().filter(|f| f.family != c.face.family && f.tags.iter().any(|t| v.contains(t))).map(|f| f.family).take(6).collect()
                };
                Ok(
                    json!({"project": p.config.slug, "family": c.face.family, "why": c.why, "note": c.face.note, "vibes": fonts::vibes(&br), "alternatives": alternatives, "prefs": fonts::load_prefs(&self.home), "rejected_here": rejected_here}),
                )
            }
        }
    }

    /// Things the user wants in every project live here, not in any one project.
    fn user_memory(&self) -> Project {
        let dir = self.home.root.join("user");
        let _ = std::fs::create_dir_all(&dir);
        Project { dir, config: crate::model::ProjectConfig { name: "user".into(), slug: "user".into(), ..Default::default() } }
    }

    /// Story clips: transcript, search, a model's plan, and the build (see `story`).
    fn story_op(&self, action: &str, a: &Value) -> Result<Value> {
        let p = self.project(a)?;
        let (file, mut cues) = self.source_and_cues(&p, a)?;
        let stem = file.file_stem().map(|x| x.to_string_lossy().to_string()).unwrap_or_default();
        if action == "transcribe" || (cues.is_empty() && b(a, "transcribe")) {
            // no captions for this recording: listen to all of it (slow, local, once)
            let words = transcribe::words(&self.home, &file, s(a, "language").as_deref().unwrap_or("auto"))?;
            cues = story::caption_lines(&words, 9)
                .into_iter()
                .map(|l| clips::Cue { start: l[0].start, end: l[l.len() - 1].end, text: l.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ") })
                .collect();
            write_atomic(&p.dir.join("transcripts").join(format!("{stem}.json")), serde_json::to_string(&cues)?.as_bytes())?;
        }
        match action {
            "transcript" | "transcribe" => {
                if cues.is_empty() {
                    return fail(
                        "this recording has no transcript yet. Pass transcript=<.srt/.vtt>, a link whose site has captions, or transcribe=true to listen to it with whisper",
                    );
                }
                let (from, to) = (n(a, "from").unwrap_or(0.0), n(a, "to").unwrap_or(f64::MAX));
                Ok(json!({"file": path_str(&file), "lines": cues.len(), "transcript": story::transcript_lines(&cues, from, to, 90_000),
                    "how_to_cut": "Plan clips as JSON and pass it to action=build as spec: {\"clips\":[{\"name\",\"title\",\"end\":\"natural|punchline\",\"segments\":[{\"start\",\"end\",\"why\"}],\"context\":[{\"query\",\"label\",\"before_segment\"}]}]}. Start on the hook, use several segments to skip dead time, aim for the length the user asked for."}))
            }
            "search" => {
                let Some(q) = s(a, "query") else { return fail("search needs a query") };
                let hits = story::search(&cues, &q, n(a, "k").map(|v| v as usize).unwrap_or(8));
                Ok(json!({"query": q, "hits": hits}))
            }
            "events" => {
                // what happened, judged by sound; then where the picture cuts after each peak
                let Some(ffmpeg) = motion::render::find_ffmpeg() else { return fail("ffmpeg not found") };
                let audio = clips::analyse_audio(&ffmpeg, &file)?;
                let mut ev = clips::events(&audio, n(a, "count").map(|v| v as usize).unwrap_or(12));
                for e in ev.iter_mut() {
                    e.cuts_after = clips::scene_cuts(&ffmpeg, &file, e.peak, (e.peak + 20.0).min(audio.duration));
                }
                Ok(json!({"file": path_str(&file), "duration": audio.duration, "events": ev,
                    "how_to_use": "Each event is a moment the sound says something happened (crowd, casters, a reaction). rise is where the build-up began, settle where it calmed, cuts_after where the picture cut (a replay usually starts at the first). Sound does not say WHAT happened: call action=look around an event to see it, then plan the clip so it includes the build-up (start some seconds before rise), the moment, and the reaction or replay."}))
            }
            "score" => {
                // where a scoreboard (or any counter) changes for good: the goals of a quiet stream
                let Some(ffmpeg) = motion::render::find_ffmpeg() else { return fail("ffmpeg not found") };
                let Some(region) = s(a, "region").as_deref().and_then(clips::parse_region) else {
                    return fail(
                        "give region=x,y,w,h as fractions of the frame around the score digits only (not the clock), e.g. region=0.47,0.0,0.06,0.05; look at a frame first",
                    );
                };
                let audio = clips::analyse_audio(&ffmpeg, &file).ok();
                let from = n(a, "from").unwrap_or(0.0).max(0.0);
                let to = n(a, "to").or(audio.as_ref().map(|x| x.duration)).unwrap_or(from + 600.0);
                let changes = clips::region_changes(&ffmpeg, &file, from, to, region)?;
                let preview = p.dir.join("looks").join(format!("{stem}-region.png"));
                clips::region_preview(&ffmpeg, &file, changes.first().map(|c| c + 0.5).unwrap_or(from + 1.0), region, &preview)?;
                Ok(json!({"file": path_str(&file), "region": s(a, "region"), "changes": changes, "contact": path_str(&preview),
                    "how_to_use": "Each time is where the region changed and stayed changed: in a match, a goal (the score) or a new round. Check the red box on the preview sits on the score and not on the clock. Plan every highlight around these: start at the start of the play that led to it, and end after the outcome and the reaction or replay; never end a segment in the seconds before one of these times. Pass score_region to action=build and the cut is checked against them."}))
            }
            "look" => {
                let Some(ffmpeg) = motion::render::find_ffmpeg() else { return fail("ffmpeg not found") };
                let from = n(a, "from").unwrap_or(0.0).max(0.0);
                let to = n(a, "to").unwrap_or(from + 30.0);
                if to <= from {
                    return fail("look needs from < to (seconds)");
                }
                let (cols, rows) = (n(a, "cols").map(|v| v as u32).unwrap_or(4).clamp(1, 6), n(a, "rows").map(|v| v as u32).unwrap_or(3).clamp(1, 6));
                let out = p.dir.join("looks").join(format!("{stem}-{:.0}-{:.0}.png", from, to));
                let times = clips::storyboard(&ffmpeg, &file, from, to, cols, rows, &out)?;
                Ok(
                    json!({"contact": path_str(&out), "from": from, "to": to, "grid": format!("{cols}x{rows}"), "times": times, "note": "frames are in reading order (left to right, top to bottom); times[i] is the second each one was taken"}),
                )
            }
            "build" | "story" | "highlights" => {
                let mut notes: Vec<String> = Vec::new();
                let mut stories: Vec<story::Story> = if action == "highlights" {
                    // no words needed: build-up, moment and aftermath around each loud event
                    let Some(ffmpeg) = motion::render::find_ffmpeg() else { return fail("ffmpeg not found") };
                    let audio = clips::analyse_audio(&ffmpeg, &file)?;
                    let mut ev = clips::events(&audio, 40);
                    ev.sort_by(|x, y| y.over.partial_cmp(&x.over).unwrap_or(std::cmp::Ordering::Equal));
                    ev.truncate(n(a, "count").map(|v| v as usize).unwrap_or(3));
                    ev.sort_by(|x, y| x.peak.partial_cmp(&y.peak).unwrap_or(std::cmp::Ordering::Equal));
                    if ev.is_empty() {
                        return fail("the sound of this recording never rises above its normal level: nothing to call a highlight. Use action=look to see it and plan by hand");
                    }
                    let (lead, tail) = (n(a, "lead").unwrap_or(10.0), n(a, "tail").unwrap_or(9.0));
                    notes.push(format!("{} event(s) found by sound; {lead:.0}s of build-up and up to {tail:.0}s of aftermath kept around each", ev.len()));
                    ev.iter()
                        .enumerate()
                        .map(|(i, e)| story::Story {
                            name: format!("{stem}-highlight-{:02}", i + 1),
                            segments: vec![story::Segment {
                                start: (e.rise.min(e.peak - 4.0) - lead).max(0.0),
                                end: (e.settle.max(e.peak + 3.0)).min(e.peak + tail).min(audio.duration),
                                why: format!("sound peaks {:.0} LU over normal at {:.0}s", e.over, e.peak),
                                ..Default::default()
                            }],
                            end: "natural".into(),
                            why: "chosen by sound alone".into(),
                            ..Default::default()
                        })
                        .collect()
                } else if action == "story" {
                    if cues.is_empty() {
                        return fail("planning needs a transcript: pass transcript=<file>, a link with captions, or transcribe=true");
                    }
                    let adapter = s(a, "adapter").or_else(|| p.config.adapter.clone()).unwrap_or_else(|| "claude".into());
                    let ad = adapters::get(&self.home, &adapter)?;
                    let lines = story::transcript_lines(&cues, n(a, "from").unwrap_or(0.0), n(a, "to").unwrap_or(f64::MAX), 60_000);
                    let rejections: Vec<String> = memory::active(&p).into_iter().filter(|m| m.kind == "rejection" || m.kind == "preference").map(|m| m.text).take(8).collect();
                    let extra = format!(
                        "{} {}",
                        s(a, "notes").unwrap_or_default(),
                        if rejections.is_empty() { String::new() } else { format!("Known preferences: {}", rejections.join(" | ")) }
                    );
                    let answer =
                        adapters::run_text(&ad, &story::plan_prompt(&lines, n(a, "count").map(|v| v as usize).unwrap_or(3), n(a, "seconds").unwrap_or(60.0), extra.trim()))?;
                    notes.push(format!("planned by {adapter}"));
                    story::parse_plan(&answer)?
                } else {
                    let spec = match (a.get("spec"), s(a, "spec_file")) {
                        (Some(v), _) if v.is_object() || v.is_array() => v.clone(),
                        (Some(Value::String(t)), _) => serde_json::from_str(t)?,
                        (_, Some(f)) => serde_json::from_str(std::fs::read_to_string(&f)?.trim_start_matches('\u{feff}'))?,
                        _ => return fail("build needs the plan: spec={\"clips\":[...]} or spec_file=<json>"),
                    };
                    story::parse_plan(&spec.to_string())?
                };
                if b(a, "plan_only") {
                    return Ok(json!({"clips": stories, "notes": notes}));
                }
                let (style, font_note) = self.text_style(&p);
                let (w, h) = match s(a, "aspect").as_deref().unwrap_or("9:16") {
                    "1:1" => (1080, 1080),
                    "4:5" => (1080, 1350),
                    "16:9" => (1920, 1080),
                    _ => (1080, 1920),
                };
                let out_dir = s(a, "out").map(PathBuf::from).unwrap_or_else(|| p.dir.join("clips"));
                let sound_events: Vec<clips::Event> =
                    motion::render::find_ffmpeg().and_then(|ff| clips::analyse_audio(&ff, &file).ok()).map(|au| clips::events(&au, 60)).unwrap_or_default();
                // the outcomes (score changes) the cut must not stop short of
                let score_changes: Vec<f64> = match (s(a, "score_region").as_deref().and_then(clips::parse_region), motion::render::find_ffmpeg()) {
                    (Some(r), Some(ff)) => {
                        let lo = stories.iter().flat_map(|s| s.segments.iter().map(|x| x.start)).fold(f64::MAX, f64::min);
                        let hi = stories.iter().flat_map(|s| s.segments.iter().map(|x| x.end)).fold(0.0, f64::max);
                        clips::region_changes(&ff, &file, (lo - 5.0).max(0.0), hi + 15.0, r).unwrap_or_default()
                    }
                    _ => Vec::new(),
                };
                let mut built: Vec<Value> = Vec::new();
                for (i, st) in stories.iter_mut().enumerate() {
                    notes.extend(story::resolve_context(st, &cues));
                    let name = if st.name.trim().is_empty() { format!("{stem}-story-{:02}", i + 1) } else { slugify(&st.name) };
                    let o = story::BuildOptions {
                        source: file.clone(),
                        out: out_dir.join(format!("{name}.mp4")),
                        width: w,
                        height: h,
                        layout: s(a, "layout").unwrap_or_else(|| "fit".into()),
                        captions: !b(a, "no_captions"),
                        language: s(a, "language").unwrap_or_else(|| "auto".into()),
                        style: style.clone(),
                        max_pause: n(a, "max_pause").unwrap_or(0.55),
                        breath: n(a, "breath").or(fonts::load_prefs(&self.home).cut_breath).unwrap_or(0.4).clamp(0.0, 1.5),
                        punch: !b(a, "no_punch"),
                        transition: s(a, "transition").unwrap_or_else(|| "zoom".into()),
                        sfx: !b(a, "no_sfx"),
                        snap: !b(a, "no_snap"),
                        rough: cues.clone(),
                    };
                    let r = story::build(&self.home, st, &o)?;
                    // show the planner what fell just outside each segment, and what the sound says about the edges
                    let review = out_dir.join(format!("{name}.review.png"));
                    let reviewed = story::review_sheet(&file, &r.cut, &review).is_ok();
                    let mut checks = story::edge_checks(&r.cut, &sound_events);
                    checks.extend(story::payoff_checks(&r.cut, &score_changes));
                    let _ = assets::register_file(&p, Path::new(&r.out), "clip", vec!["story".into(), name.clone()]);
                    write_atomic(&out_dir.join(format!("{name}.story.json")), serde_json::to_string_pretty(&st)?.as_bytes())?;
                    let mut v = serde_json::to_value(&r)?;
                    v["name"] = json!(name);
                    v["title"] = json!(st.title);
                    v["end"] = json!(st.end);
                    v["checks"] = json!(checks);
                    if reviewed {
                        v["contact"] = json!(path_str(&review));
                        v["review_rows"] = json!("one row per segment, in order");
                        v["review_columns"] = json!(story::REVIEW_COLUMNS);
                    }
                    built.push(v);
                }
                memory::add(
                    &p,
                    &format!(
                        "Story clips built from {stem}: {}",
                        built.iter().map(|b| format!("{} ({}s)", b["name"].as_str().unwrap_or(""), b["duration"])).collect::<Vec<_>>().join(", ")
                    ),
                    "run",
                    vec!["clips".into(), "story".into()],
                    "medium",
                    "agent",
                    false,
                    None,
                )?;
                self.refresh(&p);
                if built.iter().any(|b| b["captions"] != "whisper") && !b(a, "no_captions") {
                    notes.push("captions did not come from whisper: run `editemos tools install-whisper` for accurate, word-timed captions".into());
                }
                Ok(json!({"clips": built, "typeface": font_note, "notes": notes, "images": built.iter().filter_map(|b| b.get("contact").cloned()).collect::<Vec<_>>(),
                    "before_you_call_it_done": "Look at each clip's review sheet (contact): one row per segment; the two frames on the left and the two on the right were LEFT OUT. If they show the set-up (who had the ball, the score before, the question being asked) or the outcome (the goal going in, the replay, the reaction), the segment is cutting the story: move its start or end and build again. Read checks: each line is an edge the sound disagrees with. Fix what is real, rebuild once or twice, then show the user."}))
            }
            other => fail(format!("unknown clips action '{other}'")),
        }
    }

    /// A long recording in, the best moments out (see `clips`).
    pub fn clips_op(&self, a: &Value) -> Result<Value> {
        if let Some(action) = s(a, "action").filter(|x| x != "auto") {
            return self.story_op(&action, a);
        }
        let p = self.project(a)?;
        let Some(input) = s(a, "file").or_else(|| s(a, "input")) else { return fail("clips needs the recording: file=<path>") };
        // a link instead of a file: fetch it (and its subtitles) into the project first
        let mut fetched_srt: Option<PathBuf> = None;
        let input = if clips::is_url(&input) {
            let (video, srt) = clips::fetch(&input, &p.dir.join("sources"), n(a, "source_height").map(|v| v as u32).unwrap_or(1080), s(a, "section").as_deref())?;
            fetched_srt = srt;
            path_str(&video)
        } else {
            input
        };
        let input = std::fs::canonicalize(&input).map_err(|_| crate::error::Error(format!("{input} not found")))?;
        let d = clips::Options::default();
        // captions in the project's own type and colours (libass wants &HBBGGRR)
        let br = p.load_brand();
        let ass = |hex: &str| crate::color::parse_color(hex).map(|c| format!("&H00{:02X}{:02X}{:02X}", c.2, c.1, c.0)).unwrap_or_else(|| "&H00FFFFFF".into());
        let caption_style = format!(
            "Fontname={},Fontsize=15,Bold=1,PrimaryColour={},OutlineColour={},BorderStyle=1,Outline=2,Shadow=0,MarginV=64,Alignment=2",
            if br.edit.type_tokens.heading.is_empty() { "Arial" } else { &br.edit.type_tokens.heading },
            ass(&br.edit.colors.fg),
            ass(&br.edit.colors.bg)
        );
        let o = clips::Options {
            out_dir: s(a, "out").map(PathBuf::from).unwrap_or_else(|| p.dir.join("clips")),
            input,
            count: n(a, "count").map(|v| v as usize).unwrap_or(d.count),
            min_s: n(a, "min").unwrap_or(d.min_s),
            max_s: n(a, "max").unwrap_or(d.max_s),
            aspect: s(a, "aspect").unwrap_or(d.aspect),
            height: n(a, "height").map(|v| v as u32).unwrap_or(d.height),
            transcript: s(a, "transcript").map(PathBuf::from).or(fetched_srt),
            copy: b(a, "copy"),
            burn: b(a, "burn"),
            layout: s(a, "layout").unwrap_or(d.layout),
            caption_style: Some(caption_style),
            plan_only: b(a, "plan_only"),
            jobs: n(a, "jobs").map(|v| v as usize).unwrap_or(d.jobs),
        };
        let m = clips::run(&o)?;
        for c in &m.clips {
            if let Some(f) = &c.file {
                let _ = assets::register_file(&p, Path::new(f), "clip", vec!["clip".into(), format!("score {}", c.score)]);
            }
        }
        memory::add(
            &p,
            &format!(
                "Clips mode on {}: {} clips of {:.0}-{:.0}s ({}) from {:.0} min, analysed {}x faster than real time -> {}",
                Path::new(&m.input).file_name().map(|x| x.to_string_lossy().to_string()).unwrap_or_default(),
                m.clips.len(),
                o.min_s,
                o.max_s,
                o.aspect,
                m.duration / 60.0,
                m.analysis_speed,
                m.manifest
            ),
            "run",
            vec!["clips".into()],
            "medium",
            "agent",
            false,
            None,
        )?;
        self.refresh(&p);
        Ok(serde_json::to_value(m)?)
    }

    /* ---------------- sessions and studio ---------------- */

    /// Open an agent in its own window, with this project's context.
    pub fn session_op(&self, a: &Value) -> Result<Value> {
        let p = self.project(a)?;
        let l = session::Launch {
            adapter: s(a, "adapter").or_else(|| p.config.adapter.clone()).unwrap_or_else(|| "claude".into()),
            model: s(a, "model").unwrap_or_default(),
            effort: s(a, "effort").unwrap_or_default(),
            task: s(a, "task"),
            dir: s(a, "dir").map(PathBuf::from),
            dry_run: b(a, "dry_run"),
        };
        session::launch(self, &p, &l)
    }

    /// Start the live studio page in its own process and return its address.
    pub fn studio_op(&self, a: &Value) -> Result<Value> {
        let p = self.project(a)?;
        // an already running studio is reused: one page, not one per call
        let marker = self.home.root.join("studio.url");
        if let Ok(url) = std::fs::read_to_string(&marker) {
            let alive = url
                .trim()
                .strip_prefix("http://")
                .and_then(|h| h.trim_end_matches('/').parse::<std::net::SocketAddr>().ok())
                .map(|addr| std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(300)).is_ok())
                .unwrap_or(false);
            if alive {
                let full = format!("{}?project={}", url.trim(), p.config.slug);
                if !b(a, "no_open") {
                    studio::open_browser(&full);
                }
                return Ok(json!({"url": full, "started": false}));
            }
        }
        let mut c = std::process::Command::new(std::env::current_exe()?);
        c.args(["studio", "--serve", "--port", &n(a, "port").map(|v| (v as u16).to_string()).unwrap_or_else(|| "4173".into())])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            c.creation_flags(0x0000_0008 | 0x0800_0000);
        }
        c.spawn()?;
        for _ in 0..40 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            if let Ok(url) = std::fs::read_to_string(&marker) {
                let full = format!("{}?project={}", url.trim(), p.config.slug);
                if !b(a, "no_open") {
                    studio::open_browser(&full);
                }
                return Ok(json!({"url": full, "started": true}));
            }
        }
        fail("the studio did not start")
    }

    /* ---------------- kdenlive ---------------- */

    pub fn kdenlive_op(&self, a: &Value) -> Result<Value> {
        match s(a, "action").as_deref().unwrap_or("status") {
            "status" => Ok(serde_json::to_value(kdenlive::status(b(a, "start"), s(a, "project_file").or_else(|| s(a, "file")).map(PathBuf::from).as_deref()))?),
            "inspect" => {
                let Some(f) = s(a, "file") else { return fail("inspect needs a file") };
                Ok(serde_json::to_value(kdenlive::inspect(Path::new(&f))?)?)
            }
            "diff" => {
                let (Some(x), Some(y)) = (s(a, "file"), s(a, "other")) else { return fail("diff needs file and other") };
                let d = kdenlive::diff(&kdenlive::inspect(Path::new(&x))?, &kdenlive::inspect(Path::new(&y))?);
                Ok(serde_json::to_value(d)?)
            }
            "overlay" => {
                let (Some(src), Some(clip), Some(out)) = (s(a, "file"), s(a, "clip"), s(a, "out")) else { return fail("overlay needs file, clip and out") };
                let r = kdenlive::overlay(
                    Path::new(&src),
                    Path::new(&clip),
                    n(a, "at").unwrap_or(0.0),
                    n(a, "dur"),
                    Path::new(&out),
                    &s(a, "label").unwrap_or_else(|| "Motion".into()),
                )?;
                Ok(serde_json::to_value(r)?)
            }
            "draft" => {
                let (Some(file), Some(out)) = (s(a, "file"), s(a, "out")) else { return fail("draft needs file and out") };
                let r = kdenlive::render_draft(Path::new(&file), Path::new(&out), n(a, "timeout").map(|v| v as u64).unwrap_or(600))?;
                Ok(serde_json::to_value(r)?)
            }
            "learn" => self.kdenlive_learn(a),
            other => fail(format!("unknown kdenlive action '{other}' (status, inspect, diff, overlay, draft, learn)")),
        }
    }

    /// Learn from a human edit: diff a saved Kdenlive project against the last
    /// version this project saw, and write what changed into long-term memory.
    pub fn kdenlive_learn(&self, a: &Value) -> Result<Value> {
        let p = self.project(a)?;
        let Some(file) = s(a, "file") else { return fail("learn needs the .kdenlive file") };
        let now = kdenlive::inspect(Path::new(&file))?;
        let baseline_path = p.runs_dir().join("kdenlive-last.json");
        let before: Option<kdenlive::Summary> =
            std::fs::read_to_string(&baseline_path).ok().and_then(|t| serde_json::from_str::<(String, kdenlive::Summary)>(&t).ok()).filter(|(f, _)| f == &file).map(|(_, s)| s);
        write_atomic(&baseline_path, serde_json::to_string(&(&file, &now))?.as_bytes())?;
        let Some(before) = before else {
            return Ok(
                json!({"changed": false, "baseline": true, "note": "baseline stored; the next save will be diffed against it", "duration": now.duration, "tracks": now.track_count}),
            );
        };
        let d = kdenlive::diff(&before, &now);
        if d.is_empty() {
            return Ok(json!({"changed": false, "summary": d.describe()}));
        }
        let text = format!("Human edit in Kdenlive ({}): {}", Path::new(&file).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(), d.describe());
        memory::add(&p, &text, "note", vec!["kdenlive-diff".into(), "taste".into()], "medium", "watch", false, None)?;
        self.refresh(&p);
        Ok(json!({"changed": true, "summary": d.describe(), "diff": d}))
    }

    /* ---------------- skills ---------------- */

    pub fn skills_op(&self, a: &Value) -> Result<Value> {
        match s(a, "action").as_deref().unwrap_or("list") {
            "catalog" => Ok(json!({"catalog": skills::catalog()})),
            "sync" => {
                let opts = skills::SyncOptions {
                    only_ids: list(a, "ids"),
                    include_cloud: b(a, "include_cloud"),
                    max_file_bytes: 300 * 1024,
                    dry_run: b(a, "dry_run"),
                    force: b(a, "force"),
                };
                let reports = skills::sync(&self.home, &opts);
                Ok(json!({"reports": reports}))
            }
            "enable" | "disable" => {
                let mut p = self.project(a)?;
                let ids = list(a, "ids");
                let enable = s(a, "action").as_deref() == Some("enable");
                for id in ids {
                    p.config.skills.retain(|x| x != &id);
                    if enable {
                        p.config.skills.push(id);
                    }
                }
                p.config.skills.sort();
                p.config.skills.dedup();
                p.save_config()?;
                self.refresh(&p);
                Ok(json!({"enabled": p.config.skills}))
            }
            "export" => {
                let p = self.project(a).ok();
                let ids = if list(a, "ids").is_empty() { p.map(|p| p.config.skills).unwrap_or_default() } else { list(a, "ids") };
                let Some(dest) = s(a, "dest") else { return fail("export needs a destination directory") };
                let (n_skills, n_files, dirs) = skills::export(&self.home, &ids, s(a, "agent").as_deref().unwrap_or("claude"), Path::new(&dest), b(a, "allow_flagged"))?;
                Ok(json!({"skills": n_skills, "files": n_files, "destinations": dirs.iter().map(|d| path_str(d)).collect::<Vec<_>>()}))
            }
            "read" => {
                let Some(name) = s(a, "name") else { return fail("read needs a skill name or id") };
                let all = skills::local_skills(&self.home);
                let hit = all.iter().find(|k| k.name == name || k.id == name || slugify(&k.name) == slugify(&name));
                match hit {
                    Some(k) => {
                        Ok(json!({"id": k.id, "name": k.name, "path": k.skill_md, "flagged": k.flagged, "content": truncate_chars(&std::fs::read_to_string(&k.skill_md)?, 24_000)}))
                    }
                    None => fail(format!("no downloaded skill '{name}'. Run `editemos skills sync`")),
                }
            }
            _ => {
                let all = skills::local_skills(&self.home);
                let q = s(a, "query").map(|q| q.to_lowercase());
                let rows: Vec<Value> = all
                    .into_iter()
                    .filter(|k| q.as_ref().map(|q| k.name.to_lowercase().contains(q) || k.description.to_lowercase().contains(q) || k.id.contains(q.as_str())).unwrap_or(true))
                    .take(n(a, "k").map(|v| v as usize).unwrap_or(60))
                    .map(|k| json!({"id": k.id, "name": k.name, "description": k.description, "license": k.license, "flagged": k.flagged, "flagged_files": k.flagged_files.len(), "path": k.skill_md}))
                    .collect();
                Ok(json!({"count": rows.len(), "skills": rows}))
            }
        }
    }

    /* ---------------- autonomous agent run ---------------- */

    pub fn agent_run(&self, a: &Value) -> Result<Value> {
        let p = self.project(a)?;
        let Some(task) = s(a, "task") else { return fail("agent run needs a task") };
        let adapter_name = s(a, "adapter").or_else(|| p.config.adapter.clone()).unwrap_or_else(|| "claude".into());
        let ad = adapters::get(&self.home, &adapter_name)?;
        let pack = context::build(&self.home, &p, &context::PackOptions { task: Some(task.clone()), budget_tokens: 7000 });
        let prompt = format!(
            "{pack}\n---\n## Task\n\n{task}\n\n## Rules for this run\n\n- Work only inside the project and its media folders; never modify source media.\n- When you decide something worth keeping, call the `editemos_memory_add` tool (or run `editemos memory add`).\n- Finish with a short report: what you did, files produced, open questions.\n"
        );
        let started = unix_secs();
        let out = adapters::run_text(&ad, &prompt)?;
        let log = p.runs_dir().join(format!("{}-{}.md", started, slugify(&truncate_chars(&task, 40))));
        write_atomic(&log, format!("# Run {}\n\nAdapter: {adapter_name}\n\n## Prompt\n\n{prompt}\n\n## Output\n\n{out}\n", crate::util::iso_from_secs(started)).as_bytes())?;
        memory::add(
            &p,
            &format!("Agent run via {adapter_name}: {} -> {}", truncate_chars(&task, 120), truncate_chars(&out.replace('\n', " "), 200)),
            "run",
            vec!["agent".into()],
            "medium",
            "agent",
            false,
            None,
        )?;
        self.refresh(&p);
        Ok(json!({"adapter": adapter_name, "log": path_str(&log), "output": truncate_chars(&out, 12_000)}))
    }

    /* ---------------- doctor ---------------- */

    pub fn doctor(&self) -> Result<Value> {
        let k = kdenlive::status(false, None);
        let ad = adapters::load(&self.home);
        let cli_ok = |name: &str| -> bool {
            let probe = if cfg!(windows) { ("where", name) } else { ("which", name) };
            std::process::Command::new(probe.0).arg(probe.1).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().map(|s| s.success()).unwrap_or(false)
        };
        let scr = extract::scrapling::locate(&self.home);
        Ok(json!({
            "home": path_str(&self.home.root),
            "active_project": self.home.active_slug(),
            "projects": self.home.list().len(),
            "browser": crate::cdp::find_browser().map(|p| path_str(&p)),
            "ffmpeg": motion::render::find_ffmpeg().map(|p| path_str(&p)),
            "kdenlive": {"installed": k.installed, "version": k.version, "running": k.running, "melt": k.melt},
            "scrapling": scr.map(|p| path_str(&p)),
            "agents_on_path": {"claude": cli_ok("claude"), "codex": cli_ok("codex"), "gemini": cli_ok("gemini"), "ollama": cli_ok("ollama")},
            "adapters": ad.keys().collect::<Vec<_>>(),
            "skills_downloaded": skills::local_skills(&self.home).len(),
            "image_keys": {"OPENAI_API_KEY": std::env::var("OPENAI_API_KEY").is_ok(), "GEMINI_API_KEY": std::env::var("GEMINI_API_KEY").is_ok()},
        }))
    }
}
