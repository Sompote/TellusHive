//! `tellushive serve`: the bots without the window, for the TellusHive iPhone app.
//!
//! A small HTTP API on a cloud machine (behind an HTTPS tunnel or reverse proxy). Every
//! request carries `Authorization: Bearer <token>`. A message starts a task that runs on the
//! server whether or not the phone stays connected; the phone follows it as a Server-Sent
//! Events stream and can attach again later. The transcript is the same `chat-bot-*.json`
//! file the window keeps, so a conversation started on the phone continues on the desktop.
//!
//! ```text
//! GET  /api/info                        server name and version
//! GET  /api/bots                        the bots, and which are busy
//! GET  /api/bots/{id}/avatar            the bot's picture
//! GET  /api/bots/{id}/history           the conversation so far
//! POST /api/bots/{id}/messages          {"text", "attachments":[rel]} → event stream
//! GET  /api/bots/{id}/events            attach to the running task's event stream
//! POST /api/bots/{id}/stop              stop the running task
//! POST /api/bots/{id}/new               start a new conversation
//! POST /api/bots/{id}/upload?name=F     raw file body → {"path": rel} in attachments/
//! GET  /api/bots/{id}/file?path=REL     a file of the bot's project
//! ```

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use th_harness::bots::Bot;
use th_harness::{Engine, Event, Plugins, Project, Settings, UserInput};

use crate::chat::{ChatView, Entry};

const MAX_BODY: usize = 64 << 20;
const PING: Duration = Duration::from_secs(15);

/// One bot's conversation as the server keeps it.
#[derive(Default)]
struct Slot {
    chat: Option<ChatView>,
    dir: PathBuf,
    file: String,
    running: bool,
    cancel: Arc<AtomicBool>,
    /// Open event streams (each gets one JSON object per event).
    subscribers: Vec<Sender<String>>,
    /// The bot's reply streamed so far in the current step, for a phone that attaches late.
    partial: String,
    activity: String,
}

impl Slot {
    fn broadcast(&mut self, v: &Value) {
        let line = v.to_string();
        self.subscribers.retain(|s| s.send(line.clone()).is_ok());
    }
    fn save(&self) {
        if let Some(c) = &self.chat {
            c.save(&self.dir);
        }
    }
}

struct Server {
    token: String,
    library: Arc<th_kb::Library>,
    plugins: Arc<Plugins>,
    slots: Mutex<HashMap<String, Arc<Mutex<Slot>>>>,
}

fn arg(args: &[String], key: &str) -> Option<String> {
    args.iter().position(|a| a == key).and_then(|i| args.get(i + 1)).cloned()
}

/// The token from `--token`, `TELLUSHIVE_SERVER_TOKEN`, or the one saved in the data folder
/// (made on first start).
fn token(args: &[String]) -> Result<String, String> {
    if let Some(t) = arg(args, "--token").or_else(|| std::env::var("TELLUSHIVE_SERVER_TOKEN").ok()).filter(|t| !t.trim().is_empty()) {
        return Ok(t.trim().to_string());
    }
    let path = th_harness::config::data_dir().join("server-token");
    if let Ok(t) = std::fs::read_to_string(&path) {
        if !t.trim().is_empty() {
            return Ok(t.trim().to_string());
        }
    }
    let t = th_harness::config::BridgeSettings::new_token();
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, &t).map_err(|e| format!("{}: {e}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(t)
}

pub fn run(args: &[String]) -> i32 {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("usage: tellushive serve [--host 127.0.0.1] [--port 8796] [--token TOKEN]\n\nServes the bots to the TellusHive iPhone app. Put it behind HTTPS (Cloudflare Tunnel,\nTailscale or a reverse proxy); the token is read from --token, TELLUSHIVE_SERVER_TOKEN or\nthe server-token file in the data folder (made on first start).");
        return 0;
    }
    let host = arg(args, "--host").unwrap_or_else(|| "127.0.0.1".into());
    let port = arg(args, "--port").and_then(|p| p.parse::<u16>().ok()).unwrap_or(8796);
    let token = match token(args) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("tellushive serve: {e}");
            return 1;
        }
    };
    let library = match th_kb::Library::open(&th_kb::Library::default_root()) {
        Ok(l) => Arc::new(l),
        Err(e) => {
            eprintln!("tellushive serve: standards library: {e}");
            return 1;
        }
    };
    let addr = format!("{host}:{port}");
    let listener = match TcpListener::bind(&addr) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("tellushive serve: cannot listen on {addr}: {e}");
            return 1;
        }
    };
    let bots = Bot::list();
    println!("TellusHive server on http://{addr} ({} bots)", bots.len());
    println!("Token: {token}");
    if host != "127.0.0.1" && host != "localhost" {
        println!("Warning: listening beyond this machine over plain HTTP; use an HTTPS tunnel or proxy in front of it.");
    }
    let server = Arc::new(Server { token, library, plugins: Arc::new(Plugins::default()), slots: Mutex::new(HashMap::new()) });
    for conn in listener.incoming().flatten() {
        let server = server.clone();
        std::thread::spawn(move || {
            let _ = conn.set_read_timeout(Some(Duration::from_secs(60)));
            if let Err(e) = connection(&server, conn) {
                if e.kind() != std::io::ErrorKind::BrokenPipe {
                    eprintln!("connection: {e}");
                }
            }
        });
    }
    0
}

struct Request {
    method: String,
    path: String,
    query: HashMap<String, String>,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => {
                let hex = |c: u8| (c as char).to_digit(16);
                match (hex(b[i + 1]), hex(b[i + 2])) {
                    (Some(h), Some(l)) => {
                        out.push((h * 16 + l) as u8);
                        i += 3;
                        continue;
                    }
                    _ => out.push(b'%'),
                }
            }
            b'+' => out.push(b' '),
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn read_request(reader: &mut BufReader<TcpStream>) -> std::io::Result<Option<Request>> {
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("").to_string();
    let (path, qs) = target.split_once('?').unwrap_or((&target, ""));
    let query = qs.split('&').filter(|p| !p.is_empty()).map(|p| {
        let (k, v) = p.split_once('=').unwrap_or((p, ""));
        (percent_decode(k), percent_decode(v))
    }).collect();
    let mut headers = HashMap::new();
    for _ in 0..100 {
        let mut h = String::new();
        if reader.read_line(&mut h)? == 0 || h.trim().is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }
    let len: usize = headers.get("content-length").and_then(|l: &String| l.parse().ok()).unwrap_or(0);
    if len > MAX_BODY {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "body too large"));
    }
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body)?;
    Ok(Some(Request { method, path: percent_decode(path), query, headers, body }))
}

fn respond(out: &mut TcpStream, status: &str, ctype: &str, body: &[u8]) -> std::io::Result<()> {
    write!(out, "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n", body.len())?;
    out.write_all(body)?;
    out.flush()
}

fn json_reply(out: &mut TcpStream, status: &str, v: &Value) -> std::io::Result<()> {
    respond(out, status, "application/json", v.to_string().as_bytes())
}

fn error(out: &mut TcpStream, status: &str, msg: &str) -> std::io::Result<()> {
    json_reply(out, status, &json!({ "error": msg }))
}

/// Compares the tokens without stopping at the first different byte.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn connection(server: &Server, conn: TcpStream) -> std::io::Result<()> {
    let mut reader = BufReader::new(conn.try_clone()?);
    let mut out = conn;
    let Some(req) = read_request(&mut reader)? else { return Ok(()) };
    let authorised = req.headers.get("authorization").and_then(|a| a.strip_prefix("Bearer ")).is_some_and(|t| same(t.trim(), &server.token));
    if !authorised {
        return error(&mut out, "401 Unauthorized", "missing or wrong token");
    }
    let segs: Vec<&str> = req.path.trim_matches('/').split('/').collect();
    match (req.method.as_str(), segs.as_slice()) {
        ("GET", ["api", "info"]) => json_reply(&mut out, "200 OK", &json!({ "name": "TellusHive", "version": env!("CARGO_PKG_VERSION") })),
        ("GET", ["api", "bots"]) => {
            let busy: Vec<String> = server.slots.lock().map(|m| m.iter().filter(|(_, s)| s.lock().is_ok_and(|s| s.running)).map(|(k, _)| k.clone()).collect()).unwrap_or_default();
            let list: Vec<Value> = Bot::list()
                .iter()
                .map(|b| json!({ "id": b.id, "name": b.display_name(), "tagline": b.tagline, "color": b.color, "team": b.team, "has_avatar": b.has_avatar(), "busy": busy.contains(&b.id) }))
                .collect();
            json_reply(&mut out, "200 OK", &Value::Array(list))
        }
        (m, ["api", "bots", id, rest @ ..]) => {
            let Some(bot) = Bot::list().into_iter().find(|b| b.id == *id) else {
                return error(&mut out, "404 Not Found", "no such bot");
            };
            bot_route(server, &bot, m, rest, &req, &mut out)
        }
        _ => error(&mut out, "404 Not Found", "unknown endpoint"),
    }
}

fn slot_of(server: &Server, bot: &Bot) -> Result<Arc<Mutex<Slot>>, String> {
    let slot = server.slots.lock().map_err(|_| "poisoned")?.entry(bot.id.clone()).or_default().clone();
    {
        let mut s = slot.lock().map_err(|_| "poisoned")?;
        // reload from disk when idle, so the window's turns since show too
        if !s.running {
            let project = bot.project()?;
            let mut chat = ChatView::new(ChatView::load_file(&project.dir, &bot.chat_file()));
            chat.file = bot.chat_file();
            s.dir = project.dir;
            s.file = bot.chat_file();
            s.chat = Some(chat);
        }
    }
    Ok(slot)
}

fn bot_route(server: &Server, bot: &Bot, method: &str, rest: &[&str], req: &Request, out: &mut TcpStream) -> std::io::Result<()> {
    match (method, rest) {
        ("GET", ["avatar"]) => match std::fs::read(bot.avatar_path()) {
            Ok(b) => respond(out, "200 OK", content_type(&bot.avatar_path()), &b),
            Err(_) => error(out, "404 Not Found", "no picture"),
        },
        ("GET", ["history"]) => {
            let slot = match slot_of(server, bot) {
                Ok(s) => s,
                Err(e) => return error(out, "500 Internal Server Error", &e),
            };
            let s = slot.lock().unwrap_or_else(|e| e.into_inner());
            let entries: Vec<Value> = s.chat.as_ref().map(|c| c.saved.entries.iter().enumerate().filter_map(|(i, e)| entry_json(i, e, &s.dir)).collect()).unwrap_or_default();
            json_reply(out, "200 OK", &json!({ "entries": entries, "busy": s.running, "activity": s.activity }))
        }
        ("POST", ["messages"]) => {
            let v: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
            let text = v.get("text").and_then(|t| t.as_str()).unwrap_or("").trim().to_string();
            let atts: Vec<String> = v.get("attachments").and_then(|a| a.as_array()).map(|l| l.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
            if text.is_empty() && atts.is_empty() {
                return error(out, "400 Bad Request", "give `text` or `attachments`");
            }
            match start(server, bot, text, &atts) {
                Ok(rx) => stream(out, rx),
                Err((status, e)) => error(out, status, &e),
            }
        }
        ("GET", ["events"]) => {
            let slot = match slot_of(server, bot) {
                Ok(s) => s,
                Err(e) => return error(out, "500 Internal Server Error", &e),
            };
            let (tx, rx) = mpsc::channel();
            {
                let mut s = slot.lock().unwrap_or_else(|e| e.into_inner());
                if !s.running {
                    let _ = tx.send(json!({ "type": "idle" }).to_string());
                    drop(tx);
                } else {
                    let _ = tx.send(json!({ "type": "state", "activity": s.activity, "partial": s.partial }).to_string());
                    s.subscribers.push(tx);
                }
            }
            stream(out, rx)
        }
        ("POST", ["stop"]) => {
            if let Some(slot) = server.slots.lock().ok().and_then(|m| m.get(&bot.id).cloned()) {
                if let Ok(s) = slot.lock() {
                    s.cancel.store(true, Ordering::Relaxed);
                }
            }
            json_reply(out, "200 OK", &json!({ "ok": true }))
        }
        ("POST", ["new"]) => {
            let slot = match slot_of(server, bot) {
                Ok(s) => s,
                Err(e) => return error(out, "500 Internal Server Error", &e),
            };
            let mut s = slot.lock().unwrap_or_else(|e| e.into_inner());
            if s.running {
                return error(out, "409 Conflict", "the bot is working; stop it first");
            }
            match engine_for(server, bot) {
                Ok((engine, _, _)) => engine.new_conversation(),
                Err(e) => return error(out, "500 Internal Server Error", &e),
            }
            if let Some(c) = s.chat.as_mut() {
                c.clear();
            }
            s.save();
            json_reply(out, "200 OK", &json!({ "ok": true }))
        }
        ("POST", ["upload"]) => {
            let name = req.query.get("name").map(|n| Path::new(n).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default()).filter(|n| !n.is_empty() && !n.starts_with('.')).unwrap_or_else(|| "upload.bin".into());
            if req.body.is_empty() {
                return error(out, "400 Bad Request", "empty file");
            }
            let project = match bot.project() {
                Ok(p) => p,
                Err(e) => return error(out, "500 Internal Server Error", &e),
            };
            let tmp = std::env::temp_dir().join(format!("tellushive-upload-{}", th_harness::config::BridgeSettings::new_token()));
            let r = std::fs::create_dir_all(&tmp).and_then(|_| std::fs::write(tmp.join(&name), &req.body)).map_err(|e| e.to_string()).and_then(|_| project.attach(&tmp.join(&name)));
            let _ = std::fs::remove_dir_all(&tmp);
            match r {
                Ok(p) => json_reply(out, "200 OK", &json!({ "path": project.rel(&p), "name": p.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default() })),
                Err(e) => error(out, "500 Internal Server Error", &e),
            }
        }
        ("GET", ["file"]) => {
            let project = match bot.project() {
                Ok(p) => p,
                Err(e) => return error(out, "500 Internal Server Error", &e),
            };
            let rel = req.query.get("path").cloned().unwrap_or_default();
            match project.resolve(&rel) {
                Ok(p) if p.is_file() => match std::fs::read(&p) {
                    Ok(b) => respond(out, "200 OK", content_type(&p), &b),
                    Err(e) => error(out, "500 Internal Server Error", &e.to_string()),
                },
                Ok(_) => error(out, "404 Not Found", "no such file"),
                Err(e) => error(out, "403 Forbidden", &e),
            }
        }
        _ => error(out, "404 Not Found", "unknown endpoint"),
    }
}

/// An engine for the bot, built fresh so edits to the bot apply; the MCP programs and the
/// standards library are shared by every bot.
fn engine_for(server: &Server, bot: &Bot) -> Result<(Arc<Engine>, Project, HashMap<String, String>), String> {
    let bots = Bot::list();
    let project = bot.project()?;
    let settings = Bot::engine_settings(&Settings::load(), &bots);
    let team = bot.team_spec(&bots);
    let names = team.agents.iter().map(|a| (a.id.clone(), a.name.clone())).collect();
    let engine = Arc::new(Engine::new(settings, team, project.clone(), server.library.clone(), server.plugins.clone()));
    engine.use_bot_chat(bot);
    Ok((engine, project, names))
}

/// Starts a task and returns the event stream of the phone that sent it.
fn start(server: &Server, bot: &Bot, text: String, atts: &[String]) -> Result<mpsc::Receiver<String>, (&'static str, String)> {
    let internal = |e: String| ("500 Internal Server Error", e);
    let slot = slot_of(server, bot).map_err(internal)?;
    let (engine, project, names) = engine_for(server, bot).map_err(internal)?;
    let mut files = vec![];
    for a in atts {
        let p = project.resolve(a).map_err(|e| ("400 Bad Request", e))?;
        if !p.is_file() {
            return Err(("400 Bad Request", format!("no attachment {a}; upload it first")));
        }
        files.push(p);
    }
    let (tx, rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut s = slot.lock().unwrap_or_else(|e| e.into_inner());
        if s.running {
            return Err(("409 Conflict", "the bot is still working on the last message".into()));
        }
        s.running = true;
        s.cancel = cancel.clone();
        s.partial.clear();
        s.activity = "Getting started".into();
        s.subscribers = vec![tx];
        if let Some(c) = s.chat.as_mut() {
            c.begin_run(&text, files.clone(), names);
        }
        s.save();
        let started = json!({ "type": "started" });
        s.broadcast(&started);
    }
    std::thread::spawn(move || {
        let (etx, erx) = mpsc::channel::<Event>();
        let fslot = slot.clone();
        let fwd = std::thread::spawn(move || {
            let mut depth: HashMap<String, usize> = HashMap::new();
            for e in erx {
                let mut s = fslot.lock().unwrap_or_else(|e| e.into_inner());
                let dir = s.dir.clone();
                let msgs = live_json(&e, &mut depth, &dir);
                let save = matches!(e, Event::Text { .. } | Event::Artifact { .. } | Event::Model { .. } | Event::Error { .. });
                if let Some(c) = s.chat.as_mut() {
                    c.record(e);
                }
                for m in &msgs {
                    match m["type"].as_str() {
                        Some("delta") => s.partial.push_str(m["text"].as_str().unwrap_or("")),
                        Some("message") => s.partial.clear(),
                        _ => {}
                    }
                    s.broadcast(m);
                }
                let activity = s.chat.as_ref().map(|c| c.current_activity()).unwrap_or_default();
                if activity != s.activity {
                    s.activity = activity.clone();
                    s.broadcast(&json!({ "type": "activity", "text": activity }));
                }
                if save {
                    s.save();
                }
            }
        });
        let r = engine.send(UserInput { text, attachments: files }, &etx, &cancel);
        drop(etx);
        let _ = fwd.join();
        let mut s = slot.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(c) = s.chat.as_mut() {
            c.end_run(&r);
        }
        s.save();
        let outcome = match &r {
            Ok(_) => "finished",
            Err(e) if e == "cancelled" => "stopped",
            Err(_) => "failed",
        };
        let done = json!({ "type": "done", "outcome": outcome, "error": r.err().filter(|e| e != "cancelled") });
        s.broadcast(&done);
        s.running = false;
        s.partial.clear();
        s.activity.clear();
        // closing the senders ends every open stream
        s.subscribers.clear();
    });
    Ok(rx)
}

/// Writes the events as Server-Sent Events until the task ends or the phone goes away.
fn stream(out: &mut TcpStream, rx: mpsc::Receiver<String>) -> std::io::Result<()> {
    write!(out, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nX-Accel-Buffering: no\r\nConnection: close\r\n\r\n")?;
    out.flush()?;
    loop {
        match rx.recv_timeout(PING) {
            Ok(line) => write!(out, "data: {line}\n\n")?,
            Err(mpsc::RecvTimeoutError::Timeout) => write!(out, ": ping\n\n")?,
            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
        }
        out.flush()?;
    }
}

fn rel(dir: &Path, p: &Path) -> String {
    p.strip_prefix(dir).map(|r| r.display().to_string()).unwrap_or_else(|_| p.display().to_string())
}

fn file_json(kind: &str, path: &Path, dir: &Path) -> Value {
    json!({ "kind": kind, "path": rel(dir, path), "name": path.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default() })
}

/// A transcript entry as the phone shows it; tool calls and reasoning stay on the desktop.
fn entry_json(i: usize, e: &Entry, dir: &Path) -> Option<Value> {
    let v = match e {
        Entry::User { text, attachments } => json!({ "kind": "user", "text": text, "files": attachments.iter().map(|a| file_json("Attachments", a, dir)).collect::<Vec<_>>() }),
        Entry::Text { agent, depth: 0, text } => json!({ "kind": "bot", "agent": agent, "text": text }),
        Entry::Text { agent, text, .. } => json!({ "kind": "team", "agent": agent, "text": text }),
        Entry::Start { agent, task, .. } => json!({ "kind": "note", "text": format!("{agent}: {}", task.lines().next().unwrap_or("")) }),
        Entry::Artifact { kind, path } => json!({ "kind": "file", "file": file_json(kind.label(), path, dir) }),
        Entry::Model { plugin, path } => json!({ "kind": "file", "file": file_json(&th_harness::live::program_name(plugin), path, dir) }),
        Entry::Error { text } => json!({ "kind": "error", "text": text }),
        Entry::Usage { report } => {
            let (_, p, c) = report.totals();
            json!({ "kind": "end", "outcome": report.outcome, "seconds": report.seconds, "tokens": p + c })
        }
        Entry::Thinking { .. } | Entry::Tool { .. } | Entry::Done { .. } => return None,
    };
    let mut v = v;
    v["id"] = json!(i);
    Some(v)
}

/// A live engine event as the phone shows it.
fn live_json(e: &Event, depth: &mut HashMap<String, usize>, dir: &Path) -> Vec<Value> {
    let d = |depth: &HashMap<String, usize>, a: &str| depth.get(a).copied().unwrap_or(0);
    match e {
        Event::AgentStart { agent, depth: dp, task, .. } => {
            depth.insert(agent.clone(), *dp);
            if *dp > 0 {
                return vec![json!({ "type": "note", "text": format!("{agent}: {}", task.lines().next().unwrap_or("")) })];
            }
            vec![]
        }
        Event::Stream { agent, kind, text, .. } if kind == "text" && d(depth, agent) == 0 => vec![json!({ "type": "delta", "text": text })],
        Event::Text { agent, text, .. } if d(depth, agent) == 0 => vec![json!({ "type": "message", "agent": agent, "text": text })],
        Event::Text { agent, text, .. } => vec![json!({ "type": "team", "agent": agent, "text": text })],
        Event::Artifact { kind, path, .. } => vec![json!({ "type": "file", "file": file_json(kind.label(), path, dir) })],
        Event::Model { plugin, path, .. } => vec![json!({ "type": "file", "file": file_json(&th_harness::live::program_name(plugin), path, dir) })],
        Event::Error { text, .. } => vec![json!({ "type": "error", "text": text })],
        _ => vec![],
    }
}

fn content_type(p: &Path) -> &'static str {
    match p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "heic" => "image/heic",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "json" => "application/json",
        "html" | "htm" => "text/html; charset=utf-8",
        "txt" | "md" | "csv" | "log" => "text/plain; charset=utf-8",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "dxf" => "application/dxf",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_queries() {
        assert_eq!(percent_decode("reports%2Fbeam%20B1.pdf"), "reports/beam B1.pdf");
        assert_eq!(percent_decode("a+b"), "a b");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%e0%b8%81"), "ก");
    }

    #[test]
    fn compares_tokens() {
        assert!(same("abc", "abc"));
        assert!(!same("abc", "abd"));
        assert!(!same("abc", "abcd"));
    }
}
