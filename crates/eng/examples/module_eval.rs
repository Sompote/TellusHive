//! Evaluate built-in design modules on a batch of inputs, for cross-checking external
//! reference solvers. Reads JSON lines {"module": .., "args": {..}, "keys": [..]} from stdin
//! and prints one JSON line per input with the requested sheet values (or the error).
//!
//! cargo run --release -p th-eng --example module_eval < cases.jsonl

use serde_json::{json, Value};
use std::io::BufRead;

fn main() {
    for line in std::io::stdin().lock().lines() {
        let line = line.expect("stdin");
        if line.trim().is_empty() {
            continue;
        }
        let v: Value = serde_json::from_str(&line).expect("json line");
        let module = v["module"].as_str().unwrap_or_default();
        let out = match th_eng::design::run_module(module, &v["args"]) {
            Ok(d) => {
                let mut o = serde_json::Map::new();
                for k in v["keys"].as_array().cloned().unwrap_or_default() {
                    let k = k.as_str().unwrap_or_default().to_string();
                    o.insert(k.clone(), json!(d.sheet.value(&k)));
                }
                json!({"ok": true, "values": o, "pass": d.sheet.checks().iter().all(|c| c.pass)})
            }
            Err(e) => json!({"ok": false, "error": e}),
        };
        println!("{out}");
    }
}
