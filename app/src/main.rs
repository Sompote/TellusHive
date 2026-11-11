//! TellusHive: an AI design team for civil engineers.
//!
//! ```text
//! tellushive                               the window
//! tellushive --mcp [--project DIR]         this program's tools as an MCP server (stdio)
//! tellushive run --project DIR "message"   one coordinator turn in the terminal
//! ```

mod app;
mod chat;
mod chrome;
mod icons;
mod markdown;
mod plugins;
mod settings_view;
mod standards;
mod team;
mod theme;
mod viewer;

use std::path::PathBuf;
use std::sync::Arc;

fn arg_value(args: &[String], key: &str) -> Option<String> {
    args.iter().position(|a| a == key).and_then(|i| args.get(i + 1)).cloned()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    // the folders of the earlier name (AgenticDesign) move to TellusHive once
    for line in th_harness::config::migrate_legacy() {
        eprintln!("tellushive: {line}");
    }
    if args.iter().any(|a| a == "--mcp") {
        let dir = arg_value(&args, "--project").map(PathBuf::from).unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let opt = th_harness::ServeOptions {
            agent: arg_value(&args, "--agent"),
            session: arg_value(&args, "--session"),
            parent: arg_value(&args, "--parent"),
            depth: arg_value(&args, "--depth").and_then(|d| d.parse().ok()).unwrap_or(0),
        };
        if let Err(e) = th_harness::serve_mcp_with(&dir, &opt) {
            eprintln!("tellushive --mcp: {e}");
            std::process::exit(1);
        }
        return;
    }
    if args.get(1).map(String::as_str) == Some("run") {
        std::process::exit(run_headless(&args));
    }
    if args.get(1).map(String::as_str) == Some("plugins") {
        std::process::exit(dump_plugins(&args));
    }
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default().with_title("TellusHive: an AI design team for civil engineers").with_inner_size([1500.0, 920.0]).with_min_inner_size([960.0, 600.0]).with_drag_and_drop(true).with_icon(app_icon_data()),
        ..Default::default()
    };
    let r = eframe::run_native(
        "TellusHive",
        options,
        Box::new(|cc| match app::App::new(cc) {
            Ok(a) => Ok(Box::new(a) as Box<dyn eframe::App>),
            Err(e) => Err(e.into()),
        }),
    );
    if let Err(e) = r {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

/// Prints the harness plugin tree with each plugin's state (DeepSeek Harness `--dump-config`).
fn dump_plugins(args: &[String]) -> i32 {
    let settings = th_harness::Settings::load();
    let project = match arg_value(args, "--project") {
        Some(d) => th_harness::Project::open_or_create(&PathBuf::from(d)),
        None => app::default_project(&settings),
    };
    let (Ok(project), Ok(library)) = (project, th_kb::Library::open(&th_kb::Library::default_root())) else {
        eprintln!("could not open the project or the standards library");
        return 1;
    };
    let team = th_harness::TeamSpec::load(&project.info.team);
    let engine = th_harness::Engine::new(settings, team, project, Arc::new(library), Arc::new(th_harness::Plugins::default()));
    print!("{}", engine.dump_plugins());
    let tools = engine.hive.tools().names();
    println!("\n{} registered tools: {}", tools.len(), tools.join(", "));
    0
}

/// One turn in the terminal: prints every event as it happens, keeps the conversation.
fn run_headless(args: &[String]) -> i32 {
    let settings = th_harness::Settings::load();
    let msg = args.iter().skip(2).filter(|a| !a.starts_with("--")).filter(|a| Some(*a) != arg_value(args, "--project").as_ref()).cloned().collect::<Vec<_>>().join(" ");
    let project = match arg_value(args, "--project") {
        Some(d) => th_harness::Project::open_or_create(&PathBuf::from(d)),
        None => app::default_project(&settings),
    };
    let project = match project {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let library = match th_kb::Library::open(&th_kb::Library::default_root()) {
        Ok(l) => Arc::new(l),
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let team = th_harness::TeamSpec::load(&project.info.team);
    let dir = project.dir.clone();
    let engine = th_harness::Engine::new(settings, team, project, library, Arc::new(th_harness::Plugins::default()));
    let mut saved = chat::ChatView::load(&dir);
    let (tx, rx) = std::sync::mpsc::channel();
    let printer = std::thread::spawn(move || {
        use th_harness::Event::*;
        let mut usage = chat::UsageReport::default();
        for e in rx {
            match e {
                AgentStart { agent, depth, task, .. } => println!("{}▶ {agent}: {}", "  ".repeat(depth), task.lines().next().unwrap_or("")),
                Text { agent, text, .. } => println!("[{agent}] {text}"),
                ToolCall { agent, tool, args, .. } => println!("  · {agent} → {tool} {}", args.chars().take(200).collect::<String>()),
                ToolResult { tool, ok, text, .. } => println!("    {} {tool}: {}", if ok { "ok" } else { "ERR" }, text.lines().next().unwrap_or("").chars().take(160).collect::<String>()),
                Artifact { path, .. } => println!("  ⇒ {}", path.display()),
                Model { plugin, path, .. } => println!("  ⇒ {} model saved: {}", th_harness::live::program_name(&plugin), path.display()),
                Error { agent, text, .. } => eprintln!("  ! {agent}: {text}"),
                Usage { agent, model, prompt, completion, .. } => {
                    usage.add(&agent, &model, prompt, completion);
                    let (_, p, c) = usage.totals();
                    println!("    tokens: {agent} +{} in, +{} out (task so far {})", chat::fmt_tokens(prompt), chat::fmt_tokens(completion), chat::fmt_tokens(p + c));
                }
                Thinking { .. } | Stream { .. } | AgentDone { .. } | Finished { .. } => {}
            }
        }
        usage
    });
    let started = std::time::Instant::now();
    let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let r = engine.send(th_harness::UserInput { text: msg, attachments: vec![] }, &tx, &cancel);
    drop(tx);
    let mut usage = printer.join().unwrap_or_default();
    usage.seconds = started.elapsed().as_secs();
    usage.outcome = if r.is_ok() { "finished".into() } else { "failed".into() };
    let (_, up, uc) = usage.totals();
    saved.tokens.0 += up;
    saved.tokens.1 += uc;
    if usage.totals().0 > 0 {
        println!("\nToken use ({} in {}:{:02})\n{}", usage.outcome, usage.seconds / 60, usage.seconds % 60, usage.table());
        saved.entries.push(chat::Entry::Usage { report: usage });
    }
    if let Ok(s) = serde_json::to_string(&saved) {
        let _ = std::fs::write(dir.join("chat.json"), s);
    }
    match r {
        Ok(_) => 0,
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

/// The window and Dock icon (the same picture as the app's mark in the top bar).
fn app_icon_data() -> Arc<eframe::egui::IconData> {
    let img = image::load_from_memory(include_bytes!("../assets/app_icon.png")).expect("app icon is a valid PNG").to_rgba8();
    let (width, height) = img.dimensions();
    Arc::new(eframe::egui::IconData { rgba: img.into_raw(), width, height })
}
