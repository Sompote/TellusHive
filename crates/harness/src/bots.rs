//! Bots: the top layer, the project manager the engineer talks to. A bot has its own name,
//! picture, character and conversation. It calls a saved team when the work needs one (it
//! speaks as that team's coordinator and delegates to the team's agents) or works alone with
//! its own model and tools, and it works on one project, whose standards and plugins set
//! what every agent may cite and drive.
//!
//! Each bot lives in `<data>/bots/<id>/`: `bot.json`, `avatar.png` and, unless it is linked
//! to a project, a `workspace/` project folder that holds its conversation and files.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::{data_dir, read_json, write_json, AgentSpec, Settings, TeamSpec};
use crate::project::{Project, ProjectInfo};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BotMode {
    /// Calls no team (it may still call other bots).
    #[default]
    Solo,
    /// Also calls a team, through the team's coordinator as its lead.
    Team,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Bot {
    pub id: String,
    pub name: String,
    /// One line under the name in the rail and the profile.
    pub tagline: String,
    /// Who the bot is and how it talks; added to its instructions.
    pub character: String,
    /// "auto" answers in the engineer's language; otherwise a language name.
    pub language: String,
    pub color: String,
    pub mode: BotMode,
    /// Team mode: the saved team it fronts.
    pub team: String,
    /// Solo mode: the agent's work (what it does, beyond its character).
    pub instructions: String,
    pub provider: String,
    pub model: String,
    pub thinking: String,
    pub temperature: f32,
    pub tools: Vec<String>,
    pub skills: Vec<String>,
    /// The bot's bundle, applied to the project it works on: the plugins it drives and its
    /// project allows ("*" = every enabled one; empty = none for the bot, and the project's
    /// own set is left alone) and the standards (library ids) its agents may use.
    pub plugins: Vec<String>,
    pub standards: Vec<String>,
    /// Other bots (ids) it may hand work to.
    pub calls: Vec<String>,
    /// May hand work to any saved team (through its lead) or other bot; when false, only to
    /// the team and the bots chosen in `team` / `calls`.
    pub call_any: bool,
    /// Its own LLM API (base URL, key, model) instead of a provider from Settings.
    pub own_api: Option<th_llm::Provider>,
    pub can_use_external: bool,
    pub max_turns: usize,
    /// A project folder the bot works in; empty = its own workspace folder.
    pub project: String,
    pub created: u64,
}

impl Default for Bot {
    fn default() -> Self {
        Bot {
            id: String::new(),
            name: "New bot".into(),
            tagline: String::new(),
            character: String::new(),
            language: "auto".into(),
            color: "#7b4fd6".into(),
            mode: BotMode::Solo,
            team: "Design office".into(),
            instructions: String::new(),
            provider: String::new(),
            model: String::new(),
            thinking: String::new(),
            temperature: 0.3,
            tools: SOLO_TOOLS.iter().map(|s| s.to_string()).collect(),
            skills: vec!["*".into()],
            // each program starts when the bot runs, so a new bot drives none until chosen
            plugins: vec![],
            can_use_external: false,
            max_turns: 40,
            project: String::new(),
            standards: vec![],
            calls: vec![],
            call_any: true,
            own_api: None,
            created: 0,
        }
    }
}

/// The built-in tools a new solo bot starts with: it can read the standards, calculate,
/// run Python, use the design modules, draw and write files.
pub const SOLO_TOOLS: &[&str] = &[
    "list_standards",
    "search_standards",
    "get_clause",
    "list_design_modules",
    "run_design_module",
    "calc_sheet",
    "run_python",
    "make_drawing",
    "list_files",
    "read_file",
    "write_file",
    "read_skill",
    "describe_image",
];

/// Ready-made characters: (label, tagline, character).
pub const CHARACTERS: &[(&str, &str, &str)] = &[
    (
        "Senior engineer",
        "Precise, cites the standard",
        "You are a senior civil engineer with thirty years of practice. You are precise and direct, you cite the clause of the standard for every check, you state each assumption, and you say plainly when the data are too thin to support a conclusion.",
    ),
    (
        "Friendly tutor",
        "Explains step by step",
        "You are a patient teacher of civil engineering. You explain each step in plain words before the equation, show the units at every line, give a short worked example when a concept is new, and end with one question that checks understanding.",
    ),
    (
        "Site engineer",
        "Short, practical answers",
        "You are an experienced site engineer. You answer in short, practical sentences, think about constructability, safety and cost on site, and give the number the crew needs first, then the reason.",
    ),
    (
        "Independent checker",
        "Sceptical reviewer",
        "You are an independent checking engineer. You do not trust numbers you have not recomputed, you look for missing load cases, unit slips and misread clauses, and you finish with a clear verdict: PASS, or a numbered list of what must change.",
    ),
    (
        "Research professor",
        "Rigorous, flags weak evidence",
        "You are a professor of geotechnical engineering and artificial intelligence. You reason from soil mechanics and the test methods, say when a dataset is too small or a claim outruns its evidence, and write in clear, connected scientific prose.",
    ),
];

pub const LANGUAGES: &[&str] = &["auto", "English", "Thai", "Japanese", "Chinese"];

impl Bot {
    pub fn root() -> PathBuf {
        data_dir().join("bots")
    }
    pub fn folder(&self) -> PathBuf {
        Self::root().join(&self.id)
    }
    pub fn avatar_path(&self) -> PathBuf {
        self.folder().join("avatar.png")
    }
    pub fn has_avatar(&self) -> bool {
        self.avatar_path().is_file()
    }

    /// A new bot with a fresh id (not saved yet).
    pub fn new(name: &str) -> Bot {
        let now = crate::engine::stamp();
        let base = crate::project::slug(name).to_lowercase();
        let mut id = format!("{base}-{}", now % 100_000);
        let mut n = 2;
        while Self::root().join(&id).exists() {
            id = format!("{base}-{}-{n}", now % 100_000);
            n += 1;
        }
        Bot { id, name: name.to_string(), created: now, ..Default::default() }
    }

    /// Every saved bot, oldest first (the rail keeps its order).
    pub fn list() -> Vec<Bot> {
        let mut out: Vec<Bot> = std::fs::read_dir(Self::root())
            .map(|rd| rd.flatten().filter_map(|e| read_json::<Bot>(&e.path().join("bot.json")).ok()).collect())
            .unwrap_or_default();
        out.sort_by_key(|b| (b.created, b.id.clone()));
        out
    }

    pub fn save(&self) -> Result<(), String> {
        if self.id.is_empty() {
            return Err("a bot needs an id".into());
        }
        // a bot with its own API key keeps the file readable only by the engineer
        write_json(&self.folder().join("bot.json"), self, self.own_api.is_some())
    }

    /// Deletes the bot's folder, with its own workspace (a linked project is left alone).
    pub fn delete(&self) -> Result<(), String> {
        if self.id.is_empty() || self.id.contains(['/', '\\']) || self.id.starts_with('.') {
            return Err("bad bot id".into());
        }
        std::fs::remove_dir_all(self.folder()).map_err(|e| e.to_string())
    }

    /// Crops a picture to its centre square, scales it to 256 px and saves it as the avatar.
    pub fn set_avatar(&self, src: &Path) -> Result<(), String> {
        let img = image::open(src).map_err(|e| format!("{}: {e}", src.display()))?;
        let side = img.width().min(img.height());
        let x = (img.width() - side) / 2;
        let y = (img.height() - side) / 2;
        let sq = img.crop_imm(x, y, side, side).resize_exact(256, 256, image::imageops::FilterType::Lanczos3);
        std::fs::create_dir_all(self.folder()).map_err(|e| e.to_string())?;
        sq.to_rgba8().save(self.avatar_path()).map_err(|e| e.to_string())
    }

    pub fn remove_avatar(&self) {
        let _ = std::fs::remove_file(self.avatar_path());
    }

    /// The project the bot works in: the linked project, or its own workspace folder.
    pub fn project(&self) -> Result<Project, String> {
        if !self.project.trim().is_empty() {
            let dir = PathBuf::from(self.project.trim());
            let mut p = Project::open_or_create(&dir)?;
            self.apply_bundle(&mut p)?;
            return Ok(p);
        }
        let dir = self.folder().join("workspace");
        let mut p = if dir.join("project.json").is_file() {
            Project::open(&dir)?
        } else {
            Project::create(&dir, ProjectInfo { name: self.name.clone(), ..Default::default() })?
        };
        if p.info.name != self.name {
            p.info.name = self.name.clone();
            p.save()?;
        }
        self.apply_bundle(&mut p)?;
        Ok(p)
    }

    /// Takes the project's standards and plugins into the bot's bundle, after the bot changed
    /// them from the chat. Returns true when the bundle changed.
    pub fn take_bundle(&mut self, p: &Project) -> bool {
        let plugins = match &p.info.plugins {
            // every plugin allowed: a bot that drives none keeps driving none
            None if self.plugins.is_empty() => vec![],
            None => vec!["*".to_string()],
            Some(l) => l.clone(),
        };
        if self.standards == p.info.standards && self.plugins == plugins {
            return false;
        }
        self.standards = p.info.standards.clone();
        self.plugins = plugins;
        true
    }

    /// Sets the project up with the bot's bundle: its standards and plugins.
    pub fn apply_bundle(&self, p: &mut Project) -> Result<(), String> {
        let plugins = if self.plugins.iter().any(|x| x == "*") {
            None
        } else if self.plugins.is_empty() {
            p.info.plugins.clone()
        } else {
            Some(self.plugins.clone())
        };
        if p.info.standards != self.standards || p.info.plugins != plugins {
            p.info.standards = self.standards.clone();
            p.info.plugins = plugins;
            p.save()?;
        }
        Ok(())
    }

    /// The name shown and given to the model; a name being edited may be empty for a moment.
    pub fn display_name(&self) -> String {
        let n = self.name.trim();
        if n.is_empty() { "Unnamed bot".into() } else { n.to_string() }
    }

    /// The words added at the top of the bot's instructions: who it is and how it talks.
    pub fn persona(&self) -> String {
        let mut s = format!("You are {}, the project manager the engineer talks to directly: you agree the scope with the engineer, get the work done and report back.", self.display_name());
        if !self.character.trim().is_empty() {
            s.push_str(&format!(" Your character: {}", self.character.trim()));
        }
        match self.language.as_str() {
            "" | "auto" => s.push_str(" Answer in the language the engineer writes in."),
            l => s.push_str(&format!(" Always answer in {l}.")),
        }
        s.push_str(" Write like a person in a chat: short paragraphs, the answer first, no headings unless a long result needs them.");
        s
    }

    /// The bot as one agent: its character, its work, its model and tools. The project it
    /// works on decides which plugins it may drive.
    pub fn agent_spec(&self, id: &str) -> AgentSpec {
        let work = if !self.instructions.trim().is_empty() {
            self.instructions.trim().to_string()
        } else if self.calls_anyone() {
            "Agree the scope with the engineer, then plan the work and hand each task to the team lead or the bot that fits it, with all the data it needs (geometry, loads, materials, soil data, which standard). Check what comes back, combine it and report. Do small questions yourself. State every assumption and flag missing or inconsistent data.".to_string()
        } else {
            "Help the engineer with civil and geotechnical engineering work. Use your tools for every number: search the standards library and cite clause numbers, use a design module when one fits, and calculate with calc_sheet or run_python rather than from memory. State each assumption and flag missing or inconsistent data.".to_string()
        };
        AgentSpec {
            id: id.to_string(),
            name: self.display_name(),
            role: if self.tagline.is_empty() { "Project manager bot".into() } else { self.tagline.clone() },
            instructions: format!("{}\n\n{}", self.persona(), work),
            provider: if self.own_api.is_some() { self.provider_name() } else { self.provider.clone() },
            model: self.model.clone(),
            tools: {
                // a bot sets up its project, so it always has the set-up tools
                let mut t = self.tools.clone();
                if !t.iter().any(|x| x == "*") {
                    for s in ["list_standards", "list_plugins", "set_project_standards", "set_project_plugins"] {
                        if !t.iter().any(|x| x == s) {
                            t.push(s.into());
                        }
                    }
                }
                t
            },
            skills: self.skills.clone(),
            plugins: self.plugins.clone(),
            can_delegate: false,
            can_use_external: self.can_use_external,
            max_turns: self.max_turns.max(1),
            time_limit_min: 0,
            temperature: self.temperature,
            thinking: self.thinking.clone(),
            color: self.color.clone(),
            pos: [60.0, 60.0],
        }
    }

    /// The bot's own conversation in its project (several bots may work in one project).
    pub fn chat_key(&self) -> String {
        format!("bot-{}", crate::project::slug(&self.id).to_lowercase())
    }
    pub fn chat_file(&self) -> String {
        format!("chat-{}.json", self.chat_key())
    }

    /// The name of the bot's own provider in its engine's settings.
    pub fn provider_name(&self) -> String {
        format!("bot:{}", self.id)
    }

    /// The settings a bot's engine runs with: the app's, plus the own API of every bot that
    /// has one (a bot it calls may use its own API too).
    pub fn engine_settings(global: &Settings, all: &[Bot]) -> Settings {
        let mut s = global.clone();
        for b in all {
            if let Some(p) = &b.own_api {
                let name = b.provider_name();
                s.providers.retain(|x| x.name != name);
                s.providers.push(th_llm::Provider { name, ..p.clone() });
            }
        }
        s
    }

    /// Whether the bot hands work to a team or to other bots.
    pub fn calls_anyone(&self) -> bool {
        self.call_any || self.calls_team() || !self.calls.is_empty()
    }
    pub fn calls_team(&self) -> bool {
        self.mode == BotMode::Team && !self.team.trim().is_empty()
    }

    /// The team the bot's engine runs. The bot is the coordinator; the bots it calls are its
    /// members, each with the bots and the team it calls in turn; a team it calls joins with
    /// its coordinator as the team lead, who delegates to the team's agents along the team's
    /// own links. A bot called twice, or one that calls back, joins only once.
    pub fn team_spec(&self, all: &[Bot]) -> TeamSpec {
        let teams = if self.call_any || all.iter().any(|b| b.call_any) { TeamSpec::list() } else { vec![] };
        self.team_spec_with(all, &teams)
    }

    /// `team_spec` with the saved teams given (tests pass their own).
    pub fn team_spec_with(&self, all: &[Bot], teams: &[TeamSpec]) -> TeamSpec {
        let mut agents = vec![self.agent_spec("bot")];
        let mut links: Vec<(String, String)> = vec![];
        let mut seen = std::collections::HashSet::from([self.id.clone()]);
        self.add_calls("bot", all, teams, &mut agents, &mut links, &mut seen, 0);
        for a in agents.iter_mut() {
            a.can_delegate = links.iter().any(|(f, _)| *f == a.id);
        }
        TeamSpec { name: self.display_name(), description: self.tagline.clone(), coordinator: "bot".into(), agents, links, custom_links: true }
    }

    /// Adds whom this bot calls: teams join once through their lead (ids prefixed with the
    /// team, so two teams' coordinators do not clash), bots once each with whom they call.
    #[allow(clippy::too_many_arguments)]
    fn add_calls(&self, me: &str, all: &[Bot], teams: &[TeamSpec], agents: &mut Vec<AgentSpec>, links: &mut Vec<(String, String)>, seen: &mut std::collections::HashSet<String>, depth: usize) {
        if depth > 4 {
            return;
        }
        let team_names: Vec<String> = if self.call_any {
            teams.iter().map(|t| t.name.clone()).collect()
        } else if self.calls_team() {
            vec![self.team.clone()]
        } else {
            vec![]
        };
        for name in team_names {
            if !seen.insert(format!("team:{name}")) {
                continue;
            }
            let team = teams.iter().find(|t| t.name == name).cloned().unwrap_or_else(|| TeamSpec::load(&name));
            let pre = format!("team-{}", crate::project::slug(&team.name).to_lowercase());
            let id = |a: &str| format!("{pre}-{a}");
            for a in &team.agents {
                let mut a = a.clone();
                if a.id == team.coordinator {
                    a.name = format!("{} lead", team.name);
                    a.role = format!("Team lead of '{}'. {}", team.name, a.role);
                } else {
                    a.name = format!("{} ({})", a.name, team.name);
                }
                a.id = id(&a.id);
                agents.push(a);
            }
            links.push((me.to_string(), id(&team.coordinator)));
            for (f, t) in team.effective_links() {
                links.push((id(&f), id(&t)));
            }
        }
        let bot_ids: Vec<String> = if self.call_any { all.iter().map(|b| b.id.clone()).collect() } else { self.calls.clone() };
        for bid in &bot_ids {
            let Some(b) = all.iter().find(|b| &b.id == bid) else { continue };
            if !seen.insert(b.id.clone()) {
                continue;
            }
            let id = format!("bot-{}", b.id);
            agents.push(b.agent_spec(&id));
            links.push((me.to_string(), id.clone()));
            b.add_calls(&id, all, teams, agents, links, seen, depth + 1);
        }
    }

    /// One line on how the bot works, for the profile.
    pub fn engine_line(&self, settings: &Settings, all: &[Bot]) -> String {
        let provider = if let Some(p) = &self.own_api {
            format!("own API ({})", api_host(&p.base_url))
        } else if self.provider.is_empty() {
            settings.default_provider.clone()
        } else {
            self.provider.clone()
        };
        let model = if !self.model.is_empty() {
            self.model.clone()
        } else if let Some(p) = &self.own_api {
            p.default_model.clone()
        } else if self.provider.is_empty() || self.provider == settings.default_provider {
            settings.default_model.clone()
        } else {
            settings.provider(&self.provider).map(|p| p.default_model.clone()).unwrap_or_default()
        };
        let model = if model.is_empty() { "its default model".to_string() } else { model };
        let mut who: Vec<String> = vec![];
        if self.call_any {
            return format!("Calls any team or bot · {provider} · {model}");
        }
        if self.calls_team() {
            who.push(format!("the team '{}'", self.team));
        }
        for id in &self.calls {
            if let Some(b) = all.iter().find(|b| &b.id == id) {
                who.push(b.name.clone());
            }
        }
        if who.is_empty() {
            format!("Works alone · {provider} · {model}")
        } else {
            format!("Calls {} · {provider} · {model}", who.join(", "))
        }
    }
}

/// The host of an API URL, for labels ("api.openai.com").
pub fn api_host(url: &str) -> String {
    url.split("://").nth(1).unwrap_or(url).split('/').next().unwrap_or("").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bot_may_call_any_team_and_bot() {
        let mut a = Bot::new("A");
        a.id = "a".into();
        let mut b = Bot::new("B");
        b.id = "b".into();
        b.call_any = false;
        let office = crate::config::default_team();
        let mut other = crate::config::default_team();
        other.name = "Team 2".into();
        let t = a.team_spec_with(&[a.clone(), b.clone()], &[office.clone(), other.clone()]);
        let first: Vec<String> = t.reachable("bot").iter().map(|x| x.name.clone()).collect();
        assert_eq!(first.len(), 3, "{first:?}");
        assert!(first.contains(&"Design office lead".to_string()) && first.contains(&"Team 2 lead".to_string()) && first.contains(&"B".to_string()));
        // the two teams' agents keep distinct ids
        let ids: std::collections::HashSet<&String> = t.agents.iter().map(|x| &x.id).collect();
        assert_eq!(ids.len(), t.agents.len());
        assert_eq!(t.agents.len(), 1 + office.agents.len() + other.agents.len() + 1);
        assert!(t.agent("bot").unwrap().can_delegate);
    }

    #[test]
    fn a_bot_alone_is_a_team_of_one() {
        let mut b = Bot::new("Clover");
        b.character = "Calm and exact.".into();
        b.call_any = false;
        let t = b.team_spec(&[]);
        assert_eq!(t.agents.len(), 1);
        assert_eq!(t.coordinator, "bot");
        let a = &t.agents[0];
        assert_eq!(a.name, "Clover");
        assert!(a.instructions.starts_with("You are Clover"));
        assert!(a.instructions.contains("Calm and exact."));
        assert!(!a.can_delegate);
        assert!(t.reachable("bot").is_empty());
    }

    #[test]
    fn a_bot_with_its_own_api_gets_its_own_provider() {
        let mut b = Bot::new("Own");
        b.call_any = false;
        b.own_api = Some(th_llm::Provider { api_key: "sk-test".into(), default_model: "gpt-x".into(), ..th_llm::Provider::openai() });
        let s = Bot::engine_settings(&Settings::default(), &[b.clone()]);
        let p = s.provider(&b.provider_name()).expect("own provider");
        assert_eq!(p.api_key, "sk-test");
        let t = b.team_spec(&[b.clone()]);
        assert_eq!(t.agents[0].provider, b.provider_name());
        // the app's own providers are untouched
        assert_eq!(s.providers.len(), Settings::default().providers.len() + 1);
    }

    #[test]
    fn a_bot_calls_a_team_through_its_lead() {
        let mut b = Bot::new("Atlas");
        b.call_any = false;
        b.mode = BotMode::Team;
        b.team = crate::config::default_team().name;
        let t = b.team_spec(&[]);
        let n = crate::config::default_team().agents.len();
        assert_eq!(t.agents.len(), n + 1);
        let lead = t.reachable("bot");
        assert_eq!(lead.len(), 1);
        assert!(lead[0].name.ends_with("lead"));
        assert!(lead[0].can_delegate);
        assert_eq!(t.reachable(&lead[0].id).len(), n - 1);
        assert!(t.agent("bot").unwrap().can_delegate);
    }

    #[test]
    fn bots_call_bots_once_without_loops() {
        let mut a = Bot::new("A");
        let mut b = Bot::new("B");
        b.id = "b".into();
        a.id = "a".into();
        a.calls = vec!["b".into()];
        b.calls = vec!["a".into()];
        a.call_any = false;
        b.call_any = false;
        b.mode = BotMode::Team;
        b.team = crate::config::default_team().name;
        let all = vec![a.clone(), b.clone()];
        let t = a.team_spec(&all);
        let first = t.reachable("bot");
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].name, "B");
        // B calls its team lead, and not A again
        let second = t.reachable(&first[0].id);
        assert_eq!(second.len(), 1);
        assert!(second[0].name.ends_with("lead"));
    }
}
