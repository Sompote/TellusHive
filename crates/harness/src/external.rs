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
}

/// Runs one non-interactive task. `mcp_exe` + `project` give the external agent our tools.
pub fn run(which: Which, cfg: &ExternalAgent, prompt: &str, workdir: &Path, project: &Path, mcp_exe: Option<&Path>, cancel: &AtomicBool) -> Result<RunOutput, String> {
    if !cfg.enabled {
        return Err(format!("{} is switched off in Settings", which.program()));
    }
    let prog = find_program(which.program(), &cfg.path).ok_or_else(|| format!("'{}' was not found; install it or set its path in Settings", which.program()))?;
    std::fs::create_dir_all(workdir).map_err(|e| e.to_string())?;
    let mut cmd = Command::new(&prog);
    cmd.current_dir(workdir).env("PATH", child_path(&prog)).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let last_msg = workdir.join(".codex_last_message.txt");
    match which {
        Which::ClaudeCode => {
            cmd.arg("-p").arg(prompt).arg("--output-format").arg("json").arg("--add-dir").arg(project);
            if let Some(exe) = mcp_exe {
                let cfg_path = workdir.join(".tellushive_mcp.json");
                let v = json!({"mcpServers": {"design": {"command": exe, "args": ["--mcp", "--project", project]}}});
                std::fs::write(&cfg_path, v.to_string()).map_err(|e| e.to_string())?;
                cmd.arg("--mcp-config").arg(&cfg_path).arg("--allowedTools").arg("mcp__design");
            }
            cmd.args(&cfg.args);
        }
        Which::Codex => {
            cmd.arg("exec").arg("--skip-git-repo-check").arg("-C").arg(workdir).arg("--output-last-message").arg(&last_msg);
            if let Some(exe) = mcp_exe {
                cmd.arg("-c").arg(format!("mcp_servers.design.command={}", json!(exe.display().to_string())));
                cmd.arg("-c").arg(format!("mcp_servers.design.args={}", json!(["--mcp", "--project", project.display().to_string()])));
            }
            cmd.args(&cfg.args);
            cmd.arg(prompt);
        }
    }
    let t0 = Instant::now();
    let mut child = cmd.spawn().map_err(|e| format!("cannot start {}: {e}", prog.display()))?;
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
    let (text, ok) = match which {
        Which::ClaudeCode => match serde_json::from_str::<Value>(stdout.trim()) {
            Ok(v) => (v.get("result").and_then(|r| r.as_str()).unwrap_or("").to_string(), !v.get("is_error").and_then(|b| b.as_bool()).unwrap_or(false) && status.success()),
            Err(_) => (format!("{stdout}\n{stderr}"), status.success()),
        },
        Which::Codex => {
            let last = std::fs::read_to_string(&last_msg).unwrap_or_default();
            if last.trim().is_empty() {
                (format!("{}\n{}", tail(&stdout, 4000), tail(&stderr, 2000)), status.success())
            } else {
                (last, status.success())
            }
        }
    };
    Ok(RunOutput { text: text.trim().to_string(), ok, seconds })
}

fn tail(s: &str, n: usize) -> String {
    let c: Vec<char> = s.chars().collect();
    c[c.len().saturating_sub(n)..].iter().collect()
}
