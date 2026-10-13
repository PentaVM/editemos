//! Operations shared by the CLI and the MCP server. Every function takes plain
//! values and returns JSON, so both front ends stay thin and always agree.

use crate::error::{fail, Result};
use crate::model::Memory;
use crate::store::{Home, Project};
use crate::util::{path_str, slugify, truncate_chars, unix_secs, write_atomic};
use crate::{adapters, assets, brand, clips, context, extract, kdenlive, memory, motion, session, skills, studio};
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
        let p = self.project(a)?;
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
        self.refresh(&p);
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
                Ok(v)
            }
            other => fail(format!("unknown motion action '{other}' (templates, new, library, render, lint, poster)")),
        }
    }

    /* ---------------- clips ---------------- */

    /// A long recording in, the best moments out (see `clips`).
    pub fn clips_op(&self, a: &Value) -> Result<Value> {
        let p = self.project(a)?;
        let Some(input) = s(a, "file").or_else(|| s(a, "input")) else { return fail("clips needs the recording: file=<path>") };
        let input = std::fs::canonicalize(&input).map_err(|_| crate::error::Error(format!("{input} not found")))?;
        let d = clips::Options::default();
        let o = clips::Options {
            out_dir: s(a, "out").map(PathBuf::from).unwrap_or_else(|| p.dir.join("clips")),
            input,
            count: n(a, "count").map(|v| v as usize).unwrap_or(d.count),
            min_s: n(a, "min").unwrap_or(d.min_s),
            max_s: n(a, "max").unwrap_or(d.max_s),
            aspect: s(a, "aspect").unwrap_or(d.aspect),
            height: n(a, "height").map(|v| v as u32).unwrap_or(d.height),
            transcript: s(a, "transcript").map(PathBuf::from),
            copy: b(a, "copy"),
            burn: b(a, "burn"),
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
