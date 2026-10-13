//! Long memory per project: an append-only JSONL file that any agent can write to
//! and that never depends on a chat session. Retrieval is BM25 plus a little
//! weight for confidence, pinning and recency.

use crate::error::Result;
use crate::model::Memory;
use crate::store::Project;
use crate::util::{iso_from_secs, now_iso, secs_from_iso, short_id, tokenize, unix_secs, write_atomic};
use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::Write;

pub const KINDS: &[&str] = &["decision", "preference", "rejection", "fact", "note", "run"];

#[allow(clippy::too_many_arguments)]
pub fn add(p: &Project, text: &str, kind: &str, tags: Vec<String>, confidence: &str, source: &str, pinned: bool, supersedes: Option<String>) -> Result<Memory> {
    let kind = if KINDS.contains(&kind) || kind == "tombstone" { kind } else { "note" };
    let confidence = match confidence {
        "low" | "medium" | "high" => confidence,
        _ => "medium",
    };
    let m = Memory {
        id: short_id(),
        ts: now_iso(),
        kind: kind.to_string(),
        text: text.trim().to_string(),
        tags,
        confidence: confidence.to_string(),
        source: if source.is_empty() { "user".into() } else { source.into() },
        pinned,
        supersedes,
    };
    let mut f = OpenOptions::new().create(true).append(true).open(p.memory_path())?;
    writeln!(f, "{}", serde_json::to_string(&m)?)?;
    Ok(m)
}

/// Every line, in order, including superseded entries and tombstones.
pub fn load_all(p: &Project) -> Vec<Memory> {
    fs::read_to_string(p.memory_path()).unwrap_or_default().lines().filter_map(|l| serde_json::from_str::<Memory>(l).ok()).collect()
}

/// Entries that are still in force (not superseded, not tombstones).
pub fn active(p: &Project) -> Vec<Memory> {
    let all = load_all(p);
    let dead: HashSet<String> = all.iter().filter_map(|m| m.supersedes.clone()).collect();
    all.into_iter().filter(|m| m.kind != "tombstone" && !dead.contains(&m.id)).collect()
}

pub fn forget(p: &Project, id: &str) -> Result<()> {
    add(p, "forgotten", "tombstone", vec![], "high", "user", false, Some(id.to_string()))?;
    Ok(())
}

fn conf_weight(c: &str) -> f64 {
    match c {
        "high" => 1.25,
        "low" => 0.8,
        _ => 1.0,
    }
}

fn recency(ts: &str) -> f64 {
    let age_days = secs_from_iso(ts).map(|t| unix_secs().saturating_sub(t) as f64 / 86_400.0).unwrap_or(0.0);
    // 1.0 today, 0.7 at ~half a year and older
    0.7 + 0.3 * (-age_days / 90.0).exp()
}

/// BM25 over text + tags + kind. Returns (score, memory), best first.
pub fn search(mems: &[Memory], query: &str, k: usize) -> Vec<(f64, Memory)> {
    let q = tokenize(query);
    if q.is_empty() || mems.is_empty() {
        return Vec::new();
    }
    let docs: Vec<Vec<String>> = mems.iter().map(|m| tokenize(&format!("{} {} {}", m.text, m.tags.join(" "), m.kind))).collect();
    let n = docs.len() as f64;
    let avg = docs.iter().map(|d| d.len()).sum::<usize>() as f64 / n.max(1.0);
    // document frequency is only needed for the words of the query
    let mut df: HashMap<&str, usize> = HashMap::new();
    let q_set: HashSet<&str> = q.iter().map(|s| s.as_str()).collect();
    for d in &docs {
        for t in &q_set {
            if d.iter().any(|x| x == t) {
                *df.entry(*t).or_insert(0) += 1;
            }
        }
    }
    let (k1, b) = (1.5, 0.75);
    let mut scored: Vec<(f64, usize)> = Vec::new();
    for (i, d) in docs.iter().enumerate() {
        let mut score = 0.0;
        let len = d.len() as f64;
        for t in &q {
            let tf = d.iter().filter(|x| *x == t).count() as f64;
            if tf == 0.0 {
                continue;
            }
            let dfv = *df.get(t.as_str()).unwrap_or(&0) as f64;
            let idf = ((n - dfv + 0.5) / (dfv + 0.5) + 1.0).ln();
            score += idf * (tf * (k1 + 1.0)) / (tf + k1 * (1.0 - b + b * len / avg.max(1.0)));
        }
        if score > 0.0 {
            let m = &mems[i];
            score *= conf_weight(&m.confidence) * recency(&m.ts);
            if m.pinned {
                score += 0.5;
            }
            scored.push((score, i));
        }
    }
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(k);
    scored.into_iter().map(|(s, i)| (s, mems[i].clone())).collect()
}

pub struct CompactReport {
    pub before: usize,
    pub after: usize,
    pub backup: String,
}

/// Rewrite the file keeping only entries in force, dropping exact duplicates
/// (the newest copy wins). A backup of the old file is kept next to it.
pub fn compact(p: &Project) -> Result<CompactReport> {
    let all = load_all(p);
    let before = all.len();
    let mut keep = active(p);
    let mut seen: HashSet<String> = HashSet::new();
    keep.reverse();
    keep.retain(|m| seen.insert(format!("{}|{}", m.kind, m.text.to_lowercase())));
    keep.reverse();
    let backup = p.memory_path().with_file_name(format!("memory.{}.bak.jsonl", iso_from_secs(unix_secs()).replace(':', "-")));
    if p.memory_path().exists() {
        fs::copy(p.memory_path(), &backup)?;
    }
    let mut out = String::new();
    for m in &keep {
        out.push_str(&serde_json::to_string(m)?);
        out.push('\n');
    }
    write_atomic(&p.memory_path(), out.as_bytes())?;
    Ok(CompactReport { before, after: keep.len(), backup: backup.to_string_lossy().to_string() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem(id: &str, text: &str, conf: &str) -> Memory {
        Memory {
            id: id.into(),
            ts: "2026-10-12T00:00:00Z".into(),
            kind: "decision".into(),
            text: text.into(),
            tags: vec![],
            confidence: conf.into(),
            source: "user".into(),
            pinned: false,
            supersedes: None,
        }
    }

    #[test]
    fn bm25_ranks_relevant_first() {
        let ms = vec![
            mem("1", "Lower thirds use a hairline rule and no rounded box", "medium"),
            mem("2", "Export masters as ProRes 4444", "medium"),
            mem("3", "Captions are one line, never yellow highlight", "medium"),
        ];
        let r = search(&ms, "lower third rule", 2);
        assert_eq!(r[0].1.id, "1");
    }

    #[test]
    fn confidence_breaks_ties() {
        let ms = vec![mem("1", "grain on every shot", "low"), mem("2", "grain on every shot", "high")];
        let r = search(&ms, "grain", 2);
        assert_eq!(r[0].1.id, "2");
    }
}
