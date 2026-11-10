//! Connects to an MCP server and lists its tools: `cargo run -p th-mcp --example probe -- <program> [args...]`
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let cfg = th_mcp::ServerConfig { name: "probe".into(), command: a[0].clone(), args: a[1..].to_vec(), enabled: true, timeout_s: 60, ..Default::default() };
    match th_mcp::Client::connect(&cfg) {
        Ok(c) => {
            println!("server {} with {} tools", c.server_name, c.tools.len());
            for t in &c.tools {
                println!("  {}", t.name);
            }
        }
        Err(e) => eprintln!("failed: {e}"),
    }
}
