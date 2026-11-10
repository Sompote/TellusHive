//! Subagents (dsh-subagent): one activation API over several backends. A provider either
//! starts a local child agent on its own session (a fresh context, linked to the parent's
//! session) or hands the task to an external program. The model-facing `delegate` tool is
//! concurrent, so several delegations in one reply run at the same time;
//! `delegate_parallel` gives the same in one call.

use std::sync::{Arc, RwLock};

use serde_json::{json, Value};

use crate::agent::{Agent, AgentPreset, AgentRegistry, Inbound};
use crate::agent_loop::AgentLoop;
use crate::context::{Context, Plugin};
use crate::keys;
use crate::session::SessionMeta;
use crate::store::SessionStore;
use crate::tools::{ToolDef, ToolOutput, ToolRegistry};

/// A delegation backend.
pub trait SubagentProvider: Send + Sync {
    /// The id the model names (`designer`, `claude_code`).
    fn id(&self) -> String;
    fn name(&self) -> String;
    fn role(&self) -> String;
    /// Runs the task to completion and returns the answer.
    fn run(&self, ctx: &Context, parent: &Arc<Agent>, task: &str) -> Result<String, String>;
}

pub type ProviderSource = dyn Fn(&Agent) -> Vec<Arc<dyn SubagentProvider>> + Send + Sync;

/// `ctx.subagents`
pub struct Subagents {
    sources: Arc<RwLock<Vec<(String, Arc<ProviderSource>)>>>,
    pub max_depth: usize,
}

impl Subagents {
    /// Adds a source of providers for a parent agent (owned by `ctx`'s plugin).
    pub fn source(&self, ctx: &Context, f: impl Fn(&Agent) -> Vec<Arc<dyn SubagentProvider>> + Send + Sync + 'static) {
        let owner = ctx.owner().to_string();
        if let Ok(mut s) = self.sources.write() {
            s.push((owner.clone(), Arc::new(f)));
        }
        let list = self.sources.clone();
        ctx.effect(Box::new(move || {
            if let Ok(mut s) = list.write() {
                s.retain(|(o, _)| *o != owner);
            }
        }));
    }

    /// The providers this agent may delegate to.
    pub fn for_agent(&self, a: &Agent) -> Vec<Arc<dyn SubagentProvider>> {
        if a.depth + 1 > self.max_depth {
            return vec![];
        }
        let fs: Vec<Arc<ProviderSource>> = self.sources.read().map(|s| s.iter().map(|(_, f)| f.clone()).collect()).unwrap_or_default();
        let mut out: Vec<Arc<dyn SubagentProvider>> = vec![];
        for f in fs {
            for p in f(a) {
                if !out.iter().any(|x| x.id() == p.id()) {
                    out.push(p);
                }
            }
        }
        out
    }

    pub fn delegate(&self, ctx: &Context, parent: &Arc<Agent>, to: &str, task: &str) -> Result<String, String> {
        if parent.depth + 1 > self.max_depth {
            return Err("delegation is too deep; do this task yourself".into());
        }
        let p = self.for_agent(parent).into_iter().find(|p| p.id() == to).ok_or_else(|| format!("{} may not delegate to '{to}'", parent.preset.name))?;
        p.run(ctx, parent, task)
    }
}

/// A local child agent with its own preset, session and loop (dsh spawn-in-process).
pub struct ChildAgent {
    pub preset: AgentPreset,
}

impl SubagentProvider for ChildAgent {
    fn id(&self) -> String {
        self.preset.id.clone()
    }
    fn name(&self) -> String {
        self.preset.name.clone()
    }
    fn role(&self) -> String {
        self.preset.role.clone()
    }
    fn run(&self, ctx: &Context, parent: &Arc<Agent>, task: &str) -> Result<String, String> {
        let store = ctx.expect::<SessionStore>(keys::SESSIONS)?;
        let agents = ctx.expect::<AgentRegistry>(keys::AGENTS)?;
        let lp = ctx.expect::<AgentLoop>(keys::LOOP)?;
        let id = store.new_id(&self.preset.id);
        let session = store.create(SessionMeta { id, agent: self.preset.id.clone(), label: crate::clip(task, 120), parent: Some(parent.id.clone()), depth: parent.depth + 1 })?;
        let child = agents.create(self.preset.clone(), session, Some(parent), parent.cancel.clone());
        child.followup(Inbound { text: format!("Task from {}:\n{task}", parent.preset.name), images: vec![], source: "parent".into() });
        let r = lp.run_turn(ctx, &child);
        agents.remove(&child.id);
        let out = r?;
        match out.reason.as_str() {
            "completed" | "max-steps" => Ok(out.text),
            "cancelled" => Err("cancelled".into()),
            other => Err(format!("{} stopped ({other}): {}", self.preset.name, out.text)),
        }
    }
}

pub struct SubagentPlugin {
    pub max_depth: usize,
}

fn delegate_tool(subs: Arc<Subagents>, ids: Vec<String>) -> ToolDef {
    ToolDef::new(
        "delegate",
        "Give a task to one team member and wait for its answer. Several delegate calls in the same reply run at the same time.",
        json!({"type":"object","properties":{"agent":{"type":"string","enum":ids},"task":{"type":"string","description":"Complete, self-contained task with all data needed"}},"required":["agent","task"]}),
        move |t, a: Value| {
            let to = a.get("agent").and_then(|v| v.as_str()).ok_or("missing 'agent'")?;
            let task = a.get("task").and_then(|v| v.as_str()).ok_or("missing 'task'")?;
            subs.delegate(&t.ctx, &t.agent, to, task).map(ToolOutput::ok)
        },
    )
    .concurrent()
    .always()
}

fn parallel_tool(subs: Arc<Subagents>, ids: Vec<String>) -> ToolDef {
    ToolDef::new(
        "delegate_parallel",
        "Give independent tasks to several team members at once; returns all answers.",
        json!({"type":"object","properties":{"tasks":{"type":"array","items":{"type":"object","properties":{"agent":{"type":"string","enum":ids},"task":{"type":"string"}},"required":["agent","task"]}}},"required":["tasks"]}),
        move |t, a: Value| {
            let tasks: Vec<(String, String)> = a
                .get("tasks")
                .and_then(|t| t.as_array())
                .ok_or("missing 'tasks'")?
                .iter()
                .map(|t| (t.get("agent").and_then(|v| v.as_str()).unwrap_or("").to_string(), t.get("task").and_then(|v| v.as_str()).unwrap_or("").to_string()))
                .collect();
            let subs = &subs;
            let results: Vec<(String, Result<String, String>)> = std::thread::scope(|sc| {
                let hs: Vec<_> = tasks.iter().map(|(to, task)| (to.clone(), sc.spawn(move || subs.delegate(&t.ctx, &t.agent, to, task)))).collect();
                hs.into_iter().map(|(to, h)| (to, h.join().unwrap_or_else(|_| Err("agent thread panicked".into())))).collect()
            });
            Ok(ToolOutput::ok(results.into_iter().map(|(to, r)| format!("## {to}\n{}", r.unwrap_or_else(|e| format!("ERROR: {e}")))).collect::<Vec<_>>().join("\n\n")))
        },
    )
    .concurrent()
    .always()
}

impl Plugin for SubagentPlugin {
    fn name(&self) -> String {
        "hive-subagent".into()
    }
    fn describe(&self) -> String {
        format!("delegate / delegate_parallel, depth ≤ {}", self.max_depth)
    }
    fn inject(&self) -> Vec<&'static str> {
        vec![keys::TOOLS, keys::LOOP, keys::SESSIONS]
    }
    fn apply(&self, ctx: &Context) -> Result<(), String> {
        let subs = Arc::new(Subagents { sources: Arc::new(RwLock::new(vec![])), max_depth: self.max_depth });
        ctx.provide(keys::SUBAGENTS, subs.clone())?;
        let tools = ctx.expect::<ToolRegistry>(keys::TOOLS)?;
        // the tools' enum lists the providers each agent may reach, so they come from a source
        tools.source(ctx, move |a, _| {
            let ids: Vec<String> = subs.for_agent(a).iter().map(|p| p.id()).collect();
            if ids.is_empty() {
                return vec![];
            }
            vec![Arc::new(delegate_tool(subs.clone(), ids.clone())), Arc::new(parallel_tool(subs.clone(), ids))]
        });
        Ok(())
    }
}
