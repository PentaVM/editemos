//! Shared helpers for the integration tests: a tiny local web server and a
//! throwaway home directory.

#![allow(dead_code)]

use editemos::ops::Ctx;
use editemos::store::Home;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};

pub type Route = (&'static str, &'static str, Vec<u8>);

/// Serve fixed routes on 127.0.0.1 and return the base URL.
pub fn serve(routes: Vec<Route>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { continue };
            let routes = routes.clone();
            std::thread::spawn(move || {
                // read the whole request (headers, then Content-Length bytes of body)
                let mut data = Vec::new();
                let mut buf = [0u8; 8192];
                loop {
                    let n = s.read(&mut buf).unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    data.extend_from_slice(&buf[..n]);
                    if let Some(end) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&data[..end]).to_ascii_lowercase();
                        let len: usize = head.lines().find_map(|l| l.strip_prefix("content-length:")).and_then(|v| v.trim().parse().ok()).unwrap_or(0);
                        if data.len() >= end + 4 + len {
                            break;
                        }
                    }
                }
                let req = String::from_utf8_lossy(&data).to_string();
                let path = req.split_whitespace().nth(1).unwrap_or("/").split('?').next().unwrap_or("/").to_string();
                match routes.iter().find(|(p, _, _)| *p == path) {
                    Some((_, ct, body)) => {
                        let head = format!("HTTP/1.1 200 OK\r\nContent-Type: {ct}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                        let _ = s.write_all(head.as_bytes());
                        let _ = s.write_all(body);
                    }
                    None => {
                        let _ = s.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    }
                }
            });
        }
    });
    format!("http://127.0.0.1:{port}")
}

pub fn temp_home(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("editemos-it-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

pub fn ctx_in(root: &Path) -> Ctx {
    std::fs::create_dir_all(root.join("projects")).unwrap();
    Ctx { home: Home { root: root.to_path_buf() } }
}

/// A 1200x630 PNG with a vertical gradient between two brand colours.
pub fn hero_png() -> Vec<u8> {
    let mut img = image::RgbaImage::new(1200, 630);
    for (_, y, px) in img.enumerate_pixels_mut() {
        let t = y as f32 / 630.0;
        *px = image::Rgba([(255.0 * (1.0 - t) + 20.0 * t) as u8, (61.0 * (1.0 - t) + 17.0 * t) as u8, (31.0 * (1.0 - t) + 15.0 * t) as u8, 255]);
    }
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

pub const LOGO_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 120 40"><rect width="120" height="40" fill="#ff3d1f"/><text x="10" y="26" font-size="16" fill="#f6efe4">BRASA</text></svg>"##;

pub const SITE_HTML: &str = r##"<!doctype html>
<html lang="es"><head><meta charset="utf-8">
<title>Estudio Brasa - Cocina de fuego</title>
<meta name="description" content="Cocinamos con fuego real. Sin atajos.">
<meta name="theme-color" content="#ff3d1f">
<meta property="og:site_name" content="Estudio Brasa">
<meta property="og:image" content="/hero.png">
<link rel="icon" href="/logo.svg">
<link rel="stylesheet" href="/styles.css">
</head><body>
<header><a href="/"><img class="logo" src="/logo.svg" alt="Estudio Brasa logo"></a></header>
<main>
<h1>Fuego real. Sin atajos.</h1>
<p>Reserva tu mesa hoy. Cocinamos para ti.</p>
<p>Brasa, humo y sal. Nada mas.</p>
<a class="cta" href="/reservas">Reservar</a>
<img src="/hero.png" alt="Parrilla encendida" width="1200" height="630">
</main>
<footer><a href="https://www.instagram.com/estudiobrasa">Instagram</a></footer>
</body></html>"##;

pub const SITE_CSS: &str = "\
:root{--brand:#ff3d1f;--ink:#14110f;--paper:#f6efe4}\
body{background:var(--paper);color:var(--ink);font-family:'Space Grotesk',Arial,sans-serif}\
h1{font-family:'Fraunces',serif;color:var(--brand);font-size:64px}\
.cta{background:#ff3d1f;color:#ffffff;padding:12px 20px}\
";

pub fn site_routes() -> Vec<Route> {
    vec![
        ("/", "text/html; charset=utf-8", SITE_HTML.as_bytes().to_vec()),
        ("/styles.css", "text/css", SITE_CSS.as_bytes().to_vec()),
        ("/logo.svg", "image/svg+xml", LOGO_SVG.as_bytes().to_vec()),
        ("/hero.png", "image/png", hero_png()),
    ]
}

/// A video of solid-colour shots of known lengths over a tone (needs ffmpeg).
pub fn make_video(ffmpeg: &Path, out: &Path, shots: &[(&str, f64)], gain: f64) {
    let mut cmd = std::process::Command::new(ffmpeg);
    cmd.args(["-hide_banner", "-loglevel", "error", "-y"]);
    for (c, d) in shots {
        cmd.args(["-f", "lavfi", "-i", &format!("color=c={c}:s=320x180:d={d}:r=25")]);
    }
    let total: f64 = shots.iter().map(|s| s.1).sum();
    cmd.args(["-f", "lavfi", "-i", &format!("sine=f=440:d={total}")]);
    let inputs: String = (0..shots.len()).map(|i| format!("[{i}:v]")).collect();
    cmd.args(["-filter_complex", &format!("{inputs}concat=n={}:v=1:a=0[v];[{}:a]volume={gain}[a]", shots.len(), shots.len()), "-map", "[v]", "-map", "[a]"]);
    cmd.args(["-c:v", "mpeg4", "-q:v", "3", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"]).arg(out);
    let st = cmd.stdin(std::process::Stdio::null()).status().unwrap();
    assert!(st.success(), "could not make the test video");
}
