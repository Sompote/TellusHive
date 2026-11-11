//! The TellusHive bundle: the plugins that make th-hive a civil-engineering design office.
//! They are mounted after the hive base bundle, and each one only registers on the hive's
//! seams (tools, tool sources, subagent sources, prompt sections, hooks), as plugins do in
//! DeepSeek Harness.
//!
//! | plugin | registers |
//! |---|---|
//! | `tellushive-domain` | the `tellushive` service (settings, team, project, library, programs) |
//! | `tellushive-tools` | the built-in engineering tools (standards, calculations, drawings, reports, files) |
//! | `tellushive-mcp` | a per-agent tool source with its MCP programs, and the gateway for large ones |
//! | `tellushive-team` | team members as subagents, Claude Code and Codex runners |
//! | `tellushive-prompt` | the prompt sections (role, project, standards, team, skills, programs, rules) |
//! | `tellushive-tool-cap` | `agent/request`: never more tools than a provider accepts |
//! | `tellushive-ui` | session and stream events to the chat's `Event`s |
//! | `tellushive-cli-models` | a model adapter that runs Claude Code or Codex in place of an API |

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use th_hive::plugins::subagent::{ChildAgent, SubagentProvider, Subagents};
use th_hive::plugins::{base_bundle, BaseConfig};
use th_hive::{keys, Agent, AgentLoop, AgentRegistry, Context, EventBody, Plugin, SessionStore, StreamFrame, SystemPrompt, ToolDef, ToolOutput, ToolRegistry};
use th_mcp::ServerConfig;

use crate::config::HarnessSettings;
use crate::engine::{Domain, Event, DOMAIN};
use crate::external::{self, Which};
use crate::live;
use crate::project::{clip, ArtifactKind};
use crate::skills;
use crate::tools;

/// The whole profile: hive base bundle, then the TellusHive bundle.
pub fn profile(domain: &Arc<Domain>, h: &HarnessSettings) -> Vec<Arc<dyn Plugin>> {
    let d = domain.clone();
    let cfg = BaseConfig {
        sessions_dir: Some(domain.project().dir.join("sessions")),
        max_parallel: h.max_parallel.max(1),
        compaction_budget_tokens: h.compaction_budget_tokens.max(8_000),
        repeat_thresholds: h.repeat_thresholds.clone(),
        retries: h.retries,
        spill_chars: h.spill_chars,
        spill_locator: Some(Arc::new(move |_| Some((d.project().dir.join("sessions/spill"), "sessions/spill".into())))),
        disabled: h.disabled_plugins.clone(),
        ..Default::default()
    };
    let mut p = base_bundle(&cfg);
    let ours: Vec<Arc<dyn Plugin>> = vec![
        Arc::new(DomainPlugin(domain.clone())),
        Arc::new(BuiltinTools),
        Arc::new(McpTools),
        Arc::new(Team),
        Arc::new(Prompt),
        Arc::new(ToolCap),
        Arc::new(UiBridge),
        Arc::new(CliModels),
    ];
    p.extend(ours.into_iter().filter(|x| !h.disabled_plugins.contains(&x.name())));
    p
}

fn domain(ctx: &Context) -> Result<Arc<Domain>, String> {
    ctx.expect::<Domain>(DOMAIN)
}

// ------------------------------------------------------------------ domain

struct DomainPlugin(Arc<Domain>);

impl Plugin for DomainPlugin {
    fn name(&self) -> String {
        "tellushive-domain".into()
    }
    fn describe(&self) -> String {
        "settings, team, project, standards library, programs, skills".into()
    }
    fn apply(&self, ctx: &Context) -> Result<(), String> {
        ctx.provide(DOMAIN, self.0.clone())
    }
}

// ------------------------------------------------------------------ built-in tools

/// Tools that only read, so several may run at once.
const READ_ONLY: [&str; 8] = ["list_standards", "search_standards", "get_clause", "list_design_modules", "list_files", "read_file", "read_skill", "describe_image"];

struct BuiltinTools;

fn artifacts_meta(list: &[(ArtifactKind, PathBuf)]) -> Value {
    json!({"artifacts": list.iter().map(|(k, p)| json!({"kind": k, "path": p})).collect::<Vec<_>>()})
}

impl Plugin for BuiltinTools {
    fn name(&self) -> String {
        "tellushive-tools".into()
    }
    fn describe(&self) -> String {
        format!("{} engineering tools", tools::builtin_specs().len())
    }
    fn inject(&self) -> Vec<&'static str> {
        vec![keys::TOOLS, DOMAIN]
    }
    fn apply(&self, ctx: &Context) -> Result<(), String> {
        let reg = ctx.expect::<ToolRegistry>(keys::TOOLS)?;
        let dom = domain(ctx)?;
        for spec in tools::builtin_specs() {
            let d = dom.clone();
            let name = spec.name.clone();
            let mut def = ToolDef::new(&spec.name, &spec.description, spec.parameters.clone(), move |t, args: Value| {
                let (settings, project, skills) = (d.settings(), d.project(), d.skills());
                let c = tools::Ctx { project: &project, library: &d.library, skills: &skills, settings: &settings, cancel: Some(&t.agent.cancel) };
                let out = tools::exec_builtin(&c, &name, &args)?;
                Ok(ToolOutput { text: out.text, meta: artifacts_meta(&out.artifacts), ..Default::default() })
            });
            if READ_ONLY.contains(&spec.name.as_str()) {
                def = def.concurrent();
            }
            reg.register(ctx, def)?;
        }
        Ok(())
    }
}

// ------------------------------------------------------------------ MCP programs

/// Providers accept at most this many tools in one request.
pub const MAX_TOOLS: usize = 128;

/// Which plugins go through the gateway: none when everything fits, otherwise the largest
/// ones until the rest fit beside the `base` tools and the two gateway tools.
pub fn choose_gateway(base: usize, plugins: &[(String, usize)]) -> Vec<String> {
    let total: usize = plugins.iter().map(|(_, n)| n).sum();
    if base + total <= MAX_TOOLS {
        return vec![];
    }
    let mut by_size: Vec<&(String, usize)> = plugins.iter().collect();
    by_size.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let mut direct = total;
    let mut out = vec![];
    for (name, n) in by_size {
        if base + 2 + direct <= MAX_TOOLS {
            break;
        }
        direct -= n;
        out.push(name.clone());
    }
    out
}

/// OpenAI tool names allow [a-zA-Z0-9_-]{1,64}.
pub fn tool_name(server: &str, tool: &str) -> String {
    let clean = |s: &str| s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect::<String>();
    let n = format!("{}__{}", clean(server), clean(tool));
    n.chars().take(64).collect()
}

struct McpTools;

#[derive(Default)]
struct Reported(HashSet<String>);

fn plugin_tool(d: &Arc<Domain>, server: &str, tool: &str, name: String, description: String, schema: Value) -> ToolDef {
    let (d, server, tool) = (d.clone(), server.to_string(), tool.to_string());
    ToolDef::new(&name, &description, schema, move |t, args| d.call_plugin(&server, &tool, args, Some(&t.agent.cancel)))
}

fn gateway_tools(d: &Arc<Domain>, gateway: Vec<String>) -> Vec<Arc<ToolDef>> {
    let list = gateway.join(", ");
    let (d1, g1) = (d.clone(), gateway.clone());
    let lister = ToolDef::new(
        "plugin_tools",
        &format!("Lists the tools of a plugin that is not listed directly ({list}), or, with `tool`, gives one tool's full description and argument schema. Call this before plugin_call."),
        json!({"type":"object","properties":{"plugin":{"type":"string","enum":gateway},"tool":{"type":"string","description":"A tool name from the list; omit to list all tools"}},"required":["plugin"]}),
        move |_, a| {
            let server = a.get("plugin").and_then(|v| v.as_str()).ok_or("missing 'plugin'")?;
            if !g1.iter().any(|g| g == server) {
                return Err(format!("'{server}' is not one of this agent's gateway plugins ({})", g1.join(", ")));
            }
            let c = d1.plugins.ensure_for(&d1.settings(), server, Some(&d1.project().dir))?;
            match a.get("tool").and_then(|v| v.as_str()).filter(|t| !t.is_empty()) {
                None => Ok(ToolOutput::ok(format!("{} tools of {server} (call plugin_tools with `tool` for the arguments):\n{}", c.tools.len(), c.tools.iter().map(|t| format!("- {}: {}", t.name, clip(t.description.lines().next().unwrap_or(""), 200))).collect::<Vec<_>>().join("\n")))),
                Some(t) => {
                    let info = c.tools.iter().find(|x| x.name == t).ok_or_else(|| format!("{server} has no tool '{t}'"))?;
                    Ok(ToolOutput::ok(format!("{server} / {}\n{}\n\nArguments (JSON schema):\n{}", info.name, info.description, serde_json::to_string_pretty(&info.input_schema).unwrap_or_default())))
                }
            }
        },
    )
    .concurrent();
    let (d2, g2) = (d.clone(), gateway.clone());
    let caller = ToolDef::new(
        "plugin_call",
        &format!("Runs one tool of a plugin that is not listed directly ({list}). Get the tool's argument schema with plugin_tools first."),
        json!({"type":"object","properties":{"plugin":{"type":"string","enum":gateway},"tool":{"type":"string"},"arguments":{"type":"object","description":"The tool's arguments, as its schema describes"}},"required":["plugin","tool"]}),
        move |t, a| {
            let server = a.get("plugin").and_then(|v| v.as_str()).ok_or("missing 'plugin'")?;
            if !g2.iter().any(|g| g == server) {
                return Err(format!("'{server}' is not one of this agent's gateway plugins ({})", g2.join(", ")));
            }
            let tool = a.get("tool").and_then(|v| v.as_str()).ok_or("missing 'tool'")?;
            let c = d2.plugins.ensure_for(&d2.settings(), server, Some(&d2.project().dir))?;
            if !c.tools.iter().any(|x| x.name == tool) {
                return Err(format!("{server} has no tool '{tool}'; list them with plugin_tools"));
            }
            let targs = a.get("arguments").cloned().filter(|v| v.is_object()).unwrap_or_else(|| json!({}));
            d2.call_plugin(server, tool, targs, Some(&t.agent.cancel))
        },
    );
    vec![Arc::new(lister), Arc::new(caller)]
}

impl Plugin for McpTools {
    fn name(&self) -> String {
        "tellushive-mcp".into()
    }
    fn describe(&self) -> String {
        "each agent's MCP programs as tools (gateway above 128 tools)".into()
    }
    fn inject(&self) -> Vec<&'static str> {
        vec![keys::TOOLS, DOMAIN]
    }
    fn apply(&self, ctx: &Context) -> Result<(), String> {
        let reg = ctx.expect::<ToolRegistry>(keys::TOOLS)?;
        let d = domain(ctx)?;
        reg.source(ctx, move |agent, _| {
            let Some(spec) = d.spec_of(agent) else { return vec![] };
            let settings = d.settings();
            let project = d.project();
            let mut found: Vec<(String, Vec<(String, String, String, Value)>)> = vec![];
            for name in &spec.plugins {
                let Some(cfg) = settings.plugins.iter().find(|p| &p.name == name && p.enabled) else { continue };
                match d.plugins.ensure_for(&settings, &cfg.name, Some(&project.dir)) {
                    Ok(c) => found.push((cfg.name.clone(), c.tools.iter().map(|t| (t.name.clone(), tool_name(&cfg.name, &t.name), format!("[{}] {}", cfg.name, clip(&t.description, 1000)), t.input_schema.clone())).collect())),
                    Err(e) => {
                        // report a program that does not start once per agent, not at every step
                        if agent.with_state::<Reported, _>("mcp-errors", |r| r.0.insert(cfg.name.clone())) {
                            d.send(Event::Error { run: 0, agent: agent.preset.name.clone(), text: format!("plugin {}: {e}", cfg.name) });
                        }
                    }
                }
            }
            let builtin = tools::builtin_specs().iter().filter(|t| agent.preset.tools.allows(&t.name)).count();
            let base = builtin + 8; // + harness tools (todo, task board, delegation)
            let sizes: Vec<(String, usize)> = found.iter().map(|(n, t)| (n.clone(), t.len())).collect();
            let gateway = choose_gateway(base, &sizes);
            let mut out: Vec<Arc<ToolDef>> = vec![];
            for (server, list) in found {
                if gateway.contains(&server) {
                    continue;
                }
                for (orig, name, desc, schema) in list {
                    out.push(Arc::new(plugin_tool(&d, &server, &orig, name, desc, schema)));
                }
            }
            if !gateway.is_empty() {
                out.extend(gateway_tools(&d, gateway));
            }
            out
        });
        Ok(())
    }
}

// ------------------------------------------------------------------ team

/// A team member as a subagent backend: its preset is built when it starts, so a changed
/// team or model applies to the next task.
struct Member {
    d: Arc<Domain>,
    id: String,
    name: String,
    role: String,
}

impl SubagentProvider for Member {
    fn id(&self) -> String {
        self.id.clone()
    }
    fn name(&self) -> String {
        self.name.clone()
    }
    fn role(&self) -> String {
        self.role.clone()
    }
    fn run(&self, ctx: &Context, parent: &Arc<Agent>, task: &str) -> Result<String, String> {
        let spec = self.d.team().agent(&self.id).cloned().ok_or_else(|| format!("no agent '{}' in the team", self.id))?;
        let preset = self.d.preset(&spec)?;
        ChildAgent { preset }.run(ctx, parent, task)
    }
}

struct Team;

fn snapshot_files(d: &std::path::Path) -> HashMap<PathBuf, std::time::SystemTime> {
    let mut m = HashMap::new();
    let Ok(rd) = std::fs::read_dir(d) else { return m };
    for e in rd.flatten() {
        if let Ok(t) = e.metadata().and_then(|m| m.modified()) {
            m.insert(e.path(), t);
        }
    }
    m
}

fn external_tool(d: &Arc<Domain>, which: Which) -> ToolDef {
    let (name, desc) = match which {
        Which::ClaudeCode => ("run_claude_code", "Run Claude Code (Anthropic's coding agent) on a programming task in the project's work folder."),
        Which::Codex => ("run_codex", "Run Codex (OpenAI's coding agent) on a programming task in the project's work folder."),
    };
    let (d1, d2) = (d.clone(), d.clone());
    ToolDef::new(name, desc, json!({"type":"object","properties":{"task":{"type":"string","description":"Complete task description"},"subfolder":{"type":"string","description":"Folder under work/ (default: work/)"}},"required":["task"]}), move |t, a| {
        let settings = d1.settings();
        let project = d1.project();
        let cfg = if which == Which::ClaudeCode { &settings.claude_code } else { &settings.codex };
        let task = a.get("task").and_then(|v| v.as_str()).ok_or("missing 'task'")?;
        let sub = a.get("subfolder").and_then(|v| v.as_str()).unwrap_or("");
        let workdir = project.resolve(&format!("work/{}", sub.trim_start_matches("work/")))?;
        let _ = std::fs::create_dir_all(&workdir);
        let exe = std::env::current_exe().ok().filter(|e| e.file_name().is_some_and(|n| n.to_string_lossy().starts_with("tellushive")));
        let before = snapshot_files(&workdir);
        let out = external::run(which, cfg, task, &workdir, &project.dir, exe.as_deref(), &t.agent.cancel)?;
        let new: Vec<(ArtifactKind, PathBuf)> = snapshot_files(&workdir).into_iter().filter(|(p, m)| !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')) && before.get(p) != Some(m)).map(|(p, _)| (ArtifactKind::Work, p)).collect();
        Ok(ToolOutput { text: format!("{} finished in {:.0} s ({}).\n{}", which.program(), out.seconds, if out.ok { "ok" } else { "reported an error" }, out.text), meta: artifacts_meta(&new), ..Default::default() })
    })
    .always()
    .when(move |a| {
        let s = d2.settings();
        let on = if which == Which::ClaudeCode { s.claude_code.enabled } else { s.codex.enabled };
        on && d2.spec_of(a).is_some_and(|x| x.can_use_external)
    })
}

impl Plugin for Team {
    fn name(&self) -> String {
        "tellushive-team".into()
    }
    fn describe(&self) -> String {
        "team members as subagents along the canvas arrows; Claude Code / Codex".into()
    }
    fn inject(&self) -> Vec<&'static str> {
        vec![keys::TOOLS, keys::SUBAGENTS, DOMAIN]
    }
    fn apply(&self, ctx: &Context) -> Result<(), String> {
        let subs = ctx.expect::<Subagents>(keys::SUBAGENTS)?;
        let d = domain(ctx)?;
        let d1 = d.clone();
        subs.source(ctx, move |a| {
            let team = d1.team();
            let Some(spec) = team.agent(&a.preset.id) else { return vec![] };
            if !spec.can_delegate {
                return vec![];
            }
            team.reachable(&spec.id).into_iter().map(|r| Arc::new(Member { d: d1.clone(), id: r.id.clone(), name: r.name.clone(), role: r.role.clone() }) as Arc<dyn SubagentProvider>).collect()
        });
        let reg = ctx.expect::<ToolRegistry>(keys::TOOLS)?;
        reg.register(ctx, external_tool(&d, Which::ClaudeCode))?;
        reg.register(ctx, external_tool(&d, Which::Codex))?;
        Ok(())
    }
}

// ------------------------------------------------------------------ prompt

struct Prompt;

impl Plugin for Prompt {
    fn name(&self) -> String {
        "tellushive-prompt".into()
    }
    fn describe(&self) -> String {
        "role, project, standards, team, skills, programs, rules".into()
    }
    fn inject(&self) -> Vec<&'static str> {
        vec![keys::PROMPT, DOMAIN]
    }
    fn apply(&self, ctx: &Context) -> Result<(), String> {
        let p = ctx.expect::<SystemPrompt>(keys::PROMPT)?;
        let d = domain(ctx)?;
        let dd = d.clone();
        ctx.effect(p.section(0, "role", move |a, _| {
            let spec = dd.spec_of(a)?;
            Some(format!("You are {}, an agent of a civil-engineering design system. {}\n\n{}", spec.name, spec.role, spec.instructions))
        }));
        let dd = d.clone();
        ctx.effect(p.section(10, "project", move |_, _| {
            let project = dd.project();
            let mut s = format!("# Project\nName: {}\nFolder: {}\n", project.info.name, project.dir.display());
            if !project.info.description.trim().is_empty() {
                s.push_str(&format!("Description: {}\n", project.info.description.trim()));
            }
            if !project.info.notes.trim().is_empty() {
                s.push_str(&format!("Engineer's notes: {}\n", project.info.notes.trim()));
            }
            Some(s)
        }));
        let dd = d.clone();
        ctx.effect(p.section(20, "standards", move |_, _| {
            let project = dd.project();
            let lib = dd.library.list();
            let chosen: Vec<String> = lib.iter().filter(|m| project.info.standards.contains(&m.id)).map(|m| format!("{} ({})", m.code, m.name)).collect();
            Some(if chosen.is_empty() {
                let mut s = String::from("# Design standards\nThe engineer has not chosen a standard for this project. ");
                if lib.is_empty() {
                    s.push_str("The library is empty, so no clause text is available: say which standard you follow and that its clauses were not checked against the source.");
                } else {
                    s.push_str(&format!("Library: {}. Ask which to follow if it matters.", lib.iter().map(|m| m.code.clone()).collect::<Vec<_>>().join(", ")));
                }
                s
            } else {
                format!("# Design standards chosen by the engineer\n{}\nDesign to these standards only, unless the engineer says otherwise.", chosen.join("\n"))
            })
        }));
        let dd = d.clone();
        ctx.effect(p.section(30, "team", move |a, tools| {
            if !tools.iter().any(|t| t.name == "delegate") {
                return None;
            }
            let team = dd.team();
            let mut s = String::from("# Your team (delegate with `delegate`; each task must be self-contained: give all data, the standard and the expected output)\nScope: delegate only the work the engineer's request needs. Do not start a member whose deliverable the engineer did not ask for and the answer does not depend on: a slope, excavation, embankment or other geotechnical analysis needs no building, BIM model or structural frame unless the engineer asks for one (and a BIM model of a slope is its terrain, not a building), and a member design needs no FE model unless asked. When a further deliverable would help, offer it in your answer instead of making it.\nTeam members work at the same time. Whenever two or more tasks do not need each other's results, give them out in the same reply (several `delegate` calls, or one `delegate_parallel`) instead of one after another: for example, separate members or load cases designed at once, or the Checker and the Drafter started together once the design results exist (the Reporter waits for the check to pass). Keep tasks in sequence only when one needs another's output, such as the check after the design. Do not give two parallel tasks the same plugin model or the same output file, because they would overwrite each other.\nFor a job with several parts, lay the parts out on the shared task board (`task_create` with `blocked_by` for the order); team members can read it with `task_list` and record results with `task_update`.\n");
            for r in team.reachable(&a.preset.id) {
                s.push_str(&format!("- {} (id `{}`): {}\n", r.name, r.id, r.role));
            }
            Some(s)
        }));
        let dd = d.clone();
        ctx.effect(p.section(40, "skills", move |a, _| {
            let spec = dd.spec_of(a)?;
            let all = dd.skills();
            let eq = skills::equipped(&all, &spec.skills);
            if eq.is_empty() {
                return None;
            }
            let mut s = String::from("# Skills (load one with `read_skill` before a task it covers)\n");
            for k in eq {
                s.push_str(&format!("- {}: {}\n", k.name, clip(&k.description, 300)));
            }
            Some(s)
        }));
        let dd = d.clone();
        ctx.effect(p.section(50, "programs", move |a, _| {
            let spec = dd.spec_of(a)?;
            let settings = dd.settings();
            let plugs: Vec<&ServerConfig> = settings.plugins.iter().filter(|p| p.enabled && spec.plugins.contains(&p.name)).collect();
            if plugs.is_empty() {
                return None;
            }
            let mut s = String::from("# Plugins (MCP programs on this computer; their tools are named <plugin>__<tool>)\n");
            for p in plugs {
                s.push_str(&format!("- {}: {}\n", p.name, p.description));
                if let Some(n) = live::prompt_note(&p.name) {
                    s.push_str(&n);
                    s.push('\n');
                }
            }
            Some(s)
        }));
        ctx.effect(p.section(60, "external", move |_, tools| {
            tools.iter().any(|t| t.name == "run_claude_code" || t.name == "run_codex").then(|| "# External coding agents\n`run_claude_code` and `run_codex` start Claude Code / Codex in the project's work/ folder with this system's tools (standards search, calculations, drawings, reports). Use them for programming tasks: scripts, data processing, parsing files, plots. Give a complete, precise task.".to_string())
        }));
        ctx.effect(p.section(90, "rules", move |_, _| {
            // the date only, so the prompt (and the provider's prefix cache) stays stable all day
            Some(format!("# Rules\n- Every number you report must come from a tool result or the engineer's data; never estimate a result you could calculate.\n- Cite clauses only from search_standards / get_clause results, by standard and clause number. If a clause is not found, say so.\n- State every assumption explicitly and flag missing data.\n- Units: SI (kN, kPa, m; MPa, mm for sections) unless the engineer uses others.\n- For work with three or more steps keep a todo list with `todo_write`.\n- Be concise. Lead with the result, then the governing checks.\nToday is {}.", tools::today()))
        }));
        Ok(())
    }
}

// ------------------------------------------------------------------ tool cap

/// A last guard on `agent/request`: never send more tools than a provider accepts.
struct ToolCap;

impl Plugin for ToolCap {
    fn name(&self) -> String {
        "tellushive-tool-cap".into()
    }
    fn describe(&self) -> String {
        format!("at most {MAX_TOOLS} tools per request")
    }
    fn inject(&self) -> Vec<&'static str> {
        vec![keys::LOOP, DOMAIN]
    }
    fn apply(&self, ctx: &Context) -> Result<(), String> {
        let lp = ctx.expect::<AgentLoop>(keys::LOOP)?;
        let d = domain(ctx)?;
        ctx.effect(lp.request.on(false, move |rc, next| {
            if rc.req.tools.len() > MAX_TOOLS {
                let dropped: Vec<String> = rc.req.tools.drain(MAX_TOOLS..).map(|t| t.name).collect();
                if rc.agent.with_state::<Reported, _>("tool-cap", |r| r.0.insert("cap".into())) {
                    d.send(Event::Error { run: 0, agent: rc.agent.preset.name.clone(), text: format!("{} has more tools than a model accepts; left out: {}", rc.agent.preset.name, dropped.join(", ")) });
                }
            }
            next(rc)
        }));
        Ok(())
    }
}

// ------------------------------------------------------------------ chat bridge

/// Turns `session/event` and `agent/assistant-stream` into the chat view's events.
struct UiBridge;

#[derive(Clone)]
struct Track {
    run: u64,
    name: String,
    depth: usize,
    started: bool,
    last_text: String,
    last_error: String,
}

impl Plugin for UiBridge {
    fn name(&self) -> String {
        "tellushive-ui".into()
    }
    fn describe(&self) -> String {
        "session and stream events to the chat".into()
    }
    fn inject(&self) -> Vec<&'static str> {
        vec![keys::AGENTS, keys::SESSIONS, DOMAIN]
    }
    fn apply(&self, ctx: &Context) -> Result<(), String> {
        let agents = ctx.expect::<AgentRegistry>(keys::AGENTS)?;
        let store = ctx.expect::<SessionStore>(keys::SESSIONS)?;
        let d = domain(ctx)?;
        let tracks: Arc<Mutex<HashMap<String, Track>>> = Arc::new(Mutex::new(HashMap::new()));
        let t1 = tracks.clone();
        ctx.effect(agents.created.on(move |a| {
            if let Ok(mut t) = t1.lock() {
                t.insert(a.id.clone(), Track { run: a.run, name: a.preset.name.clone(), depth: a.depth, started: false, last_text: String::new(), last_error: String::new() });
            }
        }));
        let (t2, d2) = (tracks.clone(), d.clone());
        ctx.effect(store.bus.on(move |ap| {
            let Some(mut tr) = t2.lock().ok().and_then(|t| t.get(&ap.session).cloned()) else { return };
            let (run, agent) = (tr.run, tr.name.clone());
            match &ap.event.body {
                EventBody::TurnStart { .. } => tr.started = false,
                EventBody::UserMessage { text, source, .. } if (source == "user" || source == "parent") && !tr.started => {
                    tr.started = true;
                    let task = text.strip_prefix("Task from ").and_then(|t| t.split_once(":\n").map(|x| x.1)).unwrap_or(text);
                    d2.send(Event::AgentStart { run, agent, depth: tr.depth, task: clip(task, 400) });
                }
                EventBody::AssistantMessage { text, reasoning, model, usage, .. } => {
                    d2.send(Event::Usage { run, agent: agent.clone(), model: model.clone(), prompt: usage.prompt, completion: usage.completion });
                    if let Some(r) = reasoning {
                        d2.send(Event::Thinking { run, agent: agent.clone(), text: r.clone() });
                    }
                    if !text.trim().is_empty() {
                        tr.last_text = text.clone();
                        d2.send(Event::Text { run, agent, text: text.clone() });
                    }
                }
                EventBody::ToolCall { id, name, arguments } => d2.send(Event::ToolCall { run, agent, id: id.clone(), tool: name.clone(), args: clip(arguments, 4000) }),
                EventBody::ToolResult { id, name, text, is_error, meta } => {
                    if let Some(list) = meta.get("artifacts").and_then(|a| a.as_array()) {
                        for a in list {
                            if let (Ok(kind), Some(path)) = (serde_json::from_value::<ArtifactKind>(a["kind"].clone()), a["path"].as_str()) {
                                d2.send(Event::Artifact { run, kind, path: PathBuf::from(path) });
                            }
                        }
                    }
                    if let (Some(plugin), Some(path)) = (meta.pointer("/model/plugin").and_then(|v| v.as_str()), meta.pointer("/model/path").and_then(|v| v.as_str())) {
                        d2.send(Event::Model { run, plugin: plugin.into(), path: PathBuf::from(path) });
                    }
                    d2.send(Event::ToolResult { run, agent, id: id.clone(), tool: name.clone(), ok: !is_error, text: clip(text, 6000) });
                }
                EventBody::AssistantAttempt { error, .. } => {
                    tr.last_error = error.clone();
                    if error != "cancelled" {
                        d2.send(Event::Thinking { run, agent, text: format!("(model request failed: {error})") });
                    }
                }
                EventBody::CompactionSummary { shadowed, .. } => d2.send(Event::Thinking { run, agent, text: format!("(context compacted: {} earlier messages summarised to save room)", shadowed.len()) }),
                EventBody::TurnEnd { reason, .. } => {
                    if reason == "error" {
                        d2.send(Event::Error { run, agent, text: tr.last_error.clone() });
                    } else {
                        d2.send(Event::AgentDone { run, agent, answer: tr.last_text.clone() });
                    }
                }
                _ => {}
            }
            if let Ok(mut t) = t2.lock() {
                t.insert(ap.session.clone(), tr);
            }
        }));
        let (t3, d3) = (tracks, d);
        ctx.effect(agents.stream.on(move |ev| {
            let Some(tr) = t3.lock().ok().and_then(|t| t.get(&ev.agent).cloned()) else { return };
            let (kind, text) = match &ev.frame {
                StreamFrame::Text(t) => ("text", t.clone()),
                StreamFrame::Reasoning(t) => ("reasoning", t.clone()),
                StreamFrame::ToolCall(n) => ("tool", n.clone()),
                _ => return,
            };
            d3.send(Event::Stream { run: tr.run, agent: tr.name, kind: kind.into(), text });
        }));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gateway_only_when_over_the_limit_and_largest_first() {
        let p = |v: &[(&str, usize)]| v.iter().map(|(n, k)| (n.to_string(), *k)).collect::<Vec<_>>();
        assert!(choose_gateway(12, &p(&[("tellus", 60), ("cadcraft", 20)])).is_empty());
        let g = choose_gateway(12, &p(&[("tellus", 55), ("tellus3d", 50), ("cadcraft", 16), ("pondus", 40), ("aedifex", 20)]));
        assert_eq!(g, vec!["tellus".to_string(), "tellus3d".to_string()]);
        assert_eq!(choose_gateway(10, &p(&[("huge", 300)])), vec!["huge".to_string()]);
    }

    #[test]
    fn tool_names_are_valid() {
        assert_eq!(tool_name("tellus 2d", "get.results"), "tellus_2d__get_results");
    }
}

// ------------------------------------------------------------------ Claude Code / Codex as the model

/// Lets any agent use Claude Code or Codex as its model (providers `claude-code` and
/// `codex`), on the engineer's subscription instead of an API key. One step of the agent is
/// one run of the program: it gets the agent's prompt and conversation on stdin, and this
/// program's MCP server for that agent, so it calls the same tools an API model would (the
/// built-in tools, the agent's programs, `delegate`, todo and task board), each through the
/// harness pipeline. Its tool calls are logged in a session of their own and mirrored into
/// the calling agent's session, so the chat shows them as they happen.
struct CliModels;

struct CliAdapter {
    ctx: Context,
    d: Arc<Domain>,
}

/// The prompt for one step: the agent's system prompt, how it works here, and the
/// conversation as a transcript.
fn cli_prompt(req: &th_hive::LlmRequest, program: &str, project: &std::path::Path) -> String {
    let mut s = req.system.clone();
    s.push_str(&format!(
        "\n\n# How you work here\nYou are this agent inside TellusHive, run through {program} instead of a model API. Your tools are those of the MCP server `tellushive`: the built-in design tools, the standards library, your plugins, and `delegate` when you lead a team. Use them for every calculation, clause lookup, drawing, report and delegation, as the rules above require. Do not work engineering results out in your head, and do not write project files by other means when a tool can do it. The project folder is {}. When the work is done, reply with your final answer to the last message only.\n\n# Conversation so far\n",
        project.display()
    ));
    for m in &req.messages {
        let text = match &m.content {
            Some(th_llm::Content::Parts(parts)) => parts.iter().map(|p| match p { th_llm::Part::Text { text } => text.clone(), _ => "[picture]".into() }).collect::<Vec<_>>().join("\n"),
            Some(c) => c.text(),
            None => String::new(),
        };
        match m.role.as_str() {
            "user" => s.push_str(&format!("\n## Message to you\n{text}\n")),
            "assistant" => {
                if !text.trim().is_empty() {
                    s.push_str(&format!("\n## You answered\n{text}\n"));
                }
                for c in m.tool_calls.iter().flatten() {
                    s.push_str(&format!("\n## You called {} with\n{}\n", c.function.name, clip(&c.function.arguments, 2000)));
                }
            }
            "tool" => s.push_str(&format!("\n## Tool result\n{}\n", clip(&text, 6000))),
            _ => {}
        }
    }
    s.push_str("\n# Now reply to the last message.\n");
    s
}

/// Follows the tool session the program's MCP server writes and copies its calls and results
/// into the calling agent's session (as `cli:` calls: logged and shown, not model history).
fn mirror(path: PathBuf, to: Arc<Agent>, done: Arc<std::sync::atomic::AtomicBool>) {
    use std::io::{Read, Seek, SeekFrom};
    use std::sync::atomic::Ordering;
    let mut pos = 0u64;
    let mut buf = String::new();
    loop {
        let finished = done.load(Ordering::Relaxed);
        if let Ok(mut f) = std::fs::File::open(&path) {
            if f.seek(SeekFrom::Start(pos)).is_ok() {
                let mut chunk = String::new();
                if let Ok(n) = f.read_to_string(&mut chunk) {
                    pos += n as u64;
                    buf.push_str(&chunk);
                }
            }
        }
        while let Some(i) = buf.find('\n') {
            let line: String = buf.drain(..=i).collect();
            let Ok(ev) = serde_json::from_str::<th_hive::SessionEvent>(line.trim()) else { continue };
            match ev.body {
                EventBody::ToolCall { id, name, arguments } => {
                    to.session.append(EventBody::ToolCall { id: format!("cli:{id}"), name, arguments });
                }
                EventBody::ToolResult { id, name, text, is_error, meta } => {
                    to.session.append(EventBody::ToolResult { id: format!("cli:{id}"), name, text, is_error, meta });
                }
                _ => {}
            }
        }
        if finished {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
    }
}

impl th_hive::LlmAdapter for CliAdapter {
    fn id(&self) -> &str {
        "cli-coding-agents"
    }
    fn serves(&self, p: &th_llm::Provider) -> bool {
        p.is_cli()
    }
    fn stream(&self, req: &th_hive::LlmRequest, on: &mut dyn FnMut(th_llm::Delta), cancel: &std::sync::atomic::AtomicBool) -> Result<th_llm::Reply, th_llm::CallError> {
        let fail = |message: String, cancelled: bool| th_llm::CallError { message, status: Some(400), overflow: false, cancelled };
        let which = if req.provider.base_url.ends_with("codex") { Which::Codex } else { Which::ClaudeCode };
        let settings = self.d.settings();
        let project = self.d.project();
        let cfg = if which == Which::ClaudeCode { settings.claude_code.clone() } else { settings.codex.clone() };
        let agent = self.ctx.get::<AgentRegistry>(keys::AGENTS).and_then(|r| r.get(&req.session)).ok_or_else(|| fail(format!("no live agent for session {}", req.session), false))?;
        let store = self.ctx.get::<SessionStore>(keys::SESSIONS).ok_or_else(|| fail("no session store".into(), false))?;
        let exe = std::env::current_exe().ok().filter(|e| e.file_name().is_some_and(|n| n.to_string_lossy().starts_with("tellushive"))).ok_or_else(|| fail(format!("{} as a model needs the tellushive program to serve its tools", which.program()), false))?;
        let tools_sid = store.new_id(&format!("{}-tools", agent.preset.id));
        let mcp = external::McpServer {
            name: "tellushive".into(),
            command: exe,
            args: vec!["--mcp".into(), "--project".into(), project.dir.display().to_string(), "--agent".into(), agent.preset.id.clone(), "--session".into(), tools_sid.clone(), "--parent".into(), agent.id.clone(), "--depth".into(), agent.depth.to_string()],
        };
        let prompt = cli_prompt(req, which.program(), &project.dir);
        let workdir = project.dir.join("work");
        let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let follower = {
            let (path, to, done) = (project.dir.join("sessions").join(format!("{tools_sid}.jsonl")), agent.clone(), done.clone());
            std::thread::spawn(move || mirror(path, to, done))
        };
        let r = external::run_job(which, &cfg, &external::Job { prompt: &prompt, workdir: &workdir, project: &project.dir, mcp: Some(mcp), model: &req.model, mcp_only: true }, cancel);
        done.store(true, std::sync::atomic::Ordering::Relaxed);
        let _ = follower.join();
        let out = r.map_err(|e| fail(format!("{}: {e}", which.program()), e == "cancelled"))?;
        if !out.ok {
            return Err(fail(format!("{} reported an error: {}", which.program(), clip(&out.text, 1500)), false));
        }
        on(th_llm::Delta::Text(out.text.clone()));
        Ok(th_llm::Reply {
            message: th_llm::Message::assistant(out.text),
            reasoning: None,
            finish_reason: "stop".into(),
            usage: th_llm::Usage { prompt_tokens: out.input_tokens, completion_tokens: out.output_tokens, cached_tokens: out.cached_tokens },
        })
    }
}

impl Plugin for CliModels {
    fn name(&self) -> String {
        "tellushive-cli-models".into()
    }
    fn describe(&self) -> String {
        "providers claude-code and codex: the coding agent runs as the model, with this agent's tools over MCP".into()
    }
    fn inject(&self) -> Vec<&'static str> {
        vec![keys::LLM, keys::AGENTS, keys::SESSIONS, DOMAIN]
    }
    fn apply(&self, ctx: &Context) -> Result<(), String> {
        let llm = ctx.expect::<th_hive::LlmService>(keys::LLM)?;
        ctx.effect(llm.register(Arc::new(CliAdapter { ctx: ctx.clone(), d: domain(ctx)? })));
        Ok(())
    }
}
