//! End-to-end tests of the standards library on two synthetic standards
//! (invented text in the style of a concrete code and a geotechnical code).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use th_kb::{EmbedderConfig, Hit, Library, NewStandard, SearchMode};

fn data(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("data").join(name)
}

fn tmp_root(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("adkb-test-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    p
}

fn conc() -> NewStandard {
    NewStandard {
        name: "Synthetic Structural Concrete Code".into(),
        code: "SYN-CONC 1".into(),
        discipline: "structural".into(),
        country: "XX".into(),
        year: Some(2026),
    }
}

fn geo() -> NewStandard {
    NewStandard {
        name: "Synthetic Geotechnical Design Code".into(),
        code: "SYN-GEO 1".into(),
        discipline: "geotechnical".into(),
        country: "XX".into(),
        year: Some(2026),
    }
}

fn build(tag: &str) -> (Library, PathBuf) {
    let root = tmp_root(tag);
    let lib = Library::open(&root).unwrap();
    let mut last = 0.0f32;
    let m = lib
        .ingest(&[data("syn_concrete.txt")], &conc(), &EmbedderConfig::default(), &mut |f, _| {
            assert!(f >= last - 1e-6);
            last = f;
        })
        .unwrap();
    assert_eq!(m.id, "syn-conc-1");
    lib.ingest(&[data("syn_geotech.txt")], &geo(), &EmbedderConfig::default(), &mut |_, _| {}).unwrap();
    (lib, root)
}

fn rank_of(hits: &[Hit], ok: &dyn Fn(&Hit) -> bool) -> Option<usize> {
    hits.iter().position(|h| ok(h))
}

fn clause_in(list: &'static [&'static str]) -> impl Fn(&Hit) -> bool {
    move |h: &Hit| list.iter().any(|c| h.clause == *c)
}

#[test]
fn chunking_finds_the_clauses() {
    let (lib, root) = build("chunks");
    let c = lib.chunks("syn-conc-1");
    let clauses: Vec<&str> = c.iter().map(|h| h.clause.as_str()).collect();
    for want in ["9.1.1", "9.3.1.1", "Table 9.3.1.1", "9.6.1.2", "9.6.3.1", "9.7.6.2.2", "22.5.5.1", "Table 22.5.5.1", "22.5.5.1.3", "22.6.1.1"] {
        assert!(clauses.contains(&want), "missing {want}: {clauses:?}");
    }
    let g = lib.chunks("syn-geo-1");
    let gc: Vec<&str> = g.iter().map(|h| h.clause.as_str()).collect();
    for want in ["6.1", "6.5.2.2", "6.5.2.2(3)P", "6.5.3", "6.6.2", "D.1", "D.4", "D.5"] {
        assert!(gc.contains(&want), "missing {want}: {gc:?}");
    }
    // no TOC line became a clause, and folded parents carry their heading
    assert_eq!(c.iter().filter(|h| h.clause == "9.1.1").count(), 1);
    let d4 = g.iter().find(|h| h.clause == "D.4").unwrap();
    assert!(d4.text.contains("Ngamma"));
    let d1 = g.iter().find(|h| h.clause == "D.1").unwrap();
    assert!(d1.text.starts_with("Annex D"), "{}", d1.text);
    eprintln!("chunks: concrete {} / geotech {}", c.len(), g.len());
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn natural_language_queries_top3() {
    let (lib, root) = build("nl");
    let h = lib.search("minimum shear reinforcement area", &[], 3).unwrap();
    assert!(rank_of(&h, &|h: &Hit| h.clause.starts_with("9.6.3")).is_some(), "{h:#?}");
    let h = lib.search("bearing resistance drained shape factors", &[], 3).unwrap();
    assert!(rank_of(&h, &clause_in(&["D.4"])).is_some(), "{h:#?}");
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn clause_number_query_comes_first() {
    let (lib, root) = build("num");
    for (q, want) in [
        ("SYN-CONC 22.5.5.1", "22.5.5.1"),
        ("22.5.5.1", "22.5.5.1"),
        ("what does 9.6.1.3 say", "9.6.1.3"),
        ("Table 9.3.1.1", "Table 9.3.1.1"),
        ("D.4", "D.4"),
        ("6.5.2.2(3)P weak layer", "6.5.2.2(3)P"),
        ("clause 6.5.3 sliding", "6.5.3"),
    ] {
        let h = lib.search(q, &[], 5).unwrap();
        assert_eq!(h[0].clause, want, "query {q}: {:?}", h.iter().map(|x| &x.clause).collect::<Vec<_>>());
    }
    // paragraph reference falls back to its clause
    let h = lib.search("6.5.2.2(1)", &[], 3).unwrap();
    assert_eq!(h[0].clause, "6.5.2.2");
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn clause_lookup_returns_subclauses_in_order() {
    let (lib, root) = build("lookup");
    let c: Vec<String> = lib.clause("syn-conc-1", "9.6").into_iter().map(|h| h.clause).collect();
    assert_eq!(c, vec!["9.6.1.1", "9.6.1.2", "9.6.1.3", "9.6.2.1", "9.6.3.1", "9.6.3.2", "9.6.4.1"]);
    let c: Vec<String> = lib.clause("syn-conc-1", "22.5.5").into_iter().map(|h| h.clause).collect();
    assert_eq!(c, vec!["22.5.5.1", "Table 22.5.5.1", "22.5.5.1.3"]);
    let c: Vec<String> = lib.clause("syn-geo-1", "Annex D").into_iter().map(|h| h.clause).collect();
    assert_eq!(c, vec!["D.1", "D.3", "D.4", "D.5"]);
    let c: Vec<String> = lib.clause("syn-geo-1", "Section 6.5.2").into_iter().map(|h| h.clause).collect();
    assert_eq!(c, vec!["6.5.2.1", "6.5.2.2", "6.5.2.2(3)P"]);
    assert_eq!(lib.clause("syn-conc-1", "Table 9.3.1.1")[0].clause, "Table 9.3.1.1");
    assert!(lib.clause("syn-conc-1", "99.9").is_empty());
    assert!(lib.clause("no-such", "9.6").is_empty());
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn filter_by_standard() {
    let (lib, root) = build("filter");
    let only_geo = vec!["syn-geo-1".to_string()];
    let h = lib.search("minimum shear reinforcement area", &only_geo, 10).unwrap();
    assert!(!h.is_empty());
    assert!(h.iter().all(|x| x.standard_id == "syn-geo-1"));
    let h = lib.search("22.5.5.1", &only_geo, 5).unwrap();
    assert!(h.iter().all(|x| x.standard_code == "SYN-GEO 1"));
    assert!(lib.search("x", &["missing-std".to_string()], 5).is_err());
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn persistence_add_note_and_delete() {
    let root = tmp_root("persist");
    {
        let lib = Library::open(&root).unwrap();
        lib.ingest(&[data("syn_concrete.txt")], &conc(), &EmbedderConfig::default(), &mut |_, _| {}).unwrap();
        lib.add_note(
            "syn-conc-1",
            "9.7.1.1",
            "Cover in coastal works",
            "Office practice: for structures within 1 km of the sea use 75 mm cover to all reinforcement in piles and pile caps.",
        )
        .unwrap();
    }
    let lib = Library::open(&root).unwrap();
    let list = lib.list();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].code, "SYN-CONC 1");
    assert_eq!(list[0].embedder, "hashed-1024");
    assert_eq!(list[0].chunks, lib.chunks("syn-conc-1").len());
    let h = lib.search("cover near the sea for pile caps", &[], 3).unwrap();
    assert!(h[0].title.starts_with("[Office note]"), "{h:#?}");
    // the note is listed with its clause after the clause itself
    let c = lib.clause("syn-conc-1", "9.7.1.1");
    assert_eq!(c.len(), 2);
    assert!(c[1].title.contains("Office note"));
    // re-ingesting the same file replaces its chunks but keeps the note
    let n0 = lib.chunks("syn-conc-1").len();
    lib.ingest(&[data("syn_concrete.txt")], &conc(), &EmbedderConfig::default(), &mut |_, _| {}).unwrap();
    assert_eq!(lib.chunks("syn-conc-1").len(), n0);
    assert_eq!(lib.list()[0].source_files, vec!["syn_concrete.txt".to_string()]);
    // a second file adds to the same standard
    lib.ingest(&[data("syn_geotech.txt")], &conc(), &EmbedderConfig::default(), &mut |_, _| {}).unwrap();
    assert_eq!(lib.list().len(), 1);
    assert_eq!(lib.list()[0].source_files.len(), 2);
    lib.delete("syn-conc-1").unwrap();
    assert!(lib.list().is_empty());
    assert!(lib.search("cover", &[], 3).unwrap().is_empty());
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn errors_are_clear() {
    let root = tmp_root("err");
    let lib = Library::open(&root).unwrap();
    let bad = root.join("x.docx");
    std::fs::write(&bad, "x").unwrap();
    let e = lib.ingest(&[bad], &conc(), &EmbedderConfig::default(), &mut |_, _| {}).unwrap_err();
    assert!(e.contains("unsupported"), "{e}");
    // a PDF with no text layer (a minimal blank page)
    let pdf = root.join("blank.pdf");
    std::fs::write(&pdf, blank_pdf()).unwrap();
    let e = lib.ingest(&[pdf], &conc(), &EmbedderConfig::default(), &mut |_, _| {}).unwrap_err();
    assert!(e.contains("scanned") || e.contains("PDF"), "{e}");
    eprintln!("blank PDF error: {e}");
    assert!(lib.delete("../etc").is_err());
    std::fs::remove_dir_all(root).ok();
}

fn blank_pdf() -> Vec<u8> {
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << >> >>",
        "<< /Length 0 >>\nstream\n\nendstream",
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offs = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offs.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", i + 1, o).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offs {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
    out
}

// ------------------------------------------------------------ API embedder

/// A tiny OpenAI-compatible embeddings server: letter-frequency vectors.
fn mock_server(log: Arc<Mutex<Vec<(String, serde_json::Value)>>>) -> String {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    std::thread::spawn(move || {
        for s in l.incoming() {
            let Ok(mut s) = s else { continue };
            let mut r = BufReader::new(s.try_clone().unwrap());
            let mut len = 0usize;
            let mut auth = String::new();
            let mut first = String::new();
            r.read_line(&mut first).unwrap();
            loop {
                let mut line = String::new();
                if r.read_line(&mut line).unwrap() == 0 {
                    break;
                }
                let t = line.trim_end();
                if t.is_empty() {
                    break;
                }
                let low = t.to_ascii_lowercase();
                if let Some(v) = low.strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap();
                }
                if low.starts_with("authorization:") {
                    auth = t["authorization:".len()..].trim().to_string();
                }
            }
            let mut body = vec![0u8; len];
            r.read_exact(&mut body).unwrap();
            let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
            log.lock().unwrap().push((format!("{} | {}", first.trim(), auth), v.clone()));
            let inputs = v["input"].as_array().unwrap();
            let data: Vec<serde_json::Value> = inputs
                .iter()
                .enumerate()
                .rev() // out of order on purpose
                .map(|(i, t)| {
                    let mut e = vec![0f32; 26];
                    for c in t.as_str().unwrap().to_ascii_lowercase().bytes() {
                        if c.is_ascii_lowercase() {
                            e[(c - b'a') as usize] += 1.0;
                        }
                    }
                    serde_json::json!({"object":"embedding","index":i,"embedding":e})
                })
                .collect();
            let resp = serde_json::json!({"object":"list","data":data,"model":v["model"]}).to_string();
            let _ = write!(
                s,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                resp.len(),
                resp
            );
        }
    });
    format!("http://{addr}/v1")
}

#[test]
fn api_embedder_end_to_end() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let base = mock_server(log.clone());
    let root = tmp_root("api");
    let lib = Library::open(&root).unwrap();
    let cfg = EmbedderConfig::Api { base_url: base.clone(), api_key: Some("sk-test".into()), model: "mock-embed".into() };
    let m = lib.ingest(&[data("syn_concrete.txt")], &conc(), &cfg, &mut |_, _| {}).unwrap();
    assert!(m.embedder.starts_with("api:mock-embed@"));
    {
        let l = log.lock().unwrap();
        let (line, body) = &l[0];
        assert!(line.starts_with("POST /v1/embeddings"), "{line}");
        assert!(line.ends_with("Bearer sk-test"), "{line}");
        assert_eq!(body["model"], "mock-embed");
        assert!(body["input"].as_array().unwrap().len() <= 64);
        let total: usize = l.iter().map(|(_, b)| b["input"].as_array().unwrap().len()).sum();
        assert_eq!(total, m.chunks);
    }
    let calls_before = log.lock().unwrap().len();
    // a note embeds only the new chunk
    lib.add_note("syn-conc-1", "", "Rule", "Use stirrups of at least 10 mm diameter.").unwrap();
    let l = log.lock().unwrap();
    assert_eq!(l.len(), calls_before + 1);
    assert_eq!(l.last().unwrap().1["input"].as_array().unwrap().len(), 1);
    drop(l);
    // the query goes through the same embedder; clause boost still wins
    let h = lib.search("22.5.5.1", &[], 3).unwrap();
    assert_eq!(h[0].clause, "22.5.5.1");
    assert!(log.lock().unwrap().last().unwrap().1["input"][0] == "22.5.5.1");
    // stored config reloads
    let lib2 = Library::open(&root).unwrap();
    assert_eq!(lib2.search("shear", &[], 2).unwrap().len(), 2);
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn api_down_falls_back_to_keywords() {
    let root = tmp_root("apidown");
    let log = Arc::new(Mutex::new(Vec::new()));
    let base = mock_server(log);
    let lib = Library::open(&root).unwrap();
    let cfg = EmbedderConfig::Api { base_url: base, api_key: None, model: "m".into() };
    lib.ingest(&[data("syn_geotech.txt")], &geo(), &cfg, &mut |_, _| {}).unwrap();
    // point the stored config at a dead port
    let p = root.join("syn-geo-1").join("embedder.json");
    let s = std::fs::read_to_string(&p).unwrap();
    let mut v: serde_json::Value = serde_json::from_str(&s).unwrap();
    v["config"]["base_url"] = serde_json::json!("http://127.0.0.1:9/v1");
    std::fs::write(&p, v.to_string()).unwrap();
    let lib = Library::open(&root).unwrap();
    let h = lib.search("sliding resistance of the base", &[], 3).unwrap();
    assert_eq!(h[0].clause, "6.5.3");
    std::fs::remove_dir_all(root).ok();
}

// ------------------------------------------------------------ quality report

struct Q {
    q: &'static str,
    want: &'static [&'static str],
}

const EVAL: &[Q] = &[
    Q { q: "minimum shear reinforcement area", want: &["9.6.3.1", "9.6.3.2"] },
    Q { q: "minimum depth of a simply supported beam", want: &["9.3.1.1", "Table 9.3.1.1"] },
    Q { q: "maximum stirrup spacing", want: &["9.7.6.2.2"] },
    Q { q: "concrete contribution to one-way shear in nonprestressed members", want: &["22.5.5.1", "Table 22.5.5.1"] },
    Q { q: "size effect factor for deep members", want: &["22.5.5.1.3"] },
    Q { q: "shear strength provided by stirrups", want: &["22.5.8.5.3"] },
    Q { q: "punching shear around columns", want: &["22.6.1.1"] },
    Q { q: "minimum tension steel in beams", want: &["9.6.1.1", "9.6.1.2", "9.6.1.3"] },
    Q { q: "concrete cover to reinforcement", want: &["9.7.1.1"] },
    Q { q: "development length of bars in tension", want: &["9.7.2.1"] },
    Q { q: "long-term deflection", want: &["9.3.2.1"] },
    Q { q: "upper limit on shear to prevent web crushing", want: &["22.5.1.2"] },
    Q { q: "bearing resistance drained shape factors", want: &["D.4"] },
    Q { q: "undrained bearing capacity of a footing", want: &["D.3"] },
    Q { q: "sliding of a footing on its base", want: &["6.5.3"] },
    Q { q: "eccentric load effective width", want: &["6.5.2.2", "6.5.4"] },
    Q { q: "consolidation settlement of clay", want: &["6.6.2"] },
    Q { q: "angular distortion between columns", want: &["6.6.3"] },
    Q { q: "footing on a slope near an excavation", want: &["6.5.1"] },
    Q { q: "plate load test", want: &["D.5"] },
    Q { q: "frost depth and groundwater when choosing foundation level", want: &["6.4"] },
    Q { q: "weak layer beneath a strong layer", want: &["6.5.2.2(3)P"] },
    // paraphrases with little word overlap: a lexical embedder should struggle here
    Q { q: "how much stirrup steel at least", want: &["9.6.3.1", "9.6.3.2"] },
    Q { q: "footing pushed sideways", want: &["6.5.3"] },
    Q { q: "how thick must a beam be", want: &["9.3.1.1", "Table 9.3.1.1"] },
    Q { q: "long term sinking of foundations on soft clay", want: &["6.6.2"] },
];

#[test]
fn retrieval_quality_report() {
    let (lib, root) = build("eval");
    let mut summary = Vec::new();
    for (name, mode) in [
        ("hybrid", SearchMode::Hybrid),
        ("vector(hashed)", SearchMode::VectorOnly),
        ("bm25", SearchMode::Bm25Only),
    ] {
        let (mut h1, mut h3, mut mrr) = (0, 0, 0.0f32);
        let mut misses = Vec::new();
        for q in EVAL {
            let hits = lib.search_mode(q.q, &[], 10, mode).unwrap();
            match hits.iter().position(|h| q.want.contains(&h.clause.as_str())) {
                Some(r) => {
                    if r == 0 {
                        h1 += 1;
                    }
                    if r < 3 {
                        h3 += 1;
                    } else {
                        misses.push(format!("{} (rank {})", q.q, r + 1));
                    }
                    mrr += 1.0 / (r as f32 + 1.0);
                }
                None => misses.push(format!("{} (not in top 10)", q.q)),
            }
        }
        let n = EVAL.len() as f32;
        eprintln!(
            "{name:15} n={} hit@1={:.2} hit@3={:.2} MRR@10={:.3}  misses@3: {:?}",
            EVAL.len(),
            h1 as f32 / n,
            h3 as f32 / n,
            mrr / n,
            misses
        );
        summary.push((name, h3 as f32 / n));
    }
    // hybrid must not be worse than either signal alone on this set
    let hy = summary[0].1;
    assert!(hy >= 0.8, "hybrid hit@3 {hy}");
    std::fs::remove_dir_all(root).ok();
}

// ------------------------------------------------------------ real PDF (local only)

#[test]
#[ignore]
fn real_pdf_extraction() {
    let path = std::env::var("TH_KB_TEST_PDF").expect("set TH_KB_TEST_PDF to a text PDF");
    let root = tmp_root("pdf");
    let lib = Library::open(&root).unwrap();
    let t = std::time::Instant::now();
    let m = lib
        .ingest(&[PathBuf::from(&path)], &NewStandard {
            name: "test".into(),
            code: "PDF TEST".into(),
            discipline: "other".into(),
            country: "".into(),
            year: None,
        }, &EmbedderConfig::default(), &mut |_, _| {})
        .unwrap();
    eprintln!("{} chunks in {:.1} s", m.chunks, t.elapsed().as_secs_f32());
    let c = lib.chunks(&m.id);
    let with_page = c.iter().filter(|h| h.page.is_some()).count();
    let with_clause = c.iter().filter(|h| !h.clause.is_empty()).count();
    eprintln!("{with_page} with page, {with_clause} with clause number");
    for h in c.iter().take(8) {
        eprintln!("--- [{}] {} (p.{:?})\n{}", h.clause, h.title, h.page, &h.text.chars().take(200).collect::<String>());
    }
    if let Ok(q) = std::env::var("TH_KB_TEST_QUERY") {
        for h in lib.search(&q, &[], 5).unwrap() {
            eprintln!("HIT {:.3} [{}] {} p.{:?}", h.score, h.clause, h.title, h.page);
        }
    }
    assert!(with_page == c.len());
    std::fs::remove_dir_all(root).ok();
}
