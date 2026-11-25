fn main() {
    for m in th_eng::design::modules() {
        println!("== {} ({}) — {}", m.name, m.standard, m.title);
        if let Some(p) = m.input_schema.get("properties").and_then(|p| p.as_object()) {
            for (k, v) in p {
                println!("  {k}: {} {}", v.get("description").and_then(|d| d.as_str()).unwrap_or("").chars().take(110).collect::<String>(), v.get("default").map(|d| format!("[default {d}]")).unwrap_or_default());
            }
        }
        println!("  required: {}", m.input_schema.get("required").map(|r| r.to_string()).unwrap_or_default());
    }
}
