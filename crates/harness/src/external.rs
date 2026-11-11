//! Claude Code and Codex as workers of the coordinator. Each run gets this program's own MCP
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
}

impl Which {
    pub fn program(self) -> &'static str {
        match self {
            Which::ClaudeCode => "claude",
            Which::Codex => "codex",
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
    let mut dirs: Vec<PathBuf> = std::env::var("PATH").unwrap_or_default().split(':').map(PathBuf::from).collect();
    dirs.extend([home.join(".local/bin"), home.join(".npm-global/bin"), home.join(".cargo/bin"), PathBuf::from("/opt/homebrew/bin"), PathBuf::from("/usr/local/bin")]);
    // nvm installs node tools per version
    if let Ok(rd) = std::fs::read_dir(home.join(".nvm/versions/node")) {
        let mut v: Vec<PathBuf> = rd.flatten().map(|e| e.path().join("bin")).collect();
        v.sort();
        v.reverse();
        dirs.extend(v);
    }
    dirs.into_iter().map(|d| d.join(name)).find(|p| p.is_file())
}

/// PATH for the child: the program's own folder first (node for claude), then the usual ones.
fn child_path(prog: &Path) -> String {
    let mut parts = vec![];
    if let Some(d) = prog.parent() {
        parts.push(d.display().to_string());
    }
    parts.push(std::env::var("PATH").unwrap_or_default());
    parts.push("/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin".into());
    parts.join(":")
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
                let v = json!({"mcpServers": {m.name.clone(): {"command": m.command, "args": m.args}}});
                std::fs::write(&cfg_path, v.to_string()).map_err(|e| e.to_string())?;
                cmd.arg("--mcp-config").arg(&cfg_path);
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
                // our tools run under the harness's own pipeline; codex exec cannot ask anyone
                cmd.arg("-c").arg(format!("mcp_servers.{}.default_tools_approval_mode=\"approve\"", m.name));
            }
            cmd.args(&cfg.args);
            cmd.arg("-");
        }
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
        Which::Codex => {
            let last = std::fs::read_to_string(&last_msg).unwrap_or_default();
            let _ = std::fs::remove_file(&last_msg);
            out.text = if last.trim().is_empty() { format!("{}\n{}", tail(&stdout, 4000), tail(&stderr, 2000)) } else { last };
        }
    }
    out.text = out.text.trim().to_string();
    Ok(out)
}

fn tail(s: &str, n: usize) -> String {
    let c: Vec<char> = s.chars().collect();
    c[c.len().saturating_sub(n)..].iter().collect()
}
