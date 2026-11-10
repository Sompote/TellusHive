//! Step 4, Plugins: MCP servers the agents can be equipped with. Tellus, Tellus3D and CADCraft
//! are added when found; any stdio program or HTTP endpoint can be added, connected and its
//! tools inspected. The list is in the step panel, the selected server on the canvas.

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
}

impl PluginsView {
    /// The step panel. Returns true when the plugin list changed.
    pub fn left(&mut self, ui: &mut egui::Ui, list: &mut Vec<ServerConfig>, plugins: &Arc<Plugins>) -> bool {
        let t = tokens(ui);
        let mut changed = false;
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
        chrome::section(ui, &format!("Servers  ·  {}", list.len()));
        for (i, p) in list.iter_mut().enumerate() {
            let (col, state) = match plugins.state_of(&p.name) {
                PluginState::Off => (t.text_faint, if p.enabled { "available" } else { "off" }.to_string()),
                PluginState::Connecting => (t.warn, "connecting…".to_string()),
                PluginState::Ready { tools } => (t.ok, format!("{tools} tools")),
                PluginState::Failed(_) => (t.error, "failed".to_string()),
            };
            ui.horizontal(|ui| {
                if chrome::switch(ui, &mut p.enabled).on_hover_text("Available to agents").changed() {
                    changed = true;
                    if !p.enabled {
                        plugins.disconnect(&p.name);
                    }
                }
                let r = chrome::object_row(ui, col, Some(Icon::Plug), &p.name, &state, self.selected == i);
                if r.clicked() {
                    self.selected = i;
                }
            });
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if chrome::ghost_button(ui, "+ Program (stdio)").clicked() {
                list.push(ServerConfig { name: format!("server{}", list.len() + 1), enabled: true, timeout_s: 600, ..Default::default() });
                self.selected = list.len() - 1;
                changed = true;
            }
            if chrome::ghost_button(ui, "+ HTTP endpoint").clicked() {
                list.push(ServerConfig { name: format!("http{}", list.len() + 1), url: "http://127.0.0.1:8787/mcp".into(), enabled: true, timeout_s: 600, ..Default::default() });
                self.selected = list.len() - 1;
                changed = true;
            }
        });
        if chrome::ghost_button(ui, "Find installed programs again").clicked() {
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
        ui.add_space(8.0);
        chrome::hint(ui, "Equip agents with plugins in step 3. A plugin's tools reach the agent as <plugin>__<tool>.");
        changed
    }

    /// The canvas: the selected server's settings and tools. Returns true when it changed.
    pub fn center(&mut self, ui: &mut egui::Ui, list: &mut Vec<ServerConfig>, plugins: &Arc<Plugins>) -> bool {
        let t = tokens(ui);
        let mut changed = false;
        if self.selected >= list.len() {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| chrome::hint(ui, "Add or select a plugin."));
            return false;
        }
        let sel = self.selected;
        if self.args_for != Some(sel) {
            self.args_text = list[sel].args.iter().map(|a| if a.contains(' ') { format!("\"{a}\"") } else { a.clone() }).collect::<Vec<_>>().join(" ");
            self.args_for = Some(sel);
        }
        let mut remove = false;
        let card_frame = egui::Frame::NONE.fill(t.card).stroke(egui::Stroke::new(1.0_f32, t.separator)).corner_radius(10).inner_margin(16);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.set_max_width(820.0);
            let state = plugins.state_of(&list[sel].name);
            card_frame.show(ui, |ui| {
                ui.set_min_width(ui.available_width());
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
                        if ui.add(egui::Button::new(RichText::new("Remove").color(t.error)).frame(false)).clicked() {
                            remove = true;
                        }
                        if matches!(state, PluginState::Ready { .. }) {
                            if ui.button("Disconnect").clicked() {
                                plugins.disconnect(&p.name);
                            }
                        } else if ui.add_enabled(!matches!(state, PluginState::Connecting), chrome::primary_button("Connect and list tools")).clicked() {
                            let (cfg, pl) = (p.clone(), plugins.clone());
                            let ctx = ui.ctx().clone();
                            std::thread::spawn(move || {
                                let _ = pl.connect(&cfg);
                                ctx.request_repaint();
                            });
                        }
                    });
                });
                if let PluginState::Failed(e) = &state {
                    ui.label(RichText::new(e).size(12.0).color(t.error));
                }
                ui.add_space(8.0);
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
                    if ui.add(egui::TextEdit::multiline(&mut h_text).hint_text("Name: value per line").desired_rows(2).desired_width(f32::INFINITY)).changed() {
                        p.headers = h_text.lines().filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))).filter(|(k, _)| !k.is_empty()).collect();
                    }
                });
                if *p != before {
                    changed = true;
                }
            });
            if remove {
                return;
            }
            if let Some(c) = plugins.get(&list[sel].name) {
                ui.add_space(12.0);
                card_frame.show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Tools").font(medium_font(15.0)));
                        ui.label(RichText::new(format!("{} · server {}", c.tools.len(), c.server_name)).color(t.text_faint));
                    });
                    ui.add_space(4.0);
                    for tool in &c.tools {
                        egui::CollapsingHeader::new(RichText::new(&tool.name).font(medium_font(12.5))).id_salt(("tool", &tool.name)).show(ui, |ui| {
                            ui.label(RichText::new(&tool.description).size(12.0).color(t.text_dim));
                            ui.label(RichText::new(serde_json::to_string_pretty(&tool.input_schema).unwrap_or_default()).monospace().size(10.5));
                        });
                    }
                });
                let err = c.stderr();
                if !err.trim().is_empty() {
                    ui.add_space(12.0);
                    card_frame.show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.label(RichText::new("Server log").font(medium_font(15.0)));
                        let tail: String = err.chars().rev().take(4000).collect::<Vec<_>>().into_iter().rev().collect();
                        ui.label(RichText::new(tail).monospace().size(10.5));
                    });
                }
            }
        });
        if remove {
            plugins.disconnect(&list[sel].name);
            list.remove(sel);
            self.args_for = None;
            self.selected = 0;
            changed = true;
        }
        changed
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
    fn args() {
        assert_eq!(super::split_args("--mcp \"a b\"  c"), vec!["--mcp", "a b", "c"]);
    }
}
