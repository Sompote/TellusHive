//! Embedders.
//!
//! * `Hashed`: a built-in, offline, deterministic **lexical** vector. It uses
//!   feature hashing of word unigrams, word bigrams and character 3–5-grams,
//!   weighted by sublinear TF × IDF (IDF from the standard's own chunks) with
//!   signed hashing and L2 normalisation. It captures shared words and word
//!   fragments, so it tolerates inflection and some spelling variation, but it
//!   has no notion of meaning: "uplift" and "buoyancy" are unrelated to it.
//! * `Api`: any OpenAI-compatible `/embeddings` endpoint (Ollama, OpenAI,
//!   OpenRouter, LM Studio). This gives real semantic vectors.

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::text::{fnv1a, squash, terms};

pub const API_BATCH: usize = 64;

/// Unigram, bigram and char-n-gram group weights.
const W_UNI: f32 = 1.0;
const W_BI: f32 = 0.8;
const W_CHAR: f32 = 0.3;

/// Document frequencies of hashed features for one standard.
#[derive(Debug, Clone, Default)]
pub struct HashedIdf {
    pub n_docs: u32,
    pub df: HashMap<u64, u32>,
}

impl HashedIdf {
    pub fn idf(&self, key: u64) -> f32 {
        // features absent from the table had df <= 1 (pruned at save time)
        let df = self.df.get(&key).copied().unwrap_or(1) as f32;
        ((self.n_docs as f32 + 1.0) / (df + 1.0)).ln() + 1.0
    }

    pub fn build<'a>(texts: impl Iterator<Item = &'a str>) -> HashedIdf {
        let mut df: HashMap<u64, u32> = HashMap::new();
        let mut n = 0u32;
        for t in texts {
            n += 1;
            for (k, _) in features(t) {
                *df.entry(k).or_insert(0) += 1;
            }
        }
        HashedIdf { n_docs: n, df }
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let mut buf: Vec<u8> = Vec::new();
        buf.extend_from_slice(b"ADKBIDF1");
        buf.extend_from_slice(&self.n_docs.to_le_bytes());
        let kept: Vec<(&u64, &u32)> = self.df.iter().filter(|(_, d)| **d > 1).collect();
        buf.extend_from_slice(&(kept.len() as u64).to_le_bytes());
        for (k, d) in kept {
            buf.extend_from_slice(&k.to_le_bytes());
            buf.extend_from_slice(&d.to_le_bytes());
        }
        crate::store::atomic_write(path, &buf)
    }

    pub fn load(path: &Path) -> Result<HashedIdf, String> {
        let mut f = std::fs::File::open(path).map_err(|e| format!("cannot open {}: {e}", path.display()))?;
        let mut b = Vec::new();
        f.read_to_end(&mut b).map_err(|e| e.to_string())?;
        if b.len() < 20 || &b[..8] != b"ADKBIDF1" {
            return Err(format!("{} is not an IDF table", path.display()));
        }
        let n_docs = u32::from_le_bytes(b[8..12].try_into().unwrap());
        let count = u64::from_le_bytes(b[12..20].try_into().unwrap()) as usize;
        if b.len() != 20 + count * 12 {
            return Err(format!("{} is truncated", path.display()));
        }
        let mut df = HashMap::with_capacity(count);
        for i in 0..count {
            let o = 20 + i * 12;
            df.insert(
                u64::from_le_bytes(b[o..o + 8].try_into().unwrap()),
                u32::from_le_bytes(b[o + 8..o + 12].try_into().unwrap()),
            );
        }
        Ok(HashedIdf { n_docs, df })
    }
}

/// Hashed features of a text: key -> (group, raw count). Keys carry the
/// feature group in the hash seed so groups never collide by construction.
fn features(text: &str) -> HashMap<u64, (u8, f32)> {
    let mut f: HashMap<u64, (u8, f32)> = HashMap::new();
    let mut add = |k: u64, g: u8| f.entry(k).or_insert((g, 0.0)).1 += 1.0;
    let ws = terms(text);
    for w in &ws {
        add(fnv1a(1, w.as_bytes()), 1);
    }
    for p in ws.windows(2) {
        add(fnv1a(2, format!("{} {}", p[0], p[1]).as_bytes()), 2);
    }
    for word in squash(text).split(' ') {
        let padded: Vec<char> = format!(" {word} ").chars().collect();
        for n in 3..=5usize {
            if padded.len() < n {
                continue;
            }
            for g in padded.windows(n) {
                let s: String = g.iter().collect();
                add(fnv1a(3 + n as u8, s.as_bytes()), 3);
            }
        }
    }
    f
}

/// The hashed vector of `text` under `idf`, L2-normalised.
pub fn hashed_embed(text: &str, idf: &HashedIdf, dim: usize) -> Vec<f32> {
    let mut v = vec![0f32; dim];
    for (k, (g, tf)) in features(text) {
        let gw = match g {
            1 => W_UNI,
            2 => W_BI,
            _ => W_CHAR,
        };
        let w = gw * (1.0 + tf.ln()) * idf.idf(k);
        let bucket = (k % dim as u64) as usize;
        let sign = if (k >> 63) & 1 == 1 { -1.0 } else { 1.0 };
        v[bucket] += sign * w;
    }
    normalize(&mut v);
    v
}

pub fn normalize(v: &mut [f32]) {
    let n: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n > 0.0 {
        for x in v.iter_mut() {
            *x /= n;
        }
    }
}

// ---------------------------------------------------------------- API

pub fn request_body(model: &str, inputs: &[String]) -> Value {
    json!({ "model": model, "input": inputs })
}

/// Parse an OpenAI-style embeddings response: `{"data":[{"index":i,"embedding":[...]}, ...]}`.
/// Items are reordered by `index` when present.
pub fn parse_response(v: &Value, expected: usize) -> Result<Vec<Vec<f32>>, String> {
    if let Some(err) = v.get("error") {
        let msg = err.get("message").and_then(|m| m.as_str()).map(|s| s.to_string()).unwrap_or_else(|| err.to_string());
        return Err(format!("embedding API error: {msg}"));
    }
    let data = v
        .get("data")
        .and_then(|d| d.as_array())
        .ok_or_else(|| format!("embedding response has no 'data' array: {}", truncate(&v.to_string(), 300)))?;
    let mut out: Vec<(usize, Vec<f32>)> = Vec::with_capacity(data.len());
    for (i, item) in data.iter().enumerate() {
        let idx = item.get("index").and_then(|x| x.as_u64()).map(|x| x as usize).unwrap_or(i);
        let emb = item
            .get("embedding")
            .and_then(|e| e.as_array())
            .ok_or_else(|| "embedding item has no 'embedding' array (base64 encoding is not supported)".to_string())?;
        let vec: Vec<f32> = emb.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect();
        out.push((idx, vec));
    }
    out.sort_by_key(|(i, _)| *i);
    if out.len() != expected {
        return Err(format!("embedding API returned {} vectors for {} inputs", out.len(), expected));
    }
    let dim = out.first().map(|(_, v)| v.len()).unwrap_or(0);
    if dim == 0 || out.iter().any(|(_, v)| v.len() != dim) {
        return Err("embedding API returned empty or ragged vectors".into());
    }
    Ok(out.into_iter().map(|(_, mut v)| {
        normalize(&mut v);
        v
    }).collect())
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        let mut k = n;
        while !s.is_char_boundary(k) {
            k -= 1;
        }
        format!("{}…", &s[..k])
    }
}

pub fn embeddings_url(base_url: &str) -> String {
    let b = base_url.trim_end_matches('/');
    if b.ends_with("/embeddings") {
        b.to_string()
    } else {
        format!("{b}/embeddings")
    }
}

/// The OpenRouter key, set by the app from its OpenRouter provider so that it is never written
/// into a standard's files. It is sent only to openrouter.ai.
static OPENROUTER_KEY: std::sync::RwLock<Option<String>> = std::sync::RwLock::new(None);

pub fn set_openrouter_key(key: Option<String>) {
    if let Ok(mut k) = OPENROUTER_KEY.write() {
        *k = key.filter(|s| !s.trim().is_empty());
    }
}

/// Embed `inputs` through an OpenAI-compatible endpoint, `API_BATCH` per call.
pub fn api_embed(
    base_url: &str,
    api_key: Option<&str>,
    model: &str,
    inputs: &[String],
    progress: &mut dyn FnMut(usize, usize),
) -> Result<Vec<Vec<f32>>, String> {
    let url = embeddings_url(base_url);
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(300))
        .build();
    let key = api_key
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| if url.contains("openrouter.ai") { OPENROUTER_KEY.read().ok().and_then(|k| k.clone()) } else { None })
        .or_else(|| std::env::var("TH_KB_EMBED_API_KEY").ok().filter(|s| !s.is_empty()));
    if key.is_none() && url.contains("openrouter.ai") {
        return Err("no OpenRouter key: add it under Settings > Providers > OpenRouter".into());
    }
    let mut out = Vec::with_capacity(inputs.len());
    for batch in inputs.chunks(API_BATCH) {
        // very long inputs overflow small embedding contexts (e.g. 2k tokens)
        let batch: Vec<String> = batch.iter().map(|s| truncate(s, 6000)).collect();
        // a rate limit, a server error or a dropped connection is tried again after 1, 2 and 4 s
        let mut attempt = 0;
        let v: Value = loop {
            let mut req = agent.post(&url).set("Content-Type", "application/json");
            if let Some(k) = &key {
                req = req.set("Authorization", &format!("Bearer {k}"));
            }
            let err = match req.send_json(request_body(model, &batch)) {
                Ok(r) => break r.into_json().map_err(|e| format!("embedding response is not JSON: {e}"))?,
                Err(ureq::Error::Status(code, r)) => {
                    let body = r.into_string().unwrap_or_default();
                    let msg = format!("embedding API {url} returned HTTP {code}: {}", truncate(&body, 400));
                    if !(code == 408 || code == 429 || code >= 500) {
                        return Err(msg);
                    }
                    msg
                }
                Err(e) => format!("cannot reach embedding API {url}: {e}"),
            };
            attempt += 1;
            if attempt > 3 {
                return Err(err);
            }
            std::thread::sleep(std::time::Duration::from_secs(1 << (attempt - 1)));
        };
        out.extend(parse_response(&v, batch.len())?);
        progress(out.len(), inputs.len());
    }
    Ok(out)
}

/// Stored description of how a standard's vectors were made.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredEmbedder {
    pub config: crate::EmbedderConfig,
    pub dim: usize,
}

impl StoredEmbedder {
    pub fn signature(&self) -> String {
        signature(&self.config)
    }
}

pub fn signature(c: &crate::EmbedderConfig) -> String {
    match c {
        crate::EmbedderConfig::Hashed { dim } => format!("hashed-{dim}"),
        crate::EmbedderConfig::Api { base_url, model, .. } => {
            format!("api:{model}@{}", base_url.trim_end_matches('/'))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_shape() {
        let b = request_body("nomic-embed-text", &["a".into(), "b c".into()]);
        assert_eq!(b, json!({"model":"nomic-embed-text","input":["a","b c"]}));
        assert_eq!(embeddings_url("http://localhost:11434/v1/"), "http://localhost:11434/v1/embeddings");
        assert_eq!(embeddings_url("https://api.openai.com/v1/embeddings"), "https://api.openai.com/v1/embeddings");
    }

    #[test]
    fn response_parsing() {
        // out of order, as some servers return
        let v = json!({"object":"list","data":[
            {"object":"embedding","index":1,"embedding":[0.0, 2.0]},
            {"object":"embedding","index":0,"embedding":[3.0, 4.0]}],
            "model":"m","usage":{"prompt_tokens":3,"total_tokens":3}});
        let e = parse_response(&v, 2).unwrap();
        assert_eq!(e[0], vec![0.6, 0.8]);
        assert_eq!(e[1], vec![0.0, 1.0]);
        assert!(parse_response(&v, 3).is_err());
        let err = json!({"error":{"message":"model not found","type":"invalid_request_error"}});
        assert!(parse_response(&err, 1).unwrap_err().contains("model not found"));
        let ragged = json!({"data":[{"embedding":[1.0]},{"embedding":[1.0,2.0]}]});
        assert!(parse_response(&ragged, 2).is_err());
    }

    #[test]
    fn hashed_is_deterministic_and_normalised() {
        let texts = ["minimum shear reinforcement area", "bearing resistance drained", "shear strength of concrete"];
        let idf = HashedIdf::build(texts.iter().copied());
        let a = hashed_embed(texts[0], &idf, 256);
        let b = hashed_embed(texts[0], &idf, 256);
        assert_eq!(a, b);
        let n: f32 = a.iter().map(|x| x * x).sum();
        assert!((n - 1.0).abs() < 1e-4);
        let c = hashed_embed("shear reinforcement", &idf, 256);
        let d = hashed_embed("bearing", &idf, 256);
        let dot = |x: &[f32], y: &[f32]| x.iter().zip(y).map(|(p, q)| p * q).sum::<f32>();
        assert!(dot(&a, &c) > dot(&a, &d));
    }

    #[test]
    fn idf_roundtrip() {
        let idf = HashedIdf::build(["a b c", "a b", "a"].iter().copied());
        let dir = std::env::temp_dir().join(format!("adkb-idf-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("idf.bin");
        idf.save(&p).unwrap();
        let back = HashedIdf::load(&p).unwrap();
        assert_eq!(back.n_docs, 3);
        let k = fnv1a(1, b"a");
        assert_eq!(back.idf(k), idf.idf(k));
        std::fs::remove_dir_all(&dir).ok();
    }
}
