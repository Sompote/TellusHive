//! Antigravity with this program's tool server, as a bot runs it: does agy see and call
//! a plugin's tools? Build the app first (`cargo build -p tellushive`), then
//! `cargo run -p th-harness --example agy_e2e -- <project> <team.json> [tellushive program]`
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use th_harness::external::{self, McpServer, Which};
use th_harness::Settings;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let project = PathBuf::from(&a[1]);
    let team = PathBuf::from(&a[2]);
    // the tellushive program built next to this example (target/<profile>/tellushive), or the one given
    let built = std::env::current_exe().ok().and_then(|e| Some(e.parent()?.parent()?.join(format!("tellushive{}", std::env::consts::EXE_SUFFIX)))).unwrap_or_default();
    let exe = a.get(3).map(PathBuf::from).unwrap_or(built);
    let work = project.join("work");
    std::fs::create_dir_all(&work).unwrap();
    let cfg = Which::Agy.config(&Settings::load());
    let mcp = McpServer {
        name: "tellushive".into(),
        command: exe,
        args: vec!["--mcp".into(), "--project".into(), project.display().to_string(), "--agent".into(), "bot".into(), "--session".into(), "e2e-1".into(), "--depth".into(), "0".into(), "--team-file".into(), team.display().to_string()],
    };
    let prompt = "Step 1: count the tools you have whose name contains 'pondus' and show the first five names exactly. Step 2: call the Pondus get_model tool once and say in one line what it returned. Do not use the shell.";
    let r = external::run_job(Which::Agy, &cfg, &external::Job { prompt, workdir: &work, project: &project, mcp: Some(mcp), model: "", mcp_only: true }, &AtomicBool::new(false));
    match r {
        Ok(o) => println!("ok={} seconds={:.0}\n--- reply ---\n{}", o.ok, o.seconds, o.text),
        Err(e) => println!("ERROR: {e}"),
    }
}
