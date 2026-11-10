//! OpenAI-compatible chat-completions client with tool calls, image inputs and embeddings.
//!
//! One client talks to every provider that speaks the OpenAI wire format: DeepSeek (the
//! default brain of the harness), OpenRouter, Ollama, OpenAI and any self-hosted gateway.
//! The calls are blocking (ureq) because every agent already runs on its own thread.

use std::time::Duration;

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Where a model lives and how to reach it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Provider {
    /// Display name, also the key agents use to pick a provider ("deepseek", "openrouter").
    pub name: String,
    pub base_url: String,
    /// Key stored in the settings file; empty means "read `key_env`".
    pub api_key: String,
    pub key_env: String,
    /// Model used when an agent does not name one.
    pub default_model: String,
    /// Whether the provider's models accept image parts. DeepSeek's API does not.
    pub vision: bool,
}

impl Default for Provider {
    fn default() -> Self {
        Provider::deepseek()
    }
}

impl Provider {
    pub fn deepseek() -> Self {
        Provider {
            name: "deepseek".into(),
            base_url: "https://api.deepseek.com/v1".into(),
            api_key: String::new(),
            key_env: "DEEPSEEK_API_KEY".into(),
            default_model: "deepseek-chat".into(),
            vision: false,
        }
    }
    pub fn openrouter() -> Self {
        Provider {
            name: "openrouter".into(),
            base_url: "https://openrouter.ai/api/v1".into(),
            api_key: String::new(),
            key_env: "OPENROUTER_API_KEY".into(),
            default_model: "deepseek/deepseek-v4.1-flash-1017".into(),
            vision: true,
        }
    }
    pub fn ollama() -> Self {
        Provider {
            name: "ollama".into(),
            base_url: "http://localhost:11434/v1".into(),
            api_key: String::new(),
            key_env: String::new(),
            default_model: "qwen2.5:14b".into(),
            vision: false,
        }
    }
    pub fn openai() -> Self {
        Provider {
            name: "openai".into(),
            base_url: "https://api.openai.com/v1".into(),
            api_key: String::new(),
            key_env: "OPENAI_API_KEY".into(),
            default_model: "gpt-4.1-mini".into(),
            vision: true,
        }
    }
    pub fn presets() -> Vec<Provider> {
        vec![Provider::deepseek(), Provider::openrouter(), Provider::ollama(), Provider::openai()]
    }

    /// The key to send: the stored one, else the environment variable.
    pub fn key(&self) -> Option<String> {
        if !self.api_key.trim().is_empty() {
            return Some(self.api_key.trim().to_string());
        }
        if !self.key_env.is_empty() {
            if let Ok(k) = std::env::var(&self.key_env) {
                if !k.trim().is_empty() {
                    return Some(k.trim().to_string());
                }
            }
        }
        None
    }

    fn url(&self, path: &str) -> String {
        format!("{}/{}", self.base_url.trim_end_matches('/'), path)
    }
}

// ------------------------------------------------------------------ messages

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Part {
    Text { text: String },
    /// A data URI (`data:image/png;base64,...`) or an http(s) URL.
    ImageUrl { image_url: ImageUrl },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImageUrl {
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum Content {
    Text(String),
    Parts(Vec<Part>),
}

impl Content {
    pub fn text(&self) -> String {
        match self {
            Content::Text(s) => s.clone(),
            Content::Parts(p) => p
                .iter()
                .filter_map(|p| match p {
                    Part::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }
    pub fn has_images(&self) -> bool {
        matches!(self, Content::Parts(p) if p.iter().any(|p| matches!(p, Part::ImageUrl { .. })))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type", default = "function_type")]
    pub kind: String,
    pub function: FunctionCall,
}

fn function_type() -> String {
    "function".into()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FunctionCall {
    pub name: String,
    /// JSON text as the model wrote it (may be malformed; the harness reports that back).
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Message {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<Content>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// DeepSeek thinking-mode reasoning, passed back inside the turn that produced it
    /// (the harness sets it only for providers that ask for it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
}

impl Message {
    pub fn system(s: impl Into<String>) -> Self {
        Message { role: "system".into(), content: Some(Content::Text(s.into())), tool_calls: None, tool_call_id: None, name: None, reasoning_content: None }
    }
    pub fn user(s: impl Into<String>) -> Self {
        Message { role: "user".into(), content: Some(Content::Text(s.into())), tool_calls: None, tool_call_id: None, name: None, reasoning_content: None }
    }
    pub fn user_parts(parts: Vec<Part>) -> Self {
        Message { role: "user".into(), content: Some(Content::Parts(parts)), tool_calls: None, tool_call_id: None, name: None, reasoning_content: None }
    }
    pub fn assistant(s: impl Into<String>) -> Self {
        Message { role: "assistant".into(), content: Some(Content::Text(s.into())), tool_calls: None, tool_call_id: None, name: None, reasoning_content: None }
    }
    pub fn tool(id: &str, s: impl Into<String>) -> Self {
        Message { role: "tool".into(), content: Some(Content::Text(s.into())), tool_calls: None, tool_call_id: Some(id.into()), name: None, reasoning_content: None }
    }
    pub fn text(&self) -> String {
        self.content.as_ref().map(|c| c.text()).unwrap_or_default()
    }
}

/// An image file as an image part (data URI).
pub fn image_part(path: &std::path::Path) -> Result<Part, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("png").to_ascii_lowercase();
    let mime = match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => "image/png",
    };
    let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
    Ok(Part::ImageUrl { image_url: ImageUrl { url: format!("data:{mime};base64,{b64}") } })
}

/// A function the model may call.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    /// JSON Schema of the arguments (an object schema).
    pub parameters: Value,
}

impl ToolSpec {
    fn wire(&self) -> Value {
        json!({"type": "function", "function": {"name": self.name, "description": self.description, "parameters": self.parameters}})
    }
}

#[derive(Debug, Clone, Default)]
pub struct Usage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    /// Prompt tokens served from the provider's prefix cache (DeepSeek `prompt_cache_hit_tokens`).
    pub cached_tokens: u64,
}

/// The assistant turn the model returned.
#[derive(Debug, Clone)]
pub struct Reply {
    pub message: Message,
    /// DeepSeek-reasoner's thinking, kept for display only (never sent back).
    pub reasoning: Option<String>,
    pub finish_reason: String,
    pub usage: Usage,
}

#[derive(Debug, Clone)]
pub struct ChatOptions {
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub timeout: Duration,
}

impl Default for ChatOptions {
    fn default() -> Self {
        ChatOptions { temperature: Some(0.2), max_tokens: Some(8192), timeout: Duration::from_secs(300) }
    }
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(20)).timeout(timeout).build()
}

fn http_error(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(code, r) => {
            let body = r.into_string().unwrap_or_default();
            let msg = serde_json::from_str::<Value>(&body)
                .ok()
                .and_then(|v| v.pointer("/error/message").and_then(|m| m.as_str()).map(String::from))
                .unwrap_or_else(|| body.chars().take(400).collect());
            format!("HTTP {code}: {msg}")
        }
        other => other.to_string(),
    }
}

/// Builds the request body (public so it can be unit-tested and logged).
pub fn request_body(model: &str, messages: &[Message], tools: &[ToolSpec], opt: &ChatOptions) -> Value {
    let mut body = json!({"model": model, "messages": messages});
    if !tools.is_empty() {
        body["tools"] = Value::Array(tools.iter().map(|t| t.wire()).collect());
        body["tool_choice"] = json!("auto");
    }
    // the reasoner rejects sampling parameters
    if let Some(t) = opt.temperature {
        if !model.contains("reasoner") {
            body["temperature"] = json!(t);
        }
    }
    if let Some(m) = opt.max_tokens {
        body["max_tokens"] = json!(m);
    }
    body
}

/// Parses a chat-completions response body.
pub fn parse_reply(v: &Value) -> Result<Reply, String> {
    if let Some(err) = v.get("error") {
        return Err(format!("provider error: {}", err.get("message").and_then(|m| m.as_str()).unwrap_or(&err.to_string())));
    }
    let choice = v.pointer("/choices/0").ok_or_else(|| format!("no choices in reply: {}", truncate(&v.to_string(), 300)))?;
    let m = choice.get("message").ok_or("no message in reply")?;
    let content = m.get("content").and_then(|c| c.as_str()).map(|s| Content::Text(s.to_string()));
    let mut tool_calls: Vec<ToolCall> = vec![];
    if let Some(arr) = m.get("tool_calls").and_then(|t| t.as_array()) {
        for (i, tc) in arr.iter().enumerate() {
            let name = tc.pointer("/function/name").and_then(|n| n.as_str()).unwrap_or("").to_string();
            // some providers send arguments as an object instead of a string
            let args = match tc.pointer("/function/arguments") {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Null) | None => "{}".into(),
                Some(other) => other.to_string(),
            };
            let id = tc.get("id").and_then(|s| s.as_str()).filter(|s| !s.is_empty()).map(String::from).unwrap_or_else(|| format!("call_{i}"));
            tool_calls.push(ToolCall { id, kind: "function".into(), function: FunctionCall { name, arguments: args } });
        }
    }
    let reasoning = m.get("reasoning_content").or_else(|| m.get("reasoning")).and_then(|r| r.as_str()).filter(|s| !s.is_empty()).map(String::from);
    let usage = Usage {
        prompt_tokens: v.pointer("/usage/prompt_tokens").and_then(|x| x.as_u64()).unwrap_or(0),
        completion_tokens: v.pointer("/usage/completion_tokens").and_then(|x| x.as_u64()).unwrap_or(0),
        cached_tokens: cached_tokens(v.get("usage")),
    };
    Ok(Reply {
        message: Message {
            role: "assistant".into(),
            content,
            tool_calls: if tool_calls.is_empty() { None } else { Some(tool_calls) },
            tool_call_id: None,
            name: None,
            reasoning_content: None,
        },
        reasoning,
        finish_reason: choice.get("finish_reason").and_then(|f| f.as_str()).unwrap_or("").to_string(),
        usage,
    })
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n).collect::<String>())
    }
}

/// One chat-completions call. Retries twice on rate limits and server errors.
pub fn chat(p: &Provider, model: &str, messages: &[Message], tools: &[ToolSpec], opt: &ChatOptions) -> Result<Reply, String> {
    let model = if model.is_empty() { p.default_model.as_str() } else { model };
    let body = request_body(model, messages, tools, opt);
    let a = agent(opt.timeout);
    let mut last = String::new();
    for attempt in 0..3 {
        if attempt > 0 {
            std::thread::sleep(Duration::from_secs(2 * attempt as u64 + 1));
        }
        let mut req = a.post(&p.url("chat/completions")).set("Content-Type", "application/json");
        if let Some(k) = p.key() {
            req = req.set("Authorization", &format!("Bearer {k}"));
        }
        if p.base_url.contains("openrouter.ai") {
            req = req.set("X-Title", "TellusHive");
        }
        match req.send_json(body.clone()) {
            Ok(r) => {
                let v: Value = r.into_json().map_err(|e| format!("bad JSON from {}: {e}", p.name))?;
                return parse_reply(&v);
            }
            Err(ureq::Error::Status(code, r)) if code == 429 || code >= 500 => {
                last = http_error(ureq::Error::Status(code, r));
            }
            Err(e) => return Err(format!("{}: {}", p.name, http_error(e))),
        }
    }
    Err(format!("{}: {last}", p.name))
}

fn cached_tokens(u: Option<&Value>) -> u64 {
    let Some(u) = u else { return 0 };
    u.get("prompt_cache_hit_tokens").or_else(|| u.pointer("/prompt_tokens_details/cached_tokens")).and_then(|x| x.as_u64()).unwrap_or(0)
}

/// A piece of a streamed reply.
#[derive(Debug, Clone, PartialEq)]
pub enum Delta {
    Text(String),
    Reasoning(String),
    /// A tool call started (its name is known).
    ToolCall { index: usize, name: String },
}

/// Why a call failed, so the harness can decide to retry or compact.
#[derive(Debug, Clone, PartialEq)]
pub struct CallError {
    pub message: String,
    /// HTTP status, when the provider answered.
    pub status: Option<u16>,
    /// The request was longer than the model's context window.
    pub overflow: bool,
    pub cancelled: bool,
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

fn is_overflow(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    ["maximum context length", "context length", "context_length_exceeded", "too many tokens", "prompt is too long", "context window"].iter().any(|k| m.contains(k))
}

/// Folds streamed chunks (`chat.completion.chunk`) into one reply.
#[derive(Default)]
pub struct StreamFold {
    text: String,
    reasoning: String,
    calls: Vec<(String, String, String)>,
    finish: String,
    usage: Usage,
}

impl StreamFold {
    /// Takes one `data:` payload; returns the deltas it carried.
    pub fn push(&mut self, v: &Value) -> Result<Vec<Delta>, String> {
        if let Some(err) = v.get("error") {
            return Err(err.get("message").and_then(|m| m.as_str()).map(String::from).unwrap_or_else(|| err.to_string()));
        }
        let mut out = vec![];
        if let Some(u) = v.get("usage").filter(|u| !u.is_null()) {
            self.usage = Usage {
                prompt_tokens: u.get("prompt_tokens").and_then(|x| x.as_u64()).unwrap_or(0),
                completion_tokens: u.get("completion_tokens").and_then(|x| x.as_u64()).unwrap_or(0),
                cached_tokens: cached_tokens(Some(u)),
            };
        }
        let Some(choice) = v.pointer("/choices/0") else { return Ok(out) };
        if let Some(f) = choice.get("finish_reason").and_then(|f| f.as_str()) {
            self.finish = f.to_string();
        }
        let Some(d) = choice.get("delta") else { return Ok(out) };
        if let Some(t) = d.get("content").and_then(|c| c.as_str()).filter(|t| !t.is_empty()) {
            self.text.push_str(t);
            out.push(Delta::Text(t.to_string()));
        }
        if let Some(t) = d.get("reasoning_content").or_else(|| d.get("reasoning")).and_then(|c| c.as_str()).filter(|t| !t.is_empty()) {
            self.reasoning.push_str(t);
            out.push(Delta::Reasoning(t.to_string()));
        }
        if let Some(arr) = d.get("tool_calls").and_then(|t| t.as_array()) {
            for tc in arr {
                let index = tc.get("index").and_then(|i| i.as_u64()).map(|i| i as usize).unwrap_or(self.calls.len().saturating_sub(1));
                while self.calls.len() <= index {
                    self.calls.push(Default::default());
                }
                let slot = &mut self.calls[index];
                if let Some(id) = tc.get("id").and_then(|s| s.as_str()).filter(|s| !s.is_empty()) {
                    slot.0 = id.to_string();
                }
                if let Some(n) = tc.pointer("/function/name").and_then(|s| s.as_str()).filter(|s| !s.is_empty()) {
                    let first = slot.1.is_empty();
                    slot.1.push_str(n);
                    if first {
                        out.push(Delta::ToolCall { index, name: n.to_string() });
                    }
                }
                match tc.pointer("/function/arguments") {
                    Some(Value::String(a)) => slot.2.push_str(a),
                    Some(Value::Null) | None => {}
                    Some(other) => slot.2.push_str(&other.to_string()),
                }
            }
        }
        Ok(out)
    }

    pub fn finish(self) -> Reply {
        let tool_calls: Vec<ToolCall> = self
            .calls
            .into_iter()
            .enumerate()
            .filter(|(_, c)| !c.1.is_empty())
            .map(|(i, (id, name, args))| ToolCall {
                id: if id.is_empty() { format!("call_{i}") } else { id },
                kind: "function".into(),
                function: FunctionCall { name, arguments: if args.trim().is_empty() { "{}".into() } else { args } },
            })
            .collect();
        Reply {
            message: Message {
                role: "assistant".into(),
                content: if self.text.is_empty() && !tool_calls.is_empty() { None } else { Some(Content::Text(self.text)) },
                tool_calls: if tool_calls.is_empty() { None } else { Some(tool_calls) },
                tool_call_id: None,
                name: None,
                reasoning_content: None,
            },
            reasoning: if self.reasoning.is_empty() { None } else { Some(self.reasoning) },
            finish_reason: self.finish,
            usage: self.usage,
        }
    }
}

/// One streamed chat-completions call (server-sent events). `on` sees every delta as it
/// arrives; the reply is the same as `chat` would return. No retries here: the harness
/// owns retry policy.
pub fn chat_stream(p: &Provider, model: &str, messages: &[Message], tools: &[ToolSpec], opt: &ChatOptions, on: &mut dyn FnMut(Delta), cancel: &std::sync::atomic::AtomicBool) -> Result<Reply, CallError> {
    use std::io::BufRead;
    use std::sync::atomic::Ordering;
    let err = |message: String, status: Option<u16>| CallError { overflow: is_overflow(&message), message, status, cancelled: false };
    let model = if model.is_empty() { p.default_model.as_str() } else { model };
    let mut body = request_body(model, messages, tools, opt);
    body["stream"] = json!(true);
    body["stream_options"] = json!({"include_usage": true});
    let mut req = agent(opt.timeout).post(&p.url("chat/completions")).set("Content-Type", "application/json").set("Accept", "text/event-stream");
    if let Some(k) = p.key() {
        req = req.set("Authorization", &format!("Bearer {k}"));
    }
    if p.base_url.contains("openrouter.ai") {
        req = req.set("X-Title", "TellusHive");
    }
    let resp = match req.send_json(body) {
        Ok(r) => r,
        Err(ureq::Error::Status(code, r)) => return Err(err(format!("{}: {}", p.name, http_error(ureq::Error::Status(code, r))), Some(code))),
        Err(e) => return Err(err(format!("{}: {}", p.name, http_error(e)), None)),
    };
    let mut fold = StreamFold::default();
    for line in std::io::BufReader::new(resp.into_reader()).lines() {
        if cancel.load(Ordering::Relaxed) {
            return Err(CallError { message: "cancelled".into(), status: None, overflow: false, cancelled: true });
        }
        let line = line.map_err(|e| err(format!("{}: stream broke: {e}", p.name), None))?;
        let Some(data) = line.strip_prefix("data:").map(str::trim) else { continue };
        if data == "[DONE]" {
            break;
        }
        let Ok(v) = serde_json::from_str::<Value>(data) else { continue };
        for d in fold.push(&v).map_err(|m| err(format!("{}: {m}", p.name), None))? {
            on(d);
        }
    }
    Ok(fold.finish())
}

/// Embeddings through the OpenAI `/embeddings` endpoint.
pub fn embed(p: &Provider, model: &str, inputs: &[String]) -> Result<Vec<Vec<f32>>, String> {
    let a = agent(Duration::from_secs(120));
    let mut req = a.post(&p.url("embeddings"));
    if let Some(k) = p.key() {
        req = req.set("Authorization", &format!("Bearer {k}"));
    }
    let v: Value = req.send_json(json!({"model": model, "input": inputs})).map_err(http_error)?.into_json().map_err(|e| e.to_string())?;
    let data = v.get("data").and_then(|d| d.as_array()).ok_or("no data in embeddings reply")?;
    Ok(data
        .iter()
        .map(|d| d.get("embedding").and_then(|e| e.as_array()).map(|e| e.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect()).unwrap_or_default())
        .collect())
}

/// Lists the models a provider serves (for the settings combo box).
pub fn list_models(p: &Provider) -> Result<Vec<String>, String> {
    let a = agent(Duration::from_secs(20));
    let mut req = a.get(&p.url("models"));
    if let Some(k) = p.key() {
        req = req.set("Authorization", &format!("Bearer {k}"));
    }
    let v: Value = req.call().map_err(http_error)?.into_json().map_err(|e| e.to_string())?;
    let mut out: Vec<String> = v.get("data").and_then(|d| d.as_array()).map(|a| a.iter().filter_map(|m| m.get("id").and_then(|i| i.as_str()).map(String::from)).collect()).unwrap_or_default();
    out.sort();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_has_tools_and_no_temperature_for_reasoner() {
        let t = ToolSpec { name: "f".into(), description: "d".into(), parameters: json!({"type":"object","properties":{}}) };
        let b = request_body("deepseek-reasoner", &[Message::user("hi")], &[t], &ChatOptions::default());
        assert_eq!(b["tools"][0]["function"]["name"], "f");
        assert!(b.get("temperature").is_none());
        let b = request_body("deepseek-chat", &[Message::user("hi")], &[], &ChatOptions::default());
        assert!(b.get("tools").is_none());
        assert!(b.get("temperature").is_some());
    }

    #[test]
    fn parses_tool_calls_and_reasoning() {
        let v = json!({"choices":[{"finish_reason":"tool_calls","message":{"role":"assistant","content":null,"reasoning_content":"think",
            "tool_calls":[{"id":"c1","type":"function","function":{"name":"search","arguments":"{\"q\":\"x\"}"}},
                          {"function":{"name":"obj","arguments":{"a":1}}}]}}],"usage":{"prompt_tokens":10,"completion_tokens":5}});
        let r = parse_reply(&v).unwrap();
        let tc = r.message.tool_calls.unwrap();
        assert_eq!(tc.len(), 2);
        assert_eq!(tc[0].function.arguments, "{\"q\":\"x\"}");
        assert_eq!(tc[1].id, "call_1");
        assert_eq!(tc[1].function.arguments, "{\"a\":1}");
        assert_eq!(r.reasoning.as_deref(), Some("think"));
        assert_eq!(r.usage.prompt_tokens, 10);
    }

    #[test]
    fn folds_a_stream() {
        let mut f = StreamFold::default();
        let chunks = [
            json!({"choices":[{"delta":{"reasoning_content":"hm"}}]}),
            json!({"choices":[{"delta":{"content":"Hel"}}]}),
            json!({"choices":[{"delta":{"content":"lo"}}]}),
            json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c9","function":{"name":"calc","arguments":"{\"a\""}}]}}]}),
            json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":":1}"}}]},"finish_reason":"tool_calls"}]}),
            json!({"choices":[],"usage":{"prompt_tokens":7,"completion_tokens":3,"prompt_cache_hit_tokens":5}}),
        ];
        let mut deltas = vec![];
        for c in &chunks {
            deltas.extend(f.push(c).unwrap());
        }
        assert_eq!(deltas[2], Delta::Text("lo".into()));
        assert_eq!(deltas[3], Delta::ToolCall { index: 0, name: "calc".into() });
        let r = f.finish();
        assert_eq!(r.message.text(), "Hello");
        assert_eq!(r.reasoning.as_deref(), Some("hm"));
        let tc = r.message.tool_calls.unwrap();
        assert_eq!((tc[0].id.as_str(), tc[0].function.arguments.as_str()), ("c9", "{\"a\":1}"));
        assert_eq!(r.finish_reason, "tool_calls");
        assert_eq!((r.usage.prompt_tokens, r.usage.cached_tokens), (7, 5));
        assert!(is_overflow("This model's maximum context length is 65536 tokens"));
    }

    #[test]
    fn message_serialises_like_openai() {
        let m = Message::user_parts(vec![Part::Text { text: "see".into() }, Part::ImageUrl { image_url: ImageUrl { url: "data:x".into() } }]);
        let v = serde_json::to_value(&m).unwrap();
        assert_eq!(v["content"][1]["type"], "image_url");
        assert_eq!(v["content"][1]["image_url"]["url"], "data:x");
        let back: Message = serde_json::from_value(v).unwrap();
        assert!(back.content.unwrap().has_images());
    }
}
