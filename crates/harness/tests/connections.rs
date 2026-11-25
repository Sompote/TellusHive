//! Connections across the system, with a scripted model in place of a provider: projects and
//! their conversations, bots and the teams they compose, delegation from a bot through a team
//! lead to a member, a bot calling a bot, the project's plugin rule and working folder, the
//! set-up tools during a run, Stop during delegation, and (when the programs are on this
//! computer) the real Tellus and the tool server handed to coding agents.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use th_harness::bots::{Bot, BotMode};
use th_harness::th_hive::{fn_plugin, keys, LlmAdapter, LlmRequest, LlmService};
use th_harness::{config, AgentSpec, Engine, Event, Plugins, Project, ProjectInfo, Settings, TeamSpec, UserInput};
use th_llm::{CallError, Delta, FunctionCall, Message, Provider, Reply, ToolCall, Usage};
use th_mcp::ServerConfig;

/// One data folder for the whole test binary (the tests run in parallel threads).
fn home() -> PathBuf {
    static H: OnceLock<PathBuf> = OnceLock::new();
    H.get_or_init(|| {
        let h = std::env::temp_dir().join(format!("th_conn_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&h);
        std::fs::create_dir_all(&h).unwrap();
        std::env::set_var("TELLUSHIVE_HOME", h.join("home"));
        h
    })
    .clone()
}

fn dir(name: &str) -> PathBuf {
    let d = home().join(name);
    let _ = std::fs::remove_dir_all(&d);
    d
}

/// What a scripted agent sees: who it is, the request, and the tool names it was offered.
type Brain = dyn Fn(&str, &LlmRequest) -> Message + Send + Sync;

struct Play {
    brain: Box<Brain>,
    seen: Mutex<Vec<(String, Vec<String>)>>,
}

fn who(system: &str) -> String {
    system.trim_start().strip_prefix("You are ").and_then(|r| r.split(',').next()).unwrap_or("").to_string()
}

impl LlmAdapter for Play {
    fn id(&self) -> &str {
        "play"
    }
    fn serves(&self, _: &Provider) -> bool {
        true
    }
    fn stream(&self, req: &LlmRequest, on: &mut dyn FnMut(Delta), cancel: &AtomicBool) -> Result<Reply, CallError> {
        let me = who(&req.system);
        self.seen.lock().unwrap().push((me.clone(), req.tools.iter().map(|t| t.name.clone()).collect()));
        // a member told to work slowly waits, but stops at once on Stop
        if req.system.contains("WORK SLOWLY") {
            let t0 = Instant::now();
            while t0.elapsed() < Duration::from_secs(20) {
                if cancel.load(Ordering::Relaxed) {
                    return Err(CallError { message: "cancelled".into(), status: None, overflow: false, cancelled: true });
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        let m = (self.brain)(&me, req);
        if let Some(c) = m.content.as_ref() {
            on(Delta::Text(c.text()));
        }
        Ok(Reply { message: m, reasoning: None, finish_reason: "stop".into(), usage: Usage { prompt_tokens: 10, completion_tokens: 5, cached_tokens: 0 } })
    }
}

fn call(name: &str, args: Value) -> Message {
    let mut m = Message::assistant("");
    m.content = None;
    m.tool_calls = Some(vec![ToolCall { id: format!("c{}", th_harness_stamp()), kind: "function".into(), function: FunctionCall { name: name.into(), arguments: args.to_string() } }]);
    m
}

fn th_harness_stamp() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
}

/// The text of the tool results this agent has had so far.
fn results(req: &LlmRequest) -> Vec<String> {
    req.messages.iter().filter(|m| m.role == "tool").map(|m| m.text()).collect()
}

fn settings(plugins: Vec<ServerConfig>) -> Settings {
    let mut s = Settings::default();
    s.default_provider = "ollama".into(); // no key needed
    s.default_model = "scripted".into();
    s.plugins = plugins;
    s.harness.standards_briefing = false;
    s
}

fn engine(s: Settings, team: TeamSpec, project: Project, brain: Box<Brain>) -> (Engine, Arc<Play>) {
    let library = Arc::new(th_kb::Library::open(&home().join("lib")).unwrap());
    let e = Engine::new(s, team, project, library, Arc::new(Plugins::default()));
    let play = Arc::new(Play { brain, seen: Mutex::new(vec![]) });
    let p2 = play.clone();
    e.hive.ctx.plugin(fn_plugin("scripted-model", vec![keys::LLM], move |ctx| {
        ctx.effect(ctx.expect::<LlmService>(keys::LLM)?.register(p2.clone()));
        Ok(())
    }));
    (e, play)
}

fn send(e: &Engine, text: &str) -> (Result<String, String>, Vec<Event>) {
    let (tx, rx) = mpsc::channel();
    let r = e.send(UserInput { text: text.into(), attachments: vec![] }, &tx, &Arc::new(AtomicBool::new(false)));
    drop(tx);
    (r, rx.into_iter().collect())
}

fn starts(ev: &[Event]) -> Vec<(String, usize)> {
    ev.iter().filter_map(|e| if let Event::AgentStart { agent, depth, .. } = e { Some((agent.clone(), *depth)) } else { None }).collect()
}

/// A stdio MCP server in a shell script: `echo` answers pong, `pwd` answers its folder.
fn fake_plugin(name: &str) -> ServerConfig {
    let script = r#"while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
  case "$line" in
    *initialize*) printf '{"jsonrpc":"2.0","id":%s,"result":{"serverInfo":{"name":"fake"},"capabilities":{"tools":{}}}}\n' "$id";;
    *tools/list*) printf '{"jsonrpc":"2.0","id":%s,"result":{"tools":[{"name":"echo","description":"e","inputSchema":{"type":"object"}},{"name":"pwd","description":"p","inputSchema":{"type":"object"}}]}}\n' "$id";;
    *'"name":"pwd"'*) printf '{"jsonrpc":"2.0","id":%s,"result":{"content":[{"type":"text","text":"%s"}]}}\n' "$id" "$(pwd -P)";;
    *tools/call*) printf '{"jsonrpc":"2.0","id":%s,"result":{"content":[{"type":"text","text":"pong"}]}}\n' "$id";;
  esac
done"#;
    ServerConfig { name: name.into(), description: "fake program".into(), command: "sh".into(), args: vec!["-c".into(), script.into()], enabled: true, timeout_s: 30, ..Default::default() }
}

fn solo(name: &str, tools: &[&str]) -> TeamSpec {
    let a = AgentSpec { id: "lead".into(), name: name.into(), role: "answers".into(), tools: tools.iter().map(|t| t.to_string()).collect(), plugins: vec!["*".into()], max_turns: 10, ..Default::default() };
    TeamSpec { name: name.into(), description: String::new(), coordinator: "lead".into(), agents: vec![a], links: vec![], custom_links: true }
}

// ------------------------------------------------------------------ projects and conversations

#[test]
fn c01_project_plugin_rule() {
    let info = ProjectInfo::default();
    assert!(info.plugins.is_none() && info.allows_plugin("tellus"), "an existing project keeps every plugin");
    let info = ProjectInfo { plugins: Some(vec!["tellus".into()]), ..Default::default() };
    assert!(info.allows_plugin("tellus") && !info.allows_plugin("pondus"));
}

#[test]
fn c02_legacy_conversation_moves_to_its_team_once() {
    let d = dir("c02");
    let p = Project::create(&d, ProjectInfo { name: "P".into(), team: "Alpha".into(), ..Default::default() }).unwrap();
    std::fs::write(p.dir.join("chat.json"), "{\"entries\":[]}").unwrap();
    std::fs::create_dir_all(p.dir.join("sessions")).unwrap();
    std::fs::write(p.dir.join("sessions/current"), "chat-1").unwrap();
    th_harness::engine::adopt_legacy_chat(&p.dir, "Alpha");
    assert!(p.dir.join("chat-team-alpha.json").is_file() && !p.dir.join("chat.json").exists());
    assert_eq!(std::fs::read_to_string(p.dir.join("sessions/team-alpha")).unwrap(), "chat-1");
    // a second time changes nothing, and another team does not take it
    std::fs::write(p.dir.join("chat.json"), "new").unwrap();
    th_harness::engine::adopt_legacy_chat(&p.dir, "Alpha");
    assert_eq!(std::fs::read_to_string(p.dir.join("chat.json")).unwrap(), "new");
}

#[test]
fn c03_two_teams_keep_their_own_conversations_in_one_project() {
    let d = dir("c03");
    let mk = |team: &str| {
        let p = Project::open_or_create(&d).unwrap();
        let (e, _) = engine(settings(vec![]), solo(team, &[]), p, Box::new(|me, req| Message::assistant(format!("{me} heard {} messages", req.messages.len()))));
        e.use_team_chat();
        e
    };
    let (a, b) = (mk("Alpha"), mk("Beta"));
    assert_eq!(send(&a, "one").0.unwrap(), "Alpha heard 1 messages");
    assert_eq!(send(&b, "one").0.unwrap(), "Beta heard 1 messages");
    // Alpha continues its own conversation, not Beta's
    assert_eq!(send(&a, "two").0.unwrap(), "Alpha heard 3 messages");
    assert_ne!(std::fs::read_to_string(d.join("sessions/team-alpha")).unwrap(), std::fs::read_to_string(d.join("sessions/team-beta")).unwrap());
}

#[test]
fn c04_bots_on_one_project_keep_their_own_conversations() {
    let d = dir("c04");
    Project::create(&d, ProjectInfo { name: "Shared".into(), ..Default::default() }).unwrap();
    let mk = |name: &str| {
        let mut b = Bot::new(name);
        b.call_any = false;
        b.project = d.display().to_string();
        let p = b.project().unwrap();
        let (e, _) = engine(settings(vec![]), b.team_spec(&[]), p, Box::new(|me, req| Message::assistant(format!("{me} heard {} messages", req.messages.len()))));
        e.use_bot_chat(&b);
        (e, b)
    };
    let ((a, ba), (b, bb)) = (mk("Ann"), mk("Ben"));
    assert_eq!(send(&a, "hi").0.unwrap(), "Ann heard 1 messages");
    assert_eq!(send(&b, "hi").0.unwrap(), "Ben heard 1 messages");
    assert_eq!(send(&a, "again").0.unwrap(), "Ann heard 3 messages", "Ann must not continue Ben's conversation");
    assert_ne!(ba.chat_file(), bb.chat_file());
}

#[test]
fn c05_bot_bundle_sets_up_its_project_and_follows_it_back() {
    let d = dir("c05");
    let mut b = Bot::new("Setter");
    b.project = d.display().to_string();
    b.standards = vec!["aci318".into()];
    b.plugins = vec!["tellus".into()];
    let p = b.project().unwrap();
    assert_eq!(p.info.standards, vec!["aci318".to_string()]);
    assert_eq!(p.info.plugins, Some(vec!["tellus".to_string()]));
    // the bot changed its project from the chat: the bundle follows
    let mut p2 = Project::open(&d).unwrap();
    p2.info.plugins = None;
    p2.info.standards.push("ec7".into());
    p2.save().unwrap();
    assert!(b.take_bundle(&p2));
    assert_eq!(b.plugins, vec!["*".to_string()]);
    assert_eq!(b.standards.len(), 2);
    assert!(!b.take_bundle(&p2), "nothing new the second time");
    // its own folder, when not linked
    let mut own = Bot::new("Own");
    own.id = format!("own-{}", std::process::id());
    let p3 = own.project().unwrap();
    assert!(p3.dir.starts_with(Bot::root()) && p3.dir.join("project.json").is_file());
}

// ------------------------------------------------------------------ delegation

#[test]
fn c06_bot_calls_a_team_through_its_lead_down_to_a_member() {
    let d = dir("c06");
    let mut b = Bot::new("Ada");
    b.call_any = false;
    b.mode = BotMode::Team;
    b.team = config::default_team().name;
    b.project = d.display().to_string();
    let team = b.team_spec(&[]);
    let lead = team.reachable("bot")[0].id.clone();
    let designer = team.agents.iter().find(|a| a.name.starts_with("Designer")).unwrap().id.clone();
    let (l2, d2) = (lead.clone(), designer.clone());
    let p = b.project().unwrap();
    let (e, play) = engine(settings(vec![]), team, p, Box::new(move |me, req| match (me, results(req).first()) {
        ("Ada", None) => call("delegate", json!({"agent": l2, "task": "Size footing F1"})),
        ("Ada", Some(r)) => Message::assistant(format!("Ada reports: {r}")),
        ("Design office lead", None) => call("delegate", json!({"agent": d2, "task": "Size footing F1, 1000 kN"})),
        ("Design office lead", Some(r)) => Message::assistant(format!("lead: {r}")),
        ("Designer (Design office)", _) => Message::assistant("B = 1.70 m"),
        (other, _) => Message::assistant(format!("unexpected agent {other}")),
    }));
    let (r, ev) = send(&e, "Design F1");
    let a = r.unwrap();
    assert!(a.contains("Ada reports") && a.contains("lead") && a.contains("B = 1.70 m"), "{a}");
    let s = starts(&ev);
    assert_eq!(s[0], ("Ada".into(), 0));
    assert!(s.contains(&("Design office lead".into(), 1)) && s.contains(&("Designer (Design office)".into(), 2)), "{s:?}");
    // the bot sees only its lead, the lead sees its members
    let seen = play.seen.lock().unwrap();
    assert!(seen.iter().any(|(w, t)| w == "Ada" && t.contains(&"delegate".to_string())));
    assert!(seen.iter().any(|(w, t)| w == "Designer (Design office)" && !t.contains(&"delegate".to_string())));
    drop(seen);
    let _ = designer;
}

#[test]
fn c07_bot_calls_another_bot() {
    let d = dir("c07");
    let mut a = Bot::new("Alpha");
    a.id = "alpha".into();
    a.call_any = false;
    a.calls = vec!["beta".into()];
    a.project = d.display().to_string();
    let mut b = Bot::new("Beta");
    b.id = "beta".into();
    b.call_any = false;
    b.character = "Beta only gives the settlement.".into();
    let all = vec![a.clone(), b.clone()];
    let p = a.project().unwrap();
    let (e, play) = engine(settings(vec![]), a.team_spec(&all), p, Box::new(|me, req| match (me, results(req).first()) {
        ("Alpha", None) => call("delegate", json!({"agent": "bot-beta", "task": "Settlement of F1?"})),
        ("Alpha", Some(r)) => Message::assistant(format!("Beta says {r}")),
        ("Beta", _) => Message::assistant(if req.system.contains("Beta only gives the settlement") { "19.5 mm" } else { "no character" }),
        (o, _) => Message::assistant(format!("unexpected {o}")),
    }));
    assert_eq!(send(&e, "go").0.unwrap(), "Beta says 19.5 mm");
    assert!(play.seen.lock().unwrap().iter().any(|(w, _)| w == "Beta"));
}

#[test]
fn c08_any_team_two_teams_with_the_same_agent_ids() {
    let d = dir("c08");
    let mut b = Bot::new("Pat");
    b.project = d.display().to_string();
    let office = config::default_team();
    let mut t2 = config::default_team();
    t2.name = "Team 2".into();
    let team = b.team_spec_with(&[b.clone()], &[office, t2]);
    let lead2 = team.agents.iter().find(|a| a.name == "Team 2 lead").unwrap().id.clone();
    let p = b.project().unwrap();
    let l2 = lead2.clone();
    let (e, _) = engine(settings(vec![]), team, p, Box::new(move |me, req| match (me, results(req).first()) {
        ("Pat", None) => call("delegate", json!({"agent": l2, "task": "who are you?"})),
        ("Pat", Some(r)) => Message::assistant(r.clone()),
        (o, _) => Message::assistant(format!("I am {o}")),
    }));
    let a = send(&e, "go").0.unwrap();
    assert!(a.contains("I am Team 2 lead"), "the call reached the wrong lead: {a}");
}

#[test]
fn c09_stop_ends_a_delegated_task_at_once() {
    let d = dir("c09");
    let mut team = solo("Boss", &[]);
    team.agents[0].can_delegate = true;
    team.agents.push(AgentSpec { id: "slow".into(), name: "Slow".into(), role: "slow".into(), instructions: "WORK SLOWLY".into(), max_turns: 5, ..Default::default() });
    team.links = vec![("lead".into(), "slow".into())];
    let p = Project::open_or_create(&d).unwrap();
    let (e, _) = engine(settings(vec![]), team, p, Box::new(|me, _| if me == "Boss" { call("delegate", json!({"agent": "slow", "task": "take ages"})) } else { Message::assistant("done") }));
    let cancel = Arc::new(AtomicBool::new(false));
    let c2 = cancel.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(400));
        c2.store(true, Ordering::Relaxed);
    });
    let (tx, _rx) = mpsc::channel();
    let t0 = Instant::now();
    let r = e.send(UserInput { text: "go".into(), attachments: vec![] }, &tx, &cancel);
    assert_eq!(r.unwrap_err(), "cancelled");
    assert!(t0.elapsed() < Duration::from_secs(4), "Stop took {:?}", t0.elapsed());
}

// ------------------------------------------------------------------ plugins and set-up

#[test]
fn c10_project_plugin_rule_hides_and_shows_plugin_tools() {
    let d = dir("c10");
    let tools_of = |plugins: Option<Vec<String>>| {
        let p = Project::create(&d.join(format!("{}", plugins.is_some())), ProjectInfo { name: "P".into(), plugins, ..Default::default() }).unwrap();
        let (e, play) = engine(settings(vec![fake_plugin("fake")]), solo("Solo", &["list_files"]), p, Box::new(|_, _| Message::assistant("ok")));
        send(&e, "hi").0.unwrap();
        let t = play.seen.lock().unwrap()[0].1.clone();
        t
    };
    assert!(tools_of(None).iter().any(|t| t == "fake__echo"), "every plugin by default");
    assert!(!tools_of(Some(vec![])).iter().any(|t| t.starts_with("fake__")), "the project allows none");
}

#[test]
fn c11_plugin_runs_in_the_project_folder() {
    let d = dir("c11");
    let p = Project::create(&d, ProjectInfo { name: "P".into(), ..Default::default() }).unwrap();
    let (e, _) = engine(settings(vec![fake_plugin("fake")]), solo("Solo", &[]), p.clone(), Box::new(|_, req| match results(req).first() {
        None => call("fake__pwd", json!({})),
        Some(r) => Message::assistant(r.clone()),
    }));
    let a = send(&e, "where?").0.unwrap();
    let want = p.dir.canonicalize().unwrap();
    assert!(a.contains(&want.display().to_string()), "plugin ran in {a}, not {}", want.display());
}

#[test]
fn c12_set_project_plugins_takes_effect_in_the_next_step() {
    let d = dir("c12");
    let p = Project::create(&d, ProjectInfo { name: "P".into(), plugins: Some(vec![]), ..Default::default() }).unwrap();
    let (e, _) = engine(settings(vec![fake_plugin("fake")]), solo("Setter", &["set_project_plugins", "list_plugins"]), p, Box::new(|_, req| match results(req).len() {
        0 => call("set_project_plugins", json!({"add": ["fake"]})),
        _ => Message::assistant(if req.tools.iter().any(|t| t.name == "fake__echo") { "fake is here" } else { "fake is missing" }),
    }));
    assert_eq!(send(&e, "switch fake on").0.unwrap(), "fake is here");
    assert_eq!(Project::open(&d).unwrap().info.plugins, Some(vec!["fake".to_string()]));
}

#[test]
fn c13_setup_tools_refuse_bad_names_and_change_nothing() {
    let d = dir("c13");
    let p = Project::create(&d, ProjectInfo { name: "P".into(), plugins: Some(vec!["fake".into()]), ..Default::default() }).unwrap();
    let (e, _) = engine(settings(vec![fake_plugin("fake")]), solo("S", &[]), p, Box::new(|_, _| Message::assistant("ok")));
    let err = e.domain.setup_tool("set_project_plugins", &json!({"remove": ["nonexistent"]})).unwrap_err();
    assert!(err.contains("no plugin named nonexistent") && err.contains("Nothing was changed"), "{err}");
    let err = e.domain.setup_tool("set_project_standards", &json!({})).unwrap_err();
    assert!(err.contains("add") && err.contains("remove"));
    assert_eq!(Project::open(&d).unwrap().info.plugins, Some(vec!["fake".to_string()]));
}

// ------------------------------------------------------------------ real programs, when present

fn tellus_plugin() -> Option<ServerConfig> {
    let exe = Path::new("/Applications/Tellus.app/Contents/MacOS/tellus");
    exe.is_file().then(|| ServerConfig { name: "tellus".into(), description: "Tellus".into(), command: exe.display().to_string(), args: vec!["--mcp".into()], enabled: true, timeout_s: 300, ..Default::default() })
}

#[test]
fn c14_real_tellus_saves_by_a_project_relative_path() {
    let Some(t) = tellus_plugin() else {
        eprintln!("Tellus is not installed: skipped");
        return;
    };
    let d = dir("c14");
    let p = Project::create(&d, ProjectInfo { name: "P".into(), ..Default::default() }).unwrap();
    let (e, _) = engine(settings(vec![t]), solo("Analyst", &[]), p.clone(), Box::new(|_, req| match results(req).len() {
        0 => call("tellus__save_project", json!({"path": "reports/check.tellus"})),
        _ => Message::assistant(results(req)[0].clone()),
    }));
    let a = send(&e, "save it").0.unwrap();
    assert!(p.dir.join("reports/check.tellus").is_file(), "Tellus did not save into the project: {a}");
}

fn app_exe() -> Option<PathBuf> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/release/tellushive");
    p.is_file().then_some(p)
}

/// The tool server a coding agent (Claude Code, Codex, agy) gets for one agent of a team.
fn serve_tools(exe: &Path, project: &Path, agent: &str, team_file: Option<&Path>) -> Result<Vec<String>, String> {
    use std::io::{BufRead, Write};
    let mut cmd = std::process::Command::new(exe);
    cmd.args(["--mcp", "--project", &project.display().to_string(), "--agent", agent, "--session", "conn-test"]);
    if let Some(f) = team_file {
        cmd.arg("--team-file").arg(f);
    }
    let mut child = cmd.stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().map_err(|e| e.to_string())?;
    let mut stdin = child.stdin.take().unwrap();
    for m in [json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}), json!({"jsonrpc":"2.0","method":"notifications/initialized"}), json!({"jsonrpc":"2.0","id":2,"method":"tools/list"})] {
        let _ = writeln!(stdin, "{m}");
    }
    let out = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for l in std::io::BufReader::new(out).lines().map_while(Result::ok) {
            if let Ok(v) = serde_json::from_str::<Value>(&l) {
                if v["id"] == 2 {
                    let _ = tx.send(v["result"]["tools"].as_array().map(|a| a.iter().filter_map(|t| t["name"].as_str().map(String::from)).collect::<Vec<_>>()).unwrap_or_default());
                }
            }
        }
    });
    let r = rx.recv_timeout(Duration::from_secs(20));
    drop(stdin);
    let _ = child.kill();
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    r.map_err(|_| format!("no tool list; stderr: {}", String::from_utf8_lossy(&out.stderr)))
}

#[test]
fn c15_tool_server_serves_a_bots_team_handed_over_by_the_app() {
    let Some(exe) = app_exe() else {
        eprintln!("no release build: skipped");
        return;
    };
    let d = dir("c15");
    let mut a = Bot::new("Alpha");
    a.id = "alpha15".into();
    a.call_any = false;
    a.mode = BotMode::Team;
    a.team = config::default_team().name;
    a.project = d.display().to_string();
    let p = a.project().unwrap();
    let team_file = home().join("c15-team.json");
    config::write_json(&team_file, &a.team_spec(&[]), false).unwrap();
    let tools = serve_tools(&exe, &p.dir, "bot", Some(&team_file)).unwrap();
    for t in ["delegate", "run_python", "set_project_standards", "list_plugins"] {
        assert!(tools.iter().any(|x| x == t), "{t} missing from {tools:?}");
    }
    // without the hand-over the bot's agent cannot be found, and the server says so
    let e = serve_tools(&exe, &p.dir, "bot", None).unwrap_err();
    assert!(e.contains("no agent 'bot'"), "{e}");
}

#[test]
fn c16_tool_server_for_a_team_member_on_its_project() {
    let Some(exe) = app_exe() else {
        eprintln!("no release build: skipped");
        return;
    };
    let d = dir("c16");
    let team = config::default_team();
    let p = Project::create(&d, ProjectInfo { name: "P".into(), team: team.name.clone(), ..Default::default() }).unwrap();
    let team_file = home().join("c16-team.json");
    config::write_json(&team_file, &team, false).unwrap();
    let tools = serve_tools(&exe, &p.dir, "designer", Some(&team_file)).unwrap();
    assert!(tools.iter().any(|t| t == "calc_sheet") && !tools.iter().any(|t| t == "delegate"), "a member that does not delegate: {tools:?}");
    let _ = HashMap::<(), ()>::new();
}
