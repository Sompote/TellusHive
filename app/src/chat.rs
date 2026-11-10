//! The chat with the coordinator: the transcript (the engineer's messages, each agent's
//! replies, delegations, tool calls folded into chips, pictures and artifacts inline), the
//! input box with attachments (button, paste of a path, or drag-and-drop) and the worker
//! thread that runs the engine.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;

use th_harness::{ArtifactKind, Engine, Event, UserInput};
use eframe::egui::{self, Color32, RichText};
use serde::{Deserialize, Serialize};

use crate::chrome::{self, tokens};
use crate::icons::{self, Icon};
use crate::markdown;
use crate::theme::medium_font;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "entry", rename_all = "snake_case")]
pub enum Entry {
    User { text: String, attachments: Vec<PathBuf> },
    Start { agent: String, depth: usize, task: String },
    Text { agent: String, depth: usize, text: String },
    Thinking { agent: String, text: String },
    Tool { agent: String, depth: usize, id: String, tool: String, args: String, result: Option<(bool, String)> },
    Artifact { kind: ArtifactKind, path: PathBuf },
    Done { agent: String, depth: usize },
    Error { text: String },
    /// Token use of one task, added when the task ends.
    Usage { report: UsageReport },
    /// An agent changed the project's model in an analysis program (once per program and task).
    Model { plugin: String, path: PathBuf },
}

/// Tokens one agent used with one model.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AgentUsage {
    pub agent: String,
    pub model: String,
    pub calls: u64,
    pub prompt: u64,
    pub completion: u64,
}

/// Token use of one task (one message to the coordinator and everything it set off).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct UsageReport {
    pub agents: Vec<AgentUsage>,
    pub seconds: u64,
    /// "finished", "stopped" or "failed".
    pub outcome: String,
}

impl UsageReport {
    pub fn add(&mut self, agent: &str, model: &str, prompt: u64, completion: u64) {
        match self.agents.iter_mut().find(|a| a.agent == agent && a.model == model) {
            Some(a) => {
                a.calls += 1;
                a.prompt += prompt;
                a.completion += completion;
            }
            None => self.agents.push(AgentUsage { agent: agent.into(), model: model.into(), calls: 1, prompt, completion }),
        }
    }
    /// (model calls, input tokens, output tokens)
    pub fn totals(&self) -> (u64, u64, u64) {
        self.agents.iter().fold((0, 0, 0), |(n, p, c), a| (n + a.calls, p + a.prompt, c + a.completion))
    }
    /// One line: "6 calls · 41.2k in · 3.1k out · 44.3k total".
    pub fn line(&self) -> String {
        let (n, p, c) = self.totals();
        format!("{n} model call{} · {} in · {} out · {} total", if n == 1 { "" } else { "s" }, fmt_tokens(p), fmt_tokens(c), fmt_tokens(p + c))
    }
    /// A plain-text table for the terminal.
    pub fn table(&self) -> String {
        let mut s = format!("{:<22} {:<36} {:>5} {:>9} {:>9} {:>9}\n", "Agent", "Model", "Calls", "In", "Out", "Total");
        for a in &self.agents {
            s.push_str(&format!("{:<22} {:<36} {:>5} {:>9} {:>9} {:>9}\n", a.agent, a.model, a.calls, fmt_tokens(a.prompt), fmt_tokens(a.completion), fmt_tokens(a.prompt + a.completion)));
        }
        let (n, p, c) = self.totals();
        s.push_str(&format!("{:<22} {:<36} {:>5} {:>9} {:>9} {:>9}", "Total", "", n, fmt_tokens(p), fmt_tokens(c), fmt_tokens(p + c)));
        s
    }
}

/// 950, 12.3k, 1.24M
pub fn fmt_tokens(n: u64) -> String {
    if n < 1_000 {
        n.to_string()
    } else if n < 1_000_000 {
        format!("{:.1}k", n as f64 / 1e3)
    } else {
        format!("{:.2}M", n as f64 / 1e6)
    }
}

#[derive(Default, Serialize, Deserialize)]
pub struct Saved {
    pub entries: Vec<Entry>,
    pub tokens: (u64, u64),
    /// Shows the raw tool calls, the agents' reasoning and the token counts in the transcript.
    #[serde(default)]
    pub details: bool,
}

/// One agent's task going on now (the "who is working" panel).
#[derive(Debug, Clone)]
pub struct Live {
    run: u64,
    pub agent: String,
    pub depth: usize,
    pub task: String,
    since: std::time::Instant,
    /// What it is doing, in plain words.
    pub activity: String,
    /// True while it only waits for the team members it gave tasks to.
    pub waiting: bool,
}

enum WorkerMsg {
    Event(Event),
    Done(Result<String, String>),
}

pub struct ChatView {
    pub saved: Saved,
    input: String,
    attachments: Vec<PathBuf>,
    rx: Option<Receiver<WorkerMsg>>,
    cancel: Arc<AtomicBool>,
    /// agent name → depth of its current run (for indentation)
    depth: std::collections::HashMap<String, usize>,
    pub busy_since: Option<std::time::Instant>,
    /// Artifacts that arrived this frame (the app refreshes its list and image cache).
    pub new_artifacts: Vec<(ArtifactKind, PathBuf)>,
    pub open_request: Option<PathBuf>,
    scroll_to_end: bool,
    /// Token use of the task running now.
    pub run_usage: UsageReport,
    /// The agents working now, in the order they started.
    pub live: Vec<Live>,
    /// The agents that ended their part of the running (or last) task: name and whether it went well.
    finished: Vec<(String, bool)>,
    /// Agent id → name, for "Waiting for Designer".
    names: std::collections::HashMap<String, String>,
    /// Models the programs saved since the last frame (the app opens or refreshes their windows).
    pub model_saves: Vec<(String, PathBuf)>,
    /// "Open in <program>" pressed in the chat.
    pub open_model: Option<String>,
    /// The programs whose model card is already in the transcript for the running task.
    announced: std::collections::HashSet<String>,
    /// Text streamed so far by each agent run (the live "Writing: …" line).
    streaming: std::collections::HashMap<u64, String>,
}

impl ChatView {
    pub fn new(saved: Saved) -> Self {
        ChatView { saved, input: String::new(), attachments: vec![], rx: None, cancel: Arc::new(AtomicBool::new(false)), depth: Default::default(), busy_since: None, new_artifacts: vec![], open_request: None, scroll_to_end: true, run_usage: UsageReport::default(), live: vec![], finished: vec![], names: Default::default(), model_saves: vec![], open_model: None, announced: Default::default(), streaming: Default::default() }
    }

    /// For the UI screenshots (TELLUSHIVE_SHOT_BUSY=1): a task that looks as if it were running,
    /// with the first agents of the team at work, so the live panels can be checked.
    pub fn demo_busy(&mut self, agents: &[String]) {
        let (_tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        let now = std::time::Instant::now();
        self.busy_since = now.checked_sub(std::time::Duration::from_secs(134));
        let acts = [("Waiting for Designer and Geotechnical analyst", true, 134), ("Calculating: B1 beam flexure and shear", false, 58), ("Searching the standards: bearing resistance of spread footings", false, 41), ("Tellus: generate mesh", false, 17)];
        for (k, (act, waiting, secs)) in acts.iter().enumerate() {
            let Some(agent) = agents.get(if k == 3 { 5 } else { k }) else { continue };
            self.live.push(Live { run: 1000 + k as u64, agent: agent.clone(), depth: usize::from(k > 0), task: "Design the ground beam B1 to ACI 318-19 with the loads given".into(), since: now.checked_sub(std::time::Duration::from_secs(*secs)).unwrap_or(now), activity: act.to_string(), waiting: *waiting });
        }
        if let Some(a) = agents.get(3) {
            self.finished.push((a.clone(), true));
        }
    }

    pub fn busy(&self) -> bool {
        self.rx.is_some()
    }

    pub fn load(dir: &std::path::Path) -> Saved {
        std::fs::read_to_string(dir.join("chat.json")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self, dir: &std::path::Path) {
        if let Ok(s) = serde_json::to_string(&self.saved) {
            let tmp = dir.join("chat.json.tmp");
            if std::fs::write(&tmp, s).is_ok() {
                let _ = std::fs::rename(tmp, dir.join("chat.json"));
            }
        }
    }

    pub fn clear(&mut self) {
        if !self.busy() {
            self.saved = Saved::default();
        }
    }

    pub fn add_attachment(&mut self, p: PathBuf) {
        if !self.attachments.contains(&p) {
            self.attachments.push(p);
        }
    }

    fn send(&mut self, engine: &Arc<Engine>, ctx: &egui::Context) {
        let text = self.input.trim().to_string();
        if text.is_empty() && self.attachments.is_empty() {
            return;
        }
        // copy attachments into the project
        let project = engine.project.read().map(|p| p.clone());
        let mut atts = vec![];
        if let Ok(p) = &project {
            for a in self.attachments.drain(..) {
                match p.attach(&a) {
                    Ok(dst) => atts.push(dst),
                    Err(e) => self.saved.entries.push(Entry::Error { text: format!("attachment: {e}") }),
                }
            }
        }
        self.saved.entries.push(Entry::User { text: text.clone(), attachments: atts.clone() });
        self.input.clear();
        self.cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        self.busy_since = Some(std::time::Instant::now());
        self.run_usage = UsageReport::default();
        self.scroll_to_end = true;
        self.live.clear();
        self.finished.clear();
        self.announced.clear();
        self.names = engine.team.read().map(|t| t.agents.iter().map(|a| (a.id.clone(), a.name.clone())).collect()).unwrap_or_default();
        let engine = engine.clone();
        let cancel = self.cancel.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let (etx, erx) = mpsc::channel::<Event>();
            let fwd_tx = tx.clone();
            let fwd_ctx = ctx.clone();
            let fwd = std::thread::spawn(move || {
                for e in erx {
                    let _ = fwd_tx.send(WorkerMsg::Event(e));
                    fwd_ctx.request_repaint();
                }
            });
            let r = engine.send(UserInput { text, attachments: atts }, &etx, &cancel);
            drop(etx);
            let _ = fwd.join();
            let _ = tx.send(WorkerMsg::Done(r));
            ctx.request_repaint();
        });
    }

    pub fn stop(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Takes the worker's events into the transcript. Returns true when a run finished.
    pub fn poll(&mut self) -> bool {
        let Some(rx) = &self.rx else { return false };
        let mut finished = false;
        let msgs: Vec<WorkerMsg> = rx.try_iter().collect();
        for m in msgs {
            self.scroll_to_end = true;
            match m {
                WorkerMsg::Event(e) => self.on_event(e),
                WorkerMsg::Done(r) => {
                    let mut report = std::mem::take(&mut self.run_usage);
                    report.seconds = self.busy_since.map(|b| b.elapsed().as_secs()).unwrap_or(0);
                    report.outcome = match &r {
                        Ok(_) => "finished".into(),
                        Err(e) if e == "cancelled" => "stopped".into(),
                        Err(_) => "failed".into(),
                    };
                    if let Err(e) = r {
                        if !self.saved.entries.iter().rev().take(3).any(|x| matches!(x, Entry::Error { text } if *text == e)) {
                            self.saved.entries.push(Entry::Error { text: e });
                        }
                    }
                    if report.totals().0 > 0 {
                        self.saved.entries.push(Entry::Usage { report });
                    }
                    finished = true;
                }
            }
        }
        if finished {
            self.rx = None;
            self.busy_since = None;
            self.live.clear();
        }
        finished
    }

    fn on_event(&mut self, e: Event) {
        let d = |s: &Self, a: &str| s.depth.get(a).copied().unwrap_or(0);
        match e {
            Event::AgentStart { run, agent, depth, task } => {
                self.depth.insert(agent.clone(), depth);
                self.finished.retain(|f| f.0 != agent);
                self.live.push(Live { run, agent: agent.clone(), depth, task: task.clone(), since: std::time::Instant::now(), activity: "Reading the task".into(), waiting: false });
                if depth > 0 {
                    self.saved.entries.push(Entry::Start { agent, depth, task });
                }
            }
            Event::Thinking { run, agent, text } => {
                self.set_activity(run, "Thinking it through".into(), false);
                self.saved.entries.push(Entry::Thinking { agent, text });
            }
            Event::Stream { run, kind, text, .. } => {
                let activity = match kind.as_str() {
                    "reasoning" => "Thinking it through".to_string(),
                    "tool" => format!("Preparing {}", friendly(&text, "{}", &self.names, true).to_lowercase()),
                    _ => {
                        let buf = self.streaming.entry(run).or_default();
                        buf.push_str(&text);
                        let tail: String = buf.chars().rev().take(70).collect::<Vec<_>>().into_iter().rev().collect();
                        format!("Writing: …{}", tail.replace('\n', " "))
                    }
                };
                if let Some(l) = self.live.iter().find(|l| l.run == run) {
                    if l.waiting {
                        return;
                    }
                }
                self.set_activity(run, activity, false);
            }
            Event::Text { run, agent, text } => {
                self.streaming.remove(&run);
                self.set_activity(run, "Writing up".into(), false);
                let depth = d(self, &agent);
                self.saved.entries.push(Entry::Text { agent, depth, text });
            }
            Event::ToolCall { run, agent, id, tool, args } => {
                self.set_activity(run, friendly(&tool, &args, &self.names, true), tool.starts_with("delegate"));
                let depth = d(self, &agent);
                self.saved.entries.push(Entry::Tool { agent, depth, id, tool, args, result: None });
            }
            Event::ToolResult { run, id, ok, text, agent, .. } => {
                // with several calls in one reply the agent is still busy until the last one returns
                let open = self.saved.entries.iter().filter(|x| matches!(x, Entry::Tool { agent: a, result: None, .. } if *a == agent)).count();
                if open <= 1 {
                    self.set_activity(run, "Thinking it through".into(), false);
                }
                if let Some(Entry::Tool { result, .. }) = self.saved.entries.iter_mut().rev().find(|x| matches!(x, Entry::Tool { id: i, agent: a, result: None, .. } if *i == id && *a == agent)) {
                    *result = Some((ok, text));
                }
            }
            Event::Artifact { kind, path, .. } => {
                self.new_artifacts.push((kind, path.clone()));
                self.saved.entries.push(Entry::Artifact { kind, path });
            }
            Event::AgentDone { run, agent, .. } => {
                self.end_live(run, &agent, true);
                let depth = d(self, &agent);
                if depth > 0 {
                    self.saved.entries.push(Entry::Done { agent, depth });
                }
            }
            Event::Usage { agent, model, prompt, completion, .. } => {
                self.saved.tokens.0 += prompt;
                self.saved.tokens.1 += completion;
                self.run_usage.add(&agent, &model, prompt, completion);
            }
            Event::Error { run, agent, text } => {
                // run 0 marks a warning that does not end an agent's task (a plugin that did not start)
                if run != 0 {
                    self.end_live(run, &agent, false);
                }
                self.saved.entries.push(Entry::Error { text });
            }
            Event::Model { plugin, path, .. } => {
                self.model_saves.push((plugin.clone(), path.clone()));
                if self.announced.insert(plugin.clone()) {
                    self.saved.entries.push(Entry::Model { plugin, path });
                }
            }
            Event::Finished { .. } => {}
        }
    }

    fn set_activity(&mut self, run: u64, activity: String, waiting: bool) {
        if let Some(l) = self.live.iter_mut().find(|l| l.run == run) {
            l.activity = activity;
            l.waiting = waiting;
        }
    }

    fn end_live(&mut self, run: u64, agent: &str, ok: bool) {
        self.live.retain(|l| l.run != run);
        if !self.live.iter().any(|l| l.agent == agent) {
            self.finished.retain(|f| f.0 != agent);
            self.finished.push((agent.to_string(), ok));
        }
    }

    pub fn set_input(&mut self, s: &str) {
        self.input = s.to_string();
    }

    /// The agents doing work now (not those that only wait for their team).
    fn active(&self) -> Vec<&Live> {
        self.live.iter().filter(|l| !l.waiting).collect()
    }

    /// Who is working now, short, for the top bar: "Designer" or "3 working".
    pub fn current_agent(&self) -> String {
        let a = self.active();
        match a.len() {
            0 => self.live.last().map(|l| l.agent.clone()).unwrap_or_else(|| "Starting".into()),
            1 => a[0].agent.clone(),
            n => format!("{n} working"),
        }
    }

    /// What is happening now, for the status bar.
    pub fn current_activity(&self) -> String {
        let a = self.active();
        match a.len() {
            0 => "Getting started".into(),
            1 => format!("{}: {}", a[0].agent, a[0].activity),
            2 | 3 => format!("{} are working at the same time", join_names(&a.iter().map(|l| l.agent.clone()).collect::<Vec<_>>())),
            n => format!("{n} team members are working at the same time"),
        }
    }

    /// The team list of the Design office: every agent with what it is doing now.
    pub fn roster_ui(&self, ui: &mut egui::Ui, agents: &[(String, Color32)]) {
        let t = tokens(ui);
        let time = ui.input(|i| i.time);
        for (k, (name, colour)) in agents.iter().enumerate() {
            let mine: Vec<&Live> = self.live.iter().filter(|l| &l.agent == name).collect();
            let working = mine.iter().any(|l| !l.waiting);
            let done = self.finished.iter().find(|f| &f.0 == name).map(|f| f.1);
            let r = ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 7.0;
                let (r, _) = ui.allocate_exact_size(egui::vec2(12.0, 20.0), egui::Sense::hover());
                if working {
                    let pulse = 0.5 + 0.5 * ((time * 3.2 + k as f64).sin() as f32);
                    ui.painter().circle_filled(r.center(), 5.5 + 2.0 * pulse, colour.gamma_multiply(0.25));
                    ui.painter().circle_filled(r.center(), 4.0, *colour);
                } else if !mine.is_empty() {
                    ui.painter().circle_stroke(r.center(), 3.5, egui::Stroke::new(1.5_f32, *colour));
                } else {
                    ui.painter().circle_filled(r.center(), 3.5, colour.gamma_multiply(if self.busy() { 0.35 } else { 0.8 }));
                }
                let idle = mine.is_empty() && self.busy();
                ui.label(RichText::new(name).font(medium_font(12.5)).color(if idle { t.text_dim } else { t.text }));
                let (status, col) = match (mine.first(), done) {
                    (Some(_), _) if mine.len() > 1 => (format!("{} tasks · {}", mine.len(), mine.iter().find(|l| !l.waiting).unwrap_or(&mine[0]).activity), t.text_dim),
                    (Some(l), _) => (l.activity.clone(), t.text_dim),
                    (None, Some(true)) => ("done".to_string(), t.ok),
                    (None, Some(false)) => ("stopped on a problem".to_string(), t.error),
                    (None, None) => (String::new(), t.text_faint),
                };
                if !status.is_empty() {
                    ui.add(egui::Label::new(RichText::new(status).size(11.5).color(col)).truncate());
                }
            });
            if let Some(l) = mine.first() {
                let secs = l.since.elapsed().as_secs();
                r.response.on_hover_text(format!("{name} · {}:{:02}\n{}\n\nTask: {}", secs / 60, secs % 60, l.activity, first_line(&l.task, 400)));
            }
        }
    }

    /// The "working now" card above the message box: one line per agent at work.
    fn working_ui(&self, ui: &mut egui::Ui, colours: &dyn Fn(&str) -> Color32, w: f32) {
        let t = tokens(ui);
        let time = ui.input(|i| i.time);
        egui::Frame::NONE.fill(t.accent_soft).corner_radius(10).inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| {
            ui.set_width(w - 24.0);
            let n = self.active().len();
            let secs = self.busy_since.map(|b| b.elapsed().as_secs()).unwrap_or(0);
            ui.horizontal(|ui| {
                let head = match n {
                    0 => "Getting started".to_string(),
                    1 => "1 team member is working".to_string(),
                    n => format!("{n} team members are working at the same time"),
                };
                ui.label(RichText::new(head).font(medium_font(12.5)).color(t.text));
                ui.label(RichText::new(format!("{}:{:02}", secs / 60, secs % 60)).monospace().size(11.5).color(t.text_dim));
            });
            for (k, l) in self.live.iter().enumerate().take(8) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 7.0;
                    ui.add_space(16.0 * l.depth as f32);
                    let c = colours(&l.agent);
                    let (r, _) = ui.allocate_exact_size(egui::vec2(12.0, 18.0), egui::Sense::hover());
                    if l.waiting {
                        ui.painter().circle_stroke(r.center(), 3.5, egui::Stroke::new(1.5_f32, c));
                    } else {
                        let pulse = 0.5 + 0.5 * ((time * 3.2 + k as f64).sin() as f32);
                        ui.painter().circle_filled(r.center(), 5.5 + 2.0 * pulse, c.gamma_multiply(0.25));
                        ui.painter().circle_filled(r.center(), 4.0, c);
                    }
                    ui.label(RichText::new(&l.agent).font(medium_font(12.5)).color(t.text));
                    let s = l.since.elapsed().as_secs();
                    ui.label(RichText::new(format!("{}:{:02}", s / 60, s % 60)).monospace().size(11.0).color(t.text_faint));
                    ui.add(egui::Label::new(RichText::new(&l.activity).size(12.0).color(t.text_dim)).truncate()).on_hover_text(format!("Task: {}", first_line(&l.task, 400)));
                });
            }
            if self.live.len() > 8 {
                ui.label(RichText::new(format!("and {} more", self.live.len() - 8)).size(11.5).color(t.text_faint));
            }
        });
        ui.add_space(6.0);
        // the timers and the pulse move on their own
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(60));
    }

    /// A run of one agent's steps in the simple view: one line that opens into the list.
    fn steps_ui(&mut self, ui: &mut egui::Ui, from: usize, to: usize) {
        let t = tokens(ui);
        let mut agent = String::new();
        let mut depth = 0;
        let mut rows: Vec<(String, Option<(bool, String)>)> = vec![];
        for e in &self.saved.entries[from..to] {
            if let Entry::Tool { agent: a, depth: d, tool, args, result, .. } = e {
                agent = a.clone();
                depth = *d;
                // a handed-over task shows as its own line ("→ Designer"), so only a failed hand-over is listed
                if tool.starts_with("delegate") && !matches!(result, Some((false, _))) {
                    continue;
                }
                rows.push((friendly(tool, args, &self.names, false), result.clone()));
            }
        }
        if rows.is_empty() {
            return;
        }
        ui.horizontal(|ui| {
            ui.add_space(22.0 * depth as f32 + 6.0);
            if rows.len() == 1 {
                step_line(ui, &rows[0], &t);
                return;
            }
            let pending = rows.iter().rev().find(|r| r.1.is_none()).map(|r| r.0.clone());
            let fails = rows.iter().filter(|r| matches!(r.1, Some((false, _)))).count();
            let (mark, col) = if pending.is_some() { ("…", t.text_dim) } else if fails > 0 { ("!", t.warn) } else { ("✓", t.ok) };
            let last = pending.unwrap_or_else(|| rows.last().map(|r| r.0.clone()).unwrap_or_default());
            let problems = if fails > 0 { format!(" · {fails} did not work") } else { String::new() };
            let title = RichText::new(format!("{mark}  {agent} · {} steps{problems} · {}", rows.len(), short(&last, 70))).size(12.0).color(col);
            egui::CollapsingHeader::new(title).id_salt(("steps", from)).show(ui, |ui| {
                for r in &rows {
                    step_line(ui, r, &t);
                }
            });
        });
    }

    /// The last error of the last run (none once a new message is sent).
    pub fn last_error(&self) -> Option<String> {
        for e in self.saved.entries.iter().rev() {
            match e {
                Entry::Error { text } => return Some(text.clone()),
                Entry::User { .. } => return None,
                _ => {}
            }
        }
        None
    }

    /// The conversation and its message box. `full` is the Design office (wide, centred
    /// column); otherwise the docked assistant.
    pub fn ui(&mut self, ui: &mut egui::Ui, engine: &Arc<Engine>, colours: &dyn Fn(&str) -> Color32, full: bool) {
        let t = tokens(ui);
        if self.names.is_empty() {
            self.names = engine.team.read().map(|t| t.agents.iter().map(|a| (a.id.clone(), a.name.clone())).collect()).unwrap_or_default();
        }
        let pad: i8 = if full { 24 } else { 0 };
        egui::TopBottomPanel::bottom(if full { "chat_input_full" } else { "chat_input_dock" }).show_separator_line(false).frame(egui::Frame::NONE.inner_margin(egui::Margin { left: pad, right: pad, top: 6, bottom: 12 })).show_inside(ui, |ui| {
            let w = ui.available_width().min(if full { 860.0 } else { f32::INFINITY });
            ui.vertical_centered(|ui| {
                ui.set_max_width(w);
                if self.busy() {
                    self.working_ui(ui, colours, w);
                }
                egui::Frame::NONE.fill(t.card).stroke(egui::Stroke::new(1.0_f32, t.field_border)).corner_radius(12).inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
                    ui.set_width(w - 24.0);
                    ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                    if !self.attachments.is_empty() {
                        ui.horizontal_wrapped(|ui| {
                            let mut remove = None;
                            for (i, a) in self.attachments.iter().enumerate() {
                                let name = a.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                                egui::Frame::NONE.fill(t.hover).corner_radius(6).inner_margin(egui::Margin::symmetric(6, 2)).show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(RichText::new(name).size(12.0));
                                        if chrome::remove_button(ui, "Remove").clicked() {
                                            remove = Some(i);
                                        }
                                    });
                                });
                            }
                            if let Some(i) = remove {
                                self.attachments.remove(i);
                            }
                        });
                    }
                    let edit = egui::TextEdit::multiline(&mut self.input).hint_text("Ask the design office…  Enter sends, Shift+Enter for a new line").desired_rows(if full { 3 } else { 2 }).desired_width(f32::INFINITY).frame(false).lock_focus(true);
                    let resp = ui.add(edit);
                    let enter = resp.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift);
                    if enter && self.input.ends_with('\n') {
                        self.input.pop();
                    }
                    ui.horizontal(|ui| {
                        if icons::button(ui, Icon::Paperclip, "Attach pictures, PDFs, drawings, spreadsheets or text (or drop them on the window)", false).clicked() {
                            if let Some(files) = rfd::FileDialog::new().pick_files() {
                                for f in files {
                                    self.add_attachment(f);
                                }
                            }
                        }
                        ui.checkbox(&mut self.saved.details, RichText::new("Technical details").size(11.5).color(t.text_dim)).on_hover_text("Shows every tool call with its data, the agents' reasoning and the token counts");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if self.busy() {
                                if ui.add(egui::Button::new(RichText::new("Stop").color(t.error)).min_size(egui::vec2(64.0, 30.0)).corner_radius(8)).clicked() {
                                    self.stop();
                                }
                                ui.spinner();
                            } else if ui.add(chrome::primary_button("Send").fill(t.accent).min_size(egui::vec2(72.0, 30.0))).clicked() || enter {
                                self.send(engine, ui.ctx());
                            }
                        });
                    });
                    });
                });
            });
        });

        egui::ScrollArea::vertical().auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
            let w = ui.available_width().min(if full { 860.0 } else { f32::INFINITY });
            let side = if full { ((ui.available_width() - w) / 2.0).max(0.0) } else { 0.0 };
            ui.horizontal(|ui| {
                ui.add_space(side);
                ui.vertical(|ui| {
                    ui.set_width(w);
                    ui.add_space(if full { 24.0 } else { 6.0 });
                    if self.saved.entries.is_empty() {
                        self.empty_ui(ui, full);
                    }
                    // simple view: an agent's consecutive steps fold into one line; its reasoning is left out
                    let step_of = |e: &Entry| match e {
                        Entry::Tool { agent, .. } | Entry::Thinking { agent, .. } => Some(agent.clone()),
                        _ => None,
                    };
                    let n = self.saved.entries.len();
                    let mut i = 0;
                    while i < n {
                        match step_of(&self.saved.entries[i]).filter(|_| !self.saved.details) {
                            Some(agent) => {
                                let mut j = i + 1;
                                while j < n && step_of(&self.saved.entries[j]).as_ref() == Some(&agent) {
                                    j += 1;
                                }
                                self.steps_ui(ui, i, j);
                                i = j;
                            }
                            None => {
                                self.entry_ui(ui, i, colours);
                                i += 1;
                            }
                        }
                    }
                    if self.busy() && self.saved.details && self.run_usage.totals().0 > 0 {
                        ui.label(RichText::new(format!("Tokens so far: {}", self.run_usage.line())).size(11.5).color(t.text_faint)).on_hover_text(self.run_usage.table());
                    }
                    ui.add_space(16.0);
                });
            });
        });
    }

    fn empty_ui(&mut self, ui: &mut egui::Ui, full: bool) {
        let t = tokens(ui);
        if full {
            ui.add_space(30.0);
        }
        ui.label(RichText::new("What should the office design?").font(medium_font(if full { 22.0 } else { 15.0 })));
        ui.add_space(4.0);
        ui.label(RichText::new("The coordinator plans the work and delegates it: the designer calculates to the standards you chose, the checker verifies independently, the drafter draws, and the reporter writes the PDF calculation report.").color(t.text_dim));
        ui.add_space(14.0);
        let examples = [
            ("RC beam", "Design a simply supported RC beam, span 6 m, 300 x 500 mm, wD = 15 kN/m, wL = 10 kN/m, f'c 28 MPa, fy 420 MPa, to ACI 318-19. Check it, draw the section and write the report."),
            ("Footing", "Check the bearing resistance of a 2 x 2 m square footing at 1.5 m depth on sand, phi'k 32 deg, gamma 18 kN/m3, Gk 900 kN, Qk 300 kN, to EC7 DA1."),
            ("Retaining wall", "Design a 4 m high cantilever retaining wall with level backfill (phi 30 deg, gamma 18) and a 10 kPa surcharge; check overturning, sliding and bearing."),
        ];
        for (title, ex) in examples {
            let r = egui::Frame::NONE.fill(t.card).stroke(egui::Stroke::new(1.0_f32, t.separator)).corner_radius(10).inner_margin(12).show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.label(RichText::new(title).font(medium_font(13.0)));
                ui.label(RichText::new(ex).size(12.5).color(t.text_dim));
            });
            if r.response.interact(egui::Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                self.input = ex.to_string();
            }
            ui.add_space(8.0);
        }
    }

    fn entry_ui(&mut self, ui: &mut egui::Ui, i: usize, colours: &dyn Fn(&str) -> Color32) {
        let t = tokens(ui);
        let indent = |d: usize| 22.0 * d as f32;
        match self.saved.entries[i].clone() {
            Entry::User { text, attachments } => {
                ui.add_space(10.0);
                ui.with_layout(egui::Layout::top_down(egui::Align::Max), |ui| {
                    egui::Frame::NONE.fill(t.accent_soft).corner_radius(10).inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| {
                        ui.set_max_width(ui.available_width() * 0.8);
                        ui.add(egui::Label::new(RichText::new(&text).color(t.text)).wrap());
                        for a in &attachments {
                            attachment_ui(ui, a, &mut self.open_request);
                        }
                    });
                });
                ui.add_space(4.0);
            }
            Entry::Start { agent, depth, task } => {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add_space(indent(depth.saturating_sub(1)));
                    let c = colours(&agent);
                    chrome::chip(ui, &format!("→ {agent}"), c);
                    ui.label(RichText::new(first_line(&task, 120)).size(12.0).color(t.text_dim)).on_hover_text(task);
                });
            }
            Entry::Text { agent, depth, text } => {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add_space(indent(depth));
                    let c = colours(&agent);
                    egui::Frame::NONE.fill(t.card).stroke(egui::Stroke::new(1.0_f32, t.separator)).corner_radius(10).inner_margin(egui::Margin::symmetric(12, 10)).show(ui, |ui| {
                        ui.set_width(ui.available_width() - 2.0);
                        ui.vertical(|ui| {
                            ui.horizontal(|ui| {
                                let (r, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                                ui.painter().circle_filled(r.center(), 4.0, c);
                                ui.label(RichText::new(&agent).font(medium_font(12.5)).color(t.text));
                            });
                            markdown::show(ui, &text, &t);
                        });
                    });
                });
            }
            Entry::Thinking { agent, text } => {
                ui.horizontal(|ui| {
                    egui::CollapsingHeader::new(RichText::new(format!("{agent}'s reasoning")).size(11.5).color(t.text_faint)).id_salt(("think", i)).show(ui, |ui| {
                        ui.label(RichText::new(text).size(11.5).color(t.text_faint));
                    });
                });
            }
            Entry::Tool { agent, depth, tool, args, result, .. } => {
                ui.horizontal(|ui| {
                    ui.add_space(indent(depth) + 6.0);
                    let (mark, col) = match &result {
                        None => ("…", t.text_faint),
                        Some((true, _)) => ("✓", t.ok),
                        Some((false, _)) => ("✗", t.error),
                    };
                    let title = RichText::new(format!("{mark}  {}  ({tool})", friendly(&tool, &args, &self.names, false))).size(12.0).color(col);
                    egui::CollapsingHeader::new(title).id_salt(("tool", i)).show(ui, |ui| {
                        egui::Frame::NONE.fill(t.field).stroke(egui::Stroke::new(1.0_f32, t.separator)).corner_radius(6).inner_margin(8).show(ui, |ui| {
                            ui.label(RichText::new(format!("{agent} called {tool} with:")).size(11.0).color(t.text_faint));
                            ui.label(RichText::new(pretty(&args)).monospace().size(11.0));
                            if let Some((_, r)) = &result {
                                ui.separator();
                                ui.label(RichText::new(r).monospace().size(11.0));
                            }
                        });
                    });
                });
            }
            Entry::Artifact { kind, path } => {
                let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                let (icon, label) = match kind {
                    ArtifactKind::Calculation => (Icon::Calculator, "Calculation"),
                    ArtifactKind::Drawing => (Icon::Drafting, "Drawing"),
                    ArtifactKind::Report => (Icon::FileText, "Report"),
                    ArtifactKind::Image => (Icon::Picture, "Picture"),
                    ArtifactKind::Attachment => (Icon::Paperclip, "File"),
                    ArtifactKind::Work => (Icon::Edit, "Work file"),
                };
                ui.add_space(4.0);
                let r = egui::Frame::NONE.fill(t.card).stroke(egui::Stroke::new(1.0_f32, t.separator)).corner_radius(8).inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
                        icons::paint(ui, icon, r, t.accent_text);
                        ui.label(RichText::new(label).size(12.0).color(t.text_dim));
                        ui.label(RichText::new(&name).font(medium_font(12.5)));
                    });
                    let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
                    if matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "svg") {
                        let img = egui::Image::new(format!("file://{}", chrome::display_path(&path).display())).fit_to_exact_size(egui::vec2(420.0, 260.0)).maintain_aspect_ratio(true).bg_fill(Color32::WHITE).corner_radius(6);
                        ui.add(img);
                    }
                });
                if r.response.interact(egui::Sense::click()).on_hover_text("Show in the viewer").on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    self.open_request = Some(path.clone());
                }
            }
            Entry::Done { agent, depth } => {
                ui.horizontal(|ui| {
                    ui.add_space(indent(depth.saturating_sub(1)));
                    ui.label(RichText::new(format!("← {agent} finished")).size(11.0).color(t.text_faint));
                });
            }
            Entry::Usage { report } => {
                ui.add_space(6.0);
                usage_card(ui, &report, &t, i, colours, self.saved.details);
            }
            Entry::Model { plugin, path } => {
                let program = th_harness::live::program_name(&plugin);
                let label = th_harness::live::spec(&plugin).map(|s| s.label).unwrap_or("Model");
                let live = th_harness::live::spec(&plugin).is_some_and(|s| s.follows_file);
                ui.add_space(4.0);
                egui::Frame::NONE.fill(t.card).stroke(egui::Stroke::new(1.0_f32, t.separator)).corner_radius(8).inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
                        icons::paint(ui, Icon::Rotate, r, t.accent_text);
                        ui.label(RichText::new(format!("{label} in {program}")).font(medium_font(12.5)));
                        ui.label(RichText::new(if live { "the window follows the agents as they work" } else { "saved after every change" }).size(12.0).color(t.text_dim));
                        let tip = format!("Opens {} in {program}'s own window, where you can rotate it, cut sections and read the results.", path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
                        if ui.add(egui::Button::new(format!("Open in {program}")).corner_radius(6)).on_hover_text(tip).clicked() {
                            self.open_model = Some(plugin.clone());
                        }
                    });
                });
            }
            Entry::Error { text } => {
                ui.add_space(4.0);
                egui::Frame::NONE.fill(t.error.gamma_multiply(0.10)).stroke(egui::Stroke::new(1.0_f32, t.error.gamma_multiply(0.5))).corner_radius(8).inner_margin(egui::Margin::symmetric(10, 6)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(text).color(t.error));
                });
            }
        }
    }
}

/// The token report of one task: a summary line, then one row per agent and model.
fn usage_card(ui: &mut egui::Ui, r: &UsageReport, t: &crate::theme::Tokens, i: usize, colours: &dyn Fn(&str) -> Color32, details: bool) {
    let (_, p, c) = r.totals();
    let (mark, col) = match r.outcome.as_str() {
        "finished" => ("Finished", t.ok),
        "stopped" => ("Stopped", t.warn),
        _ => ("Failed", t.error),
    };
    egui::Frame::NONE.fill(t.field).stroke(egui::Stroke::new(1.0_f32, t.separator)).corner_radius(8).inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new(mark).font(medium_font(12.5)).color(col));
            ui.label(RichText::new(format!("in {}:{:02}", r.seconds / 60, r.seconds % 60)).size(12.0).color(t.text_dim));
            if !details {
                let mut who: Vec<&str> = r.agents.iter().map(|a| a.agent.as_str()).collect();
                who.dedup();
                who.sort();
                who.dedup();
                ui.label(RichText::new(format!("· {} took part", if who.len() == 1 { "1 team member".to_string() } else { format!("{} team members", who.len()) })).size(12.0).color(t.text_dim)).on_hover_text(who.join(", "));
                return;
            }
            ui.label(RichText::new(format!("· {}", r.line())).size(12.0).color(t.text));
        });
        if !details {
            return;
        }
        egui::CollapsingHeader::new(RichText::new("Token use by agent").size(11.5).color(t.text_dim)).id_salt(("usage", i)).show(ui, |ui| {
            egui::Grid::new(("usage_grid", i)).num_columns(6).spacing(egui::vec2(18.0, 4.0)).striped(true).show(ui, |ui| {
                for h in ["Agent", "Model", "Calls", "In", "Out", "Share"] {
                    ui.label(RichText::new(h).size(11.0).color(t.text_faint));
                }
                ui.end_row();
                let all = (p + c).max(1) as f64;
                for a in &r.agents {
                    ui.horizontal(|ui| {
                        let (rr, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                        ui.painter().circle_filled(rr.center(), 4.0, colours(&a.agent));
                        ui.label(RichText::new(&a.agent).size(12.0));
                    });
                    ui.label(RichText::new(&a.model).size(11.5).color(t.text_dim));
                    ui.label(RichText::new(a.calls.to_string()).monospace().size(11.5));
                    ui.label(RichText::new(fmt_tokens(a.prompt)).monospace().size(11.5));
                    ui.label(RichText::new(fmt_tokens(a.completion)).monospace().size(11.5));
                    ui.label(RichText::new(format!("{:.0}%", 100.0 * (a.prompt + a.completion) as f64 / all)).monospace().size(11.5));
                    ui.end_row();
                }
            });
        });
    });
}

fn attachment_ui(ui: &mut egui::Ui, a: &std::path::Path, open: &mut Option<PathBuf>) {
    let name = a.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    if th_harness::project::is_image(a) {
        let img = egui::Image::new(format!("file://{}", a.display())).fit_to_exact_size(egui::vec2(320.0, 180.0)).maintain_aspect_ratio(true).corner_radius(6);
        if ui.add(img.sense(egui::Sense::click())).clicked() {
            *open = Some(a.to_path_buf());
        }
    } else if ui.link(format!("Attached: {name}")).clicked() {
        *open = Some(a.to_path_buf());
    }
}

fn first_line(s: &str, n: usize) -> String {
    let l = s.lines().next().unwrap_or("");
    if l.chars().count() > n {
        format!("{}…", l.chars().take(n).collect::<String>())
    } else {
        l.to_string()
    }
}

fn pretty(args: &str) -> String {
    serde_json::from_str::<serde_json::Value>(args).ok().and_then(|v| serde_json::to_string_pretty(&v).ok()).unwrap_or_else(|| args.to_string())
}

fn short(s: &str, n: usize) -> String {
    let s = s.trim();
    if s.chars().count() > n {
        format!("{}…", s.chars().take(n).collect::<String>())
    } else {
        s.to_string()
    }
}

/// "A", "A and B", "A, B and C".
fn join_names(n: &[String]) -> String {
    match n {
        [] => String::new(),
        [a] => a.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

fn capital(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// One step of the simple view: a mark, what was done and, when it failed, why.
fn step_line(ui: &mut egui::Ui, row: &(String, Option<(bool, String)>), t: &crate::theme::Tokens) {
    let (mark, col) = match &row.1 {
        None => ("…", t.text_dim),
        Some((true, _)) => ("✓", t.ok),
        Some((false, _)) => ("✗", t.error),
    };
    ui.vertical(|ui| {
        ui.label(RichText::new(format!("{mark}  {}", row.0)).size(12.0).color(col));
        if let Some((false, why)) = &row.1 {
            ui.label(RichText::new(short(first_line(why.trim_start_matches("ERROR: "), 200).as_str(), 200)).size(11.0).color(t.text_dim));
        }
    });
}

/// A tool call in the engineer's words: "Searching the standards: minimum shear reinforcement".
/// `live` is the wording for work going on now ("Waiting for Designer").
pub fn friendly(tool: &str, args: &str, names: &std::collections::HashMap<String, String>, live: bool) -> String {
    let v: serde_json::Value = serde_json::from_str(args).unwrap_or(serde_json::Value::Null);
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(|x| short(x, 60)).filter(|x| !x.is_empty());
    let file = |k: &str| v.get(k).and_then(|x| x.as_str()).map(|p| p.rsplit('/').next().unwrap_or(p).to_string()).filter(|x| !x.is_empty());
    let who = |id: &str| names.get(id).cloned().unwrap_or_else(|| capital(&id.replace('_', " ")));
    let with = |base: &str, d: Option<String>| match d {
        Some(d) => format!("{base}: {d}"),
        None => base.to_string(),
    };
    match tool {
        "search_standards" => with("Searching the standards", s("query")),
        "get_clause" => match (s("code"), s("clause")) {
            (Some(c), Some(n)) => format!("Reading clause {n} of {c}"),
            _ => "Reading a clause of the standard".into(),
        },
        "list_standards" => "Checking which standards apply".into(),
        "list_design_modules" => "Looking up the design calculations available".into(),
        "run_design_module" => with("Running the design calculation", s("name").or_else(|| s("module"))),
        "calc_sheet" => with("Calculating", s("title").or_else(|| s("name"))),
        "make_drawing" => with("Drawing", s("name").or_else(|| v.pointer("/drawing/title").and_then(|x| x.as_str()).map(|x| short(x, 60)))),
        "write_report" => with("Writing the report", s("title").or_else(|| s("name"))),
        "list_files" => "Looking through the project files".into(),
        "read_file" => with("Reading", file("path")),
        "write_file" => with("Saving", file("path")),
        "read_skill" => with("Opening the guide", s("name")),
        "describe_image" => with("Looking at the picture", file("path")),
        "run_python" => with("Running a Python script", s("name").or_else(|| file("file"))),
        "delegate" => {
            let to = who(v.get("agent").and_then(|x| x.as_str()).unwrap_or("a team member"));
            if live { format!("Waiting for {to}") } else { format!("Task for {to}") }
        }
        "delegate_parallel" => {
            let mut to: Vec<String> = v.get("tasks").and_then(|x| x.as_array()).map(|a| a.iter().filter_map(|x| x.get("agent").and_then(|y| y.as_str())).map(who).collect()).unwrap_or_default();
            to.dedup();
            let to = if to.is_empty() { "the team".to_string() } else { join_names(&to) };
            if live { format!("Waiting for {to}") } else { format!("Tasks for {to}") }
        }
        "run_claude_code" => "Claude Code is writing a program".into(),
        "run_codex" => "Codex is writing a program".into(),
        "plugin_tools" => with("Checking what the program can do", s("plugin").map(|p| capital(&p))),
        "plugin_call" => format!("{}: {}", capital(&s("plugin").unwrap_or_else(|| "Program".into())), s("tool").unwrap_or_default().replace('_', " ")),
        other => match other.split_once("__") {
            Some((plugin, name)) => format!("{}: {}", capital(plugin), name.replace('_', " ")),
            None => capital(&other.replace('_', " ")),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_calls_read_as_plain_words() {
        let names: std::collections::HashMap<String, String> = [("designer".to_string(), "Designer".to_string()), ("checker".to_string(), "Checker".to_string())].into();
        assert_eq!(friendly("search_standards", r#"{"query":"minimum shear reinforcement"}"#, &names, false), "Searching the standards: minimum shear reinforcement");
        assert_eq!(friendly("get_clause", r#"{"code":"ACI 318-19","clause":"9.6.3"}"#, &names, false), "Reading clause 9.6.3 of ACI 318-19");
        assert_eq!(friendly("pondus__add_members", "{}", &names, false), "Pondus: add members");
        assert_eq!(friendly("delegate", r#"{"agent":"designer","task":"x"}"#, &names, true), "Waiting for Designer");
        assert_eq!(friendly("delegate_parallel", r#"{"tasks":[{"agent":"designer","task":"x"},{"agent":"checker","task":"y"}]}"#, &names, true), "Waiting for Designer and Checker");
        assert_eq!(friendly("read_file", r#"{"path":"attachments/soil report.pdf"}"#, &names, false), "Reading: soil report.pdf");
        // arguments cut off mid-way still give a readable line
        assert_eq!(friendly("calc_sheet", r#"{"title":"B1 be"#, &names, false), "Calculating");
    }
}
