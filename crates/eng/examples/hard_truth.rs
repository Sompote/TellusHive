//! Reference answers for the hard ablation task: for each column, the smallest footing width
//! B (a multiple of `step` m) whose allowable gross load from the built-in `bearing_terzaghi`
//! module carries the column load, with q_ult and Q_all at that width. A column with a
//! `final` block is designed again with those overrides (the site correction of turn 2), and
//! its final answer is the redesign.
//!
//! cargo run --release -p th-eng --example hard_truth -- experiments/ablation/hard.json

use serde_json::{json, Value};

fn design(base: &Value, col: &Value, over: Option<&Value>, step: f64) -> Value {
    let mut args = base.clone();
    for (k, v) in col["args"].as_object().unwrap() {
        args[k] = v.clone();
    }
    if let Some(o) = over {
        for (k, v) in o.as_object().unwrap() {
            args[k] = v.clone();
        }
    }
    let load = col["load_kN"].as_f64().unwrap();
    let ratio = col["L_over_B"].as_f64();
    let mut n = 1;
    loop {
        let b = (n as f64 * step * 1000.0).round() / 1000.0;
        args["B"] = json!(b);
        if let Some(r) = ratio {
            args["L"] = json!((b * r * 1000.0).round() / 1000.0);
        }
        let d = th_eng::design::run_module("bearing_terzaghi", &args).expect("module run");
        let q_all = d.sheet.value("Q_all").unwrap();
        if q_all >= load {
            return json!({"B": b, "L": args.get("L").cloned().unwrap_or(Value::Null), "q_ult": d.sheet.value("q_ult"), "Q_all": q_all, "load_kN": load});
        }
        n += 1;
        assert!(n < 400, "no width up to 20 m carries {}", col["id"]);
    }
}

fn main() {
    let path = std::env::args().nth(1).expect("usage: hard_truth <hard.json>");
    let spec: Value = serde_json::from_str(&std::fs::read_to_string(&path).expect("spec file")).expect("spec JSON");
    let step = spec["step_m"].as_f64().unwrap_or(0.05);
    let mut out = vec![];
    for c in spec["columns"].as_array().unwrap() {
        let first = design(&spec["base_args"], c, None, step);
        let fin = c.get("final").map(|o| design(&spec["base_args"], c, Some(o), step));
        out.push(json!({"id": c["id"], "first": first, "final": fin.clone().unwrap_or(first.clone()), "redesigned": fin.is_some()}));
    }
    println!("{}", serde_json::to_string_pretty(&json!({"columns": out})).unwrap());
}
