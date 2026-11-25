//! Claude Code, Codex and Antigravity (`agy`) as workers of the coordinator or as an
//! agent's model. Claude Code and Codex as workers of the coordinator. Each run gets this program's own MCP
//! server (standards library + design tools of the open project), so the external agent can
//! search the same standards and save calculations into the same project.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::config::ExternalAgent;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Which {
    ClaudeCode,
    Codex,
    /// Google Antigravity's CLI, driven through its Agent Client Protocol server (`agy acp`).
    Agy,
}

impl Which {
    pub fn program(self) -> &'static str {
        match self {
            Which::ClaudeCode => "claude",
            Which::Codex => "codex",
            Which::Agy => "agy",
        }
    }
    /// The coding agent a `cli://…` provider runs.
    pub fn of_provider(base_url: &str) -> Which {
        if base_url.ends_with("codex") {
            Which::Codex
        } else if base_url.ends_with("agy") {
            Which::Agy
        } else {
            Which::ClaudeCode
        }
    }
    /// Its settings.
    pub fn config(self, s: &crate::config::Settings) -> ExternalAgent {
        match self {
            Which::ClaudeCode => s.claude_code.clone(),
            Which::Codex => s.codex.clone(),
            Which::Agy => s.agy.clone(),
        }
    }
}

/// Finds a program on PATH, also in the usual install folders a GUI app does not inherit.
pub fn find_program(name: &str, configured: &str) -> Option<PathBuf> {
    if !configured.trim().is_empty() {
        let p = PathBuf::from(configured.trim());
        return p.exists().then_some(p);
    }
    let home = dirs::home_dir().unwrap_or_default();
    let mut dirs: Vec<PathBuf> = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).collect();
    dirs.extend([home.join(".local/bin"), home.join(".npm-global/bin"), home.join(".cargo/bin"), PathBuf::from("/opt/homebrew/bin"), PathBuf::from("/usr/local/bin")]);
    #[cfg(windows)]
    if let Some(app) = dirs::data_dir() {
        // npm installs claude.cmd / codex.cmd here
        dirs.push(app.join("npm"));
    }
    // nvm installs node tools per version
    if let Ok(rd) = std::fs::read_dir(home.join(".nvm/versions/node")) {
        let mut v: Vec<PathBuf> = rd.flatten().map(|e| e.path().join("bin")).collect();
        v.sort();
        v.reverse();
        dirs.extend(v);
    }
    let names: Vec<String> = if cfg!(windows) { vec![format!("{name}.exe"), format!("{name}.cmd"), name.to_string()] } else { vec![name.to_string()] };
    dirs.into_iter().flat_map(|d| names.iter().map(move |n| d.join(n)).collect::<Vec<_>>()).find(|p| p.is_file())
}

/// PATH for the child: the program's own folder first (node for claude), then the usual ones.
fn child_path(prog: &Path) -> std::ffi::OsString {
    let mut parts: Vec<PathBuf> = prog.parent().map(|d| vec![d.to_path_buf()]).unwrap_or_default();
    parts.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()));
    if cfg!(unix) {
        parts.extend(["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"].map(PathBuf::from));
    }
    std::env::join_paths(parts).unwrap_or_default()
}

pub struct RunOutput {
    pub text: String,
    pub ok: bool,
    pub seconds: f64,
    /// Tokens the program reported (Claude Code's JSON result); zero when unknown.
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_tokens: u64,
}

/// An MCP server the external agent is given (`design` for the tool runs, `tellushive` when
/// the program runs as an agent's model).
#[derive(Debug, Clone)]
pub struct McpServer {
    pub name: String,
    pub command: PathBuf,
    pub args: Vec<String>,
}

/// One non-interactive run of Claude Code or Codex.
#[derive(Debug, Clone)]
pub struct Job<'a> {
    pub prompt: &'a str,
    pub workdir: &'a Path,
    /// The project folder, readable by the program.
    pub project: &'a Path,
    pub mcp: Option<McpServer>,
    /// Model name for the program (`--model` / `-m`); empty = its default.
    pub model: &'a str,
    /// Only the MCP server's tools are allowed (Claude Code `--allowedTools`).
    pub mcp_only: bool,
}

/// Runs one non-interactive task. `mcp_exe` + `project` give the external agent our tools.
pub fn run(which: Which, cfg: &ExternalAgent, prompt: &str, workdir: &Path, project: &Path, mcp_exe: Option<&Path>, cancel: &AtomicBool) -> Result<RunOutput, String> {
    let mcp = mcp_exe.map(|exe| McpServer { name: "design".into(), command: exe.to_path_buf(), args: vec!["--mcp".into(), "--project".into(), project.display().to_string()] });
    run_job(which, cfg, &Job { prompt, workdir, project, mcp, model: "", mcp_only: true }, cancel)
}

/// The environment of the tool server the coding agent starts. Claude Code starts MCP servers
/// with a basic environment only, so without this the server falls back to the user's own data
/// folder, settings and team (an experiment's delegated agents then ran on the user's default
/// model). Folders are passed as values. API keys are passed as `${NAME}` references, which
/// Claude Code expands from its own environment, so no key is written to the config file;
/// `with_keys` is false for Codex, which has no such expansion.
fn mcp_env(with_keys: bool) -> serde_json::Value {
    let mut env = serde_json::Map::new();
    for k in ["TELLUSHIVE_HOME", "XDG_DATA_HOME"] {
        if let Ok(v) = std::env::var(k) {
            // absolute: the server starts in the work folder, where a relative path points nowhere
            let v = std::path::absolute(&v).map(|p| p.display().to_string()).unwrap_or(v);
            env.insert(k.into(), json!(v));
        }
    }
    if with_keys {
        for (k, v) in std::env::vars() {
            let key_like = k.ends_with("_API_KEY") || k.starts_with("TH_KB_");
            if key_like && !v.is_empty() && k != "ANTHROPIC_API_KEY" && k != "OPENAI_API_KEY" {
                env.insert(k.clone(), json!(format!("${{{k}}}")));
            }
        }
    }
    serde_json::Value::Object(env)
}

/// Runs a job. The prompt goes in on stdin, so long conversations are not limited by the
/// size of a command line.
pub fn run_job(which: Which, cfg: &ExternalAgent, job: &Job, cancel: &AtomicBool) -> Result<RunOutput, String> {
    use std::io::Write;
    if !cfg.enabled {
        return Err(format!("{} is switched off in Settings", which.program()));
    }
    let prog = find_program(which.program(), &cfg.path).ok_or_else(|| format!("'{}' was not found; install it or set its path in Settings", which.program()))?;
    let workdir = job.workdir;
    std::fs::create_dir_all(workdir).map_err(|e| e.to_string())?;
    let mut cmd = Command::new(&prog);
    cmd.current_dir(workdir).env("PATH", child_path(&prog)).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    // a fresh session, not a child of a Claude Code session this app may run inside
    for (k, _) in std::env::vars() {
        if k == "CLAUDECODE" || k.starts_with("CLAUDE_CODE_") {
            cmd.env_remove(k);
        }
    }
    if which == Which::Agy {
        return run_acp(&prog, cfg, job, cancel);
    }
    if cfg.subscription {
        cmd.env_remove(if which == Which::ClaudeCode { "ANTHROPIC_API_KEY" } else { "OPENAI_API_KEY" });
    }
    static RUNS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let last_msg = workdir.join(format!(".codex_last_message_{}_{}_{}.txt", std::process::id(), crate::engine::stamp(), RUNS.fetch_add(1, Ordering::Relaxed)));
    match which {
        Which::ClaudeCode => {
            cmd.arg("-p").arg("--output-format").arg("json").arg("--add-dir").arg(job.project);
            if !job.model.is_empty() {
                cmd.arg("--model").arg(job.model);
            }
            if let Some(m) = &job.mcp {
                let cfg_path = workdir.join(format!(".tellushive_mcp_{}.json", m.name));
                let v = json!({"mcpServers": {m.name.clone(): {"command": m.command, "args": m.args, "env": mcp_env(true)}}});
                std::fs::write(&cfg_path, v.to_string()).map_err(|e| e.to_string())?;
                cmd.arg("--mcp-config").arg(&cfg_path);
                // Load only this server, not the user-scope ones (Zotero, Rhino, ...),
                // so batch runs do not start extra MCP processes per call.
                cmd.arg("--strict-mcp-config");
                if job.mcp_only {
                    cmd.arg("--allowedTools").arg(format!("mcp__{}", m.name));
                }
            }
            cmd.args(&cfg.args);
        }
        Which::Codex => {
            cmd.arg("exec").arg("--skip-git-repo-check").arg("-C").arg(workdir).arg("--output-last-message").arg(&last_msg);
            if !job.model.is_empty() {
                cmd.arg("-m").arg(job.model);
            }
            if let Some(m) = &job.mcp {
                cmd.arg("-c").arg(format!("mcp_servers.{}.command={}", m.name, json!(m.command.display().to_string())));
                cmd.arg("-c").arg(format!("mcp_servers.{}.args={}", m.name, json!(m.args)));
                for (k, v) in mcp_env(false).as_object().into_iter().flatten() {
                    cmd.arg("-c").arg(format!("mcp_servers.{}.env.{k}={v}", m.name));
                }
                // our tools run under the harness's own pipeline; codex exec cannot ask anyone
                cmd.arg("-c").arg(format!("mcp_servers.{}.default_tools_approval_mode=\"approve\"", m.name));
            }
            cmd.args(&cfg.args);
            cmd.arg("-");
        }
        Which::Agy => unreachable!("agy runs through ACP"),
    }
    let t0 = Instant::now();
    let mut child = cmd.spawn().map_err(|e| format!("cannot start {}: {e}", prog.display()))?;
    if let Some(mut stdin) = child.stdin.take() {
        let p = job.prompt.to_string();
        std::thread::spawn(move || {
            let _ = stdin.write_all(p.as_bytes());
        });
    }
    let mut out_pipe = child.stdout.take().ok_or("no stdout")?;
    let mut err_pipe = child.stderr.take().ok_or("no stderr")?;
    let out_t = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = out_pipe.read_to_string(&mut s);
        s
    });
    let err_t = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = err_pipe.read_to_string(&mut s);
        s
    });
    let limit = Duration::from_secs(cfg.timeout_min.max(1) * 60);
    let status = loop {
        if let Some(st) = child.try_wait().map_err(|e| e.to_string())? {
            break st;
        }
        if cancel.load(Ordering::Relaxed) || t0.elapsed() > limit {
            let _ = child.kill();
            let _ = child.wait();
            return Err(if cancel.load(Ordering::Relaxed) { "cancelled".into() } else { format!("{} timed out after {} min", which.program(), cfg.timeout_min) });
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    let stdout = out_t.join().unwrap_or_default();
    let stderr = err_t.join().unwrap_or_default();
    let seconds = t0.elapsed().as_secs_f64();
    let mut out = RunOutput { text: String::new(), ok: status.success(), seconds, input_tokens: 0, output_tokens: 0, cached_tokens: 0 };
    match which {
        Which::ClaudeCode => match serde_json::from_str::<Value>(stdout.trim()) {
            Ok(v) => {
                out.text = v.get("result").and_then(|r| r.as_str()).unwrap_or("").to_string();
                out.ok = !v.get("is_error").and_then(|b| b.as_bool()).unwrap_or(false) && status.success();
                let u = |k: &str| v.pointer(&format!("/usage/{k}")).and_then(|x| x.as_u64()).unwrap_or(0);
                out.cached_tokens = u("cache_read_input_tokens");
                out.input_tokens = u("input_tokens") + u("cache_creation_input_tokens") + out.cached_tokens;
                out.output_tokens = u("output_tokens");
            }
            Err(_) => out.text = format!("{stdout}\n{stderr}"),
        },
        Which::Agy => {}
        Which::Codex => {
            let last = std::fs::read_to_string(&last_msg).unwrap_or_default();
            let _ = std::fs::remove_file(&last_msg);
            out.text = if last.trim().is_empty() { format!("{}\n{}", tail(&stdout, 4000), tail(&stderr, 2000)) } else { last };
        }
    }
    out.text = out.text.trim().to_string();
    Ok(out)
}

/// One turn of Antigravity through its Agent Client Protocol server: `agy acp` on stdio,
/// a session in the work folder with this program's tool server attached (ACP hands MCP
/// servers over per session, so nothing is written to the engineer's own agy set-up), the
/// model chosen, the prompt sent, and the reply gathered from the session's updates.
/// With `mcp_only`, a permission request is granted for reading and for this program's tools
/// and refused for agy's own shell and file edits, as Claude Code is limited to the tools.
fn run_acp(prog: &Path, cfg: &ExternalAgent, job: &Job, cancel: &AtomicBool) -> Result<RunOutput, String> {
    use std::io::{BufRead, Write};
    let t0 = Instant::now();
    let mut cmd = Command::new(prog);
    cmd.arg("acp").arg("--add-dir").arg(job.project).args(&cfg.args);
    cmd.current_dir(job.workdir).env("PATH", child_path(prog)).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    let mut child = cmd.spawn().map_err(|e| format!("cannot start {} acp: {e}", prog.display()))?;
    let mut stdin = child.stdin.take().ok_or("no stdin")?;
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let (tx, rx) = std::sync::mpsc::channel::<Value>();
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Ok(v) = serde_json::from_str::<Value>(line.trim()) {
                if tx.send(v).is_err() {
                    break;
                }
            }
        }
    });
    struct Kill(std::process::Child);
    impl Drop for Kill {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut child = Kill(child);
    let limit = Duration::from_secs(cfg.timeout_min.max(1) * 60);
    let mcp_name = job.mcp.as_ref().map(|m| m.name.clone()).unwrap_or_default();
    let mut next_id = 0u64;
    let mut text = String::new();
    fn send(stdin: &mut std::process::ChildStdin, v: Value) -> Result<(), String> {
        writeln!(stdin, "{v}").and_then(|_| stdin.flush()).map_err(|e| format!("agy acp closed: {e}"))
    }
    // sends a request and waits for its answer, answering the agent's own requests and
    // gathering the reply text meanwhile
    let mut call = |stdin: &mut std::process::ChildStdin, method: &str, params: Value, text: &mut String, session: &str| -> Result<Value, String> {
        next_id += 1;
        let id = next_id;
        send(stdin, json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))?;
        loop {
            if cancel.load(Ordering::Relaxed) {
                if !session.is_empty() {
                    let _ = send(stdin, json!({"jsonrpc": "2.0", "method": "session/cancel", "params": {"sessionId": session}}));
                }
                return Err("cancelled".into());
            }
            if t0.elapsed() > limit {
                return Err(format!("agy timed out after {} min", cfg.timeout_min));
            }
            let m = match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(m) => m,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(_) => return Err(format!("agy acp ended during {method}")),
            };
            match (m.get("method").and_then(|x| x.as_str()), m.get("id")) {
                (Some("session/update"), None) => {
                    let u = &m["params"]["update"];
                    if u["sessionUpdate"] == "agent_message_chunk" {
                        if let Some(t) = u["content"]["text"].as_str() {
                            text.push_str(t);
                        }
                    }
                }
                (Some("session/request_permission"), Some(rid)) => {
                    let p = &m["params"];
                    let kind = p["toolCall"]["kind"].as_str().unwrap_or("other");
                    let title = p["toolCall"]["title"].as_str().unwrap_or("").to_ascii_lowercase();
                    let ours = !mcp_name.is_empty() && title.contains(&mcp_name.to_ascii_lowercase());
                    let allow = !job.mcp_only || ours || matches!(kind, "read" | "search" | "think" | "fetch");
                    let opts = p["options"].as_array().cloned().unwrap_or_default();
                    let want = if allow { "allow" } else { "reject" };
                    let pick = opts.iter().find(|o| o["kind"].as_str().is_some_and(|k| k.starts_with(want))).or(if allow { opts.first() } else { None });
                    let outcome = match pick.and_then(|o| o["optionId"].as_str()) {
                        Some(oid) => json!({"outcome": "selected", "optionId": oid}),
                        None => json!({"outcome": "cancelled"}),
                    };
                    send(stdin, json!({"jsonrpc": "2.0", "id": rid, "result": {"outcome": outcome}}))?;
                }
                (Some(other), Some(rid)) => {
                    // no file system or terminal on the client side: the agent uses its tools
                    send(stdin, json!({"jsonrpc": "2.0", "id": rid, "error": {"code": -32601, "message": format!("{other} is not offered by this client")}}))?;
                }
                (None, Some(rid)) if rid.as_u64() == Some(id) => {
                    if let Some(e) = m.get("error") {
                        return Err(format!("agy {method}: {}", e.get("message").and_then(|x| x.as_str()).unwrap_or(&e.to_string())));
                    }
                    return Ok(m.get("result").cloned().unwrap_or(Value::Null));
                }
                _ => {}
            }
        }
    };
    let _ = &mut child;
    call(&mut stdin, "initialize", json!({"protocolVersion": 1, "clientCapabilities": {"fs": {"readTextFile": false, "writeTextFile": false}, "terminal": false}}), &mut text, "")?;
    let servers: Vec<Value> = job
        .mcp
        .iter()
        .map(|m| {
            let env: Vec<Value> = mcp_env(false).as_object().into_iter().flatten().map(|(k, v)| json!({"name": k, "value": v.as_str().unwrap_or_default()})).collect();
            json!({"name": m.name, "command": m.command, "args": m.args, "env": env})
        })
        .collect();
    let new = call(&mut stdin, "session/new", json!({"cwd": job.workdir, "mcpServers": servers}), &mut text, "")?;
    let session = new["sessionId"].as_str().ok_or("agy started no session")?.to_string();
    if !job.model.is_empty() && new["models"]["currentModelId"].as_str() != Some(job.model) {
        let known: Vec<String> = new["models"]["availableModels"].as_array().map(|a| a.iter().filter_map(|m| m["modelId"].as_str().map(String::from)).collect()).unwrap_or_default();
        if !known.is_empty() && !known.iter().any(|k| k == job.model) {
            return Err(format!("agy has no model '{}'; it offers: {}", job.model, known.join(", ")));
        }
        call(&mut stdin, "session/set_model", json!({"sessionId": session, "modelId": job.model}), &mut text, &session)?;
    }
    let done = call(&mut stdin, "session/prompt", json!({"sessionId": session, "prompt": [{"type": "text", "text": job.prompt}]}), &mut text, &session)?;
    let u = |k: &str| done["usage"][k].as_u64().unwrap_or(0);
    let stop = done["stopReason"].as_str().unwrap_or("");
    Ok(RunOutput { text: text.trim().to_string(), ok: matches!(stop, "end_turn" | "max_tokens" | ""), seconds: t0.elapsed().as_secs_f64(), input_tokens: u("inputTokens"), output_tokens: u("outputTokens"), cached_tokens: u("cachedReadTokens") })
}

fn tail(s: &str, n: usize) -> String {
    let c: Vec<char> = s.chars().collect();
    c[c.len().saturating_sub(n)..].iter().collect()
}
