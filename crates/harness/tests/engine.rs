//! The TellusHive engine on th-hive, with a scripted model in place of a provider: the
//! coordinator delegates to two team members in one reply, the chat receives the events,
//! and the conversation continues from the session log.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use serde_json::json;
use th_harness::th_hive::{fn_plugin, keys, LlmAdapter, LlmRequest, LlmService};
use th_harness::{config, Engine, Event, Plugins, Project, Settings, UserInput};
use th_llm::{CallError, Delta, FunctionCall, Message, Provider, Reply, ToolCall, Usage};

struct Scripted(AtomicUsize);

fn reply(m: Message) -> Reply {
    Reply { message: m, reasoning: None, finish_reason: "stop".into(), usage: Usage { prompt_tokens: 100, completion_tokens: 20, cached_tokens: 0 } }
}

impl LlmAdapter for Scripted {
    fn id(&self) -> &str {
        "scripted"
    }
    fn serves(&self, _: &Provider) -> bool {
        true
    }
    fn stream(&self, req: &LlmRequest, on: &mut dyn FnMut(Delta), _: &AtomicBool) -> Result<Reply, CallError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        let who = req.system.lines().next().unwrap_or("").to_string();
        if who.starts_with("You are Designer") || who.starts_with("You are Checker") {
            assert!(!req.tools.iter().any(|t| t.name == "delegate"), "members may not delegate");
            assert!(req.messages[0].text().contains("beam B1"));
            std::thread::sleep(Duration::from_millis(300));
            on(Delta::Text("phiMn = 210 kN m".into()));
            return Ok(reply(Message::assistant(if who.contains("Designer") { "phiMn = 210 kN m >= Mu = 150 kN m" } else { "CHECK: PASS" })));
        }
        assert!(who.starts_with("You are Coordinator"), "{who}");
        assert!(req.system.contains("# Your team") && req.system.contains("`designer`"));
        let tools = req.messages.iter().filter(|m| m.role == "tool").count();
        let users = req.messages.iter().filter(|m| m.role == "user").count();
        Ok(reply(match (users, tools) {
            (1, 0) => {
                let mut m = Message::assistant("");
                m.content = None;
                m.tool_calls = Some(vec![
                    ToolCall { id: "d1".into(), kind: "function".into(), function: FunctionCall { name: "delegate".into(), arguments: json!({"agent": "designer", "task": "Design beam B1, Mu = 150 kN m"}).to_string() } },
                    ToolCall { id: "d2".into(), kind: "function".into(), function: FunctionCall { name: "delegate".into(), arguments: json!({"agent": "checker", "task": "Check beam B1, Mu = 150 kN m"}).to_string() } },
                ]);
                m
            }
            (1, 2) => Message::assistant("B1 passes: phiMn = 210 kN m >= 150 kN m, checked."),
            (n, _) => Message::assistant(format!("I remember {} earlier messages.", req.messages.len() - 1 + 0 * n)),
        }))
    }
}

#[test]
fn coordinator_delegates_in_parallel_and_resumes() {
    let tmp = std::env::temp_dir().join(format!("th_engine_{}", std::process::id()));
    std::env::set_var("TELLUSHIVE_HOME", tmp.join("home"));
    let mut settings = Settings::default();
    settings.default_provider = "ollama".into(); // no key needed
    settings.default_model = "scripted".into();
    settings.plugins = vec![];
    let project = Project::open_or_create(&tmp.join("proj")).unwrap();
    let library = Arc::new(th_kb::Library::open(&tmp.join("lib")).unwrap());
    let engine = Engine::new(settings, config::default_team(), project, library, Arc::new(Plugins::default()));
    let script = Arc::new(Scripted(AtomicUsize::new(0)));
    let s2 = script.clone();
    engine.hive.ctx.plugin(fn_plugin("scripted-model", vec![keys::LLM], move |ctx| {
        ctx.effect(ctx.expect::<LlmService>(keys::LLM)?.register(s2.clone()));
        Ok(())
    }));

    let (tx, rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let t0 = Instant::now();
    let answer = engine.send(UserInput { text: "Design beam B1 for Mu = 150 kN m".into(), attachments: vec![] }, &tx, &cancel).unwrap();
    let took = t0.elapsed();
    assert_eq!(answer, "B1 passes: phiMn = 210 kN m >= 150 kN m, checked.");
    assert!(took < Duration::from_millis(550), "designer and checker should work at the same time: {took:?}");
    drop(tx);
    let events: Vec<Event> = rx.into_iter().collect();
    let starts: Vec<(String, usize)> = events.iter().filter_map(|e| if let Event::AgentStart { agent, depth, .. } = e { Some((agent.clone(), *depth)) } else { None }).collect();
    assert_eq!(starts[0], ("Coordinator".to_string(), 0));
    assert!(starts.contains(&("Designer".to_string(), 1)) && starts.contains(&("Checker".to_string(), 1)));
    assert_eq!(events.iter().filter(|e| matches!(e, Event::AgentDone { .. })).count(), 3);
    assert!(events.iter().any(|e| matches!(e, Event::Stream { agent, .. } if agent == "Designer")));
    assert!(events.iter().any(|e| matches!(e, Event::ToolResult { tool, ok: true, text, .. } if tool == "delegate" && text.contains("CHECK: PASS"))));
    assert!(matches!(events.last(), Some(Event::Finished { .. })));

    // the coordinator's session and the two children's sessions are on disk
    let dir = tmp.join("proj/sessions");
    let files: Vec<String> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).filter(|n| n.ends_with(".jsonl")).collect();
    assert_eq!(files.len(), 3, "{files:?}");
    assert!(files.iter().any(|f| f.starts_with("designer-")) && files.iter().any(|f| f.starts_with("checker-")));

    // a second engine (a restart) continues the same conversation from the log
    let mut settings = Settings::default();
    settings.default_provider = "ollama".into();
    settings.plugins = vec![];
    let project = Project::open_or_create(&tmp.join("proj")).unwrap();
    let engine = Engine::new(settings, config::default_team(), project, Arc::new(th_kb::Library::open(&tmp.join("lib")).unwrap()), Arc::new(Plugins::default()));
    let s3 = script.clone();
    engine.hive.ctx.plugin(fn_plugin("scripted-model", vec![keys::LLM], move |ctx| {
        ctx.effect(ctx.expect::<LlmService>(keys::LLM)?.register(s3.clone()));
        Ok(())
    }));
    let (tx, _rx) = mpsc::channel();
    let a = engine.send(UserInput { text: "And B2?".into(), attachments: vec![] }, &tx, &cancel).unwrap();
    // user, assistant(calls), 2 results, assistant = 5 earlier messages + this one
    assert_eq!(a, "I remember 5 earlier messages.");
    engine.new_conversation();
    let a = engine.send(UserInput { text: "fresh".into(), attachments: vec![] }, &tx, &cancel);
    assert!(a.is_ok());
    let _ = std::fs::remove_dir_all(tmp);
}
