//! The local store. Nothing leaves the machine: a home directory with a registry,
//! and one folder per project holding config, brand, memory, assets and context.
//!
//! ```text
//! ~/.editemos/
//!   active                  slug of the active project
//!   registry.json           slug -> directory (for projects stored elsewhere)
//!   adapters.toml           optional adapter overrides
//!   skills/                 downloaded skill library (see `editemos skills sync`)
//!   tools/                  optional tools (e.g. a Scrapling venv)
//!   projects/<slug>/
//!     project.toml  brand.json  memory.jsonl  style.md
//!     assets/ (+ index.json)   context/   motion/   runs/
//! ```

use crate::error::{fail, Result};
use crate::model::{Brand, ProjectConfig};
use crate::util::{ensure_dir, home_base, now_iso, slugify, write_atomic};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Default)]
struct Registry {
    #[serde(default)]
    projects: BTreeMap<String, String>,
}

#[derive(Clone)]
pub struct Home {
    pub root: PathBuf,
}

#[derive(Clone)]
pub struct Project {
    pub dir: PathBuf,
    pub config: ProjectConfig,
}

impl Home {
    pub fn open() -> Result<Home> {
        let root = home_base();
        ensure_dir(&root)?;
        ensure_dir(&root.join("projects"))?;
        Ok(Home { root })
    }

    pub fn skills_dir(&self) -> PathBuf {
        self.root.join("skills")
    }

    pub fn tools_dir(&self) -> PathBuf {
        self.root.join("tools")
    }

    pub fn adapters_path(&self) -> PathBuf {
        self.root.join("adapters.toml")
    }

    fn registry_path(&self) -> PathBuf {
        self.root.join("registry.json")
    }

    fn load_registry(&self) -> Registry {
        fs::read_to_string(self.registry_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    fn save_registry(&self, r: &Registry) -> Result<()> {
        write_atomic(&self.registry_path(), serde_json::to_string_pretty(r)?.as_bytes())
    }

    pub fn active_slug(&self) -> Option<String> {
        if let Ok(s) = std::env::var("EDITEMOS_PROJECT") {
            if !s.trim().is_empty() {
                return Some(slugify(&s));
            }
        }
        fs::read_to_string(self.root.join("active")).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
    }

    pub fn set_active(&self, slug: &str) -> Result<()> {
        write_atomic(&self.root.join("active"), slug.as_bytes())
    }

    fn project_dir(&self, slug: &str) -> PathBuf {
        if let Some(p) = self.load_registry().projects.get(slug) {
            return PathBuf::from(p);
        }
        self.root.join("projects").join(slug)
    }

    /// All known projects: those under `projects/` plus registered external ones.
    pub fn list(&self) -> Vec<Project> {
        let mut dirs: BTreeMap<String, PathBuf> = BTreeMap::new();
        if let Ok(rd) = fs::read_dir(self.root.join("projects")) {
            for e in rd.flatten() {
                if e.path().join("project.toml").exists() {
                    dirs.insert(e.file_name().to_string_lossy().to_string(), e.path());
                }
            }
        }
        for (slug, p) in self.load_registry().projects {
            dirs.insert(slug, PathBuf::from(p));
        }
        dirs.into_values().filter_map(|d| Project::load(&d).ok()).collect()
    }

    pub fn create(&self, name: &str, dir: Option<&Path>) -> Result<Project> {
        let slug = slugify(name);
        let target = match dir {
            Some(d) => d.to_path_buf(),
            None => self.root.join("projects").join(&slug),
        };
        if target.join("project.toml").exists() {
            return fail(format!("project '{slug}' already exists at {}", target.display()));
        }
        ensure_dir(&target)?;
        for sub in ["assets", "context", "motion/scenes", "motion/renders", "runs"] {
            ensure_dir(&target.join(sub))?;
        }
        let config = ProjectConfig { name: name.trim().to_string(), slug: slug.clone(), created: now_iso(), size: Some("1920x1080".into()), fps: Some(30.0), ..Default::default() };
        let project = Project { dir: target.clone(), config };
        project.save_config()?;
        if dir.is_some() {
            let mut reg = self.load_registry();
            reg.projects.insert(slug.clone(), target.to_string_lossy().to_string());
            self.save_registry(&reg)?;
        }
        if self.active_slug().is_none() {
            self.set_active(&slug)?;
        }
        Ok(project)
    }

    pub fn open_project(&self, name_or_slug: &str) -> Result<Project> {
        let slug = slugify(name_or_slug);
        let dir = self.project_dir(&slug);
        if !dir.join("project.toml").exists() {
            let known: Vec<String> = self.list().into_iter().map(|p| p.config.slug).collect();
            return fail(format!("no project '{slug}'. Known projects: {}", if known.is_empty() { "(none; run `editemos init <name>`)".into() } else { known.join(", ") }));
        }
        Project::load(&dir)
    }

    pub fn active(&self) -> Result<Project> {
        match self.active_slug() {
            Some(s) => self.open_project(&s),
            None => fail("no active project. Run `editemos init <name>` or `editemos use <name>`"),
        }
    }
}

impl Project {
    pub fn load(dir: &Path) -> Result<Project> {
        let text = fs::read_to_string(dir.join("project.toml"))?;
        let config: ProjectConfig = toml::from_str(&text)?;
        Ok(Project { dir: dir.to_path_buf(), config })
    }

    pub fn save_config(&self) -> Result<()> {
        write_atomic(&self.dir.join("project.toml"), toml::to_string_pretty(&self.config)?.as_bytes())
    }

    pub fn brand_path(&self) -> PathBuf {
        self.dir.join("brand.json")
    }
    pub fn memory_path(&self) -> PathBuf {
        self.dir.join("memory.jsonl")
    }
    pub fn assets_dir(&self) -> PathBuf {
        self.dir.join("assets")
    }
    pub fn assets_index(&self) -> PathBuf {
        self.dir.join("assets").join("index.json")
    }
    pub fn context_dir(&self) -> PathBuf {
        self.dir.join("context")
    }
    pub fn motion_dir(&self) -> PathBuf {
        self.dir.join("motion")
    }
    pub fn runs_dir(&self) -> PathBuf {
        self.dir.join("runs")
    }
    pub fn style_path(&self) -> PathBuf {
        self.dir.join("style.md")
    }

    pub fn load_brand(&self) -> Brand {
        // tolerate a byte-order mark: editors on Windows add one when a person edits the file
        fs::read_to_string(self.brand_path()).ok().and_then(|s| serde_json::from_str(s.trim_start_matches('\u{feff}')).ok()).unwrap_or_else(|| Brand {
            version: 1,
            name: self.config.name.clone(),
            ..Default::default()
        })
    }

    pub fn save_brand(&self, b: &Brand) -> Result<()> {
        write_atomic(&self.brand_path(), serde_json::to_string_pretty(b)?.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_open_roundtrip() {
        let _g = crate::util::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = std::env::temp_dir().join(format!("editemos-test-{}", crate::util::short_id()));
        std::env::set_var("EDITEMOS_HOME", &tmp);
        let home = Home::open().unwrap();
        let p = home.create("Studio A", None).unwrap();
        assert_eq!(p.config.slug, "studio-a");
        assert!(home.create("Studio A", None).is_err());
        let again = home.open_project("studio a").unwrap();
        assert_eq!(again.config.name, "Studio A");
        assert_eq!(home.active_slug().as_deref(), Some("studio-a"));
        let _ = fs::remove_dir_all(&tmp);
    }
}
