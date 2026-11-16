//! Prints the built-in Design office team as JSON, so experiments can write a team file that
//! differs from the default in a few fields (for example an agent's provider).
//!
//! cargo run --release -p th-harness --example dump_team > team.json

fn main() {
    println!("{}", serde_json::to_string_pretty(&th_harness::config::default_team()).expect("team serialises"));
}
