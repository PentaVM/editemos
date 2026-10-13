//! Thin blocking HTTP client shared by extraction, skill sync and HTTP adapters.

use crate::error::{fail, Result};
use std::io::Read;
use std::time::Duration;

pub const USER_AGENT: &str = concat!("editemos/", env!("CARGO_PKG_VERSION"), " (+https://github.com/PentaVM/editemos)");

pub struct Fetched {
    pub bytes: Vec<u8>,
    pub content_type: String,
    pub url: String,
    pub status: u16,
}

pub fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(10)).timeout(Duration::from_secs(30)).user_agent(USER_AGENT).redirects(5).build()
}

/// GET with a byte cap. Non-2xx statuses are returned as errors.
pub fn get(url: &str, max_bytes: usize, headers: &[(&str, &str)]) -> Result<Fetched> {
    get_within(url, max_bytes, headers, 30)
}

/// Download many URLs at once (8 at a time), each with its own byte cap and a short
/// timeout. Results come back in input order; failures are `None`.
pub fn get_many(jobs: &[(String, usize)], timeout_s: u64) -> Vec<Option<Fetched>> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;
    let next = AtomicUsize::new(0);
    let out: Mutex<Vec<(usize, Option<Fetched>)>> = Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for _ in 0..jobs.len().clamp(1, 8) {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some((url, cap)) = jobs.get(i) else { break };
                let r = get_within(url, *cap, &[], timeout_s).ok();
                out.lock().unwrap().push((i, r));
            });
        }
    });
    let mut v = out.into_inner().unwrap();
    v.sort_by_key(|(i, _)| *i);
    v.into_iter().map(|(_, r)| r).collect()
}

/// Stream a large file to disk (models, tools) without holding it in memory.
pub fn download(url: &str, dest: &std::path::Path, max_bytes: u64, timeout_s: u64) -> Result<u64> {
    let agent = ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(15)).timeout(Duration::from_secs(timeout_s)).user_agent(USER_AGENT).redirects(8).build();
    let resp = match agent.get(url).call() {
        Ok(r) => r,
        Err(ureq::Error::Status(code, _)) => return fail(format!("HTTP {code} for {url}")),
        Err(e) => return fail(format!("request to {url} failed: {e}")),
    };
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let part = dest.with_extension("part");
    let mut file = std::fs::File::create(&part)?;
    let n = std::io::copy(&mut resp.into_reader().take(max_bytes + 1), &mut file)?;
    drop(file);
    if n > max_bytes {
        let _ = std::fs::remove_file(&part);
        return fail(format!("{url}: larger than {max_bytes} bytes"));
    }
    std::fs::rename(&part, dest)?;
    Ok(n)
}

/// Like `get`, with an overall time limit in seconds.
pub fn get_within(url: &str, max_bytes: usize, headers: &[(&str, &str)], timeout_s: u64) -> Result<Fetched> {
    let agent =
        ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(timeout_s.min(10))).timeout(Duration::from_secs(timeout_s)).user_agent(USER_AGENT).redirects(5).build();
    let mut req = agent.get(url);
    for (k, v) in headers {
        req = req.set(k, v);
    }
    match req.call() {
        Ok(resp) => read_response(resp, max_bytes),
        Err(ureq::Error::Status(code, _)) => fail(format!("HTTP {code} for {url}")),
        Err(e) => fail(format!("request to {url} failed: {e}")),
    }
}

fn read_response(resp: ureq::Response, max_bytes: usize) -> Result<Fetched> {
    let status = resp.status();
    let content_type = resp.header("content-type").unwrap_or("").to_string();
    let url = resp.get_url().to_string();
    let mut bytes = Vec::new();
    resp.into_reader().take(max_bytes as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > max_bytes {
        return fail(format!("{url}: response larger than {max_bytes} bytes"));
    }
    Ok(Fetched { bytes, content_type, url, status })
}

pub fn get_text(url: &str, max_bytes: usize) -> Result<(String, String)> {
    let f = get(url, max_bytes, &[("Accept", "text/html,application/xhtml+xml,text/css,*/*;q=0.8")])?;
    Ok((String::from_utf8_lossy(&f.bytes).to_string(), f.url))
}

/// POST JSON and return the response body as text.
pub fn post_json(url: &str, body: &serde_json::Value, headers: &[(&str, &str)], timeout_s: u64) -> Result<String> {
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(timeout_s)).user_agent(USER_AGENT).build();
    let mut req = agent.post(url).set("Content-Type", "application/json");
    for (k, v) in headers {
        req = req.set(k, v);
    }
    match req.send_string(&body.to_string()) {
        Ok(r) => Ok(r.into_string()?),
        Err(ureq::Error::Status(code, r)) => {
            let text = r.into_string().unwrap_or_default();
            fail(format!("HTTP {code}: {}", crate::util::truncate_chars(&text, 400)))
        }
        Err(e) => fail(format!("request to {url} failed: {e}")),
    }
}

/// Bearer token for GitHub API calls: GITHUB_TOKEN or GH_TOKEN if set.
pub fn github_token() -> Option<String> {
    std::env::var("GITHUB_TOKEN").or_else(|_| std::env::var("GH_TOKEN")).ok().filter(|t| !t.is_empty())
}
