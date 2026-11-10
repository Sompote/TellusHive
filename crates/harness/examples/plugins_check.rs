//! Connects every default plugin and counts each default agent's tools:
//! `cargo run --release -p th-harness --example plugins_check`
use th_harness::config::{default_plugins, default_team, Settings};
use th_harness::Plugins;

fn main() {
    let mut settings = Settings::default();
    settings.plugins = default_plugins();
    let plugins = Plugins::default();
    let mut tools = std::collections::HashMap::new();
    for p in &settings.plugins {
        if p.name == "cadcraft" {
            continue; // starts the CADCraft window; checked separately
        }
        let r = if p.enabled { plugins.ensure(&settings, &p.name).map(|c| c.tools.len()) } else { Err("not found".into()) };
        println!("{:10} {:>4}  {}", p.name, r.as_ref().map(|n| n.to_string()).unwrap_or_else(|e| format!("FAIL {e}")), p.command);
        tools.insert(p.name.clone(), r.unwrap_or(0));
    }
    tools.insert("cadcraft".into(), 15);
    let builtin = th_harness::tools::builtin_specs().len();
    for a in default_team().agents {
        let own = if a.tools.iter().any(|t| t == "*") { builtin } else { a.tools.len() };
        let plug: usize = a.plugins.iter().map(|p| tools.get(p).copied().unwrap_or(0)).sum();
        println!("agent {:20} {:>3} built-in + {:>3} plugin = {:>3} tools  {:?}", a.name, own, plug, own + plug, a.plugins);
    }
}
