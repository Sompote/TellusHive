//! The TellusHive harness: settings, the agent team, skills, plugins (MCP), projects and
//! the built-in engineering tools, mounted as a plugin bundle on th-hive (the Rust port of
//! the DeepSeek Harness architecture).

pub mod bundle;
pub mod config;
pub mod engine;
pub mod external;
pub mod live;
pub mod project;
pub mod skills;
pub mod template;
pub mod tools;

pub use config::{AgentSpec, HarnessSettings, Settings, TeamSpec};
pub use engine::{Domain, Engine, Event, PluginState, Plugins, UserInput};
pub use th_hive;
pub use project::{Artifact, ArtifactKind, Project, ProjectInfo};
pub use template::Template;

use std::path::Path;
use std::sync::Arc;

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
                    let ctx = tools::Ctx { project: &p, library: &l, skills: &s, settings: &st };
                    tools::exec_builtin(&ctx, &name, &args).map(|o| o.text)
                }),
            }
        })
        .collect();
    th_mcp::serve_stdio("tellushive", served);
    Ok(())
}
