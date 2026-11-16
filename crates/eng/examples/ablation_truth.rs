//! Reference answers for the ablation study: runs the built-in `bearing_terzaghi` module on
//! every case of a cases file and prints q_ult and Q_all as JSON.
//!
//! cargo run --release -p th-eng --example ablation_truth -- experiments/ablation/cases.json

use serde_json::{json, Value};

fn main() {
    let path = std::env::args().nth(1).expect("usage: ablation_truth <cases.json>");
    let spec: Value = serde_json::from_str(&std::fs::read_to_string(&path).expect("cases file")).expect("cases JSON");
    let mut out = vec![];
    for c in spec["cases"].as_array().expect("cases array") {
        let mut args = spec["fixed"].clone();
        for (k, v) in c.as_object().unwrap() {
            if k != "id" {
                args[k] = v.clone();
            }
        }
        let d = th_eng::design::run_module("bearing_terzaghi", &args).expect("module run");
        out.push(json!({"id": c["id"], "B": c["B"], "phi": c["phi"], "q_ult": d.sheet.value("q_ult"), "Q_all": d.sheet.value("Q_all")}));
    }
    let load = spec["column_load_kN"].as_f64().unwrap_or(0.0);
    let carries: Vec<Value> = out.iter().filter(|r| r["Q_all"].as_f64().unwrap_or(0.0) >= load).map(|r| r["id"].clone()).collect();
    println!("{}", serde_json::to_string_pretty(&json!({"column_load_kN": load, "cases": out, "carries": carries})).unwrap());
}
