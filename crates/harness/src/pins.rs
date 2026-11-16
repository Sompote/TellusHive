//! Pinned facts from MCP programs (Tellus, Motus, Aedifex, any other server).
//!
//! A program's result is free text or JSON that this crate knows nothing about, so nothing
//! is interpreted: values are copied as the program wrote them and named by where they
//! stood. A JSON result gives its numbers, booleans and short strings by path
//! (`phases[1].msf`); a plain-text result gives its lines that hold a number, whole. Calls
//! that build or change a model are left out, because their replies carry no result, and so
//! are whole-model dumps, which the agent can read again at any time.
//!
//! The group of a set is the program, the tool and the arguments, so asking the same
//! question again after a new calculation replaces the earlier answer, while results of
//! different phases or points stay side by side.

use serde_json::Value;
use th_hive::{Fact, FactSet};

use crate::project::clip;

/// Most values pinned from one result.
const MAX_FACTS: usize = 60;
/// Arrays longer than this are tables or histories, not single results.
const MAX_ARRAY: usize = 8;
/// Longest string value and longest text line kept.
const MAX_STR: usize = 80;
const MAX_LINE: usize = 160;
const MAX_LINES: usize = 12;

/// Tool-name beginnings of calls that set a model up, change the view or move files.
const SETUP: &[&str] = &["add_", "set_", "update_", "delete_", "remove_", "new_", "open_", "save_", "load_", "list_", "show", "undo", "redo", "screenshot", "render", "select_", "cancel", "import_", "export_", "arrange_", "document_", "page_", "shape_", "stencil_", "view_"];
/// Whole-model dumps.
const DUMPS: &[&str] = &["get_model", "get_project", "get_scene_context"];

/// Whether a tool's reply can hold a result worth pinning.
pub fn reports_results(tool: &str) -> bool {
    let t = tool.to_ascii_lowercase();
    !SETUP.iter().any(|p| t.starts_with(p)) && !DUMPS.contains(&t.as_str())
}

#[derive(Default)]
struct Walk {
    facts: Vec<Fact>,
    /// Scalars found, kept or not.
    found: usize,
    long_arrays: usize,
    /// Whether any number or boolean was found (strings alone are not a result).
    measured: bool,
}

fn walk(v: &Value, path: &str, w: &mut Walk) {
    let keep = |text: String, w: &mut Walk| {
        w.found += 1;
        if w.facts.len() < MAX_FACTS {
            w.facts.push(Fact::new("result", if path.is_empty() { "value" } else { path }, text, "").key());
        }
    };
    match v {
        Value::Null => {}
        Value::Bool(b) => {
            w.measured = true;
            keep(b.to_string(), w);
        }
        Value::Number(n) => {
            w.measured = true;
            keep(n.to_string(), w);
        }
        Value::String(s) => {
            let s = s.trim();
            if !s.is_empty() && s.chars().count() <= MAX_STR && !s.contains('\n') {
                keep(s.to_string(), w);
            }
        }
        Value::Array(a) => {
            if a.len() > MAX_ARRAY {
                w.long_arrays += 1;
            } else {
                for (i, x) in a.iter().enumerate() {
                    walk(x, &format!("{path}[{i}]"), w);
                }
            }
        }
        Value::Object(o) => {
            for (k, x) in o {
                let p = if path.is_empty() { k.clone() } else { format!("{path}.{k}") };
                walk(x, &p, w);
            }
        }
    }
}

/// The facts of one successful call of a program's tool, or `None` when the reply holds
/// nothing to pin.
pub fn plugin_facts(server: &str, tool: &str, args: &Value, text: &str) -> Option<FactSet> {
    if !reports_results(tool) {
        return None;
    }
    let origin = format!("{server}.{tool}");
    let shown_args = match args {
        Value::Object(o) if o.is_empty() => String::new(),
        Value::Null => String::new(),
        a => a.to_string(),
    };
    let group = if shown_args.is_empty() { origin.clone() } else { format!("{origin} {shown_args}") };
    let mut head: Vec<String> = vec![];
    if !shown_args.is_empty() {
        head.push(format!("arguments {}", clip(&shown_args, 200)));
    }
    let body = text.trim();
    let facts = match serde_json::from_str::<Value>(body) {
        Ok(v) if v.is_object() || v.is_array() => {
            let mut w = Walk::default();
            walk(&v, "", &mut w);
            if !w.measured {
                return None;
            }
            if w.found > w.facts.len() {
                head.push(format!("the first {} of {} values", w.facts.len(), w.found));
            }
            if w.long_arrays > 0 {
                head.push(format!("long tables or histories left out: {}", w.long_arrays));
            }
            if w.found > w.facts.len() || w.long_arrays > 0 {
                head.push("call the tool again for the rest".into());
            }
            w.facts
        }
        _ => {
            // plain text: the lines that hold a number, whole and in order
            let lines: Vec<&str> = body.lines().map(str::trim).filter(|l| l.chars().any(|c| c.is_ascii_digit()) && !l.starts_with("[picture saved")).collect();
            if lines.is_empty() {
                return None;
            }
            if lines.len() > MAX_LINES {
                head.push(format!("the first {MAX_LINES} of {} lines with numbers; call the tool again for the rest", lines.len()));
            }
            lines.iter().take(MAX_LINES).map(|l| Fact::new("output", "", clip(l, MAX_LINE), "").key()).collect()
        }
    };
    Some(FactSet { group, origin, headline: head.join("; "), facts })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn text(set: FactSet) -> String {
        th_hive::facts::render(&th_hive::FactRecord { source: 1, tool: "tellus__get_results".into(), set })
    }

    #[test]
    fn json_result_is_pinned_by_path_with_the_digits_the_program_wrote() {
        let reply = r#"{"phase":"Excavation 2","converged":true,"msf":1.3274,"max_displacement":{"ux":-0.01842,"uy":0.0031},"wall":{"M_max":212.6,"anchors":[118.4,96.25]},"history":[1,2,3,4,5,6,7,8,9,10],"note":null}"#;
        let set = plugin_facts("tellus", "get_results", &json!({"phase": "Excavation 2"}), reply).unwrap();
        assert_eq!(set.origin, "tellus.get_results");
        assert_eq!(set.group, r#"tellus.get_results {"phase":"Excavation 2"}"#);
        let t = text(set);
        for exact in ["msf = 1.3274", "converged = true", "max_displacement.ux = -0.01842", "max_displacement.uy = 0.0031", "wall.M_max = 212.6", "wall.anchors[0] = 118.4", "wall.anchors[1] = 96.25", "phase = Excavation 2"] {
            assert!(t.contains(exact), "missing `{exact}` in:\n{t}");
        }
        assert!(t.contains("long tables or histories left out: 1; call the tool again for the rest"), "{t}");
        assert!(!t.contains("history"), "{t}");
    }

    #[test]
    fn different_arguments_are_different_groups() {
        let a = plugin_facts("tellus", "get_results", &json!({"phase": "P1"}), r#"{"msf":1.5}"#).unwrap();
        let b = plugin_facts("tellus", "get_results", &json!({"phase": "P2"}), r#"{"msf":1.2}"#).unwrap();
        let again = plugin_facts("tellus", "get_results", &json!({"phase": "P1"}), r#"{"msf":1.41}"#).unwrap();
        assert_ne!(a.group, b.group);
        assert_eq!(a.group, again.group);
    }

    #[test]
    fn plain_text_keeps_its_lines_with_numbers() {
        let reply = "Analysis finished.\nFactor of safety = 1.32 (phi-c reduction)\nMaximum settlement: 45.2 mm at x = 12.5 m\n[picture saved: images/tellus_1.png]";
        let set = plugin_facts("tellus", "calculate", &json!({}), reply).unwrap();
        assert_eq!(set.group, "tellus.calculate");
        assert_eq!(text(set), "tellus.calculate [tellus__get_results]\n  output: Factor of safety = 1.32 (phi-c reduction); Maximum settlement: 45.2 mm at x = 12.5 m");
    }

    #[test]
    fn setup_calls_dumps_and_empty_replies_pin_nothing() {
        assert!(plugin_facts("tellus", "add_soil_material", &json!({"name": "clay", "c": 10.0}), r#"{"id":3}"#).is_none());
        assert!(plugin_facts("tellus", "get_model", &json!({}), r#"{"width":60.0}"#).is_none());
        assert!(plugin_facts("tellus", "validate", &json!({}), "The model is ready to calculate.").is_none());
        assert!(plugin_facts("tellus", "validate", &json!({}), r#"{"status":"ok"}"#).is_none());
    }

    #[test]
    fn a_large_result_is_capped_and_says_so() {
        let o: serde_json::Map<String, Value> = (0..150).map(|i| (format!("v{i:03}"), json!(i as f64 + 0.5))).collect();
        let set = plugin_facts("motus", "get_results", &json!({"what": "profile"}), &Value::Object(o).to_string()).unwrap();
        assert_eq!(set.facts.len(), MAX_FACTS);
        assert!(set.headline.contains("the first 60 of 150 values"), "{}", set.headline);
    }
}
