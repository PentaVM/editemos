//! Scrapling backend. Scrapling (BSD-3-Clause, Python) is an adaptive scraper with
//! TLS impersonation and stealth browsers. editemos drives its CLI as an external
//! process, so the Rust binary stays dependency-free and Python is optional.
//!
//! Install it into an isolated venv with `editemos tools install-scrapling`.

use crate::error::{fail, Result};
use crate::store::Home;
use crate::util::short_id;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn venv_exe(home: &Home) -> PathBuf {
    let base = home.tools_dir().join("scrapling");
    if cfg!(windows) {
        base.join("Scripts").join("scrapling.exe")
    } else {
        base.join("bin").join("scrapling")
    }
}

fn on_path(name: &str) -> Option<PathBuf> {
    let cmd = if cfg!(windows) { ("where", vec![name.to_string()]) } else { ("which", vec![name.to_string()]) };
    let out = Command::new(cmd.0).args(&cmd.1).stderr(Stdio::null()).output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout).lines().next().map(|l| PathBuf::from(l.trim())).filter(|p| p.exists())
}

pub fn locate(home: &Home) -> Option<PathBuf> {
    if let Ok(p) = std::env::var("EDITEMOS_SCRAPLING") {
        if Path::new(&p).exists() {
            return Some(PathBuf::from(p));
        }
    }
    let v = venv_exe(home);
    if v.exists() {
        return Some(v);
    }
    on_path("scrapling")
}

/// Fetch a page's HTML with `scrapling extract <mode> <url> <file.html>`.
/// mode: get (HTTP with browser TLS fingerprint) | fetch (browser) | stealthy-fetch.
pub fn fetch_html(exe: &Path, url: &str, mode: &str) -> Result<String> {
    let mode = match mode {
        "get" | "fetch" | "stealthy-fetch" => mode,
        _ => "get",
    };
    let out = std::env::temp_dir().join(format!("editemos-scrapling-{}.html", short_id()));
    let status = Command::new(exe).args(["extract", mode, url]).arg(&out).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).output()?;
    if !status.status.success() {
        let err = String::from_utf8_lossy(&status.stderr);
        return fail(format!("scrapling failed ({}): {}", status.status, crate::util::truncate_chars(err.trim(), 300)));
    }
    let html = std::fs::read_to_string(&out).map_err(|e| crate::error::Error(format!("scrapling wrote no output: {e}")))?;
    let _ = std::fs::remove_file(&out);
    Ok(html)
}

/// Create `~/.editemos/tools/scrapling` and install Scrapling (wheels only: nothing
/// from source is executed). The browsers are NOT downloaded; the `browser` backend
/// uses the Edge/Chrome already on the machine.
pub fn install(home: &Home, python: Option<&str>) -> Result<PathBuf> {
    let venv = home.tools_dir().join("scrapling");
    crate::util::ensure_dir(&home.tools_dir())?;
    let py = python.map(String::from).or_else(|| on_path("python").map(|p| p.to_string_lossy().to_string())).unwrap_or_else(|| "python".into());
    if !venv.exists() {
        let st = Command::new(&py).args(["-m", "venv"]).arg(&venv).stdout(std::process::Stdio::from(std::io::stderr())).status()?;
        if !st.success() {
            return fail("python -m venv failed (is Python 3.10+ installed?)");
        }
    }
    let pip = if cfg!(windows) { venv.join("Scripts").join("python.exe") } else { venv.join("bin").join("python") };
    let st = Command::new(&pip)
        .args(["-m", "pip", "install", "--disable-pip-version-check", "--only-binary", ":all:", "scrapling[fetchers,shell]"])
        .stdout(std::process::Stdio::from(std::io::stderr()))
        .status()?;
    if !st.success() {
        return fail("pip install scrapling failed");
    }
    let exe = venv_exe(home);
    if exe.exists() {
        Ok(exe)
    } else {
        fail("scrapling installed but its CLI was not found in the venv")
    }
}
