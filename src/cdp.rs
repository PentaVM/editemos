//! A tiny Chrome DevTools Protocol client over a blocking WebSocket.
//!
//! Launches the user's own Edge or Chrome headless (no downloads), connects to a
//! page target and exposes `call`, `eval` and `screenshot`. Used by the motion
//! renderer and by the browser extraction backend.

use crate::error::{fail, Result};
use base64::Engine;
use serde_json::{json, Value};
use std::fs;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use tungstenite::{stream::MaybeTlsStream, Message, WebSocket};

pub fn find_browser() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("EDITEMOS_BROWSER") {
        if Path::new(&p).exists() {
            return Some(PathBuf::from(p));
        }
    }
    let candidates = [
        r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files\Google\Chrome\Application\chrome.exe",
        r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
        "/usr/bin/google-chrome",
        "/usr/bin/chromium",
        "/usr/bin/chromium-browser",
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
    ];
    if let Some(p) = candidates.iter().map(PathBuf::from).find(|p| p.exists()) {
        return Some(p);
    }
    // anything Chromium-based on PATH (distro packages, snaps, Brave, Edge for Linux)
    let names = ["google-chrome", "google-chrome-stable", "chromium", "chromium-browser", "microsoft-edge", "microsoft-edge-stable", "brave-browser", "chrome"];
    std::env::var_os("PATH").and_then(|paths| std::env::split_paths(&paths).flat_map(|d| names.iter().map(move |n| d.join(n))).find(|p| p.is_file()))
}

pub struct Browser {
    child: Child,
    user_data: PathBuf,
    port: u16,
}

impl Browser {
    pub fn launch(width: u32, height: u32) -> Result<Browser> {
        let Some(exe) = find_browser() else {
            return fail("no Edge or Chrome found; set EDITEMOS_BROWSER to a Chromium-based browser");
        };
        let _ = std::thread::Builder::new().name("editemos-sweep".into()).spawn(sweep_stale);
        let user_data = std::env::temp_dir().join(format!("{PROFILE_PREFIX}{}", crate::util::short_id()));
        fs::create_dir_all(&user_data)?;
        // containers and some CI images cannot use Chromium's sandbox
        let no_sandbox: &[&str] = if std::env::var_os("EDITEMOS_NO_SANDBOX").is_some() { &["--no-sandbox"] } else { &[] };
        let child = Command::new(&exe)
            .args(no_sandbox)
            .args([
                "--headless=new",
                "--disable-gpu",
                "--hide-scrollbars",
                "--mute-audio",
                "--no-first-run",
                "--no-default-browser-check",
                "--disable-extensions",
                "--disable-background-networking",
                "--disable-component-update",
                "--disable-sync",
                "--force-device-scale-factor=1",
                "--allow-file-access-from-files",
                "--autoplay-policy=no-user-gesture-required",
                // Several tabs capture at once: none may be throttled as "background".
                "--disable-background-timer-throttling",
                "--disable-renderer-backgrounding",
                "--disable-backgrounding-occluded-windows",
                "--disable-features=CalculateNativeWinOcclusion",
                "--remote-debugging-port=0",
            ])
            .arg(format!("--window-size={width},{height}"))
            .arg(format!("--user-data-dir={}", user_data.display()))
            .arg("about:blank")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let port_file = user_data.join("DevToolsActivePort");
        let t0 = Instant::now();
        let port = loop {
            if let Ok(s) = fs::read_to_string(&port_file) {
                if let Some(p) = s.lines().next().and_then(|l| l.trim().parse::<u16>().ok()) {
                    break p;
                }
            }
            if t0.elapsed() > Duration::from_secs(25) {
                kill_tree(&child);
                return fail("timed out waiting for the browser's DevTools port");
            }
            std::thread::sleep(Duration::from_millis(80));
        };
        crate::util::trace(&format!("browser ready on port {port} after {} ms", t0.elapsed().as_millis()));
        Ok(Browser { child, user_data, port })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Connect to the first page target.
    pub fn page(&self) -> Result<Cdp> {
        let t0 = Instant::now();
        let ws_url = loop {
            let url = format!("http://127.0.0.1:{}/json/list", self.port);
            if let Ok(resp) = ureq::get(&url).timeout(Duration::from_secs(5)).call() {
                if let Ok(list) = resp.into_json::<Value>() {
                    if let Some(t) = list.as_array().and_then(|a| a.iter().find(|t| t["type"] == "page")) {
                        if let Some(u) = t["webSocketDebuggerUrl"].as_str() {
                            break u.to_string();
                        }
                    }
                }
            }
            if t0.elapsed() > Duration::from_secs(20) {
                return fail("browser started but exposed no page target");
            }
            std::thread::sleep(Duration::from_millis(100));
        };
        connect(&ws_url)
    }

    /// Open a page in its own window (used for parallel frame capture).
    pub fn new_page(&self, width: u32, height: u32) -> Result<Cdp> {
        new_page_on(self.port, width, height)
    }
}

/// Open a page in a new window on a browser identified only by its debugging port,
/// so worker threads do not need a reference to the `Browser`. A separate window
/// matters: tabs of one window are "background" and never get a frame to capture.
pub fn new_page_on(port: u16, width: u32, height: u32) -> Result<Cdp> {
    let version: Value = ureq::get(&format!("http://127.0.0.1:{port}/json/version")).timeout(Duration::from_secs(10)).call()?.into_json()?;
    let Some(browser_ws) = version["webSocketDebuggerUrl"].as_str() else { return fail("the browser exposed no control endpoint") };
    let mut control = connect(browser_ws)?;
    let created = control.call("Target.createTarget", json!({"url": "about:blank", "newWindow": true, "width": width, "height": height}))?;
    let Some(id) = created["targetId"].as_str() else { return fail("the browser did not create a window") };
    connect(&format!("ws://127.0.0.1:{port}/devtools/page/{id}"))
}

fn connect(ws_url: &str) -> Result<Cdp> {
    let (mut ws, _) = tungstenite::connect(ws_url)?;
    if let MaybeTlsStream::Plain(s) = ws.get_mut() {
        let _ = s.set_read_timeout(Some(Duration::from_secs(60)));
        let _ = s.set_nodelay(true);
    }
    Ok(Cdp { ws, next_id: 0 })
}

fn kill_tree(child: &Child) {
    // Fire and forget: on some machines Windows takes tens of seconds to tear Edge
    // down, and nothing here needs to wait for it.
    if cfg!(windows) {
        let _ = Command::new("taskkill").args(["/PID", &child.id().to_string(), "/T", "/F"]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
    } else {
        let _ = Command::new("kill").args(["-9", &child.id().to_string()]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
    }
}

const PROFILE_PREFIX: &str = "editemos-ud-";

/// Delete a browser profile folder, retrying while the dying browser still holds
/// files. Refuses anything that is not one of our temporary profiles.
pub fn sweep_profile(dir: &Path, attempts: u32) {
    let ours =
        dir.parent().map(|p| p == std::env::temp_dir().as_path()).unwrap_or(false) && dir.file_name().map(|n| n.to_string_lossy().starts_with(PROFILE_PREFIX)).unwrap_or(false);
    if !ours {
        return;
    }
    for _ in 0..attempts {
        if !dir.exists() || fs::remove_dir_all(dir).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(1000));
    }
}

/// Hand the profile folder to a detached copy of this program (`__cleanup`) so the
/// caller returns at once and the folder still disappears after we exit.
fn schedule_cleanup(dir: &Path) {
    if let Ok(exe) = std::env::current_exe() {
        let ours = exe.file_stem().map(|s| s.to_string_lossy().eq_ignore_ascii_case("editemos")).unwrap_or(false);
        if ours {
            let mut c = Command::new(&exe);
            c.arg("__cleanup").arg(dir).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                c.creation_flags(0x0000_0008 | 0x0800_0000); // DETACHED_PROCESS | CREATE_NO_WINDOW
            }
            if c.spawn().is_ok() {
                return;
            }
        }
    }
    let d = dir.to_path_buf();
    let _ = std::thread::Builder::new().name("editemos-cleanup".into()).spawn(move || sweep_profile(&d, 60));
}

/// Remove profile folders left behind by crashed runs (older than ten minutes).
fn sweep_stale() {
    let Ok(rd) = fs::read_dir(std::env::temp_dir()) else { return };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if !name.starts_with(PROFILE_PREFIX) {
            continue;
        }
        let old = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).map(|d| d > Duration::from_secs(600)).unwrap_or(false);
        if old {
            let _ = fs::remove_dir_all(e.path());
        }
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        kill_tree(&self.child);
        let _ = self.child.kill();
        schedule_cleanup(&self.user_data);
        crate::util::trace("browser: released");
    }
}

pub struct Cdp {
    ws: WebSocket<MaybeTlsStream<TcpStream>>,
    next_id: u64,
}

impl Cdp {
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        self.next_id += 1;
        let id = self.next_id;
        self.ws.send(Message::Text(json!({"id": id, "method": method, "params": params}).to_string()))?;
        loop {
            match self.ws.read()? {
                Message::Text(t) => {
                    let v: Value = serde_json::from_str(&t)?;
                    if v["id"].as_u64() == Some(id) {
                        if let Some(e) = v.get("error") {
                            return fail(format!("{method}: {}", e["message"].as_str().unwrap_or("error")));
                        }
                        return Ok(v["result"].clone());
                    }
                }
                Message::Close(_) => return fail("browser closed the connection"),
                _ => {}
            }
        }
    }

    /// Evaluate JavaScript; the result must be JSON-serialisable.
    pub fn eval(&mut self, expression: &str, await_promise: bool) -> Result<Value> {
        let r = self.call("Runtime.evaluate", json!({"expression": expression, "awaitPromise": await_promise, "returnByValue": true}))?;
        if let Some(d) = r.get("exceptionDetails") {
            let msg = d["exception"]["description"].as_str().or(d["text"].as_str()).unwrap_or("script error");
            return fail(format!("page script error: {msg}"));
        }
        Ok(r["result"]["value"].clone())
    }

    pub fn navigate(&mut self, url: &str, timeout: Duration) -> Result<()> {
        self.call("Page.enable", json!({}))?;
        self.call("Page.navigate", json!({"url": url}))?;
        let t0 = Instant::now();
        loop {
            let state = self.eval("document.readyState", false).unwrap_or(Value::Null);
            if state == "complete" {
                return Ok(());
            }
            if t0.elapsed() > timeout {
                return fail(format!("page did not finish loading within {}s", timeout.as_secs()));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    pub fn screenshot(&mut self, full_page: bool) -> Result<Vec<u8>> {
        let r = self.call("Page.captureScreenshot", json!({"format": "png", "optimizeForSpeed": true, "fromSurface": true, "captureBeyondViewport": full_page}))?;
        let b64 = r["data"].as_str().unwrap_or("");
        Ok(base64::engine::general_purpose::STANDARD.decode(b64)?)
    }

    pub fn set_viewport(&mut self, width: u32, height: u32) -> Result<()> {
        self.call("Emulation.setDeviceMetricsOverride", json!({"width": width, "height": height, "deviceScaleFactor": 1, "mobile": false}))?;
        Ok(())
    }

    pub fn transparent_background(&mut self) -> Result<()> {
        self.call("Emulation.setDefaultBackgroundColorOverride", json!({"color": {"r": 0, "g": 0, "b": 0, "a": 0}}))?;
        Ok(())
    }

    pub fn inject_on_new_document(&mut self, source: &str) -> Result<()> {
        self.call("Page.addScriptToEvaluateOnNewDocument", json!({"source": source}))?;
        Ok(())
    }
}
