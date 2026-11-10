//! Model Context Protocol client: stdio servers (a local program such as `tellus --mcp`) and
//! Streamable-HTTP servers (a URL). Also a minimal stdio server loop so this program can offer
//! its own tools to Claude Code and Codex.
//!
//! Only the parts the harness needs: `initialize`, `tools/list`, `tools/call`.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const PROTOCOL: &str = "2025-03-26";

/// How to start or reach a server; the "plugins" an agent can be equipped with.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct ServerConfig {
    pub name: String,
    pub description: String,
    /// stdio: the program and its arguments.
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub cwd: String,
    /// http: the endpoint (used when `command` is empty).
    pub url: String,
    pub headers: HashMap<String, String>,
    pub enabled: bool,
    /// Seconds allowed for one tool call (FE analyses can take minutes).
    pub timeout_s: u64,
}

impl ServerConfig {
    pub fn is_http(&self) -> bool {
        self.command.trim().is_empty() && !self.url.trim().is_empty()
    }
    fn timeout(&self) -> Duration {
        Duration::from_secs(if self.timeout_s == 0 { 600 } else { self.timeout_s })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolInfo {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(rename = "inputSchema", default = "empty_schema")]
    pub input_schema: Value,
}

fn empty_schema() -> Value {
    json!({"type": "object", "properties": {}})
}

/// The result of a tool call: its text and any images (base64 + MIME type).
#[derive(Debug, Clone, Default)]
pub struct CallResult {
    pub text: String,
    pub images: Vec<(String, String)>,
    pub is_error: bool,
}

pub fn parse_call_result(v: &Value) -> CallResult {
    let mut out = CallResult { is_error: v.get("isError").and_then(|b| b.as_bool()).unwrap_or(false), ..Default::default() };
    let mut texts = vec![];
    if let Some(arr) = v.get("content").and_then(|c| c.as_array()) {
        for c in arr {
            match c.get("type").and_then(|t| t.as_str()) {
                Some("text") => texts.push(c.get("text").and_then(|t| t.as_str()).unwrap_or("").to_string()),
                Some("image") => out.images.push((
                    c.get("data").and_then(|t| t.as_str()).unwrap_or("").to_string(),
                    c.get("mimeType").and_then(|t| t.as_str()).unwrap_or("image/png").to_string(),
                )),
                Some("resource") => texts.push(c.pointer("/resource/text").and_then(|t| t.as_str()).unwrap_or("").to_string()),
                _ => {}
            }
        }
    }
    if texts.is_empty() {
        if let Some(s) = v.get("structuredContent") {
            texts.push(s.to_string());
        }
    }
    out.text = texts.join("\n");
    out
}

enum Transport {
    Stdio { child: Child, stdin: ChildStdin, pending: Arc<Mutex<HashMap<u64, Sender<Value>>>>, stderr: Arc<Mutex<String>> },
    Http { agent: ureq::Agent, url: String, headers: HashMap<String, String>, session: Option<String> },
}

/// A connected server.
pub struct Client {
    pub config: ServerConfig,
    pub server_name: String,
    pub tools: Vec<ToolInfo>,
    transport: Mutex<Transport>,
    next_id: Mutex<u64>,
}

impl Client {
    pub fn connect(config: &ServerConfig) -> Result<Client, String> {
        let transport = if config.is_http() {
            let agent = ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(15)).timeout(config.timeout()).build();
            Transport::Http { agent, url: config.url.clone(), headers: config.headers.clone(), session: None }
        } else {
            spawn_stdio(config)?
        };
        let mut c = Client { config: config.clone(), server_name: String::new(), tools: vec![], transport: Mutex::new(transport), next_id: Mutex::new(1) };
        let init = c.request(
            "initialize",
            json!({"protocolVersion": PROTOCOL, "capabilities": {}, "clientInfo": {"name": "tellushive", "version": env!("CARGO_PKG_VERSION")}}),
            Duration::from_secs(60),
        )?;
        c.server_name = init.pointer("/serverInfo/name").and_then(|n| n.as_str()).unwrap_or(&config.name).to_string();
        c.notify("notifications/initialized", json!({}))?;
        c.refresh_tools()?;
        Ok(c)
    }

    pub fn refresh_tools(&mut self) -> Result<(), String> {
        let mut tools = vec![];
        let mut cursor: Option<String> = None;
        loop {
            let params = match &cursor {
                Some(c) => json!({"cursor": c}),
                None => json!({}),
            };
            let r = self.request("tools/list", params, Duration::from_secs(60))?;
            if let Some(arr) = r.get("tools") {
                let page: Vec<ToolInfo> = serde_json::from_value(arr.clone()).map_err(|e| format!("tools/list: {e}"))?;
                tools.extend(page);
            }
            cursor = r.get("nextCursor").and_then(|c| c.as_str()).map(String::from);
            if cursor.is_none() {
                break;
            }
        }
        self.tools = tools;
        Ok(())
    }

    pub fn call(&self, tool: &str, args: Value) -> Result<CallResult, String> {
        let r = self.request("tools/call", json!({"name": tool, "arguments": args}), self.config.timeout())?;
        Ok(parse_call_result(&r))
    }

    fn id(&self) -> u64 {
        let mut n = self.next_id.lock().unwrap_or_else(|e| e.into_inner());
        *n += 1;
        *n
    }

    fn notify(&self, method: &str, params: Value) -> Result<(), String> {
        let msg = json!({"jsonrpc": "2.0", "method": method, "params": params});
        let mut t = self.transport.lock().map_err(|_| "transport poisoned")?;
        match &mut *t {
            Transport::Stdio { stdin, .. } => write_line(stdin, &msg),
            Transport::Http { agent, url, headers, session } => {
                let _ = http_post(agent, url, headers, session, &msg);
                Ok(())
            }
        }
    }

    fn request(&self, method: &str, params: Value, timeout: Duration) -> Result<Value, String> {
        let id = self.id();
        let msg = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let rx: Receiver<Value>;
        {
            let mut t = self.transport.lock().map_err(|_| "transport poisoned")?;
            match &mut *t {
                Transport::Stdio { stdin, pending, child, stderr } => {
                    if let Ok(Some(status)) = child.try_wait() {
                        let err = stderr.lock().map(|s| s.clone()).unwrap_or_default();
                        return Err(format!("server '{}' exited ({status}). {}", self.config.name, tail(&err, 600)));
                    }
                    let (tx, r) = mpsc::channel();
                    pending.lock().map_err(|_| "poisoned")?.insert(id, tx);
                    write_line(stdin, &msg)?;
                    rx = r;
                }
                Transport::Http { agent, url, headers, session } => {
                    let v = http_post(agent, url, headers, session, &msg)?.ok_or_else(|| format!("{method}: empty HTTP reply"))?;
                    return unwrap_rpc(v, method);
                }
            }
        }
        let v = rx.recv_timeout(timeout).map_err(|_| format!("{method}: no reply from '{}' within {} s", self.config.name, timeout.as_secs()))?;
        unwrap_rpc(v, method)
    }

    /// The server's stderr so far (stdio only), for the plugin panel.
    pub fn stderr(&self) -> String {
        match &*self.transport.lock().unwrap_or_else(|e| e.into_inner()) {
            Transport::Stdio { stderr, .. } => stderr.lock().map(|s| s.clone()).unwrap_or_default(),
            _ => String::new(),
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        if let Ok(mut t) = self.transport.lock() {
            if let Transport::Stdio { child, .. } = &mut *t {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

fn tail(s: &str, n: usize) -> String {
    let c: Vec<char> = s.chars().collect();
    c[c.len().saturating_sub(n)..].iter().collect()
}

fn unwrap_rpc(v: Value, method: &str) -> Result<Value, String> {
    if let Some(e) = v.get("error") {
        return Err(format!("{method}: {}", e.get("message").and_then(|m| m.as_str()).unwrap_or(&e.to_string())));
    }
    Ok(v.get("result").cloned().unwrap_or(Value::Null))
}

fn write_line(w: &mut impl Write, v: &Value) -> Result<(), String> {
    let mut s = serde_json::to_string(v).map_err(|e| e.to_string())?;
    s.push('\n');
    w.write_all(s.as_bytes()).and_then(|_| w.flush()).map_err(|e| format!("write to server: {e}"))
}

fn spawn_stdio(cfg: &ServerConfig) -> Result<Transport, String> {
    let mut cmd = Command::new(&cfg.command);
    cmd.args(&cfg.args).envs(&cfg.env).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    if !cfg.cwd.trim().is_empty() {
        cmd.current_dir(&cfg.cwd);
    }
    let mut child = cmd.spawn().map_err(|e| format!("cannot start '{}': {e}", cfg.command))?;
    let stdin = child.stdin.take().ok_or("no stdin")?;
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let err_pipe = child.stderr.take().ok_or("no stderr")?;
    let pending: Arc<Mutex<HashMap<u64, Sender<Value>>>> = Arc::default();
    let stderr: Arc<Mutex<String>> = Arc::default();
    let p = pending.clone();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            let Ok(v) = serde_json::from_str::<Value>(line.trim()) else { continue };
            // a reply carries our id; server requests and notifications are ignored
            if v.get("method").is_none() {
                if let Some(id) = v.get("id").and_then(|i| i.as_u64()) {
                    if let Some(tx) = p.lock().ok().and_then(|mut m| m.remove(&id)) {
                        let _ = tx.send(v);
                    }
                }
            }
        }
    });
    let e = stderr.clone();
    std::thread::spawn(move || {
        for line in BufReader::new(err_pipe).lines().map_while(Result::ok) {
            if let Ok(mut s) = e.lock() {
                s.push_str(&line);
                s.push('\n');
                if s.len() > 64_000 {
                    let cut = s.len() - 32_000;
                    let cut = (cut..s.len()).find(|i| s.is_char_boundary(*i)).unwrap_or(0);
                    s.drain(..cut);
                }
            }
        }
    });
    Ok(Transport::Stdio { child, stdin, pending, stderr })
}

/// POSTs one JSON-RPC message; the reply may be JSON or an SSE stream.
fn http_post(agent: &ureq::Agent, url: &str, headers: &HashMap<String, String>, session: &mut Option<String>, msg: &Value) -> Result<Option<Value>, String> {
    let mut req = agent.post(url).set("Content-Type", "application/json").set("Accept", "application/json, text/event-stream").set("MCP-Protocol-Version", PROTOCOL);
    for (k, v) in headers {
        req = req.set(k, v);
    }
    if let Some(s) = session.as_ref() {
        req = req.set("Mcp-Session-Id", s);
    }
    let r = req.send_string(&msg.to_string()).map_err(|e| match e {
        ureq::Error::Status(c, r) => format!("HTTP {c}: {}", r.into_string().unwrap_or_default().chars().take(300).collect::<String>()),
        e => e.to_string(),
    })?;
    if let Some(s) = r.header("Mcp-Session-Id") {
        *session = Some(s.to_string());
    }
    if r.status() == 202 {
        return Ok(None);
    }
    let ctype = r.content_type().to_string();
    let body = r.into_string().map_err(|e| e.to_string())?;
    if ctype.contains("event-stream") {
        let want = msg.get("id").cloned();
        let mut data = String::new();
        for line in body.lines().chain(std::iter::once("")) {
            if let Some(d) = line.strip_prefix("data:") {
                data.push_str(d.trim_start());
            } else if line.trim().is_empty() && !data.is_empty() {
                if let Ok(v) = serde_json::from_str::<Value>(&data) {
                    if v.get("id") == want.as_ref() {
                        return Ok(Some(v));
                    }
                }
                data.clear();
            }
        }
        return Err("SSE stream ended without a reply".into());
    }
    if body.trim().is_empty() {
        return Ok(None);
    }
    serde_json::from_str(&body).map(Some).map_err(|e| format!("bad JSON from server: {e}"))
}

// ------------------------------------------------------------------ server

/// A tool this program offers over MCP.
pub struct ServedTool {
    pub info: ToolInfo,
    pub run: Box<dyn Fn(Value) -> Result<String, String> + Send + Sync>,
}

/// Serves tools over stdio until stdin closes (newline-delimited JSON-RPC).
pub fn serve_stdio(name: &str, tools: Vec<ServedTool>) {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in stdin.lock().lines().map_while(Result::ok) {
        let Ok(v) = serde_json::from_str::<Value>(line.trim()) else { continue };
        let Some(id) = v.get("id").cloned() else { continue };
        let method = v.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let result = match method {
            "initialize" => Ok(json!({"protocolVersion": v.pointer("/params/protocolVersion").cloned().unwrap_or(json!(PROTOCOL)),
                "capabilities": {"tools": {}}, "serverInfo": {"name": name, "version": env!("CARGO_PKG_VERSION")}})),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({"tools": tools.iter().map(|t| serde_json::to_value(&t.info).unwrap_or(Value::Null)).collect::<Vec<_>>()})),
            "tools/call" => {
                let tname = v.pointer("/params/name").and_then(|n| n.as_str()).unwrap_or("");
                let args = v.pointer("/params/arguments").cloned().unwrap_or(json!({}));
                match tools.iter().find(|t| t.info.name == tname) {
                    Some(t) => match (t.run)(args) {
                        Ok(text) => Ok(json!({"content": [{"type": "text", "text": text}]})),
                        Err(e) => Ok(json!({"content": [{"type": "text", "text": e}], "isError": true})),
                    },
                    None => Err(format!("unknown tool {tname}")),
                }
            }
            _ => Err(format!("method not found: {method}")),
        };
        let reply = match result {
            Ok(r) => json!({"jsonrpc": "2.0", "id": id, "result": r}),
            Err(e) => json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": e}}),
        };
        if write_line(&mut out, &reply).is_err() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_text_and_images() {
        let r = parse_call_result(&json!({"content":[{"type":"text","text":"a"},{"type":"image","data":"QQ==","mimeType":"image/png"},{"type":"text","text":"b"}],"isError":false}));
        assert_eq!(r.text, "a\nb");
        assert_eq!(r.images.len(), 1);
        assert!(!r.is_error);
    }

    /// Talks to a tiny shell-script server to prove the stdio framing.
    #[test]
    fn stdio_round_trip() {
        let script = r#"while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
  case "$line" in
    *initialize*) printf '{"jsonrpc":"2.0","id":%s,"result":{"serverInfo":{"name":"fake"},"capabilities":{}}}\n' "$id";;
    *tools/list*) printf '{"jsonrpc":"2.0","id":%s,"result":{"tools":[{"name":"echo","description":"e","inputSchema":{"type":"object"}}]}}\n' "$id";;
    *tools/call*) printf '{"jsonrpc":"2.0","id":%s,"result":{"content":[{"type":"text","text":"pong"}]}}\n' "$id";;
  esac
done"#;
        let cfg = ServerConfig { name: "fake".into(), command: "sh".into(), args: vec!["-c".into(), script.into()], enabled: true, ..Default::default() };
        let c = Client::connect(&cfg).unwrap();
        assert_eq!(c.server_name, "fake");
        assert_eq!(c.tools[0].name, "echo");
        assert_eq!(c.call("echo", json!({})).unwrap().text, "pong");
    }
}
