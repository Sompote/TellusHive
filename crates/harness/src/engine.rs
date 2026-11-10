//! The TellusHive engine: a thin facade over a booted th-hive plugin tree.
//!
//! The agent harness itself is th-hive, a Rust port of the DeepSeek Harness architecture:
//! a plugin context, the append-only session log, the turn/step agent loop and the tool
//! pipeline. This module composes the profile (the hive base bundle plus the TellusHive
//! bundle in [`crate::bundle`]), keeps the engineer's conversation as the coordinator's
//! session in the project's `sessions/` folder, and turns session events into the
//! [`Event`]s the chat view shows.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, RwLock};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use th_hive::{keys, AgentPreset, AgentLoop, AgentRegistry, Hive, Inbound, SessionMeta, SessionStore, ToolFilter, ToolOutput};
use th_kb::Library;
use th_mcp::{Client, ServerConfig};

use crate::config::{AgentSpec, Settings, TeamSpec};
use crate::live;
use crate::project::{is_image, read_text, ArtifactKind, Project};
use crate::skills::{self, Skill};
use crate::tools;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    AgentStart { run: u64, agent: String, depth: usize, task: String },
    Thinking { run: u64, agent: String, text: String },
    Text { run: u64, agent: String, text: String },
    /// A piece of a reply while the model is still writing it (`agent/assistant-stream`):
    /// kind "text", "reasoning" or "tool".
    Stream { run: u64, agent: String, kind: String, text: String },
    ToolCall { run: u64, agent: String, id: String, tool: String, args: String },
    ToolResult { run: u64, agent: String, id: String, tool: String, ok: bool, text: String },
    Artifact { run: u64, kind: ArtifactKind, path: PathBuf },
    AgentDone { run: u64, agent: String, answer: String },
    /// A program saved the project's model (models/<plugin>.<ext>) after an agent changed it.
    Model { run: u64, plugin: String, path: PathBuf },
    Usage { run: u64, agent: String, model: String, prompt: u64, completion: u64 },
    Error { run: u64, agent: String, text: String },
    Finished { answer: String },
}

/// What the engineer sends: text plus files (already copied into attachments/).
#[derive(Debug, Clone, Default)]
pub struct UserInput {
    pub text: String,
    pub attachments: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PluginState {
    Off,
    Connecting,
    Ready { tools: usize },
    Failed(String),
}

/// Connected MCP servers, shared by every agent and the plugin panel.
#[derive(Default)]
pub struct Plugins {
    clients: Mutex<HashMap<String, Arc<Client>>>,
    pub state: Mutex<HashMap<String, PluginState>>,
    /// The project folder each live-model program was started for (see `live`).
    bound: Mutex<HashMap<String, PathBuf>>,
}

impl Plugins {
    pub fn connect(&self, cfg: &ServerConfig) -> Result<Arc<Client>, String> {
        self.set_state(&cfg.name, PluginState::Connecting);
        match Client::connect(cfg) {
            Ok(c) => {
                let c = Arc::new(c);
                self.set_state(&cfg.name, PluginState::Ready { tools: c.tools.len() });
                self.clients.lock().map_err(|_| "poisoned")?.insert(cfg.name.clone(), c.clone());
                Ok(c)
            }
            Err(e) => {
                self.set_state(&cfg.name, PluginState::Failed(e.clone()));
                Err(e)
            }
        }
    }
    pub fn disconnect(&self, name: &str) {
        if let Ok(mut m) = self.clients.lock() {
            m.remove(name);
        }
        if let Ok(mut b) = self.bound.lock() {
            b.remove(name);
        }
        self.set_state(name, PluginState::Off);
    }
    pub fn get(&self, name: &str) -> Option<Arc<Client>> {
        self.clients.lock().ok().and_then(|m| m.get(name).cloned())
    }
    /// The connected client, connecting on first use.
    pub fn ensure(&self, settings: &Settings, name: &str) -> Result<Arc<Client>, String> {
        self.ensure_for(settings, name, None)
    }

    /// The connected client for a project: a live-model program (Aedifex, Pondus, Tellus,
    /// Tellus3D) works on that project's model file, so it is restarted when the project
    /// changes, and a model saved earlier is opened again.
    pub fn ensure_for(&self, settings: &Settings, name: &str, project: Option<&Path>) -> Result<Arc<Client>, String> {
        let live = live::spec(name).filter(|_| project.is_some());
        let bound = self.bound.lock().ok().and_then(|b| b.get(name).cloned());
        if let Some(c) = self.get(name) {
            if live.is_none() || bound.as_deref() == project {
                return Ok(c);
            }
            self.disconnect(name);
        }
        let cfg = settings.plugins.iter().find(|p| p.name == name).ok_or_else(|| format!("plugin '{name}' is not configured"))?;
        if !cfg.enabled {
            return Err(format!("plugin '{name}' is switched off"));
        }
        let (Some(s), Some(dir)) = (live, project) else { return self.connect(cfg) };
        let c = self.connect(&live::bind(cfg, dir))?;
        if let Ok(mut b) = self.bound.lock() {
            b.insert(name.to_string(), dir.to_path_buf());
        }
        let m = live::model_path(dir, s);
        if !s.autosave_flags && m.is_file() {
            // a failed open leaves the program's default model; the agent's first change saves over it
            let _ = c.call("open_project", json!({"path": m.display().to_string()}));
        }
        Ok(c)
    }
    /// After a tool changed a live-model program's model: makes sure the project's model file
    /// holds the change (Tellus and Tellus3D save through their tool; Aedifex and Pondus have
    /// saved already). Returns the file when it is up to date.
    pub fn keep_model(c: &Client, server: &str, tool: &str, project_dir: &Path) -> Option<PathBuf> {
        let s = live::spec(server)?;
        if live::reads_only(tool) {
            return None;
        }
        let m = live::model_path(project_dir, s);
        if !s.autosave_flags {
            let _ = std::fs::create_dir_all(m.parent()?);
            let r = c.call("save_project", json!({"path": m.display().to_string()})).ok()?;
            if r.is_error {
                return None;
            }
        }
        m.is_file().then_some(m)
    }

    fn set_state(&self, name: &str, s: PluginState) {
        if let Ok(mut m) = self.state.lock() {
            m.insert(name.to_string(), s);
        }
    }
    pub fn state_of(&self, name: &str) -> PluginState {
        self.state.lock().ok().and_then(|m| m.get(name).cloned()).unwrap_or(PluginState::Off)
    }
}

/// What the TellusHive plugins work on (the `tellushive` service): the engineer's settings,
/// team, project, standards library, MCP programs and skills, plus the chat's event channel
/// for the turn in progress.
pub struct Domain {
    pub settings: Arc<RwLock<Settings>>,
    pub team: Arc<RwLock<TeamSpec>>,
    pub project: Arc<RwLock<Project>>,
    pub library: Arc<Library>,
    pub plugins: Arc<Plugins>,
    pub skills: Arc<RwLock<Vec<Skill>>>,
    pub ui: Mutex<Option<Sender<Event>>>,
}

pub const DOMAIN: &str = "tellushive";

impl Domain {
    pub fn send(&self, e: Event) {
        if let Some(tx) = self.ui.lock().ok().and_then(|u| u.clone()) {
            let _ = tx.send(e);
        }
    }
    pub fn settings(&self) -> Settings {
        self.settings.read().map(|s| s.clone()).unwrap_or_default()
    }
    pub fn team(&self) -> TeamSpec {
        self.team.read().map(|s| s.clone()).unwrap_or_default()
    }
    pub fn project(&self) -> Project {
        self.project.read().map(|p| p.clone()).expect("project lock")
    }
    pub fn skills(&self) -> Vec<Skill> {
        self.skills.read().map(|s| s.clone()).unwrap_or_default()
    }
    /// The team member an agent runs as.
    pub fn spec_of(&self, a: &th_hive::Agent) -> Option<AgentSpec> {
        self.team().agent(&a.preset.id).cloned()
    }

    /// The hive preset of a team member: provider and model from the settings, its built-in
    /// tools, and the output limit the provider allows.
    pub fn preset(&self, a: &AgentSpec) -> Result<AgentPreset, String> {
        let settings = self.settings();
        let provider = settings.provider(&a.provider).cloned().ok_or_else(|| format!("provider '{}' not found", a.provider))?;
        if provider.key().is_none() && !provider.base_url.contains("localhost") && !provider.base_url.contains("127.0.0.1") {
            return Err(format!("No API key for provider '{}'. Add it in Settings (or set {}).", provider.name, provider.key_env));
        }
        let model = if a.model.is_empty() {
            if a.provider.is_empty() || a.provider == settings.default_provider {
                settings.default_model.clone()
            } else {
                provider.default_model.clone()
            }
        } else {
            a.model.clone()
        };
        // DeepSeek's own API stops at 8k output tokens; OpenRouter and others allow long tool calls (reports)
        let max_out = if provider.base_url.contains("api.deepseek.com") { 8192 } else { 32_000 };
        let tools = if a.tools.iter().any(|t| t == "*") { ToolFilter::All } else { ToolFilter::Only(a.tools.clone()) };
        Ok(AgentPreset { id: a.id.clone(), name: a.name.clone(), role: a.role.clone(), provider, model, temperature: Some(a.temperature), max_tokens: Some(max_out), max_steps: a.max_turns.max(1), tools, meta: json!({"color": a.color}) })
    }

    /// Calls one plugin tool. Pictures it returns are saved to images/; a live model the
    /// program saved is reported in `meta`.
    pub fn call_plugin(&self, server: &str, tool: &str, args: Value) -> Result<ToolOutput, String> {
        let settings = self.settings();
        let project = self.project();
        let c = self.plugins.ensure_for(&settings, server, Some(&project.dir))?;
        let r = c.call(tool, args)?;
        let mut meta = json!({});
        if !r.is_error {
            if let Some(m) = Plugins::keep_model(&c, server, tool, &project.dir) {
                meta["model"] = json!({"plugin": server, "path": m});
            }
        }
        let mut text = r.text.clone();
        let mut arts = vec![];
        for (k, (data, mime)) in r.images.iter().enumerate() {
            let ext = if mime.contains("jpeg") { "jpg" } else { "png" };
            let p = project.folder(ArtifactKind::Image).join(format!("{server}_{}_{k}.{ext}", stamp()));
            if let Ok(bytes) = base64_lite::decode(data) {
                let _ = std::fs::create_dir_all(project.folder(ArtifactKind::Image));
                if std::fs::write(&p, bytes).is_ok() {
                    text.push_str(&format!("\n[picture saved: {}]", project.rel(&p)));
                    arts.push(json!({"kind": ArtifactKind::Image, "path": p}));
                }
            }
        }
        meta["artifacts"] = Value::Array(arts);
        if r.is_error {
            Err(text)
        } else {
            Ok(ToolOutput { text, meta, ..Default::default() })
        }
    }
}

pub struct Engine {
    pub settings: Arc<RwLock<Settings>>,
    pub team: Arc<RwLock<TeamSpec>>,
    pub project: Arc<RwLock<Project>>,
    pub library: Arc<Library>,
    pub plugins: Arc<Plugins>,
    pub skills: Arc<RwLock<Vec<Skill>>>,
    pub domain: Arc<Domain>,
    pub hive: Hive,
}

/// The file that names the coordinator session the chat continues.
const CURRENT: &str = "sessions/current";

impl Engine {
    pub fn new(settings: Settings, team: TeamSpec, project: Project, library: Arc<Library>, plugins: Arc<Plugins>) -> Engine {
        let skills = Arc::new(RwLock::new(skills::scan(&settings.skill_dirs)));
        let harness = settings.harness.clone();
        let domain = Arc::new(Domain {
            settings: Arc::new(RwLock::new(settings)),
            team: Arc::new(RwLock::new(team)),
            project: Arc::new(RwLock::new(project)),
            library,
            plugins,
            skills,
            ui: Mutex::new(None),
        });
        let hive = Hive::boot(crate::bundle::profile(&domain, &harness));
        Engine { settings: domain.settings.clone(), team: domain.team.clone(), project: domain.project.clone(), library: domain.library.clone(), plugins: domain.plugins.clone(), skills: domain.skills.clone(), domain, hive }
    }

    pub fn rescan_skills(&self) {
        let dirs = self.settings.read().map(|s| s.skill_dirs.clone()).unwrap_or_default();
        if let Ok(mut s) = self.skills.write() {
            *s = skills::scan(&dirs);
        }
    }

    /// The plugin tree and each plugin's state.
    pub fn dump_plugins(&self) -> String {
        self.hive.dump()
    }

    /// The coordinator session file of the conversation in progress, if any.
    pub fn session_path(&self) -> Option<PathBuf> {
        let dir = self.domain.project().dir;
        let id = std::fs::read_to_string(dir.join(CURRENT)).ok()?;
        Some(dir.join("sessions").join(format!("{}.jsonl", id.trim())))
    }

    /// Starts a new conversation: the next message opens a fresh coordinator session (the
    /// old session files stay in sessions/).
    pub fn new_conversation(&self) {
        let _ = std::fs::remove_file(self.domain.project().dir.join(CURRENT));
    }

    /// One turn of the engineer's conversation with the coordinator. The conversation is
    /// the coordinator's session log, so it continues across turns and restarts.
    pub fn send(&self, input: UserInput, tx: &Sender<Event>, cancel: &Arc<AtomicBool>) -> Result<String, String> {
        let project = self.domain.project();
        let team = self.domain.team();
        let store = self.hive.ctx.expect::<SessionStore>(keys::SESSIONS)?;
        let agents = self.hive.ctx.expect::<AgentRegistry>(keys::AGENTS)?;
        let lp = self.hive.ctx.expect::<AgentLoop>(keys::LOOP)?;
        store.set_root(Some(project.dir.join("sessions")));
        let coord = team.agent(&team.coordinator).cloned().ok_or("the team has no coordinator")?;
        let preset = match self.domain.preset(&coord) {
            Ok(p) => p,
            Err(e) => {
                let _ = tx.send(Event::Error { run: 0, agent: coord.name.clone(), text: e.clone() });
                return Err(e);
            }
        };
        let session = std::fs::read_to_string(project.dir.join(CURRENT)).ok().and_then(|id| store.open(id.trim()).ok());
        let session = match session {
            Some(s) => s,
            None => {
                let id = store.new_id("chat");
                let s = store.create(SessionMeta { id: id.clone(), agent: coord.id.clone(), label: crate::project::clip(&input.text, 120), parent: None, depth: 0 })?;
                std::fs::write(project.dir.join(CURRENT), &id).map_err(|e| e.to_string())?;
                s
            }
        };
        let msg = self.user_message(&project, &input, preset.provider.vision);
        if let Ok(mut u) = self.domain.ui.lock() {
            *u = Some(tx.clone());
        }
        let agent = agents.create(preset, session, None, cancel.clone());
        agent.followup(msg);
        let r = lp.run_turn(&self.hive.ctx, &agent);
        agents.remove(&agent.id);
        if let Ok(mut u) = self.domain.ui.lock() {
            *u = None;
        }
        let out = r?;
        match out.reason.as_str() {
            "cancelled" => Err("cancelled".into()),
            _ => {
                let _ = tx.send(Event::Finished { answer: out.text.clone() });
                Ok(out.text)
            }
        }
    }

    /// The engineer's message with attachments: text files are inlined (clipped), images go
    /// to vision models as pictures and to text-only models as a description.
    fn user_message(&self, project: &Project, input: &UserInput, vision: bool) -> Inbound {
        let settings = self.domain.settings();
        let mut text = input.text.clone();
        let mut images = vec![];
        for p in &input.attachments {
            let rel = project.rel(p);
            if is_image(p) {
                if vision {
                    images.push(p.clone());
                    text.push_str(&format!("\n\n[Attached image: {rel}]"));
                } else {
                    let d = tools::describe_image(&settings, p, "").unwrap_or_else(|e| format!("(could not read the image: {e})"));
                    text.push_str(&format!("\n\n[Attached image: {rel}. Its content, as read by the vision model:]\n{d}"));
                }
            } else {
                match read_text(p, 12_000) {
                    Ok(t) => text.push_str(&format!("\n\n[Attached file: {rel}; first part shown, use read_file for the rest]\n{t}")),
                    Err(e) => text.push_str(&format!("\n\n[Attached file: {rel} ({e})]")),
                }
            }
        }
        Inbound { text, images, source: "user".into() }
    }
}

pub(crate) fn stamp() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// A tiny base64 decoder so the harness has no extra dependency for MCP pictures.
pub(crate) mod base64_lite {
    pub fn decode(s: &str) -> Result<Vec<u8>, String> {
        let mut out = Vec::with_capacity(s.len() * 3 / 4);
        let mut buf = 0u32;
        let mut bits = 0;
        for c in s.bytes() {
            let v = match c {
                b'A'..=b'Z' => c - b'A',
                b'a'..=b'z' => c - b'a' + 26,
                b'0'..=b'9' => c - b'0' + 52,
                b'+' | b'-' => 62,
                b'/' | b'_' => 63,
                b'=' | b'\n' | b'\r' | b' ' => continue,
                _ => return Err("bad base64".into()),
            } as u32;
            buf = (buf << 6) | v;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push((buf >> bits) as u8);
                buf &= (1 << bits) - 1;
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn b64() {
        assert_eq!(base64_lite::decode("aGVsbG8=").unwrap(), b"hello");
    }

    #[test]
    fn default_team_reachability() {
        let t = crate::config::default_team();
        assert_eq!(t.reachable("coordinator").len(), t.agents.len() - 1);
        assert!(t.reachable("designer").is_empty());
    }
}
