//! The window's MCP server: other agents (Claude, ChatGPT, Grok, Antigravity …) list the
//! bots and teams and send them messages. A message goes into that bot's or team's chat in
//! the window, so the engineer sees the work and the conversation is kept; the reply goes
//! back to the calling agent when the run ends.

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Mutex;
use std::time::Duration;

use eframe::egui;
use serde_json::{json, Value};
use th_harness::bots::Bot;
use th_harness::config::{Settings, TeamSpec};
use th_mcp::{ServedTool, ToolInfo};

/// Whom a message is for.
#[derive(Debug, Clone)]
pub enum Target {
    Bot(String),
    Team(String),
}

/// A message from a connected agent, answered on `reply` when the run ends.
pub struct Request {
    pub target: Target,
    pub text: String,
    pub reply: Sender<Result<String, String>>,
}

/// How long a connected agent waits for a bot or team before it is told to ask again later.
const WAIT: Duration = Duration::from_secs(60 * 60);

/// Starts the server when it is switched on. The app takes the requests from the receiver.
pub fn start(settings: &Settings, ctx: egui::Context) -> Result<Receiver<Request>, String> {
    let b = &settings.bridge;
    let (tx, rx) = mpsc::channel::<Request>();
    let tx = std::sync::Arc::new(Mutex::new(tx));
    let ask = |target: fn(String) -> Target, key: &'static str| {
        let (tx, ctx) = (tx.clone(), ctx.clone());
        Box::new(move |a: Value| -> Result<String, String> {
            let who = a.get(key).and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty()).ok_or(format!("give `{key}`"))?;
            let message = a.get("message").and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty()).ok_or("give `message`")?;
            let from = a.get("from").and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty()).unwrap_or("a connected agent");
            let (rtx, rrx) = mpsc::channel();
            let text = format!("[Message from {from}, through the MCP server]\n{message}");
            tx.lock().map_err(|_| "the window is closing")?.send(Request { target: target(who.to_string()), text, reply: rtx }).map_err(|_| "the window is closing".to_string())?;
            ctx.request_repaint();
            match rrx.recv_timeout(WAIT) {
                Ok(r) => r,
                Err(_) => Err(format!("{who} did not finish within {} min; the work goes on in the window, ask again later", WAIT.as_secs() / 60)),
            }
        }) as Box<dyn Fn(Value) -> Result<String, String> + Send + Sync>
    };
    let tools = vec![
        ServedTool {
            info: ToolInfo {
                name: "list_bots".into(),
                description: "List the bots of this TellusHive design office: each one's name, what it is for, and whom it calls.".into(),
                input_schema: json!({"type": "object", "properties": {}}),
            },
            run: Box::new(|_| {
                let bots = Bot::list();
                if bots.is_empty() {
                    return Ok("No bots yet. Use list_teams and ask_team, or ask the engineer to make a bot.".into());
                }
                let settings = Settings::load();
                Ok(bots.iter().map(|b| format!("- {}: {} ({})", b.name, if b.tagline.is_empty() { "no tagline" } else { &b.tagline }, b.engine_line(&settings, &bots))).collect::<Vec<_>>().join("\n"))
            }),
        },
        ServedTool {
            info: ToolInfo {
                name: "list_teams".into(),
                description: "List the teams of this TellusHive design office (civil and geotechnical engineering agents: designers, checkers, analysts with finite element programs, drafters, reporters).".into(),
                input_schema: json!({"type": "object", "properties": {}}),
            },
            run: Box::new(|_| {
                Ok(TeamSpec::list()
                    .iter()
                    .map(|t| format!("- {}: {}", t.name, t.agents.iter().map(|a| a.name.clone()).collect::<Vec<_>>().join(", ")))
                    .collect::<Vec<_>>()
                    .join("\n"))
            }),
        },
        ServedTool {
            info: ToolInfo {
                name: "ask_bot".into(),
                description: "Send a message to one bot and wait for its reply. The bot continues its own conversation, so follow-ups keep their context. A bot may run calculations, finite element analyses and write reports, which can take many minutes.".into(),
                input_schema: json!({"type": "object", "properties": {
                    "bot": {"type": "string", "description": "The bot's name, from list_bots"},
                    "message": {"type": "string", "description": "What to ask or tell the bot, with all the data it needs"},
                    "from": {"type": "string", "description": "Who is asking, shown to the engineer (e.g. 'ChatGPT')"}},
                    "required": ["bot", "message"]}),
            },
            run: ask(Target::Bot, "bot"),
        },
        ServedTool {
            info: ToolInfo {
                name: "ask_team".into(),
                description: "Send a message to a team, through its lead, on the project open in TellusHive, and wait for the reply. The team continues its own conversation on that project.".into(),
                input_schema: json!({"type": "object", "properties": {
                    "team": {"type": "string", "description": "The team's name, from list_teams"},
                    "message": {"type": "string", "description": "What to ask or tell the team, with all the data it needs"},
                    "from": {"type": "string", "description": "Who is asking, shown to the engineer (e.g. 'Grok')"}},
                    "required": ["team", "message"]}),
            },
            run: ask(Target::Team, "team"),
        },
    ];
    let token = (!b.token.is_empty()).then(|| b.token.clone());
    th_mcp::serve_http("tellushive", &format!("127.0.0.1:{}", b.port), token, tools)?;
    Ok(rx)
}
