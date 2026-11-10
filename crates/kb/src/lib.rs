//! # th-kb: the standards library
//!
//! A local, all-Rust store of design standards (ACI 318, EN 1997-1, AISC 360,
//! Thai DPT standards, office rules) that agents search for the clauses of
//! the standards the user selected.
//!
//! * Ingest PDF / TXT / MD files. Text is split into clause-sized chunks by a
//!   heading detector that understands clause numbering (`22.5.5.1`,
//!   `6.5.2.2(1)P`, `D.4`, `Table 9.3.1.1`, `Section 4.3`, `ข้อ 3.2`).
//! * Each chunk is embedded either by the built-in hashed lexical embedder
//!   (offline, deterministic, **not semantic**) or by an OpenAI-compatible
//!   embeddings API (Ollama `nomic-embed-text`, OpenAI, OpenRouter).
//! * Search fuses exact cosine similarity and BM25 by reciprocal-rank fusion,
//!   and boosts chunks whose clause number the query names.
//!
//! Storage per standard, under `<root>/<id>/`: `meta.json`, `embedder.json`,
//! `chunks.jsonl`, `vectors.f32` (little-endian `n × dim`) and, for the
//! hashed embedder, `idf.bin`. BM25 is rebuilt when a standard is loaded.

mod chunk;
mod embed;
mod extract;
mod store;
mod text;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

use serde::{Deserialize, Serialize};

use embed::{HashedIdf, StoredEmbedder};
use store::{Bm25, Chunk};

pub use chunk::{clause_key, detect_heading};

/// A standard in the library.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StandardMeta {
    pub id: String,
    pub name: String,
    pub code: String,
    pub discipline: String,
    pub country: String,
    pub year: Option<u32>,
    pub source_files: Vec<String>,
    pub chunks: usize,
    pub embedder: String,
    pub created: String,
}

/// Description of a standard to ingest.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct NewStandard {
    pub name: String,
    pub code: String,
    pub discipline: String,
    pub country: String,
    pub year: Option<u32>,
}

/// One search result. `score` is the fused score: the reciprocal-rank-fusion
/// value (at most about 0.033 per ranking) plus a clause-number boost (1.0 for
/// an exact match of a clause the query names).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hit {
    pub standard_id: String,
    pub standard_code: String,
    pub clause: String,
    pub title: String,
    pub page: Option<u32>,
    pub text: String,
    pub score: f32,
}

/// How a standard's chunks are embedded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EmbedderConfig {
    /// Built-in feature-hashing lexical vectors (offline).
    Hashed { dim: usize },
    /// OpenAI-compatible `POST {base_url}/embeddings`.
    Api { base_url: String, api_key: Option<String>, model: String },
}

impl Default for EmbedderConfig {
    fn default() -> Self {
        EmbedderConfig::Hashed { dim: 1024 }
    }
}

/// Which signals a search uses. `Hybrid` is what [`Library::search`] does;
/// the others exist for evaluation.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SearchMode {
    Hybrid,
    VectorOnly,
    Bm25Only,
    NoBoost,
}

struct Loaded {
    meta: StandardMeta,
    emb: StoredEmbedder,
    chunks: Vec<Chunk>,
    vectors: Vec<f32>,
    bm25: Bm25,
    idf: Option<HashedIdf>,
}

/// The standards library. Cheap to share: wrap in `Arc` and use from any
/// thread. Reads use a cache of loaded standards; writes are serialised.
pub struct Library {
    root: PathBuf,
    cache: RwLock<HashMap<String, Arc<Loaded>>>,
    write: Mutex<()>,
}

const RRF_K: f32 = 60.0;
const LIST_DEPTH: usize = 200;

impl Library {
    /// Open (and create if needed) a library rooted at `root`.
    pub fn open(root: &Path) -> Result<Library, String> {
        std::fs::create_dir_all(root).map_err(|e| format!("cannot create library folder {}: {e}", root.display()))?;
        Ok(Library { root: root.to_path_buf(), cache: RwLock::new(HashMap::new()), write: Mutex::new(()) })
    }

    /// `<data dir>/TellusHive/standards` (on macOS
    /// `~/Library/Application Support/TellusHive/standards`).
    pub fn default_root() -> PathBuf {
        dirs::data_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("TellusHive")
            .join("standards")
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// All standards, sorted by code.
    pub fn list(&self) -> Vec<StandardMeta> {
        let mut out = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&self.root) {
            for e in rd.flatten() {
                let p = e.path().join("meta.json");
                if let Ok(m) = store::read_json::<StandardMeta>(&p) {
                    out.push(m);
                }
            }
        }
        out.sort_by(|a, b| a.code.to_lowercase().cmp(&b.code.to_lowercase()));
        out
    }

    fn dir(&self, id: &str) -> PathBuf {
        self.root.join(id)
    }

    fn load(&self, id: &str) -> Result<Arc<Loaded>, String> {
        if !store::valid_id(id) {
            return Err(format!("invalid standard id '{id}'"));
        }
        if let Some(l) = self.cache.read().map_err(|_| "library cache poisoned")?.get(id) {
            return Ok(l.clone());
        }
        let d = self.dir(id);
        if !d.join("meta.json").exists() {
            return Err(format!("no standard with id '{id}' in {}", self.root.display()));
        }
        let meta: StandardMeta = store::read_json(&d.join("meta.json"))?;
        let emb: StoredEmbedder = store::read_json(&d.join("embedder.json"))?;
        let chunks = store::read_chunks(&d.join("chunks.jsonl"))?;
        let vectors = store::read_vectors(&d.join("vectors.f32"))?;
        if vectors.len() != chunks.len() * emb.dim {
            return Err(format!(
                "standard '{id}' is inconsistent ({} chunks, {} floats, dim {}); delete and ingest it again",
                chunks.len(),
                vectors.len(),
                emb.dim
            ));
        }
        let idf = match emb.config {
            EmbedderConfig::Hashed { .. } => Some(HashedIdf::load(&d.join("idf.bin"))?),
            _ => None,
        };
        let bm25 = Bm25::build(&chunks);
        let l = Arc::new(Loaded { meta, emb, chunks, vectors, bm25, idf });
        self.cache.write().map_err(|_| "library cache poisoned")?.insert(id.to_string(), l.clone());
        Ok(l)
    }

    fn invalidate(&self, id: &str) {
        if let Ok(mut c) = self.cache.write() {
            c.remove(id);
        }
    }

    /// Ingest files (PDF, TXT, MD) into the standard whose code is
    /// `meta.code`; it is created if it does not exist. Re-ingesting a file
    /// with the same file name replaces that file's chunks. If `embedder`
    /// differs from the one the standard was built with, every chunk is
    /// re-embedded so the standard stays consistent.
    pub fn ingest(
        &self,
        files: &[PathBuf],
        meta: &NewStandard,
        embedder: &EmbedderConfig,
        progress: &mut dyn FnMut(f32, &str),
    ) -> Result<StandardMeta, String> {
        if meta.code.trim().is_empty() {
            return Err("the standard needs a code, e.g. \"ACI 318-19\"".into());
        }
        if files.is_empty() {
            return Err("no files to ingest".into());
        }
        if let EmbedderConfig::Hashed { dim } = embedder {
            if *dim < 16 || *dim > 65_536 {
                return Err(format!("hashed embedder dim {dim} is out of range (16..65536)"));
            }
        }
        let id = store::slug(&meta.code);
        // 1. extract and chunk (no lock needed)
        let mut new_chunks: Vec<Chunk> = Vec::new();
        let mut names = Vec::new();
        for (i, f) in files.iter().enumerate() {
            let name = f.file_name().and_then(|n| n.to_str()).unwrap_or("file").to_string();
            progress(0.3 * i as f32 / files.len() as f32, &format!("reading {name}"));
            let lines = extract::extract_file(f)?;
            let raw = chunk::chunk_lines(&lines);
            if raw.is_empty() {
                return Err(format!("{name}: no text found to index"));
            }
            for r in raw {
                new_chunks.push(Chunk {
                    id: 0,
                    clause: r.clause,
                    title: r.title,
                    page: r.page,
                    text: r.text,
                    context: r.context,
                    part: r.part,
                    kind: "clause".into(),
                    source: name.clone(),
                });
            }
            names.push(name);
        }
        progress(0.3, &format!("{} chunks from {} file(s)", new_chunks.len(), files.len()));

        let _w = self.write.lock().map_err(|_| "library write lock poisoned")?;
        let d = self.dir(&id);
        let existing = if d.join("meta.json").exists() { Some(self.load(&id)?) } else { None };
        // 2. merge with existing chunks; keep old vectors where reusable
        let reuse = existing.as_ref().map(|e| matches!(e.emb.config, EmbedderConfig::Api { .. }) && &e.emb.config == embedder).unwrap_or(false);
        let mut items: Vec<(Chunk, Option<Vec<f32>>)> = Vec::new();
        let mut sources: Vec<String> = Vec::new();
        let mut created = store::now_rfc3339();
        if let Some(e) = &existing {
            created = e.meta.created.clone();
            for s in &e.meta.source_files {
                if !names.contains(s) {
                    sources.push(s.clone());
                }
            }
            for (i, c) in e.chunks.iter().enumerate() {
                if c.kind == "clause" && names.contains(&c.source) {
                    continue;
                }
                let v = if reuse { Some(e.vectors[i * e.emb.dim..(i + 1) * e.emb.dim].to_vec()) } else { None };
                items.push((c.clone(), v));
            }
        }
        // notes stay last so that document order is preserved for clauses
        let (mut notes, mut docs): (Vec<_>, Vec<_>) = items.into_iter().partition(|(c, _)| c.kind == "note");
        docs.extend(new_chunks.into_iter().map(|c| (c, None)));
        docs.append(&mut notes);
        sources.extend(names);
        let m = StandardMeta {
            id: id.clone(),
            name: meta.name.clone(),
            code: meta.code.clone(),
            discipline: meta.discipline.clone(),
            country: meta.country.clone(),
            year: meta.year,
            source_files: sources,
            chunks: docs.len(),
            embedder: embed::signature(embedder),
            created,
        };
        self.write_standard(&m, embedder, docs, progress)
    }

    /// Embed what is missing and write everything for one standard.
    fn write_standard(
        &self,
        meta: &StandardMeta,
        embedder: &EmbedderConfig,
        mut items: Vec<(Chunk, Option<Vec<f32>>)>,
        progress: &mut dyn FnMut(f32, &str),
    ) -> Result<StandardMeta, String> {
        for (i, (c, _)) in items.iter_mut().enumerate() {
            c.id = i as u32;
        }
        let texts: Vec<String> = items.iter().map(|(c, _)| c.index_text()).collect();
        let (vectors, dim, idf) = match embedder {
            EmbedderConfig::Hashed { dim } => {
                progress(0.4, "computing IDF and hashed vectors");
                let idf = HashedIdf::build(texts.iter().map(|s| s.as_str()));
                let mut v = Vec::with_capacity(texts.len() * dim);
                for (i, t) in texts.iter().enumerate() {
                    v.extend(embed::hashed_embed(t, &idf, *dim));
                    if i % 500 == 0 {
                        progress(0.4 + 0.5 * i as f32 / texts.len() as f32, "embedding");
                    }
                }
                (v, *dim, Some(idf))
            }
            EmbedderConfig::Api { base_url, api_key, model } => {
                let missing: Vec<usize> = (0..items.len()).filter(|i| items[*i].1.is_none()).collect();
                let inputs: Vec<String> = missing.iter().map(|i| texts[*i].clone()).collect();
                let total = inputs.len();
                let got = if inputs.is_empty() {
                    Vec::new()
                } else {
                    embed::api_embed(base_url, api_key.as_deref(), model, &inputs, &mut |done, n| {
                        progress(0.35 + 0.55 * done as f32 / n.max(1) as f32, &format!("embedding {done}/{total} via {model}"))
                    })?
                };
                for (j, i) in missing.iter().enumerate() {
                    items[*i].1 = Some(got[j].clone());
                }
                let dim = items.iter().find_map(|(_, v)| v.as_ref().map(|v| v.len())).unwrap_or(0);
                if dim == 0 || items.iter().any(|(_, v)| v.as_ref().map(|v| v.len()) != Some(dim)) {
                    return Err("embedding dimension changed between calls; re-ingest with one model".into());
                }
                let mut v = Vec::with_capacity(items.len() * dim);
                for (_, x) in &items {
                    v.extend_from_slice(x.as_ref().unwrap());
                }
                (v, dim, None)
            }
        };
        progress(0.92, "writing");
        let d = self.dir(&meta.id);
        std::fs::create_dir_all(&d).map_err(|e| format!("cannot create {}: {e}", d.display()))?;
        let chunks: Vec<Chunk> = items.into_iter().map(|(c, _)| c).collect();
        store::write_chunks(&d.join("chunks.jsonl"), &chunks)?;
        store::write_vectors(&d.join("vectors.f32"), &vectors)?;
        match &idf {
            Some(idf) => idf.save(&d.join("idf.bin"))?,
            None => {
                let _ = std::fs::remove_file(d.join("idf.bin"));
            }
        }
        store::write_json(&d.join("embedder.json"), &StoredEmbedder { config: embedder.clone(), dim })?;
        let mut meta = meta.clone();
        meta.chunks = chunks.len();
        store::write_json(&d.join("meta.json"), &meta)?;
        self.invalidate(&meta.id);
        progress(1.0, &format!("{} ready: {} chunks", meta.code, meta.chunks));
        Ok(meta)
    }

    /// Add a user-written rule or office practice to a standard. It is
    /// searchable like a clause; its title is prefixed "[Office note]" so
    /// agents can tell it from the standard's own text.
    pub fn add_note(&self, standard_id: &str, clause: &str, title: &str, text: &str) -> Result<(), String> {
        if text.trim().is_empty() {
            return Err("the note is empty".into());
        }
        let _w = self.write.lock().map_err(|_| "library write lock poisoned")?;
        let e = self.load(standard_id)?;
        let reuse = matches!(e.emb.config, EmbedderConfig::Api { .. });
        let mut items: Vec<(Chunk, Option<Vec<f32>>)> = e
            .chunks
            .iter()
            .enumerate()
            .map(|(i, c)| (c.clone(), if reuse { Some(e.vectors[i * e.emb.dim..(i + 1) * e.emb.dim].to_vec()) } else { None }))
            .collect();
        let clause = clause.trim();
        let mut ctx = String::new();
        if !clause.is_empty() {
            // inherit the breadcrumb of the clause the note refers to
            let (_, bare) = clause_key(clause);
            if let Some(c) = e.chunks.iter().find(|c| clause_key(&c.clause).1 == bare && c.kind == "clause") {
                ctx = if c.context.is_empty() { c.title.clone() } else { format!("{} > {}", c.context, c.title) };
            }
        }
        items.push((
            Chunk {
                id: 0,
                clause: clause.to_string(),
                title: format!("[Office note] {}", title.trim()),
                page: None,
                text: text.trim().to_string(),
                context: ctx,
                part: 0,
                kind: "note".into(),
                source: "note".into(),
            },
            None,
        ));
        let meta = e.meta.clone();
        let cfg = e.emb.config.clone();
        drop(e);
        self.write_standard(&meta, &cfg, items, &mut |_, _| {})?;
        Ok(())
    }

    /// Delete a standard and its files.
    pub fn delete(&self, standard_id: &str) -> Result<(), String> {
        if !store::valid_id(standard_id) {
            return Err(format!("invalid standard id '{standard_id}'"));
        }
        let _w = self.write.lock().map_err(|_| "library write lock poisoned")?;
        let d = self.dir(standard_id);
        if !d.exists() {
            return Err(format!("no standard with id '{standard_id}'"));
        }
        self.invalidate(standard_id);
        std::fs::remove_dir_all(&d).map_err(|e| format!("cannot delete {}: {e}", d.display()))
    }

    /// Hybrid search over the given standards (empty = all).
    pub fn search(&self, query: &str, standard_ids: &[String], k: usize) -> Result<Vec<Hit>, String> {
        self.search_mode(query, standard_ids, k, SearchMode::Hybrid)
    }

    #[doc(hidden)]
    pub fn search_mode(&self, query: &str, standard_ids: &[String], k: usize, mode: SearchMode) -> Result<Vec<Hit>, String> {
        let query = query.trim();
        if query.is_empty() || k == 0 {
            return Ok(Vec::new());
        }
        let ids: Vec<String> = if standard_ids.is_empty() {
            self.list().into_iter().map(|m| m.id).collect()
        } else {
            standard_ids.to_vec()
        };
        let mut stds: Vec<Arc<Loaded>> = Vec::new();
        for id in &ids {
            stds.push(self.load(id)?);
        }
        if stds.is_empty() {
            return Ok(Vec::new());
        }
        // fused score per (standard index, chunk index)
        let mut fused: HashMap<(usize, usize), f32> = HashMap::new();
        let add_list = |list: Vec<(usize, usize, f32)>, fused: &mut HashMap<(usize, usize), f32>| {
            for (rank, (s, c, _)) in list.into_iter().enumerate() {
                *fused.entry((s, c)).or_insert(0.0) += 1.0 / (RRF_K + rank as f32 + 1.0);
            }
        };

        // --- BM25 with statistics over all selected standards
        if mode != SearchMode::VectorOnly {
            let qterms = {
                let mut t = text::terms(query);
                t.sort();
                t.dedup();
                t
            };
            let n_docs: usize = stds.iter().map(|s| s.chunks.len()).sum();
            let total: u64 = stds.iter().map(|s| s.bm25.total_len).sum();
            let avgdl = (total as f32 / n_docs.max(1) as f32).max(1.0);
            let (k1, b) = (1.2f32, 0.75f32);
            let mut scores: HashMap<(usize, usize), f32> = HashMap::new();
            for t in &qterms {
                let df: usize = stds.iter().map(|s| s.bm25.postings.get(t).map(|p| p.len()).unwrap_or(0)).sum();
                if df == 0 {
                    continue;
                }
                let idf = (1.0 + (n_docs as f32 - df as f32 + 0.5) / (df as f32 + 0.5)).ln();
                for (si, s) in stds.iter().enumerate() {
                    if let Some(p) = s.bm25.postings.get(t) {
                        for (doc, tf) in p {
                            let dl = s.bm25.doc_len[*doc as usize] as f32;
                            let tf = *tf as f32;
                            let sc = idf * tf * (k1 + 1.0) / (tf + k1 * (1.0 - b + b * dl / avgdl));
                            *scores.entry((si, *doc as usize)).or_insert(0.0) += sc;
                        }
                    }
                }
            }
            let mut list: Vec<(usize, usize, f32)> = scores.into_iter().map(|((s, c), v)| (s, c, v)).collect();
            sort_desc(&mut list);
            list.truncate(LIST_DEPTH);
            add_list(list, &mut fused);
        }

        // --- vectors, one ranking per embedder kind
        if mode != SearchMode::Bm25Only {
            let mut groups: HashMap<String, Vec<usize>> = HashMap::new();
            for (si, s) in stds.iter().enumerate() {
                groups.entry(s.emb.signature()).or_default().push(si);
            }
            let mut gkeys: Vec<&String> = groups.keys().collect();
            gkeys.sort();
            for g in gkeys {
                let members = &groups[g];
                let mut list: Vec<(usize, usize, f32)> = Vec::new();
                let mut api_q: Option<Vec<f32>> = None;
                for &si in members {
                    let s = &stds[si];
                    let q = match &s.emb.config {
                        EmbedderConfig::Hashed { dim } => {
                            embed::hashed_embed(query, s.idf.as_ref().expect("hashed standard without idf"), *dim)
                        }
                        EmbedderConfig::Api { base_url, api_key, model } => {
                            if api_q.is_none() {
                                match embed::api_embed(base_url, api_key.as_deref(), model, &[query.to_string()], &mut |_, _| {}) {
                                    Ok(mut v) => api_q = v.pop(),
                                    Err(e) => {
                                        // keep searching with BM25 + clause boost rather than failing
                                        eprintln!("th-kb: query embedding failed for {}: {e}; using keyword search only", s.meta.code);
                                        break;
                                    }
                                }
                            }
                            api_q.clone().unwrap()
                        }
                    };
                    if q.len() != s.emb.dim {
                        continue;
                    }
                    let dim = s.emb.dim;
                    for (ci, v) in s.vectors.chunks_exact(dim).enumerate() {
                        let dot: f32 = v.iter().zip(&q).map(|(a, b)| a * b).sum();
                        list.push((si, ci, dot));
                    }
                }
                sort_desc(&mut list);
                list.truncate(LIST_DEPTH);
                add_list(list, &mut fused);
            }
        }

        // --- clause-number boost
        if mode == SearchMode::Hybrid || mode == SearchMode::VectorOnly || mode == SearchMode::Bm25Only {
            let refs = query_clause_refs(query);
            if !refs.is_empty() {
                let qlow = query.to_lowercase();
                for (si, s) in stds.iter().enumerate() {
                    let code_named = s
                        .meta
                        .code
                        .split(|c: char| !c.is_alphanumeric())
                        .filter(|w| w.len() >= 2)
                        .any(|w| qlow.split(|c: char| !c.is_alphanumeric()).any(|q| q == w.to_lowercase()));
                    for (ci, c) in s.chunks.iter().enumerate() {
                        if c.clause.is_empty() {
                            continue;
                        }
                        let (ck, cb) = clause_key(&c.clause);
                        let mut best = 0.0f32;
                        for (qk, qb) in &refs {
                            let b = if cb == *qb {
                                if ck == *qk { 1.0 } else { 0.3 }
                            } else if chunk::is_descendant(&cb, qb) {
                                let extra = cb[qb.len()..].matches(|ch| ch == '.' || ch == '(').count().max(1);
                                0.4 / extra as f32
                            } else if chunk::is_descendant(qb, &cb) && ck == *qk {
                                // query names a paragraph (6.5.2.2(1)P) of this clause
                                0.6
                            } else {
                                0.0
                            };
                            best = best.max(b);
                        }
                        if best > 0.0 {
                            let mut b = best - 0.002 * c.part as f32 - if c.kind == "note" { 0.01 } else { 0.0 };
                            if code_named {
                                b += 0.05;
                            }
                            *fused.entry((si, ci)).or_insert(0.0) += b;
                        }
                    }
                }
            }
        }

        let mut all: Vec<(usize, usize, f32)> = fused.into_iter().map(|((s, c), v)| (s, c, v)).collect();
        sort_desc(&mut all);
        all.truncate(k);
        Ok(all.into_iter().map(|(si, ci, sc)| hit(&stds[si], &stds[si].chunks[ci], sc)).collect())
    }

    /// Exact clause lookup plus its sub-clauses (and tables/figures numbered
    /// under it), in clause order. Accepts "22.5", "Section 4.3", "Table
    /// 9.3.1.1", "Annex D", "ข้อ 3.2". Unknown standard -> empty.
    pub fn clause(&self, standard_id: &str, clause: &str) -> Vec<Hit> {
        let Ok(s) = self.load(standard_id) else { return Vec::new() };
        let (qk, qb) = clause_key(clause);
        if qb.is_empty() {
            return Vec::new();
        }
        let mut found: Vec<(&Chunk, u8)> = Vec::new();
        for c in &s.chunks {
            let (ck, cb) = clause_key(&c.clause);
            let annex_child = qk == "annex" && cb.starts_with(&format!("{qb}."));
            if cb == qb && (qk.is_empty() || ck == qk) || chunk::is_descendant(&cb, &qb) || annex_child {
                let kind_rank = match ck.as_str() {
                    "" | "annex" => 0,
                    "table" => 1,
                    _ => 2,
                };
                found.push((c, kind_rank));
            }
        }
        found.sort_by(|a, b| {
            let ka = chunk::natural_key(&clause_key(&a.0.clause).1);
            let kb = chunk::natural_key(&clause_key(&b.0.clause).1);
            ka.cmp(&kb)
                .then(a.1.cmp(&b.1))
                .then((a.0.kind == "note").cmp(&(b.0.kind == "note")))
                .then(a.0.id.cmp(&b.0.id))
        });
        found.into_iter().map(|(c, _)| hit(&s, c, 1.0)).collect()
    }

    /// All chunks of a standard in document order (notes last).
    pub fn chunks(&self, standard_id: &str) -> Vec<Hit> {
        match self.load(standard_id) {
            Ok(s) => s.chunks.iter().map(|c| hit(&s, c, 0.0)).collect(),
            Err(_) => Vec::new(),
        }
    }
}

fn hit(s: &Loaded, c: &Chunk, score: f32) -> Hit {
    Hit {
        standard_id: s.meta.id.clone(),
        standard_code: s.meta.code.clone(),
        clause: c.clause.clone(),
        title: c.title.clone(),
        page: c.page,
        text: c.text.clone(),
        score,
    }
}

fn sort_desc(v: &mut [(usize, usize, f32)]) {
    v.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal).then(a.0.cmp(&b.0)).then(a.1.cmp(&b.1)));
}

/// Clause references named in a query, as (kind, bare) keys.
fn query_clause_refs(q: &str) -> Vec<(String, String)> {
    use std::sync::OnceLock;
    static RX: OnceLock<regex::Regex> = OnceLock::new();
    let rx = RX.get_or_init(|| {
        regex::Regex::new(
            r"(?:\b((?i:table|tab\.|figure|fig\.|annex|appendix|section|clause|cl\.|chapter|article))\s*|(ข้อ)\s*|§\s*|\b)((?:[A-Z]\.)?\d{1,3}(?:\.\d{1,3})+(?:\(\d{1,2}\)P?)?|[A-Z]\.\d{1,3}(?:\.\d{1,3})*|\d{1,3}(?:\(\d{1,2}\)P?)?|[A-Z])\b",
        )
        .unwrap()
    });
    let mut out = Vec::new();
    for c in rx.captures_iter(q) {
        let word = c.get(1).map(|m| m.as_str().to_lowercase());
        let thai = c.get(2).is_some();
        let num = &c[3];
        let multi = num.contains('.');
        let lettered_alone = num.len() == 1 && num.chars().all(|ch| ch.is_ascii_uppercase());
        // bare single integers and single letters only count after a keyword ("Section 6", "Annex D")
        if !multi && word.is_none() && !thai {
            continue;
        }
        if lettered_alone && !matches!(word.as_deref(), Some("annex") | Some("appendix")) {
            continue;
        }
        let kind = match word.as_deref() {
            Some("table") | Some("tab.") => "table",
            Some("figure") | Some("fig.") => "figure",
            Some("annex") | Some("appendix") => "annex",
            _ => "",
        };
        // a paragraph-level reference also counts as its clause
        let bare = num.to_lowercase().replace(' ', "");
        if !out.contains(&(kind.to_string(), bare.clone())) {
            out.push((kind.to_string(), bare));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn query_refs() {
        let r = query_clause_refs("ACI 318-19 22.5.5.1 shear");
        assert_eq!(r, vec![("".to_string(), "22.5.5.1".to_string())]);
        let r = query_clause_refs("Table 9.3.1.1 and Annex D and 6.5.2.2(1)P");
        assert!(r.contains(&("table".into(), "9.3.1.1".into())));
        assert!(r.contains(&("annex".into(), "d".into())));
        assert!(r.contains(&("".into(), "6.5.2.2(1)p".into())));
        assert!(query_clause_refs("minimum shear reinforcement area").is_empty());
        assert_eq!(query_clause_refs("ข้อ 3.2"), vec![("".to_string(), "3.2".to_string())]);
    }
    #[test]
    fn library_is_send_sync() {
        fn f<T: Send + Sync>() {}
        f::<Library>();
    }
}
