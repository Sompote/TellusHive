//! The TellusHive harness: settings, the agent team, skills, plugins (MCP), projects and
//! the built-in engineering tools, mounted as a plugin bundle on th-hive (the Rust port of
//! the DeepSeek Harness architecture).

pub mod bots;
pub mod bundle;
pub mod config;
pub mod engine;
pub mod external;
pub mod live;
pub mod pins;
pub mod project;
pub mod skills;
pub mod standards_briefing;
pub mod team_memory;
pub mod template;
pub mod tools;

pub use config::{AgentSpec, HarnessSettings, Settings, TeamSpec};
pub use engine::{Domain, Engine, Event, PluginState, Plugins, UserInput};
pub use th_hive;
pub use project::{Artifact, ArtifactKind, Project, ProjectInfo};
pub use template::Template;

use std::path::Path;
use std::sync::Arc;

/// Who the MCP server serves: nobody in particular (the built-in tools), or one team member
/// whose model is Claude Code or Codex (that member's whole tool set, through the harness).
#[derive(Debug, Clone, Default)]
pub struct ServeOptions {
    pub agent: Option<String>,
    /// Session id for the tool calls (the app follows this file to show them in the chat).
    pub session: Option<String>,
    /// The calling agent's session id.
    pub parent: Option<String>,
    pub depth: usize,
    /// The team the calling engine runs (a bot's team exists only in the app, not in teams/).
    pub team_file: Option<std::path::PathBuf>,
}

/// Serves tools over MCP stdio. Without an agent: the built-in tools of the project (used by
/// Claude Code and Codex when the coordinator hands them a task). With an agent: everything
/// that agent may call (built-in tools, its MCP programs, `delegate`, todo and task board),
/// each call run through the th-hive tool pipeline and logged in a session.
pub fn serve_mcp_with(project_dir: &Path, opt: &ServeOptions) -> Result<(), String> {
    let Some(agent_id) = &opt.agent else { return serve_mcp(project_dir) };
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use th_hive::{keys, AgentRegistry, EventBody, SessionMeta, SessionStore, ToolRegistry};
    // the bots' own APIs, for an agent of a bot that has one
    let settings = crate::bots::Bot::engine_settings(&Settings::load(), &crate::bots::Bot::list());
    let project = Project::open_or_create(project_dir)?;
    let library = Arc::new(th_kb::Library::open(&th_kb::Library::default_root())?);
    let handed: Option<TeamSpec> = opt.team_file.as_ref().and_then(|f| crate::config::read_json(f).ok());
    let team = handed.clone().unwrap_or_else(|| TeamSpec::load(&project.info.team));
    // what this server runs with, since its environment comes from the coding agent: a wrong
    // data folder silently gives the engineer's own settings and team
    {
        let file = TeamSpec::dir().join(TeamSpec::file_name(&project.info.team));
        let source = match (&handed, crate::config::read_json::<TeamSpec>(&file)) {
            (Some(_), _) => format!("handed over by the app ({})", opt.team_file.as_ref().map(|f| f.display().to_string()).unwrap_or_default()),
            (None, Ok(_)) => format!("file {}", file.display()),
            (None, Err(e)) => format!("BUILT-IN FALLBACK ({e})"),
        };
        let agents: Vec<String> = team.agents.iter().map(|a| format!("{}={}", a.id, if a.provider.is_empty() { format!("(default {}/{})", settings.default_provider, settings.default_model) } else { a.provider.clone() })).collect();
        let line = format!(
            "{} pid {} agent {agent_id} data {} TELLUSHIVE_HOME {:?} team '{}' from {source}: {}\n",
            crate::engine::stamp(),
            std::process::id(),
            crate::config::data_dir().display(),
            std::env::var("TELLUSHIVE_HOME").ok(),
            team.name,
            agents.join(", ")
        );
        let _ = std::fs::create_dir_all(project.dir.join("sessions"));
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(project.dir.join("sessions/mcp-server.log")) {
            use std::io::Write;
            let _ = f.write_all(line.as_bytes());
        }
    }
    let spec = team.agent(agent_id).cloned().ok_or_else(|| format!("no agent '{agent_id}' in the team"))?;
    let engine = Arc::new(Engine::new(settings, team, project.clone(), library, Arc::new(Plugins::default())));
    let ctx = engine.hive.ctx.clone();
    let store = ctx.expect::<SessionStore>(keys::SESSIONS)?;
    store.set_root(Some(project.dir.join("sessions")));
    let preset = engine.domain.preset(&spec)?;
    let sid = opt.session.clone().unwrap_or_else(|| store.new_id(&format!("{}-tools", spec.id)));
    let session = store.create(SessionMeta { id: sid, agent: spec.id.clone(), label: "tools served to Claude Code / Codex".into(), parent: opt.parent.clone(), depth: opt.depth })?;
    let agent = ctx.expect::<AgentRegistry>(keys::AGENTS)?.create_at(preset, session, opt.depth, opt.parent.clone(), Arc::new(AtomicBool::new(false)));
    let reg = ctx.expect::<ToolRegistry>(keys::TOOLS)?;
    // run_python is served too: the program runs with only these tools allowed, so its own
    // shell would wait for an approval nobody can give
    let toolset: Vec<_> = reg.resolve(&agent, &ctx);
    let calls = Arc::new(AtomicU64::new(1));
    let served = toolset
        .iter()
        .map(|def| {
            let (ctx, reg, agent, toolset, calls, name) = (ctx.clone(), reg.clone(), agent.clone(), toolset.clone(), calls.clone(), def.name.clone());
            let spec = def.spec();
            th_mcp::ServedTool {
                info: th_mcp::ToolInfo { name: spec.name, description: spec.description, input_schema: spec.parameters },
                run: Box::new(move |args| {
                    let id = format!("mcp{}", calls.fetch_add(1, Ordering::Relaxed));
                    let before = agent.session.len();
                    reg.run_batch(&ctx, &agent, &[th_hive::session::CallRecord { id: id.clone(), name: name.clone(), arguments: args.to_string() }], &toolset);
                    let mut text = String::new();
                    let mut err = false;
                    for e in agent.session.events().into_iter().skip(before) {
                        match e.body {
                            EventBody::ToolResult { id: i, text: t, is_error, .. } if i == id => {
                                text = t;
                                err = is_error;
                            }
                            // reminders that harness plugins add after a result
                            EventBody::UserMessage { text: t, source, .. } if source.starts_with("plugin:") => text.push_str(&format!("\n\n[{source}] {t}")),
                            _ => {}
                        }
                    }
                    if err {
                        Err(text)
                    } else {
                        Ok(text)
                    }
                }),
            }
        })
        .collect();
    th_mcp::serve_stdio("tellushive", served);
    Ok(())
}

/// Serves the built-in tools of one project over MCP stdio (used by Claude Code and Codex
/// when the coordinator hands them a task, and by any other MCP client).
pub fn serve_mcp(project_dir: &Path) -> Result<(), String> {
    let settings = Settings::load();
    let project = Arc::new(Project::open_or_create(project_dir)?);
    let library = Arc::new(th_kb::Library::open(&th_kb::Library::default_root())?);
    let skills = Arc::new(skills::scan(&settings.skill_dirs));
    let settings = Arc::new(settings);
    // run_python stays inside the app: an external agent's own permission settings decide
    // whether it may run code, and serving it here would bypass them
    let served = tools::builtin_specs()
        .into_iter()
        .filter(|spec| spec.name != "run_python")
        .map(|spec| {
            let (p, l, s, st) = (project.clone(), library.clone(), skills.clone(), settings.clone());
            let name = spec.name.clone();
            th_mcp::ServedTool {
                info: th_mcp::ToolInfo { name: spec.name, description: spec.description, input_schema: spec.parameters },
                run: Box::new(move |args| {
                    let ctx = tools::Ctx { project: &p, library: &l, skills: &s, settings: &st, cancel: None };
                    tools::exec_builtin(&ctx, &name, &args).map(|o| o.text)
                }),
            }
        })
        .collect();
    th_mcp::serve_stdio("tellushive", served);
    Ok(())
}
