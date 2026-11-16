//! Prints the standards briefing an agent would get for each task, to check it before a run.
//! usage: OPENROUTER_API_KEY=… briefing_preview items.jsonl [ID,ID,…]
//! Each line of items.jsonl is a JSON object with `problem_id` and `statement`.

use std::path::PathBuf;
use th_harness::standards_briefing::{relevant_hits, render};
use th_kb::Library;

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let file = args.first().ok_or("usage: briefing_preview items.jsonl [IDS]")?;
    let ids: Vec<&str> = args.get(1).map(|s| s.split(',').collect()).unwrap_or_default();
    th_kb::set_openrouter_key(std::env::var("OPENROUTER_API_KEY").ok());
    let root = std::env::var("TH_KB_ROOT").map(PathBuf::from).unwrap_or_else(|_| Library::default_root());
    let lib = Library::open(&root)?;
    let text = std::fs::read_to_string(file).map_err(|e| e.to_string())?;
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let v: serde_json::Value = serde_json::from_str(line).map_err(|e| e.to_string())?;
        let id = v["problem_id"].as_str().unwrap_or("");
        if !ids.is_empty() && !ids.contains(&id) {
            continue;
        }
        let hits = relevant_hits(&lib, v["statement"].as_str().unwrap_or(""), &[]);
        let list: Vec<String> = hits.iter().map(|h| format!("{} {} p{:?} sim {:.2} cov {:.2}{}", h.standard_code, h.clause, h.page, h.similarity.unwrap_or(0.0), h.coverage, if h.clause_match { " clause" } else { "" })).collect();
        println!("{id}: {}", if list.is_empty() { "(no briefing)".to_string() } else { list.join(" | ") });
        if std::env::var("SHOW").is_ok() {
            println!("{}", render(&hits));
        }
    }
    Ok(())
}
