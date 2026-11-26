//! Plugins: the MCP servers the agents can drive (the Tellus family, Pondus, Aedifex, CADCraft,
//! or any stdio program or HTTP endpoint).
//!
//! The list is one pool for the design office, kept under Settings › Plugins & MCP (`manage`):
//! add, edit, connect, test and remove a server, and see its tools and log. A project only
//! chooses which of them its agents may use (`project_left`, the project's Plugins step), and
//! sees the chosen server's tools (`project_center`).

use std::sync::Arc;

use th_harness::{PluginState, Plugins};
use th_mcp::ServerConfig;
use eframe::egui::{self, RichText};

use crate::chrome::{self, tokens};
use crate::icons::Icon;
use crate::theme::medium_font;

#[derive(Default)]
pub struct PluginsView {
    selected: usize,
    args_text: String,
    args_for: Option<usize>,
    confirm_remove: Option<usize>,
    /// "Manage plugins in Settings" was pressed on the project page.
    pub open_settings: bool,
    /// The JSON box (Claude Desktop / Claude Code `mcpServers` style) and what the last paste did.
    json_open: bool,
    json_text: String,
    json_msg: Option<(bool, String)>,
}

/// One server in the `mcpServers` style that Claude Desktop, Claude Code and Cursor use.
fn server_from_json(name: &str, v: &serde_json::Value, notes: &mut Vec<String>) -> Result<ServerConfig, String> {
    let o = v.as_object().ok_or_else(|| format!("{name}: expected an object with \"command\" or \"url\""))?;
    let text = |k: &str| o.get(k).and_then(|x| x.as_str()).unwrap_or("").trim().to_string();
    let map = |k: &str| -> std::collections::HashMap<String, String> {
        o.get(k).and_then(|x| x.as_object()).map(|m| m.iter().map(|(k, v)| (k.clone(), v.as_str().map(String::from).unwrap_or_else(|| v.to_string()))).collect()).unwrap_or_default()
    };
    let args: Vec<String> = match o.get("args") {
        None => vec![],
        Some(serde_json::Value::Array(a)) => a.iter().map(|x| x.as_str().map(String::from).unwrap_or_else(|| x.to_string())).collect(),
        Some(_) => return Err(format!("{name}: \"args\" must be a list of strings")),
    };
    let (command, url) = (text("command"), text("url").to_string());
    if command.is_empty() && url.is_empty() {
        return Err(format!("{name}: give \"command\" (a program) or \"url\" (an HTTP endpoint)"));
    }
    let kind = text("type").to_ascii_lowercase();
    if kind == "sse" {
        notes.push(format!("{name} is an SSE server; TellusHive speaks MCP's streamable HTTP, so it connects only if the server answers that too"));
    }
    let disabled = o.get("disabled").and_then(|x| x.as_bool()).unwrap_or(false);
    let timeout_s = o.get("timeout").and_then(|x| x.as_u64()).map(|t| if t > 10_000 { t / 1000 } else { t }).unwrap_or(600);
    Ok(ServerConfig {
        name: name.trim().to_string(),
        description: text("description"),
        command: if url.is_empty() || kind == "stdio" { command } else { String::new() },
        args,
        env: map("env"),
        cwd: text("cwd"),
        url,
        headers: map("headers"),
        enabled: !disabled,
        timeout_s: timeout_s.max(5),
    })
}

/// The servers in a pasted JSON: `{"mcpServers": {name: server}}` (Claude Desktop, Claude
/// Code, Cursor), `{"servers": {...}}` (VS Code), a bare `{name: server}` map, or one server
/// with its own `"name"`. Returns the servers and notes about them.
pub fn from_json(text: &str) -> Result<(Vec<ServerConfig>, Vec<String>), String> {
    let v: serde_json::Value = serde_json::from_str(text.trim()).map_err(|e| format!("not valid JSON: {e}"))?;
    let root = v.as_object().ok_or("expected a JSON object, such as {\"mcpServers\": {...}}")?;
    let mut notes = vec![];
    let single = root.contains_key("command") || root.contains_key("url");
    let servers: Vec<(String, &serde_json::Value)> = if single {
        let name = root.get("name").and_then(|n| n.as_str()).unwrap_or("server").to_string();
        vec![(name, &v)]
    } else {
        let map = root.get("mcpServers").or_else(|| root.get("servers")).and_then(|m| m.as_object()).unwrap_or(root);
        map.iter().map(|(k, v)| (k.clone(), v)).collect()
    };
    if servers.is_empty() {
        return Err("no server in it".into());
    }
    let mut out = vec![];
    for (name, v) in servers {
        if name.trim().is_empty() || name.contains(char::is_whitespace) || name.contains("__") {
            return Err(format!("'{name}' cannot be a plugin name: use letters, digits, - or _ (no spaces, no double underscore)"));
        }
        out.push(server_from_json(&name, v, &mut notes)?);
    }
    Ok((out, notes))
}

/// The servers in the `mcpServers` style, to paste into Claude Desktop, Claude Code or Cursor.
pub fn to_json(list: &[&ServerConfig]) -> String {
    let mut m = serde_json::Map::new();
    for p in list {
        let mut o = serde_json::Map::new();
        if p.is_http() {
            o.insert("type".into(), "http".into());
            o.insert("url".into(), p.url.clone().into());
            if !p.headers.is_empty() {
                o.insert("headers".into(), serde_json::json!(p.headers));
            }
        } else {
            o.insert("command".into(), p.command.clone().into());
            o.insert("args".into(), serde_json::json!(p.args));
            if !p.env.is_empty() {
                o.insert("env".into(), serde_json::json!(p.env));
            }
            if !p.cwd.trim().is_empty() {
                o.insert("cwd".into(), p.cwd.clone().into());
            }
        }
        m.insert(p.name.clone(), serde_json::Value::Object(o));
    }
    serde_json::to_string_pretty(&serde_json::json!({ "mcpServers": m })).unwrap_or_default()
}

fn state_line(t: &crate::theme::Tokens, state: &PluginState, enabled: bool) -> (egui::Color32, String) {
    match state {
        PluginState::Off => (t.text_faint, if enabled { "available" } else { "off" }.to_string()),
        PluginState::Connecting => (t.warn, "connecting…".to_string()),
        PluginState::Ready { tools } => (t.ok, format!("{tools} tools")),
        PluginState::Failed(_) => (t.error, "failed".to_string()),
    }
}

/// Connects in the background, so the window stays responsive while a program starts.
fn connect(ui: &egui::Ui, cfg: &ServerConfig, plugins: &Arc<Plugins>) {
    let (cfg, pl, ctx) = (cfg.clone(), plugins.clone(), ui.ctx().clone());
    std::thread::spawn(move || {
        let _ = pl.connect(&cfg);
        ctx.request_repaint();
    });
}

/// A titled card, as the groups of the Settings page.
fn card(ui: &mut egui::Ui, title: &str, context: &str, add: impl FnOnce(&mut egui::Ui)) {
    let t = tokens(ui);
    ui.add_space(14.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).font(medium_font(14.0)));
        if !context.is_empty() {
            ui.label(RichText::new(context).size(12.0).color(t.text_faint));
        }
    });
    ui.add_space(4.0);
    egui::Frame::NONE.fill(t.card).stroke(egui::Stroke::new(1.0_f32, t.separator)).corner_radius(10).inner_margin(14).show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        add(ui);
    });
}

impl PluginsView {
    /// Settings › Plugins & MCP: the servers, each with its switch and state, the buttons that
    /// add one, and the selected server's settings, tools and log below. Returns true when
    /// the list changed.
    pub fn manage(&mut self, ui: &mut egui::Ui, list: &mut Vec<ServerConfig>, plugins: &Arc<Plugins>) -> bool {
        let t = tokens(ui);
        let mut changed = false;
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
        let ready = list.iter().filter(|p| matches!(plugins.state_of(&p.name), PluginState::Ready { .. })).count();
        card(ui, "Plugins", &format!("{} servers · {ready} connected", list.len()), |ui| {
            if list.is_empty() {
                chrome::hint(ui, "No plugin yet. Add an MCP server below, or let TellusHive look for the programs it knows.");
            }
            for (i, p) in list.iter_mut().enumerate() {
                let (col, state) = state_line(&t, &plugins.state_of(&p.name), p.enabled);
                ui.horizontal(|ui| {
                    if chrome::switch(ui, &mut p.enabled).on_hover_text("Switched on: projects, teams and bots may use it").changed() {
                        changed = true;
                        if !p.enabled {
                            plugins.disconnect(&p.name);
                        }
                    }
                    let what = if p.url.trim().is_empty() { format!("{state} · program") } else { format!("{state} · HTTP") };
                    if chrome::object_row(ui, col, Some(Icon::Plug), &p.name, &what, self.selected == i).clicked() {
                        self.selected = i;
                    }
                });
            }
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                if chrome::ghost_button(ui, "+ Program (stdio)").on_hover_text("A program TellusHive starts, speaking MCP on stdin and stdout").clicked() {
                    list.push(ServerConfig { name: format!("server{}", list.len() + 1), enabled: true, timeout_s: 600, ..Default::default() });
                    self.selected = list.len() - 1;
                    changed = true;
                }
                if chrome::ghost_button(ui, "+ HTTP endpoint").on_hover_text("An MCP server that is already running, on this computer or elsewhere").clicked() {
                    list.push(ServerConfig { name: format!("http{}", list.len() + 1), url: "http://127.0.0.1:8787/mcp".into(), enabled: true, timeout_s: 600, ..Default::default() });
                    self.selected = list.len() - 1;
                    changed = true;
                }
                if chrome::ghost_button(ui, "+ Paste JSON").on_hover_text("The mcpServers block of Claude Desktop, Claude Code or Cursor").clicked() {
                    self.json_open = !self.json_open;
                    self.json_msg = None;
                }
                if !list.is_empty() && chrome::ghost_button(ui, "Copy all as JSON").on_hover_text("In the mcpServers style, for Claude Desktop, Claude Code or Cursor").clicked() {
                    ui.ctx().copy_text(to_json(&list.iter().collect::<Vec<_>>()));
                    self.json_msg = Some((true, format!("Copied {} plugins as JSON.", list.len())));
                }
                if chrome::ghost_button(ui, "Find installed programs again").on_hover_text("Tellus, Tellus3D, TellusLEM, Pondus, Aedifex, CADCraft and the others it knows").clicked() {
                    for d in th_harness::config::default_plugins() {
                        match list.iter().position(|p| p.name == d.name) {
                            Some(i) => {
                                let p = &mut list[i];
                                if !std::path::Path::new(&p.command).exists() && std::path::Path::new(&d.command).exists() {
                                    p.command = d.command.clone();
                                    p.enabled = true;
                                }
                            }
                            None => list.push(d),
                        }
                    }
                    changed = true;
                }
            });
            if self.json_open {
                ui.add_space(6.0);
                ui.add(
                    egui::TextEdit::multiline(&mut self.json_text)
                        .code_editor()
                        .desired_rows(9)
                        .desired_width(f32::INFINITY)
                        .hint_text("{\n  \"mcpServers\": {\n    \"filesystem\": {\n      \"command\": \"npx\",\n      \"args\": [\"-y\", \"@modelcontextprotocol/server-filesystem\", \"/Users/me/Projects\"]\n    },\n    \"remote\": {\n      \"type\": \"http\",\n      \"url\": \"https://example.com/mcp\",\n      \"headers\": { \"Authorization\": \"Bearer …\" }\n    }\n  }\n}"),
                );
                ui.horizontal(|ui| {
                    if ui.add_enabled(!self.json_text.trim().is_empty(), chrome::primary_button("Add these plugins")).clicked() {
                        match from_json(&self.json_text) {
                            Ok((servers, notes)) => {
                                let (mut added, mut replaced) = (vec![], vec![]);
                                for cfg in servers {
                                    match list.iter().position(|p| p.name == cfg.name) {
                                        Some(i) => {
                                            plugins.disconnect(&cfg.name);
                                            list[i] = cfg.clone();
                                            replaced.push(cfg.name);
                                        }
                                        None => {
                                            list.push(cfg.clone());
                                            added.push(cfg.name);
                                        }
                                    }
                                }
                                let mut msg = String::new();
                                if !added.is_empty() {
                                    msg.push_str(&format!("Added {}. ", added.join(", ")));
                                }
                                if !replaced.is_empty() {
                                    msg.push_str(&format!("Replaced {}. ", replaced.join(", ")));
                                }
                                msg.push_str("Select one and press Connect and list tools to test it.");
                                for n in notes {
                                    msg.push_str(&format!(" Note: {n}."));
                                }
                                if let Some(i) = list.iter().position(|p| Some(&p.name) == added.first().or(replaced.first())) {
                                    self.selected = i;
                                    self.args_for = None;
                                }
                                self.json_msg = Some((true, msg));
                                self.json_text.clear();
                                self.json_open = false;
                                changed = true;
                            }
                            Err(e) => self.json_msg = Some((false, format!("Nothing was added: {e}"))),
                        }
                    }
                    if ui.button("Cancel").clicked() {
                        self.json_open = false;
                        self.json_msg = None;
                    }
                });
                chrome::hint(ui, "Paste the mcpServers block from Claude Desktop (claude_desktop_config.json), Claude Code (.mcp.json) or Cursor, a bare map of servers, or one server. A plugin with the same name is replaced.");
            }
            if let Some((ok, m)) = &self.json_msg {
                ui.label(RichText::new(m).size(12.0).color(if *ok { t.text_dim } else { t.error }));
            }
            chrome::hint(ui, "Agents see a plugin's tools as <plugin>__<tool>. A project chooses which plugins it uses under Project › Plugins, a team equips its agents in the Team layer, and a bot picks its own in its profile.");
        });
        if self.selected < list.len() {
            let name = list[self.selected].name.clone();
            card(ui, &name, "the selected plugin", |ui| {
                changed |= self.detail(ui, list, plugins, true);
            });
            self.tools(ui, list, plugins);
        }
        changed
    }

    /// The project's Plugins step: which plugins of the Settings list the project's agents may
    /// use. Returns true when the project's choice changed.
    pub fn project_left(&mut self, ui: &mut egui::Ui, list: &[ServerConfig], plugins: &Arc<Plugins>, chosen: &mut Option<Vec<String>>) -> bool {
        let t = tokens(ui);
        let mut changed = false;
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
        let mut all = chosen.is_none();
        if chrome::switch_row(ui, "Use every plugin switched on in Settings", &mut all).changed() {
            *chosen = if all { None } else { Some(list.iter().filter(|p| p.enabled).map(|p| p.name.clone()).collect()) };
            changed = true;
        }
        ui.add_space(4.0);
        chrome::section(ui, &format!("Plugins  ·  {}", list.len()));
        for (i, p) in list.iter().enumerate() {
            let (col, state) = state_line(&t, &plugins.state_of(&p.name), p.enabled);
            ui.horizontal(|ui| {
                let mut on = p.enabled && chosen.as_ref().is_none_or(|c| c.contains(&p.name));
                let hint = if !p.enabled { "Switched off in Settings › Plugins & MCP" } else { "Let this project's agents use it" };
                if ui.add_enabled_ui(p.enabled && chosen.is_some(), |ui| chrome::switch(ui, &mut on)).inner.on_hover_text(hint).on_disabled_hover_text(hint).changed() {
                    if let Some(c) = chosen.as_mut() {
                        if on {
                            c.push(p.name.clone());
                        } else {
                            c.retain(|x| *x != p.name);
                        }
                    }
                    changed = true;
                }
                let what = if p.enabled { state } else { "off in Settings".to_string() };
                if chrome::object_row(ui, col, Some(Icon::Plug), &p.name, &what, self.selected == i).clicked() {
                    self.selected = i;
                }
            });
        }
        ui.add_space(8.0);
        if chrome::ghost_button(ui, "Add or manage plugins in Settings").clicked() {
            self.open_settings = true;
        }
        chrome::hint(ui, "The agents of this project can drive only the plugins switched on here. A bot changes its project's plugins from its profile, and the coordinator can too, from the chat.");
        changed
    }

    /// The project's canvas: the selected plugin, read only, with its tools.
    pub fn project_center(&mut self, ui: &mut egui::Ui, list: &mut Vec<ServerConfig>, plugins: &Arc<Plugins>) {
        if self.selected >= list.len() {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| chrome::hint(ui, "Select a plugin, or add one under Settings › Plugins & MCP."));
            return;
        }
        let t = tokens(ui);
        let frame = egui::Frame::NONE.fill(t.card).stroke(egui::Stroke::new(1.0_f32, t.separator)).corner_radius(10).inner_margin(16);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.set_max_width(820.0);
            frame.show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                self.detail(ui, list, plugins, false);
            });
            self.tools(ui, list, plugins);
        });
    }

    /// One server: its state and Connect, and its settings (editable in Settings, shown
    /// read only in a project). Returns true when it changed.
    fn detail(&mut self, ui: &mut egui::Ui, list: &mut Vec<ServerConfig>, plugins: &Arc<Plugins>, editable: bool) -> bool {
        let t = tokens(ui);
        let sel = self.selected;
        if self.args_for != Some(sel) {
            self.args_text = list[sel].args.iter().map(|a| if a.contains(' ') { format!("\"{a}\"") } else { a.clone() }).collect::<Vec<_>>().join(" ");
            self.args_for = Some(sel);
            self.confirm_remove = None;
        }
        let mut changed = false;
        let mut remove = false;
        let state = plugins.state_of(&list[sel].name);
        let p = &mut list[sel];
        let before = p.clone();
        ui.horizontal(|ui| {
            ui.label(RichText::new(&p.name).font(medium_font(17.0)));
            let (col, s) = match &state {
                PluginState::Off => (t.text_faint, "not connected".to_string()),
                PluginState::Connecting => (t.warn, "connecting…".to_string()),
                PluginState::Ready { tools } => (t.ok, format!("connected · {tools} tools")),
                PluginState::Failed(_) => (t.error, "failed".to_string()),
            };
            chrome::status_dot(ui, col, s);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if editable && ui.add(egui::Button::new(RichText::new("Remove").color(t.error)).frame(false)).clicked() {
                    self.confirm_remove = Some(sel);
                }
                if editable && ui.add(egui::Button::new("Copy as JSON").frame(false)).on_hover_text("In the mcpServers style, for Claude Desktop, Claude Code or Cursor").clicked() {
                    ui.ctx().copy_text(to_json(&[&*p]));
                }
                if matches!(state, PluginState::Ready { .. }) {
                    if ui.button("Disconnect").clicked() {
                        plugins.disconnect(&p.name);
                    }
                } else if ui.add_enabled(p.enabled && !matches!(state, PluginState::Connecting), chrome::primary_button("Connect and list tools")).on_disabled_hover_text("Switch it on first").clicked() {
                    connect(ui, p, plugins);
                }
            });
        });
        if self.confirm_remove == Some(sel) {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("Remove {} from the plugins? Projects, teams and bots that use it lose its tools.", p.name)).size(12.0).color(t.error));
                if ui.add(egui::Button::new(RichText::new("Remove").color(t.error))).clicked() {
                    remove = true;
                }
                if ui.button("Keep").clicked() {
                    self.confirm_remove = None;
                }
            });
        }
        if let PluginState::Failed(e) = &state {
            ui.label(RichText::new(e).size(12.0).color(t.error));
        }
        ui.add_space(8.0);
        if editable {
            chrome::field(ui, "Name", &mut p.name).on_hover_text("Agents see the tools as <name>__<tool>");
            chrome::field(ui, "Description", &mut p.description).on_hover_text("Shown to the agents so they know what it is for");
            ui.horizontal(|ui| {
                chrome::row_label(ui, "Program");
                ui.add_sized([ui.available_width() - 80.0, 28.0], egui::TextEdit::singleline(&mut p.command).vertical_align(egui::Align::Center).hint_text("stdio: the program; empty for HTTP"));
                if ui.button("Browse…").clicked() {
                    if let Some(f) = rfd::FileDialog::new().pick_file() {
                        p.command = f.display().to_string();
                    }
                }
            });
            if chrome::field(ui, "Arguments", &mut self.args_text).changed() {
                p.args = split_args(&self.args_text);
            }
            chrome::field(ui, "Working dir", &mut p.cwd);
            chrome::field(ui, "URL (HTTP)", &mut p.url);
            ui.horizontal(|ui| {
                chrome::row_label(ui, "Call timeout");
                ui.add_sized([110.0, 28.0], egui::DragValue::new(&mut p.timeout_s).range(5..=86_400).suffix(" s"));
            });
            ui.horizontal(|ui| {
                chrome::row_label(ui, "Environment");
                let mut env_text: String = {
                    let mut v: Vec<String> = p.env.iter().map(|(k, v)| format!("{k}={v}")).collect();
                    v.sort();
                    v.join("\n")
                };
                if ui.add(egui::TextEdit::multiline(&mut env_text).hint_text("KEY=value per line").desired_rows(2).desired_width(f32::INFINITY)).changed() {
                    p.env = env_text.lines().filter_map(|l| l.split_once('=').map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))).filter(|(k, _)| !k.is_empty()).collect();
                }
            });
            ui.horizontal(|ui| {
                chrome::row_label(ui, "HTTP headers");
                let mut h_text: String = p.headers.iter().map(|(k, v)| format!("{k}: {v}")).collect::<Vec<_>>().join("\n");
                if ui.add(egui::TextEdit::multiline(&mut h_text).hint_text("Name: value per line, e.g. Authorization: Bearer …").desired_rows(2).desired_width(f32::INFINITY)).changed() {
                    p.headers = h_text.lines().filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))).filter(|(k, _)| !k.is_empty()).collect();
                }
            });
        } else {
            let row = |ui: &mut egui::Ui, k: &str, v: &str| {
                if !v.trim().is_empty() {
                    ui.horizontal(|ui| {
                        chrome::row_label(ui, k);
                        ui.label(RichText::new(v).size(12.5).color(t.text_dim));
                    });
                }
            };
            row(ui, "Description", &p.description);
            row(ui, "Program", &p.command);
            row(ui, "Arguments", &self.args_text);
            row(ui, "URL (HTTP)", &p.url);
            if !p.enabled {
                chrome::hint(ui, "Switched off in Settings › Plugins & MCP, so no project can use it.");
            }
            if chrome::ghost_button(ui, "Edit in Settings").clicked() {
                self.open_settings = true;
            }
        }
        if *p != before {
            changed = true;
        }
        if remove {
            plugins.disconnect(&list[sel].name);
            list.remove(sel);
            self.args_for = None;
            self.confirm_remove = None;
            self.selected = 0;
            changed = true;
        }
        changed
    }

    /// The connected server's tools and its log.
    fn tools(&self, ui: &mut egui::Ui, list: &[ServerConfig], plugins: &Arc<Plugins>) {
        let t = tokens(ui);
        let Some(p) = list.get(self.selected) else { return };
        let Some(c) = plugins.get(&p.name) else { return };
        card(ui, "Tools", &format!("{} · server {}", c.tools.len(), c.server_name), |ui| {
            for tool in &c.tools {
                egui::CollapsingHeader::new(RichText::new(&tool.name).font(medium_font(12.5))).id_salt(("tool", &tool.name)).show(ui, |ui| {
                    ui.label(RichText::new(&tool.description).size(12.0).color(t.text_dim));
                    ui.label(RichText::new(serde_json::to_string_pretty(&tool.input_schema).unwrap_or_default()).monospace().size(10.5));
                });
            }
        });
        let err = c.stderr();
        if !err.trim().is_empty() {
            card(ui, "Server log", "", |ui| {
                let tail: String = err.chars().rev().take(4000).collect::<Vec<_>>().into_iter().rev().collect();
                ui.label(RichText::new(tail).monospace().size(10.5));
            });
        }
    }
}

/// Splits a command line on spaces, keeping "quoted parts" together.
pub fn split_args(s: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut quoted = false;
    for c in s.chars() {
        match c {
            '"' => quoted = !quoted,
            ' ' if !quoted => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn json_in_the_claude_style() {
        let (s, notes) = super::from_json(r#"{"mcpServers": {
            "filesystem": {"command": "npx", "args": ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"], "env": {"DEBUG": "1"}},
            "remote": {"type": "http", "url": "https://example.com/mcp", "headers": {"Authorization": "Bearer x"}},
            "old": {"type": "sse", "url": "http://localhost:9000/sse", "disabled": true}
        }}"#).unwrap();
        assert_eq!(s.len(), 3);
        let fs = s.iter().find(|p| p.name == "filesystem").unwrap();
        assert_eq!((fs.command.as_str(), fs.args.len(), fs.env["DEBUG"].as_str(), fs.enabled), ("npx", 3, "1", true));
        let r = s.iter().find(|p| p.name == "remote").unwrap();
        assert!(r.is_http() && r.headers["Authorization"] == "Bearer x");
        assert!(!s.iter().find(|p| p.name == "old").unwrap().enabled);
        assert_eq!(notes.len(), 1);
        // a bare map, VS Code's "servers", and one server with its name
        assert_eq!(super::from_json(r#"{"a": {"command": "x"}}"#).unwrap().0[0].name, "a");
        assert_eq!(super::from_json(r#"{"servers": {"b": {"url": "http://h/mcp"}}}"#).unwrap().0[0].name, "b");
        assert_eq!(super::from_json(r#"{"name": "c", "command": "y"}"#).unwrap().0[0].name, "c");
        // mistakes say what is wrong
        assert!(super::from_json("{not json").is_err());
        assert!(super::from_json(r#"{"mcpServers": {"x": {"args": []}}}"#).unwrap_err().contains("command"));
        assert!(super::from_json(r#"{"mcpServers": {"bad name": {"command": "x"}}}"#).is_err());
        // and back: what is copied out reads in the same
        let back = super::from_json(&super::to_json(&s.iter().collect::<Vec<_>>())).unwrap().0;
        assert_eq!(back.iter().find(|p| p.name == "filesystem").unwrap().args, fs.args);
        assert!(back.iter().find(|p| p.name == "remote").unwrap().is_http());
    }

    #[test]
    fn args() {
        assert_eq!(super::split_args("--mcp \"a b\"  c"), vec!["--mcp", "a b", "c"]);
    }
}
