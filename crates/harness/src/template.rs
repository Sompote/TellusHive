//! Templates: the whole set-up of the agentic system saved under a name, so a new project can
//! start from it. A template holds the team (every agent with its role, instructions, model,
//! tools, skills and plugins, and the delegation links), the project's description, notes and
//! chosen standards, the default and vision models, the plugins the team uses and the skill
//! folders. It never holds API keys: plugin environment values and HTTP headers are blanked.
//! The conversation and the artifacts are project results, not set-up, so they stay out.
//!
//! ```text
//! <data dir>/templates/<name>.json
//! ```

use std::path::{Path, PathBuf};

use th_mcp::ServerConfig;
use serde::{Deserialize, Serialize};

use crate::config::{data_dir, read_json, write_json, Settings, TeamSpec};
use crate::project::ProjectInfo;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct Template {
    pub name: String,
    pub description: String,
    /// Date the template was saved (YYYY-MM-DD).
    pub created: String,
    pub app_version: String,
    pub team: TeamSpec,
    pub project_description: String,
    pub notes: String,
    /// Library ids of the chosen standards, and their codes so a reader without the same
    /// library still sees which standards were meant.
    pub standards: Vec<String>,
    pub standard_codes: Vec<String>,
    pub default_provider: String,
    pub default_model: String,
    pub vision_provider: String,
    pub vision_model: String,
    /// The plugins the team's agents use, with secrets blanked.
    pub plugins: Vec<ServerConfig>,
    pub skill_dirs: Vec<String>,
    /// A first message put in the chat box of a project made from this template.
    pub starter_prompt: String,
}

/// What applying a template did, for the status line.
#[derive(Debug, Default)]
pub struct Applied {
    pub info: ProjectInfo,
    pub team: TeamSpec,
    /// Plugins and skill folders added to the settings.
    pub settings_changed: bool,
    pub notes: Vec<String>,
}

fn blank_secrets(mut p: ServerConfig) -> ServerConfig {
    for v in p.env.values_mut() {
        v.clear();
    }
    for v in p.headers.values_mut() {
        v.clear();
    }
    p
}

impl Template {
    pub fn dir() -> PathBuf {
        data_dir().join("templates")
    }

    pub fn path_for(name: &str) -> PathBuf {
        Self::dir().join(TeamSpec::file_name(name))
    }

    /// The saved templates, newest first.
    pub fn list() -> Vec<Template> {
        let mut out: Vec<Template> = std::fs::read_dir(Self::dir())
            .map(|rd| rd.flatten().filter(|e| e.path().extension().is_some_and(|x| x == "json")).filter_map(|e| read_json(&e.path()).ok()).collect())
            .unwrap_or_default();
        out.sort_by(|a, b| b.created.cmp(&a.created).then(a.name.cmp(&b.name)));
        out
    }

    /// Captures the current set-up. `codes` maps library ids to standard codes.
    pub fn capture(name: &str, description: &str, settings: &Settings, team: &TeamSpec, info: &ProjectInfo, codes: &dyn Fn(&str) -> Option<String>, starter_prompt: &str) -> Template {
        let used: Vec<&String> = team.agents.iter().flat_map(|a| a.plugins.iter()).collect();
        let plugins = settings.plugins.iter().filter(|p| used.contains(&&p.name)).cloned().map(blank_secrets).collect();
        Template {
            name: name.trim().to_string(),
            description: description.trim().to_string(),
            created: crate::tools::today(),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            team: team.clone(),
            project_description: info.description.clone(),
            notes: info.notes.clone(),
            standards: info.standards.clone(),
            standard_codes: info.standards.iter().map(|id| codes(id).unwrap_or_else(|| id.clone())).collect(),
            default_provider: settings.default_provider.clone(),
            default_model: settings.default_model.clone(),
            vision_provider: settings.vision_provider.clone(),
            vision_model: settings.vision_model.clone(),
            plugins,
            skill_dirs: settings.skill_dirs.clone(),
            starter_prompt: starter_prompt.trim().to_string(),
        }
    }

    pub fn save(&self) -> Result<PathBuf, String> {
        if self.name.trim().is_empty() {
            return Err("give the template a name".into());
        }
        let p = Self::path_for(&self.name);
        write_json(&p, self, false)?;
        Ok(p)
    }

    pub fn export(&self, to: &Path) -> Result<(), String> {
        write_json(to, self, false)
    }

    /// Reads a template file without adding it to the library.
    pub fn read(from: &Path) -> Result<Template, String> {
        let t: Template = read_json(from)?;
        if t.team.agents.is_empty() {
            return Err(format!("{} is not a TellusHive template (it has no team)", from.display()));
        }
        Ok(t)
    }

    /// Reads a template file (an export from another computer) and saves it to the library.
    pub fn import(from: &Path) -> Result<Template, String> {
        let t = Self::read(from)?;
        t.save()?;
        Ok(t)
    }

    pub fn delete(name: &str) -> Result<(), String> {
        std::fs::remove_file(Self::path_for(name)).map_err(|e| e.to_string())
    }

    /// Makes the project set-up from the template. Plugins and skill folders missing from
    /// `settings` are added (switched on only when their program exists here); the user's
    /// keys and existing plugins are left alone. Agents that use the default model get the
    /// template's model written in, so the team runs as it did when it was saved.
    /// `has_standard` says whether a library id exists on this computer.
    pub fn apply(&self, project_name: &str, settings: &mut Settings, has_standard: &dyn Fn(&str) -> bool) -> Applied {
        let mut notes = vec![];
        let mut settings_changed = false;
        for p in &self.plugins {
            if settings.plugins.iter().any(|q| q.name == p.name) {
                continue;
            }
            let mut p = p.clone();
            p.enabled = if p.is_http() { true } else { Path::new(&p.command).is_file() };
            if !p.enabled {
                notes.push(format!("plugin '{}' added but switched off: {} was not found", p.name, p.command));
            }
            if p.env.values().chain(p.headers.values()).any(|v| v.is_empty()) {
                notes.push(format!("plugin '{}' needs its keys filled in under Plugins", p.name));
            }
            settings.plugins.push(p);
            settings_changed = true;
        }
        for d in &self.skill_dirs {
            if !settings.skill_dirs.contains(d) && Path::new(d).is_dir() {
                settings.skill_dirs.push(d.clone());
                settings_changed = true;
            }
        }
        let mut team = self.team.clone();
        for a in team.agents.iter_mut() {
            if a.provider.is_empty() && a.model.is_empty() && (self.default_provider != settings.default_provider || self.default_model != settings.default_model) && settings.provider(&self.default_provider).is_some() {
                a.provider = self.default_provider.clone();
                a.model = self.default_model.clone();
            }
        }
        if settings.provider(&self.default_provider).is_none() {
            notes.push(format!("provider '{}' is not set up here; agents without their own model use {}", self.default_provider, settings.default_provider));
        }
        // keep the user's own team of the same name if it differs
        let existing = TeamSpec::dir().join(TeamSpec::file_name(&team.name));
        if existing.is_file() && read_json::<TeamSpec>(&existing).map(|t| t != team).unwrap_or(true) {
            team.name = format!("{} ({})", team.name, self.name);
        }
        if let Err(e) = team.save() {
            notes.push(format!("team not saved: {e}"));
        }
        let standards: Vec<String> = self.standards.iter().filter(|id| has_standard(id)).cloned().collect();
        if standards.len() < self.standards.len() {
            let missing: Vec<String> = self.standards.iter().enumerate().filter(|(_, id)| !has_standard(id)).map(|(i, id)| self.standard_codes.get(i).cloned().unwrap_or_else(|| id.clone())).collect();
            notes.push(format!("{} of {} standards are not in this library ({}); import them under Standards", self.standards.len() - standards.len(), self.standards.len(), missing.join(", ")));
        }
        let info = ProjectInfo { name: project_name.to_string(), description: self.project_description.clone(), standards, team: team.name.clone(), notes: self.notes.clone() };
        Applied { info, team, settings_changed, notes }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_blanks_secrets_and_keeps_only_used_plugins() {
        let mut settings = Settings::default();
        let mut p = ServerConfig { name: "secret_plugin".into(), command: "/nope".into(), ..Default::default() };
        p.env.insert("API_KEY".into(), "sk-123".into());
        p.headers.insert("Authorization".into(), "Bearer x".into());
        settings.plugins = vec![p, ServerConfig { name: "unused".into(), ..Default::default() }];
        let mut team = crate::config::default_team();
        team.agents[0].plugins = vec!["secret_plugin".into()];
        for a in team.agents.iter_mut().skip(1) {
            a.plugins.clear();
        }
        let info = ProjectInfo { standards: vec!["aci318".into()], notes: "office rule".into(), ..Default::default() };
        let t = Template::capture("T", "d", &settings, &team, &info, &|id| Some(id.to_uppercase()), "go");
        assert_eq!(t.plugins.len(), 1);
        assert_eq!(t.plugins[0].env["API_KEY"], "");
        assert_eq!(t.plugins[0].headers["Authorization"], "");
        assert_eq!(t.standard_codes, vec!["ACI318".to_string()]);
        let s = serde_json::to_string(&t).unwrap();
        assert!(!s.contains("sk-123"));
        let back: Template = serde_json::from_str(&s).unwrap();
        assert_eq!(back, t);
    }
}
