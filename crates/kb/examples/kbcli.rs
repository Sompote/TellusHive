//! Tiny command-line front end to the standards library.
//!
//! ```text
//! kbcli ingest <code> <name> <files...>   [env TH_KB_EMBED_URL + TH_KB_EMBED_MODEL for an API embedder]
//! kbcli search <query> [-k N] [--std ID]
//! kbcli clause <std-id> <clause>
//! kbcli list
//! kbcli delete <std-id>
//! ```
//! The library lives in `TH_KB_ROOT` or the default data folder.

use std::path::PathBuf;

use th_kb::{EmbedderConfig, Library, NewStandard};

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = std::env::var("TH_KB_ROOT").map(PathBuf::from).unwrap_or_else(|_| Library::default_root());
    let lib = Library::open(&root)?;
    match args.first().map(|s| s.as_str()) {
        Some("ingest") if args.len() >= 4 => {
            let files: Vec<PathBuf> = args[3..].iter().map(PathBuf::from).collect();
            let embedder = match (std::env::var("TH_KB_EMBED_URL"), std::env::var("TH_KB_EMBED_MODEL")) {
                (Ok(u), Ok(m)) => EmbedderConfig::Api { base_url: u, api_key: std::env::var("TH_KB_EMBED_API_KEY").ok(), model: m },
                _ => EmbedderConfig::default(),
            };
            let meta = NewStandard {
                code: args[1].clone(),
                name: args[2].clone(),
                discipline: std::env::var("TH_KB_DISCIPLINE").unwrap_or_else(|_| "other".into()),
                ..Default::default()
            };
            let m = lib.ingest(&files, &meta, &embedder, &mut |f, msg| eprintln!("[{:3.0}%] {msg}", f * 100.0))?;
            println!("{} ({}) : {} chunks, embedder {}", m.code, m.id, m.chunks, m.embedder);
        }
        Some("search") if args.len() >= 2 => {
            let mut k = 5;
            let mut ids = Vec::new();
            let mut q = Vec::new();
            let mut i = 1;
            while i < args.len() {
                match args[i].as_str() {
                    "-k" if i + 1 < args.len() => {
                        k = args[i + 1].parse().map_err(|_| "bad -k")?;
                        i += 1;
                    }
                    "--std" if i + 1 < args.len() => {
                        ids.push(args[i + 1].clone());
                        i += 1;
                    }
                    w => q.push(w.to_string()),
                }
                i += 1;
            }
            for h in lib.search(&q.join(" "), &ids, k)? {
                print_hit(&h);
            }
        }
        Some("clause") if args.len() == 3 => {
            for h in lib.clause(&args[1], &args[2]) {
                print_hit(&h);
            }
        }
        Some("list") => {
            for m in lib.list() {
                println!("{:24} {:16} {:5} chunks  {:14} {}", m.id, m.code, m.chunks, m.embedder, m.name);
            }
        }
        Some("delete") if args.len() == 2 => lib.delete(&args[1])?,
        _ => {
            return Err("usage: kbcli ingest <code> <name> <files...> | search <query> [-k N] [--std ID] | clause <id> <clause> | list | delete <id>".into())
        }
    }
    Ok(())
}

fn print_hit(h: &th_kb::Hit) {
    let page = h.page.map(|p| format!(" p.{p}")).unwrap_or_default();
    println!("== {:.3}  {} {}  {}{}", h.score, h.standard_code, h.clause, h.title, page);
    let t: String = h.text.chars().take(400).collect();
    println!("{t}\n");
}
