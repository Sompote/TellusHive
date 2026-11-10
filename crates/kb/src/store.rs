//! On-disk storage helpers and the BM25 index.

use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::text::terms;

/// One stored chunk (a line of `chunks.jsonl`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    pub id: u32,
    pub clause: String,
    pub title: String,
    pub page: Option<u32>,
    pub text: String,
    #[serde(default)]
    pub context: String,
    #[serde(default)]
    pub part: u32,
    /// "clause" (from a document) or "note" (user-written).
    pub kind: String,
    /// File name the chunk came from, or "note".
    pub source: String,
}

impl Chunk {
    /// Text that is embedded and indexed: breadcrumb, label, title and body.
    pub fn index_text(&self) -> String {
        format!("{}\n{} {}\n{}", self.context, self.clause, self.title, self.text)
    }
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension(format!(
        "{}.tmp{}",
        path.extension().and_then(|e| e.to_str()).unwrap_or(""),
        std::process::id()
    ));
    {
        let mut f = std::fs::File::create(&tmp).map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
        f.write_all(bytes).map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
        f.sync_all().ok();
    }
    std::fs::rename(&tmp, path).map_err(|e| format!("cannot rename {} -> {}: {e}", tmp.display(), path.display()))
}

pub fn write_chunks(path: &Path, chunks: &[Chunk]) -> Result<(), String> {
    let mut s = String::new();
    for c in chunks {
        s.push_str(&serde_json::to_string(c).map_err(|e| e.to_string())?);
        s.push('\n');
    }
    atomic_write(path, s.as_bytes())
}

pub fn read_chunks(path: &Path) -> Result<Vec<Chunk>, String> {
    let s = std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    s.lines()
        .filter(|l| !l.trim().is_empty())
        .enumerate()
        .map(|(i, l)| serde_json::from_str(l).map_err(|e| format!("{} line {}: {e}", path.display(), i + 1)))
        .collect()
}

pub fn write_vectors(path: &Path, v: &[f32]) -> Result<(), String> {
    let mut b = Vec::with_capacity(v.len() * 4);
    for x in v {
        b.extend_from_slice(&x.to_le_bytes());
    }
    atomic_write(path, &b)
}

pub fn read_vectors(path: &Path) -> Result<Vec<f32>, String> {
    let b = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    if b.len() % 4 != 0 {
        return Err(format!("{} is truncated", path.display()));
    }
    Ok(b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
}

pub fn write_json<T: Serialize>(path: &Path, v: &T) -> Result<(), String> {
    let s = serde_json::to_string_pretty(v).map_err(|e| e.to_string())?;
    atomic_write(path, s.as_bytes())
}

pub fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, String> {
    let s = std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    serde_json::from_str(&s).map_err(|e| format!("{}: {e}", path.display()))
}

/// Inverted index for BM25, rebuilt when a standard is loaded.
#[derive(Debug, Clone, Default)]
pub struct Bm25 {
    pub postings: HashMap<String, Vec<(u32, u32)>>,
    pub doc_len: Vec<u32>,
    pub total_len: u64,
}

impl Bm25 {
    pub fn build(chunks: &[Chunk]) -> Bm25 {
        let mut idx = Bm25::default();
        for (i, c) in chunks.iter().enumerate() {
            let ts = terms(&c.index_text());
            idx.doc_len.push(ts.len() as u32);
            idx.total_len += ts.len() as u64;
            let mut tf: HashMap<String, u32> = HashMap::new();
            for t in ts {
                *tf.entry(t).or_insert(0) += 1;
            }
            for (t, n) in tf {
                idx.postings.entry(t).or_default().push((i as u32, n));
            }
        }
        idx
    }
}

/// Seconds since the Unix epoch as an RFC 3339 UTC timestamp.
pub fn now_rfc3339() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // civil-from-days (H. Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// Directory id of a standard code: "ACI 318-19" -> "aci-318-19". Codes with
/// non-ASCII characters (e.g. Thai) get a short hash suffix so ids stay
/// unique and filesystem-safe.
pub fn slug(code: &str) -> String {
    let mut s = String::new();
    let mut dash = false;
    for c in code.chars() {
        if c.is_ascii_alphanumeric() {
            s.push(c.to_ascii_lowercase());
            dash = false;
        } else if !dash && !s.is_empty() {
            s.push('-');
            dash = true;
        }
    }
    let mut s = s.trim_end_matches('-').to_string();
    if !code.is_ascii() || s.is_empty() {
        let h = crate::text::fnv1a(0, code.trim().as_bytes());
        if !s.is_empty() {
            s.push('-');
        }
        s.push_str(&format!("{:08x}", h as u32));
    }
    s
}

pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 120 && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn slugs() {
        assert_eq!(slug("ACI 318-19"), "aci-318-19");
        assert_eq!(slug("EN 1997-1:2004"), "en-1997-1-2004");
        let t = slug("มยผ. 1301-61");
        assert!(t.starts_with("1301-61-") && valid_id(&t), "{t}");
        assert!(now_rfc3339().ends_with('Z'));
    }
}
