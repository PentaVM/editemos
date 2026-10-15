//! Studio: a local page that shows the project as it changes. Extracted assets,
//! motion renders, clips and memory appear the moment the agent (or you) makes
//! them, so a person can watch, correct and keep editing by hand.
//!
//! It is a plain HTTP server on 127.0.0.1 with no dependencies: one page, a small
//! JSON API and a media endpoint with byte ranges for scrubbing video.

use crate::error::{fail, Result};
use crate::motion::library;
use crate::ops::Ctx;
use crate::store::Project;
use crate::util::{path_str, sha256_hex};
use crate::{assets, memory};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::UNIX_EPOCH;

const PAGE: &str = include_str!("../assets/studio.html");

fn mtime(p: &Path) -> u64 {
    std::fs::metadata(p).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Changes whenever anything the page shows changes.
pub fn version(p: &Project) -> String {
    let files = [p.brand_path(), p.memory_path(), p.assets_index(), p.motion_dir().join("library.json"), p.dir.join("project.toml")];
    let stamp: u64 = files.iter().map(|f| mtime(f)).fold(0, |a, b| a.wrapping_mul(31).wrapping_add(b));
    format!("{stamp:x}")
}

fn ext_of(path: &str) -> String {
    Path::new(path).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

fn media_url(slug: &str, path: &str) -> String {
    format!("/media?project={}&path={}", urlencode(slug), urlencode(path))
}

fn urlencode(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

fn urldecode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => {
                if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                    out.push(v);
                    i += 3;
                    continue;
                }
                out.push(b'%');
            }
            b'+' => out.push(b' '),
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

fn abs_asset(p: &Project, file: &str) -> PathBuf {
    let f = Path::new(file);
    if f.is_absolute() {
        f.to_path_buf()
    } else {
        p.assets_dir().join(file)
    }
}

/// Everything the page shows, newest first.
pub fn state(ctx: &Ctx, p: &Project) -> Value {
    let slug = &p.config.slug;
    let mut items: Vec<(String, Value)> = Vec::new();
    let pieces = library::load(p);
    let mut in_library: Vec<String> = Vec::new();
    for x in &pieces {
        let last = x.renders.last();
        if let Some(r) = last {
            in_library.push(r.file.to_lowercase());
        }
        let label = if x.text.is_empty() { x.kind.clone() } else { format!("{}: {}", x.kind, x.text) };
        items.push((
            last.map(|r| r.created.clone()).unwrap_or_else(|| x.created.clone()),
            json!({
                "id": format!("m{}", x.key), "kind": "motion", "label": label, "path": last.map(|r| r.file.clone()).unwrap_or_else(|| x.scene.clone()),
                "meta": last.map(|r| format!("{} {:.1}s{}", r.format, r.duration, if x.reused > 0 { format!(" - reused {}x", x.reused) } else { String::new() })).unwrap_or_else(|| "scene, not rendered".into()),
                "video": last.map(|r| format!("/preview?project={}&path={}&size={}", urlencode(slug), urlencode(&r.file), urlencode(&x.size))),
                "poster": last.and_then(|r| r.contact.clone()).map(|c| media_url(slug, &c)),
            }),
        ));
    }
    for a in assets::load(p) {
        let abs = path_str(&abs_asset(p, &a.file));
        if in_library.contains(&abs.to_lowercase()) {
            continue; // already shown as a motion piece
        }
        let ext = ext_of(&abs);
        let is_img = matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "ico");
        let is_vid = matches!(ext.as_str(), "mp4" | "webm" | "m4v");
        let name = Path::new(&abs).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        items.push((
            a.added.clone(),
            json!({
                "id": a.id, "kind": a.kind, "label": name, "path": abs, "ext": ext,
                "meta": if a.width > 0 { format!("{}x{} {}", a.width, a.height, a.origin) } else { a.origin.clone() },
                "image": if is_img { Some(media_url(slug, &abs)) } else { None },
                "video": if is_vid { Some(media_url(slug, &abs)) } else if ext == "mov" { Some(format!("/preview?project={}&path={}", urlencode(slug), urlencode(&abs))) } else { None },
            }),
        ));
    }
    items.sort_by(|a, b| b.0.cmp(&a.0));
    let mut mems = memory::active(p);
    mems.reverse();
    mems.truncate(60);
    json!({
        "version": version(p),
        "project": {"slug": p.config.slug, "name": p.config.name, "dir": path_str(&p.dir), "site": p.config.site},
        "projects": ctx.home.list().into_iter().map(|x| x.config.slug).collect::<Vec<_>>(),
        "brand": p.load_brand(),
        "items": items.into_iter().map(|(_, v)| v).collect::<Vec<_>>(),
        "memories": mems,
    })
}

/// Only files the project knows about may be served.
fn allowed(p: &Project, path: &Path) -> bool {
    let Ok(canon) = std::fs::canonicalize(path) else { return false };
    if std::fs::canonicalize(&p.dir).map(|d| canon.starts_with(d)).unwrap_or(false) {
        return true;
    }
    let same = |other: &str| std::fs::canonicalize(other).map(|o| o == canon).unwrap_or(false);
    assets::load(p).iter().any(|a| same(&path_str(&abs_asset(p, &a.file))))
        || library::load(p).iter().any(|x| x.renders.iter().any(|r| same(&r.file) || r.contact.as_deref().map(same).unwrap_or(false)))
}

fn mime(path: &Path) -> &'static str {
    match ext_of(&path.to_string_lossy()).as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        "json" => "application/json",
        _ => "application/octet-stream",
    }
}

/// Browsers cannot play ProRes with alpha: make (once) a small H.264 preview of
/// the graphic over the project's background colour.
fn preview(p: &Project, src: &Path, size: &str) -> Result<PathBuf> {
    let Some(ffmpeg) = crate::motion::render::find_ffmpeg() else { return fail("ffmpeg not found") };
    let key = sha256_hex(format!("{}|{}", src.display(), mtime(src)).as_bytes());
    let out = p.motion_dir().join("previews").join(format!("{}.mp4", &key[..16]));
    if out.exists() {
        return Ok(out);
    }
    std::fs::create_dir_all(out.parent().unwrap())?;
    let bg = p.load_brand().edit.colors.bg.trim_start_matches('#').to_string();
    let bg = if bg.len() == 6 { bg } else { "0e0e10".into() };
    let size = if size.contains('x') { size.to_string() } else { "1920x1080".into() };
    let tmp = out.with_extension("part.mp4");
    let r = Command::new(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", &format!("color=c=0x{bg}:s={size}:r=30"), "-i"])
        .arg(src)
        .args([
            "-filter_complex",
            "[0:v][1:v]overlay=shortest=1:format=auto,scale=960:-2,format=yuv420p",
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "-crf",
            "24",
            "-an",
            "-movflags",
            "+faststart",
        ])
        .arg(&tmp)
        .stdin(Stdio::null())
        .output()?;
    if !r.status.success() {
        return fail(format!("preview failed: {}", crate::util::truncate_chars(String::from_utf8_lossy(&r.stderr).trim(), 200)));
    }
    std::fs::rename(&tmp, &out)?;
    Ok(out)
}

struct Request {
    method: String,
    path: String,
    query: Vec<(String, String)>,
    range: Option<(u64, Option<u64>)>,
    origin: Option<String>,
    host: String,
    body: Vec<u8>,
}

impl Request {
    fn q(&self, k: &str) -> Option<&str> {
        self.query.iter().find(|(a, _)| a == k).map(|(_, v)| v.as_str())
    }
}

fn read_request(s: &TcpStream) -> Option<Request> {
    let mut r = BufReader::new(s.try_clone().ok()?);
    let mut line = String::new();
    r.read_line(&mut line).ok()?;
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_string();
    let target = parts.next()?.to_string();
    let (path, qs) = target.split_once('?').unwrap_or((&target, ""));
    let query = qs.split('&').filter(|x| !x.is_empty()).map(|kv| kv.split_once('=').unwrap_or((kv, ""))).map(|(k, v)| (urldecode(k), urldecode(v))).collect();
    let (mut len, mut range, mut origin, mut host) = (0usize, None, None, String::new());
    loop {
        let mut h = String::new();
        if r.read_line(&mut h).ok()? == 0 || h.trim().is_empty() {
            break;
        }
        let Some((k, v)) = h.split_once(':') else { continue };
        let v = v.trim();
        match k.to_ascii_lowercase().as_str() {
            "content-length" => len = v.parse().unwrap_or(0),
            "origin" => origin = Some(v.to_string()),
            "host" => host = v.to_string(),
            "range" => {
                if let Some((a, b)) = v.strip_prefix("bytes=").and_then(|x| x.split_once('-')) {
                    if let Ok(start) = a.parse::<u64>() {
                        range = Some((start, b.parse::<u64>().ok()));
                    }
                }
            }
            _ => {}
        }
    }
    let mut body = vec![0u8; len.min(1024 * 1024)];
    r.read_exact(&mut body).ok()?;
    Some(Request { method, path: path.to_string(), query, range, origin, host, body })
}

fn respond(s: &mut TcpStream, status: &str, ctype: &str, body: &[u8]) {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = s.write_all(head.as_bytes());
    let _ = s.write_all(body);
}

fn send_json(s: &mut TcpStream, v: &Value) {
    respond(s, "200 OK", "application/json; charset=utf-8", v.to_string().as_bytes());
}

fn send_file(s: &mut TcpStream, path: &Path, range: Option<(u64, Option<u64>)>) {
    let Ok(mut f) = std::fs::File::open(path) else { return respond(s, "404 Not Found", "text/plain", b"not found") };
    let total = f.metadata().map(|m| m.len()).unwrap_or(0);
    let (start, end, status) = match range {
        Some((a, b)) if a < total => (a, b.unwrap_or(total - 1).min(total - 1), "206 Partial Content"),
        _ => (0, total.saturating_sub(1), "200 OK"),
    };
    let len = if total == 0 { 0 } else { end - start + 1 };
    let mut head =
        format!("HTTP/1.1 {status}\r\nContent-Type: {}\r\nContent-Length: {len}\r\nAccept-Ranges: bytes\r\nCache-Control: no-store\r\nConnection: close\r\n", mime(path));
    if status.starts_with("206") {
        head.push_str(&format!("Content-Range: bytes {start}-{end}/{total}\r\n"));
    }
    head.push_str("\r\n");
    if s.write_all(head.as_bytes()).is_err() || f.seek(SeekFrom::Start(start)).is_err() {
        return;
    }
    let mut left = len;
    let mut buf = vec![0u8; 256 * 1024];
    while left > 0 {
        let want = buf.len().min(left as usize);
        match f.read(&mut buf[..want]) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if s.write_all(&buf[..n]).is_err() {
                    break;
                }
                left -= n as u64;
            }
        }
    }
}

fn local_origin(req: &Request) -> bool {
    // a page on another site must not be able to write to this machine's memory
    let ok_host = |h: &str| {
        let h = h.trim_start_matches("http://");
        h.starts_with("127.0.0.1") || h.starts_with("localhost") || h.starts_with("[::1]")
    };
    ok_host(&req.host) && req.origin.as_deref().map(ok_host).unwrap_or(true)
}

fn handle(ctx: &Ctx, mut s: TcpStream) {
    let Some(req) = read_request(&s) else { return };
    if !local_origin(&req) {
        return respond(&mut s, "403 Forbidden", "text/plain", b"local only");
    }
    let project = match req.q("project").filter(|p| !p.is_empty()) {
        Some(name) => ctx.home.open_project(name),
        None => ctx.home.active(),
    };
    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/") | ("GET", "/index.html") => respond(&mut s, "200 OK", "text/html; charset=utf-8", PAGE.as_bytes()),
        ("GET", "/api/version") => match project {
            Ok(p) => send_json(&mut s, &json!({"version": version(&p)})),
            Err(e) => respond(&mut s, "404 Not Found", "text/plain", e.0.as_bytes()),
        },
        ("GET", "/api/state") => match project {
            Ok(p) => send_json(&mut s, &state(ctx, &p)),
            Err(e) => respond(&mut s, "404 Not Found", "text/plain", e.0.as_bytes()),
        },
        ("POST", "/api/memory") => {
            let Ok(p) = project else { return respond(&mut s, "404 Not Found", "text/plain", b"no project") };
            let v: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
            let text = v["text"].as_str().unwrap_or("").trim().to_string();
            if text.is_empty() {
                return respond(&mut s, "400 Bad Request", "text/plain", b"text is required");
            }
            let kind = v["kind"].as_str().filter(|k| memory::KINDS.contains(k)).unwrap_or("note");
            match ctx.memory_add(&json!({"project": p.config.slug, "text": text, "kind": kind, "confidence": "high", "source": "studio"})) {
                Ok(m) => send_json(&mut s, &m),
                Err(e) => respond(&mut s, "500 Internal Server Error", "text/plain", e.0.as_bytes()),
            }
        }
        ("GET", "/media") | ("GET", "/preview") => {
            let Ok(p) = project else { return respond(&mut s, "404 Not Found", "text/plain", b"no project") };
            let path = PathBuf::from(req.q("path").unwrap_or(""));
            if !allowed(&p, &path) {
                return respond(&mut s, "403 Forbidden", "text/plain", b"not a file of this project");
            }
            if req.path == "/preview" {
                match preview(&p, &path, req.q("size").unwrap_or("")) {
                    Ok(f) => send_file(&mut s, &f, req.range),
                    Err(e) => respond(&mut s, "500 Internal Server Error", "text/plain", e.0.as_bytes()),
                }
            } else {
                send_file(&mut s, &path, req.range)
            }
        }
        _ => respond(&mut s, "404 Not Found", "text/plain", b"not found"),
    }
}

/// Bind the studio; `port` 0 picks a free one. Returns the listener and its URL.
pub fn bind(port: u16) -> Result<(TcpListener, String)> {
    let l = TcpListener::bind(("127.0.0.1", port)).or_else(|_| TcpListener::bind(("127.0.0.1", 0)))?;
    let url = format!("http://127.0.0.1:{}/", l.local_addr()?.port());
    Ok((l, url))
}

/// Serve until the process ends.
pub fn serve(ctx: &Ctx, listener: TcpListener) {
    std::thread::scope(|sc| {
        for stream in listener.incoming().flatten() {
            sc.spawn(move || {
                let keep = stream.try_clone();
                handle(ctx, stream);
                // close gracefully: a socket dropped with unread bytes is reset on
                // Windows, and the browser loses a response it was already sent
                if let Ok(mut s) = keep {
                    let _ = s.flush();
                    let _ = s.shutdown(std::net::Shutdown::Write);
                    let _ = s.set_read_timeout(Some(std::time::Duration::from_millis(300)));
                    let mut sink = [0u8; 4096];
                    while matches!(s.read(&mut sink), Ok(n) if n > 0) {}
                }
            });
        }
    });
}

/// Open a URL in the default browser.
pub fn open_browser(url: &str) {
    let mut c = if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.args(["/C", "start", "", url]);
        c
    } else if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg(url);
        c
    } else {
        let mut c = Command::new("xdg-open");
        c.arg(url);
        c
    };
    let _ = c.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
}
