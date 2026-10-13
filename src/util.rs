//! Small helpers shared by every module: time, ids, slugs, hashing, text.

use crate::error::Result;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn unix_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// `2026-10-12T19:00:00Z` for a unix timestamp.
pub fn iso_from_secs(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, (rem % 3600) / 60, rem % 60)
}

pub fn now_iso() -> String {
    iso_from_secs(unix_secs())
}

/// Parse the leading `YYYY-MM-DDTHH:MM:SSZ` of an ISO string back to unix seconds.
pub fn secs_from_iso(s: &str) -> Option<u64> {
    let b = s.as_bytes();
    if b.len() < 19 {
        return None;
    }
    let n = |a: usize, z: usize| s.get(a..z)?.parse::<i64>().ok();
    let (y, m, d) = (n(0, 4)?, n(5, 7)?, n(8, 10)?);
    let (hh, mm, ss) = (n(11, 13)?, n(14, 16)?, n(17, 19)?);
    // days from civil
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = if y2 >= 0 { y2 } else { y2 - 399 } / 400;
    let yoe = y2 - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let total = days * 86_400 + hh * 3600 + mm * 60 + ss;
    if total < 0 {
        None
    } else {
        Some(total as u64)
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Short, unique-enough id (12 hex chars).
pub fn short_id() -> String {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let seed = format!("{nanos}-{n}-{}", std::process::id());
    sha256_hex(seed.as_bytes())[..12].to_string()
}

/// Lowercase ASCII slug; common accented letters are folded.
pub fn slugify(s: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for ch in s.chars() {
        let c = match ch {
            'á' | 'à' | 'ä' | 'â' | 'ã' | 'Á' | 'À' | 'Ä' | 'Â' | 'Ã' => 'a',
            'é' | 'è' | 'ë' | 'ê' | 'É' | 'È' | 'Ë' | 'Ê' => 'e',
            'í' | 'ì' | 'ï' | 'î' | 'Í' | 'Ì' | 'Ï' | 'Î' => 'i',
            'ó' | 'ò' | 'ö' | 'ô' | 'õ' | 'Ó' | 'Ò' | 'Ö' | 'Ô' | 'Õ' => 'o',
            'ú' | 'ù' | 'ü' | 'û' | 'Ú' | 'Ù' | 'Ü' | 'Û' => 'u',
            'ñ' | 'Ñ' => 'n',
            'ç' | 'Ç' => 'c',
            c => c.to_ascii_lowercase(),
        };
        if c.is_ascii_alphanumeric() {
            out.push(c);
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        "project".to_string()
    } else {
        out.chars().take(48).collect()
    }
}

const STOP: &[&str] = &[
    "the", "and", "for", "that", "with", "this", "from", "are", "was", "not", "but", "you", "your", "has", "have", "los", "las", "una", "uno", "que", "con", "para", "por", "del",
    "como", "mas", "pero", "sus", "este", "esta",
];

/// Lowercased alphanumeric tokens (>= 2 chars), stopwords removed.
pub fn tokenize(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            for l in ch.to_lowercase() {
                cur.push(l);
            }
        } else if !cur.is_empty() {
            push_token(&mut out, std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        push_token(&mut out, cur);
    }
    out
}

fn push_token(out: &mut Vec<String>, t: String) {
    if t.chars().count() >= 2 && !STOP.contains(&t.as_str()) {
        out.push(t);
    }
}

pub fn truncate_chars(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(n.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}

pub fn approx_tokens(s: &str) -> usize {
    s.chars().count() / 4 + 1
}

pub fn ensure_dir(p: &Path) -> Result<()> {
    fs::create_dir_all(p)?;
    Ok(())
}

/// Write via a temp file and rename so readers never see a partial file.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        ensure_dir(parent)?;
    }
    let tmp = path.with_extension(format!("tmp-{}", short_id()));
    fs::write(&tmp, bytes)?;
    if fs::rename(&tmp, path).is_err() {
        // Windows can refuse to replace an open file; fall back to copy.
        fs::copy(&tmp, path)?;
        let _ = fs::remove_file(&tmp);
    }
    Ok(())
}

pub fn home_base() -> PathBuf {
    if let Ok(h) = std::env::var("EDITEMOS_HOME") {
        if !h.trim().is_empty() {
            return PathBuf::from(h);
        }
    }
    let user = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap_or_else(|_| ".".into());
    PathBuf::from(user).join(".editemos")
}

pub fn human_bytes(n: u64) -> String {
    const U: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", U[i])
    }
}

pub fn path_str(p: &Path) -> String {
    p.to_string_lossy().to_string()
}

static TRACE_T0: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

/// Timing trace to stderr when `EDITEMOS_TRACE` is set (never touches stdout,
/// which belongs to the MCP protocol).
pub fn trace(msg: &str) {
    let t0 = TRACE_T0.get_or_init(std::time::Instant::now);
    if std::env::var_os("EDITEMOS_TRACE").is_some() {
        eprintln!("[trace {:7.2}s] {msg}", t0.elapsed().as_secs_f64());
    }
}

/// Tests that change process-wide environment variables take this lock.
#[cfg(test)]
pub static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_folds_accents() {
        assert_eq!(slugify("Corto de prueba áéí!"), "corto-de-prueba-aei");
        assert_eq!(slugify("***"), "project");
    }

    #[test]
    fn iso_roundtrip() {
        let s = iso_from_secs(1_791_840_000);
        assert_eq!(secs_from_iso(&s), Some(1_791_840_000));
        assert_eq!(iso_from_secs(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn tokens_drop_stopwords() {
        assert_eq!(tokenize("The Quick fox, y los ZORROS"), vec!["quick", "fox", "zorros"]);
    }
}
