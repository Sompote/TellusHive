//! End-to-end checks of the harness with a scripted model in place of a provider.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::json;
use th_hive::plugins::subagent::{ChildAgent, Subagents};
use th_hive::plugins::{base_bundle, BaseConfig};
use th_hive::*;
use th_llm::{CallError, Content, Delta, FunctionCall, Message, Provider, Reply, ToolCall, Usage};

type Script = dyn Fn(&LlmRequest, usize) -> Result<Reply, CallError> + Send + Sync;

/// A model adapter that answers from a script; it sees every request.
struct Scripted {
    calls: AtomicUsize,
    script: Box<Script>,
    seen: Mutex<Vec<LlmRequest>>,
}

impl LlmAdapter for Scripted {
    fn id(&self) -> &str {
        "scripted"
    }
    fn serves(&self, _: &Provider) -> bool {
        true
    }
    fn reasoning_passback(&self, p: &Provider) -> bool {
        p.base_url.contains("deepseek.com")
    }
    fn stream(&self, req: &LlmRequest, on: &mut dyn FnMut(Delta), _: &AtomicBool) -> Result<Reply, CallError> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        self.seen.lock().unwrap().push(req.clone());
        let r = (self.script)(req, n)?;
        on(Delta::Text(r.message.text()));
        Ok(r)
    }
}

fn text(t: &str) -> Reply {
    Reply { message: Message::assistant(t), reasoning: None, finish_reason: "stop".into(), usage: Usage { prompt_tokens: 10, completion_tokens: 5, cached_tokens: 0 } }
}

fn calls(cs: &[(&str, &str, serde_json::Value)]) -> Reply {
    let mut m = Message::assistant("");
    m.content = Some(Content::Text(String::new()));
    m.tool_calls = Some(cs.iter().map(|(id, n, a)| ToolCall { id: id.to_string(), kind: "function".into(), function: FunctionCall { name: n.to_string(), arguments: a.to_string() } }).collect());
    Reply { message: m, reasoning: Some("thinking".into()), finish_reason: "tool_calls".into(), usage: Usage::default() }
}

fn preset(id: &str) -> AgentPreset {
    AgentPreset { id: id.into(), name: id.to_uppercase(), role: format!("{id} role"), provider: Provider::deepseek(), model: "m".into(), temperature: Some(0.2), max_tokens: Some(1000), max_steps: 12, tools: ToolFilter::All, meta: json!({}) }
}

fn boot(cfg: BaseConfig, script: Box<Script>) -> (Hive, Arc<Scripted>) {
    let adapter = Arc::new(Scripted { calls: AtomicUsize::new(0), script, seen: Mutex::new(vec![]) });
    let a2 = adapter.clone();
    let mut plugins = base_bundle(&cfg);
    plugins.push(fn_plugin("test-adapter", vec![keys::LLM], move |ctx| {
        let llm = ctx.expect::<LlmService>(keys::LLM)?;
        ctx.effect(llm.register(a2.clone()));
        Ok(())
    }));
    plugins.push(fn_plugin("test-tools", vec![keys::TOOLS, keys::SUBAGENTS], |ctx| {
        let tools = ctx.expect::<ToolRegistry>(keys::TOOLS)?;
        tools.register(ctx, ToolDef::new("add", "adds", json!({"type":"object"}), |_, a| Ok(ToolOutput::ok(format!("{}", a["a"].as_i64().unwrap_or(0) + a["b"].as_i64().unwrap_or(0))))))?;
        tools.register(ctx, ToolDef::new("big", "long output", json!({"type":"object"}), |_, _| Ok(ToolOutput::ok("x".repeat(5000)))))?;
        let subs = ctx.expect::<Subagents>(keys::SUBAGENTS)?;
        subs.source(ctx, |a| if a.depth == 0 { vec![Arc::new(ChildAgent { preset: preset("worker") }) as Arc<dyn th_hive::plugins::subagent::SubagentProvider>] } else { vec![] });
        Ok(())
    }));
    (Hive::boot(plugins), adapter)
}

fn new_agent(h: &Hive, id: &str) -> Arc<Agent> {
    let s = h.sessions().create(SessionMeta { id: id.into(), agent: "lead".into(), ..Default::default() }).unwrap();
    h.agent(preset("lead"), s, Arc::new(AtomicBool::new(false)))
}

fn user(t: &str) -> Inbound {
    Inbound { text: t.into(), images: vec![], source: "user".into() }
}

#[test]
fn every_base_plugin_is_active() {
    let (h, _) = boot(BaseConfig::default(), Box::new(|_, _| Ok(text("hi"))));
    for (name, st, _) in h.ctx.plugins() {
        assert_eq!(st, PluginStatus::Active, "{name}");
    }
    let names = h.tools().names();
    for t in ["todo_write", "task_create", "task_update", "task_list", "add"] {
        assert!(names.contains(&t.to_string()), "{t}");
    }
}

#[test]
fn tool_step_then_answer_and_reasoning_passback() {
    let (h, ad) = boot(BaseConfig::default(), Box::new(|req, n| match n {
        0 => Ok(calls(&[("c1", "add", json!({"a": 2, "b": 3}))])),
        _ => {
            // the open turn's reasoning goes back to DeepSeek with the call
            let a = req.messages.iter().find(|m| m.role == "assistant").unwrap();
            assert_eq!(a.reasoning_content.as_deref(), Some("thinking"));
            let r = req.messages.iter().find(|m| m.role == "tool").unwrap();
            assert_eq!(r.text(), "5");
            Ok(text("The sum is 5."))
        }
    }));
    let a = new_agent(&h, "t1");
    let out = h.send(&a, user("add 2 and 3")).unwrap();
    assert_eq!(out, TurnOutcome { text: "The sum is 5.".into(), reason: "completed".into() });
    assert_eq!(ad.calls.load(Ordering::SeqCst), 2);
    let kinds: Vec<String> = a.session.events().iter().map(|e| serde_json::to_value(&e.body).unwrap()["type"].as_str().unwrap().to_string()).collect();
    assert_eq!(
        kinds,
        ["session/start", "turn/start", "step/start", "system/message", "user/message", "assistant/message", "tool/call", "tool/result", "step/end", "step/start", "assistant/message", "step/end", "turn/end"]
    );
    // the system prompt is logged once while it does not change
    assert_eq!(a.session.events().iter().filter(|e| matches!(e.body, EventBody::SystemMessage { .. })).count(), 1);
}

#[test]
fn delegations_in_one_reply_run_concurrently() {
    let (h, _) = boot(BaseConfig::default(), Box::new(|req, _| {
        if req.messages[0].text().starts_with("Task from LEAD") {
            std::thread::sleep(Duration::from_millis(300));
            return Ok(text(&format!("done: {}", req.messages[0].text().lines().nth(1).unwrap_or(""))));
        }
        if req.messages.iter().any(|m| m.role == "tool") {
            return Ok(text("both finished"));
        }
        Ok(calls(&[("d1", "delegate", json!({"agent": "worker", "task": "A"})), ("d2", "delegate", json!({"agent": "worker", "task": "B"}))]))
    }));
    let a = new_agent(&h, "t2");
    let t0 = Instant::now();
    let out = h.send(&a, user("do A and B")).unwrap();
    assert_eq!(out.text, "both finished");
    assert!(t0.elapsed() < Duration::from_millis(550), "ran one after another: {:?}", t0.elapsed());
    let results: Vec<String> = a.session.events().iter().filter_map(|e| if let EventBody::ToolResult { text, .. } = &e.body { Some(text.clone()) } else { None }).collect();
    assert_eq!(results, ["done: A", "done: B"]);
    // each child ran on its own session linked to the parent
    assert!(h.agents().children(&a.id).is_empty(), "children are forgotten after they finish");
}

#[test]
fn retry_then_continuation_nudge() {
    let (h, _) = boot(BaseConfig::default(), Box::new(|_, n| match n {
        0 => Err(CallError { message: "HTTP 503".into(), status: Some(503), overflow: false, cancelled: false }),
        1 => Ok(text("Let me run the check now.")),
        _ => Ok(text("CHECK: PASS")),
    }));
    // keep the test fast: shorten the retry wait by answering first
    h.ctx.plugin(fn_plugin("fast-retry", vec![keys::LOOP], |ctx| {
        let lp = ctx.expect::<AgentLoop>(keys::LOOP)?;
        ctx.effect(lp.request_error.on(true, |re, next| if re.error.status == Some(503) { ErrorAction::Retry(0) } else { next(re) }));
        Ok(())
    }));
    let a = new_agent(&h, "t3");
    let out = h.send(&a, user("check it")).unwrap();
    assert_eq!(out.text, "CHECK: PASS");
    let evs = a.session.events();
    assert!(evs.iter().any(|e| matches!(&e.body, EventBody::AssistantAttempt { error, .. } if error == "HTTP 503")));
    assert!(evs.iter().any(|e| matches!(&e.body, EventBody::UserMessage { source, .. } if source == "plugin:turn-stopping")));
}

#[test]
fn repeat_reminder_and_spill_and_prune() {
    let dir = std::env::temp_dir().join(format!("hive_spill_{}", std::process::id()));
    let d2 = dir.clone();
    let cfg = BaseConfig { spill_chars: 1000, spill_locator: Some(Arc::new(move |_| Some((d2.clone(), "spill".into())))), ..Default::default() };
    let (h, _) = boot(cfg, Box::new(|_, n| if n < 3 { Ok(calls(&[(&format!("c{n}"), "big", json!({})) ])) } else { Ok(text("ok")) }));
    let a = new_agent(&h, "t4");
    h.send(&a, user("go")).unwrap();
    let evs = a.session.events();
    let r: Vec<&String> = evs.iter().filter_map(|e| if let EventBody::ToolResult { text, .. } = &e.body { Some(text) } else { None }).collect();
    assert!(r[0].contains("5000 characters in all") && r[0].len() < 1000);
    assert!(std::fs::read_to_string(dir.join("big_c0.txt")).unwrap().len() == 5000);
    assert!(evs.iter().any(|e| matches!(&e.body, EventBody::UserMessage { source, text, .. } if source == "plugin:repeat-tool-reminder" && text.contains("3 times"))));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn compaction_summarises_old_surface() {
    let cfg = BaseConfig { compaction_budget_tokens: 300, compaction_keep_recent: 2, spill_chars: 0, ..Default::default() };
    let (h, ad) = boot(cfg, Box::new(|req, _| {
        if req.purpose == "compaction" {
            return Ok(text("SUMMARY: earlier work"));
        }
        Ok(text(&"answer ".repeat(80)))
    }));
    let a = new_agent(&h, "t5");
    for k in 0..4 {
        h.send(&a, user(&format!("question {k} {}", "words ".repeat(60)))).unwrap();
    }
    let evs = a.session.events();
    assert!(evs.iter().any(|e| matches!(e.body, EventBody::CompactionSummary { .. })));
    let last = ad.seen.lock().unwrap().iter().rev().find(|r| r.purpose == "agent").unwrap().clone();
    assert!(last.messages[0].text().contains("SUMMARY: earlier work"), "{:?}", last.messages[0].text());
    assert!(last.messages.len() <= 4);
}

#[test]
fn team_board_is_shared_and_rejects_cycles() {
    let (h, _) = boot(BaseConfig::default(), Box::new(|req, _| {
        if req.messages[0].text().starts_with("Task from LEAD") {
            if req.messages.iter().any(|m| m.role == "tool") {
                return Ok(text("updated"));
            }
            return Ok(calls(&[("w1", "task_update", json!({"id": "task-1", "status": "completed", "note": "B1 passes", "revision": 1}))]));
        }
        let n_tools = req.messages.iter().filter(|m| m.role == "tool").count();
        match n_tools {
            0 => Ok(calls(&[("t1", "task_create", json!({"title": "Design B1"})), ("t2", "task_create", json!({"title": "Check B1", "blocked_by": ["task-1"]}))])),
            2 => Ok(calls(&[("t3", "task_update", json!({"id": "task-1", "blocked_by": ["task-2"]}))])),
            3 => Ok(calls(&[("t4", "delegate", json!({"agent": "worker", "task": "finish task-1"}))])),
            4 => Ok(calls(&[("t5", "task_list", json!({}))])),
            _ => Ok(text("board read")),
        }
    }));
    let a = new_agent(&h, "t6");
    h.send(&a, user("plan")).unwrap();
    let results: Vec<String> = a.session.events().iter().filter_map(|e| if let EventBody::ToolResult { text, .. } = &e.body { Some(text.clone()) } else { None }).collect();
    assert!(results[2].contains("cycle"), "{}", results[2]);
    assert!(results[4].contains("task-1 [completed] Design B1"), "{}", results[4]);
    assert!(results[4].contains("WORKER: B1 passes"), "{}", results[4]);
}

#[test]
fn resumed_session_keeps_history() {
    let dir = std::env::temp_dir().join(format!("hive_resume_{}", std::process::id()));
    let cfg = BaseConfig { sessions_dir: Some(dir.clone()), ..Default::default() };
    let (h, _) = boot(cfg.clone(), Box::new(|req, _| Ok(text(&format!("seen {} messages", req.messages.len())))));
    let a = new_agent(&h, "main");
    h.send(&a, user("one")).unwrap();
    drop(h);
    let (h, _) = boot(cfg, Box::new(|req, _| Ok(text(&format!("seen {} messages", req.messages.len())))));
    let s = h.sessions().open("main").unwrap();
    let a = h.agent(preset("lead"), s, Arc::new(AtomicBool::new(false)));
    assert_eq!(h.send(&a, user("two")).unwrap().text, "seen 3 messages");
    let _ = std::fs::remove_dir_all(dir);
}
