//! Settings: model providers and keys, the default and vision models, the embedding model of
//! the standards library, Claude Code and Codex, skill folders, projects folder, theme. A
//! section rail on the left, one section's form at a time.

use std::sync::mpsc::{self, Receiver};

use th_harness::config::Settings;
use th_harness::external::find_program;
use th_kb::EmbedderConfig;
use eframe::egui::{self, Color32, RichText};

use crate::chrome::{self, tokens};
use crate::icons::{self, Icon};
use crate::plugins::split_args;
use crate::theme::medium_font;

const SECTIONS: [(Icon, &str, &str); 5] = [
    (Icon::Chat, "Models", "Which language model each agent uses, and the API keys"),
    (Icon::Library, "Standards search", "How the standards library is indexed"),
    (Icon::Edit, "Claude Code & Codex", "Coding agents the coordinator can hand work to"),
    (Icon::Book, "Skills", "The skills agents can be equipped with: add, write or remove them"),
    (Icon::Settings, "General", "Projects folder and appearance"),
];

#[derive(Default)]
pub struct SettingsView {
    section: usize,
    models: std::collections::HashMap<String, Result<Vec<String>, String>>,
    models_rx: Option<Receiver<(String, Result<Vec<String>, String>)>>,
    claude_args: Option<String>,
    codex_args: Option<String>,
    skills: Option<Vec<th_harness::skills::Skill>>,
    skills_changed: bool,
    skill_msg: String,
    new_skill_open: bool,
    new_skill: (String, String, String),
}

/// The thinking levels a provider or an agent can choose.
const THINKING: [(&str, &str); 5] = [("", "model default"), ("off", "off (fastest)"), ("low", "low"), ("medium", "medium"), ("high", "high (most careful)")];

fn group(ui: &mut egui::Ui, title: &str, context: &str, add: impl FnOnce(&mut egui::Ui)) {
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

impl SettingsView {
    pub fn nav(&mut self, ui: &mut egui::Ui) {
        let t = tokens(ui);
        ui.spacing_mut().item_spacing.y = 2.0;
        for (i, (icon, label, _)) in SECTIONS.iter().enumerate() {
            let on = self.section == i;
            let (r, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 38.0), egui::Sense::click());
            let fill = if on { t.accent_soft } else if resp.hovered() { t.hover } else { Color32::TRANSPARENT };
            ui.painter().rect_filled(r, 8.0, fill);
            icons::paint(ui, *icon, egui::Rect::from_center_size(egui::pos2(r.left() + 20.0, r.center().y), egui::Vec2::splat(16.0)), if on { t.accent_text } else { t.text_dim });
            ui.painter().text(egui::pos2(r.left() + 40.0, r.center().y), egui::Align2::LEFT_CENTER, *label, if on { medium_font(13.0) } else { egui::FontId::proportional(13.0) }, if on { t.text } else { t.text_dim });
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                self.section = i;
            }
        }
        ui.add_space(16.0);
        ui.label(RichText::new(format!("Saved in {}", Settings::path().display())).size(11.0).color(t.text_faint));
    }

    /// Returns true when the settings changed.
    pub fn ui(&mut self, ui: &mut egui::Ui, s: &mut Settings) -> bool {
        let t = tokens(ui);
        let before = s.clone();
        if let Some(rx) = &self.models_rx {
            if let Ok((name, r)) = rx.try_recv() {
                self.models.insert(name, r);
                self.models_rx = None;
            }
        }
        let (_, title, about) = SECTIONS[self.section];
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.set_max_width(820.0);
            ui.label(RichText::new(title).font(medium_font(22.0)));
            ui.label(RichText::new(about).color(t.text_dim));
            match self.section {
                0 => self.models_ui(ui, s),
                1 => embed_ui(ui, s),
                2 => self.external_ui(ui, s),
                3 => self.skills_ui(ui, s),
                _ => general_ui(ui, s),
            }
            ui.add_space(24.0);
        });
        *s != before
    }

    fn models_ui(&mut self, ui: &mut egui::Ui, s: &mut Settings) {
        let t = tokens(ui);
        group(ui, "Default model", "for agents that do not choose one", |ui| {
            ui.horizontal(|ui| {
                chrome::row_label(ui, "Provider");
                egui::ComboBox::from_id_salt("def_prov").selected_text(&s.default_provider).width(200.0).show_ui(ui, |ui| {
                    for p in &s.providers {
                        ui.selectable_value(&mut s.default_provider, p.name.clone(), &p.name);
                    }
                });
            });
            chrome::field(ui, "Model", &mut s.default_model);
            chrome::hint(ui, "OpenRouter (default): deepseek/deepseek-v4.1-flash-1017 uses tools and reads images. DeepSeek direct: deepseek-chat or deepseek-reasoner.");
        });
        group(ui, "Vision", "reads pictures for text-only models", |ui| {
            ui.horizontal(|ui| {
                chrome::row_label(ui, "Provider");
                egui::ComboBox::from_id_salt("vis_prov").selected_text(&s.vision_provider).width(200.0).show_ui(ui, |ui| {
                    for p in &s.providers {
                        ui.selectable_value(&mut s.vision_provider, p.name.clone(), &p.name);
                    }
                });
            });
            chrome::field(ui, "Model", &mut s.vision_model);
            chrome::hint(ui, "DeepSeek's API cannot see images, so attached pictures are described by this model first. Without a key here, Claude Code reads them.");
        });
        for i in 0..s.providers.len() {
            let name = s.providers[i].name.clone();
            if s.providers[i].is_cli() {
                group(ui, &format!("Provider: {name}"), "no key", |ui| {
                    let program = if name == "codex" { "Codex" } else { "Claude Code" };
                    chrome::hint(ui, &format!("An agent with this provider runs {program} as its model, on your {program} login instead of an API key. {program} gets that agent's tools (calculations, standards, drawings, its plugins, delegate) through this program's MCP server. Its path, arguments, time limit and the \"use my subscription\" switch are under External agents."));
                    chrome::field(ui, "Model", &mut s.providers[i].default_model);
                    chrome::hint(ui, if name == "codex" { "Empty = Codex's default model." } else { "Empty = Claude Code's default; or sonnet, opus, haiku." });
                });
                continue;
            }
            let has_key = s.providers[i].key().is_some();
            group(ui, &format!("Provider: {name}"), if has_key { "key set" } else { "" }, |ui| {
                let p = &mut s.providers[i];
                chrome::field(ui, "Base URL", &mut p.base_url);
                chrome::password(ui, "API key", &mut p.api_key).on_hover_text(format!("Stored in the settings file (owner-only), or set the environment variable {}.", p.key_env));
                chrome::field(ui, "Key variable", &mut p.key_env);
                chrome::field(ui, "Default model", &mut p.default_model);
                ui.horizontal(|ui| {
                    chrome::row_label(ui, "Thinking");
                    let shown = THINKING.iter().find(|(v, _)| *v == p.thinking).map_or(p.thinking.as_str(), |(_, l)| *l).to_string();
                    egui::ComboBox::from_id_salt(("prov_thinking", i)).selected_text(shown).width(200.0).show_ui(ui, |ui| {
                        for (v, l) in THINKING {
                            ui.selectable_value(&mut p.thinking, v.to_string(), l);
                        }
                    })
                    .response
                    .on_hover_text(if p.base_url.contains("openrouter.ai") {
                        "How long the models think before each reply, for agents whose own Thinking is 'provider default'. Low keeps replies quick; raise it for hard design checks, or set it per agent in step 3 Team."
                    } else {
                        "Sent through OpenRouter's reasoning setting, so it applies only to OpenRouter; this provider's API does not take it."
                    });
                });
                ui.horizontal(|ui| {
                    chrome::row_label(ui, "");
                    ui.checkbox(&mut p.vision, "Models accept images");
                    if ui.add_enabled(self.models_rx.is_none(), egui::Button::new("List models")).clicked() {
                        let (tx, rx) = mpsc::channel();
                        self.models_rx = Some(rx);
                        let pc = p.clone();
                        let ctx = ui.ctx().clone();
                        std::thread::spawn(move || {
                            let r = th_llm::list_models(&pc);
                            let _ = tx.send((pc.name.clone(), r));
                            ctx.request_repaint();
                        });
                    }
                });
                match self.models.get(&name) {
                    Some(Ok(m)) => {
                        ui.horizontal(|ui| {
                            chrome::row_label(ui, "");
                            egui::ComboBox::from_id_salt(("models", i)).selected_text(format!("{} models: pick the default", m.len())).height(320.0).width(300.0).show_ui(ui, |ui| {
                                for id in m {
                                    if ui.selectable_label(p.default_model == *id, id).clicked() {
                                        p.default_model = id.clone();
                                    }
                                }
                            });
                        });
                    }
                    Some(Err(e)) => {
                        ui.label(RichText::new(e).size(11.5).color(t.error));
                    }
                    None => {}
                }
            });
        }
        ui.add_space(6.0);
        if chrome::ghost_button(ui, "+ Provider (any OpenAI-compatible endpoint)").clicked() {
            let mut p = th_llm::Provider::openai();
            p.name = format!("custom{}", s.providers.len());
            p.key_env.clear();
            s.providers.push(p);
        }
    }

    fn external_ui(&mut self, ui: &mut egui::Ui, s: &mut Settings) {
        for which in 0..2 {
            let found = find_program(if which == 0 { "claude" } else { "codex" }, if which == 0 { &s.claude_code.path } else { &s.codex.path });
            let cfg = if which == 0 { &mut s.claude_code } else { &mut s.codex };
            let args_slot = if which == 0 { &mut self.claude_args } else { &mut self.codex_args };
            let ctxt = match &found {
                Some(p) => format!("found: {}", p.display()),
                None => "not found".into(),
            };
            group(ui, if which == 0 { "Claude Code" } else { "Codex" }, &ctxt, |ui| {
                chrome::switch_row(ui, "Enabled (as a worker tool and as an agent's model)", &mut cfg.enabled);
                chrome::switch_row(ui, if which == 0 { "Use my Claude subscription (ignore ANTHROPIC_API_KEY)" } else { "Use my ChatGPT subscription (ignore OPENAI_API_KEY)" }, &mut cfg.subscription);
                chrome::field(ui, "Program", &mut cfg.path).on_hover_text("Empty = search PATH and the usual install folders");
                let txt = args_slot.get_or_insert_with(|| cfg.args.join(" "));
                if chrome::field(ui, "Arguments", txt).changed() {
                    cfg.args = split_args(txt);
                }
                ui.horizontal(|ui| {
                    chrome::row_label(ui, "Time limit");
                    ui.add_sized([100.0, 28.0], egui::DragValue::new(&mut cfg.timeout_min).range(1..=600).suffix(" min"));
                });
                chrome::hint(ui, if which == 0 {
                    "Runs `claude -p` in the project's work folder with this program's tools as an MCP server. The default permission mode acceptEdits lets it edit files there but not run shell commands, unless you add e.g. --allowedTools Bash."
                } else {
                    "Runs `codex exec` in the project's work folder (workspace-write sandbox) with this program's tools as an MCP server."
                });
            });
        }
    }
}

fn embed_ui(ui: &mut egui::Ui, s: &mut Settings) {
    group(ui, "Embedding", "how the standards library is searched", |ui| {
        let simple = matches!(s.embedder, EmbedderConfig::Hashed { .. });
        let openrouter = s.embedder.is_openrouter();
        if ui.radio(simple, "Simple: built-in, offline").clicked() && !simple {
            s.embedder = EmbedderConfig::Hashed { dim: 1024 };
        }
        if ui.radio(openrouter, "Embedding model through OpenRouter").clicked() && !openrouter {
            s.embedder = EmbedderConfig::openrouter(th_kb::OPENROUTER_EMBED_MODEL);
        }
        ui.add_space(6.0);
        match &mut s.embedder {
            EmbedderConfig::Hashed { .. } => {
                chrome::hint(ui, "Matches words, word pieces and clause numbers (feature-hashed n-grams with IDF, fused with BM25 keyword search). Free and offline, and good when the query names the clause or the exact term, but it does not understand meaning: 'uplift' does not find 'buoyancy'.");
            }
            EmbedderConfig::Api { base_url, model, .. } if base_url.contains("openrouter.ai") => {
                chrome::field(ui, "Model", model).on_hover_text("An embedding model on OpenRouter, e.g. baai/bge-m3, qwen/qwen3-embedding-8b, openai/text-embedding-3-large");
                let key = s.providers.iter().find(|p| p.name == "openrouter").and_then(|p| p.key()).is_some();
                if !key {
                    ui.label(egui::RichText::new("No OpenRouter key yet: add it under Providers.").size(12.0).color(chrome::tokens(ui).error));
                }
                chrome::hint(ui, "Semantic search: finds a clause by meaning as well as by its words, in Thai and English. baai/bge-m3 is multilingual and costs about USD 0.01 per million tokens, a fraction of a cent for a whole standard. Uses the OpenRouter key from Providers, and the standard's text is sent to OpenRouter to be embedded.");
            }
            EmbedderConfig::Api { base_url, model, .. } => {
                chrome::hint(ui, &format!("Currently a custom embedding API ({model} at {base_url}). Choose one of the two options above to replace it."));
            }
        }
        chrome::hint(ui, "Standards imported from now on use this setting. To change those already in the library, use Re-index on the Standards page.");
    });
}

fn skill_folders_ui(ui: &mut egui::Ui, s: &mut Settings) {
    group(ui, "Skill folders", "each holds skill folders with SKILL.md", |ui| {
        let mut remove = None;
        for (i, d) in s.skill_dirs.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.add_sized([ui.available_width() - 34.0, 28.0], egui::TextEdit::singleline(d).vertical_align(egui::Align::Center));
                if chrome::remove_button(ui, "Remove").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            s.skill_dirs.remove(i);
        }
        if chrome::ghost_button(ui, "+ Folder").clicked() {
            if let Some(d) = rfd::FileDialog::new().pick_folder() {
                s.skill_dirs.push(d.display().to_string());
            }
        }
        chrome::hint(ui, "The Claude skills format: a folder per skill with SKILL.md (front matter name and description) and its reference files. ~/.claude/skills is read by default.");
    });
}

impl SettingsView {
    /// The skills found, adding one (a folder, its SKILL.md, or a .skill / .zip file), writing a
    /// new one, and removing those in TellusHive's own folder.
    fn skills_ui(&mut self, ui: &mut egui::Ui, s: &mut Settings) {
        let t = tokens(ui);
        let own = th_harness::skills::own_dir();
        if self.skills.is_none() {
            self.skills = Some(th_harness::skills::scan(&s.skill_dirs));
        }
        let mut changed = false;
        group(ui, "Skills", &format!("{} found", self.skills.as_ref().map_or(0, Vec::len)), |ui| {
            let list = self.skills.clone().unwrap_or_default();
            if list.is_empty() {
                chrome::hint(ui, "No skills yet. Add one below, or write a new one.");
            }
            for k in &list {
                ui.horizontal(|ui| {
                    let room = (ui.available_width() - 40.0).max(80.0);
                    let mine = k.dir.parent().zip(own.canonicalize().ok()).is_some_and(|(p, o)| p.canonicalize().ok().as_deref() == Some(o.as_path()));
                    ui.vertical(|ui| {
                        ui.set_max_width(room);
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.label(RichText::new(chrome::fit_text(ui, &k.name, &medium_font(13.0), room)).font(medium_font(13.0)));
                        let where_ = if mine { "TellusHive".to_string() } else { k.dir.parent().map(|p| p.display().to_string()).unwrap_or_default() };
                        let line = format!("{} · {where_}", k.description);
                        ui.label(RichText::new(chrome::fit_text(ui, &line, &egui::FontId::proportional(11.5), room)).size(11.5).color(t.text_dim)).on_hover_text(format!("{}\n{}", k.description, k.dir.display()));
                    });
                    if mine {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if chrome::remove_button(ui, "Remove this skill").clicked() {
                                self.skill_msg = match th_harness::skills::remove(k, &own) {
                                    Ok(()) => format!("Removed '{}'.", k.name),
                                    Err(e) => e,
                                };
                                changed = true;
                            }
                        });
                    }
                });
                ui.add_space(4.0);
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let mut picked: Option<std::path::PathBuf> = None;
                if chrome::ghost_button(ui, "+ Add skill folder…").on_hover_text("A folder with SKILL.md and its reference files").clicked() {
                    picked = rfd::FileDialog::new().pick_folder();
                }
                if chrome::ghost_button(ui, "+ Add .skill or .zip file…").clicked() {
                    picked = rfd::FileDialog::new().add_filter("Skill", &["skill", "zip", "md"]).pick_file();
                }
                if chrome::ghost_button(ui, "+ New skill").clicked() {
                    self.new_skill_open = !self.new_skill_open;
                }
                if let Some(p) = picked {
                    let mut r = th_harness::skills::install(&p, &own, false);
                    if matches!(&r, Err(e) if e.contains("already installed")) {
                        r = th_harness::skills::install(&p, &own, true).map(|k| th_harness::skills::Skill { name: format!("{} (replaced)", k.name), ..k });
                    }
                    self.skill_msg = match r {
                        Ok(k) => format!("Added '{}'. Equip it to agents in step 3 Team, or leave their skills on 'all'.", k.name),
                        Err(e) => format!("Not added: {e}"),
                    };
                    changed = true;
                }
            });
            if self.new_skill_open {
                ui.add_space(6.0);
                chrome::field(ui, "Name", &mut self.new_skill.0);
                chrome::field(ui, "Description", &mut self.new_skill.1).on_hover_text("One sentence saying when to use it: agents choose skills by this line");
                ui.add(egui::TextEdit::multiline(&mut self.new_skill.2).hint_text("Instructions: the method, the clauses to follow, the checks to make, the output to give").desired_rows(6).desired_width(f32::INFINITY));
                if ui.add(chrome::primary_button("Create skill")).clicked() {
                    self.skill_msg = match th_harness::skills::create(&own, &self.new_skill.0, &self.new_skill.1, &self.new_skill.2) {
                        Ok(k) => {
                            self.new_skill = Default::default();
                            self.new_skill_open = false;
                            format!("Created '{}' in {}.", k.name, k.dir.display())
                        }
                        Err(e) => e,
                    };
                    changed = true;
                }
            }
            if !self.skill_msg.is_empty() {
                ui.label(RichText::new(&self.skill_msg).size(12.0).color(t.text_dim));
            }
            chrome::hint(ui, "Skills are added to TellusHive's own skills folder; those in other folders, such as ~/.claude/skills, are used but not changed here.");
        });
        if changed {
            if !s.skill_dirs.iter().any(|d| std::path::Path::new(d) == own.as_path()) {
                s.skill_dirs.insert(0, own.display().to_string());
            }
            self.skills = None;
            self.skills_changed = true;
        }
        skill_folders_ui(ui, s);
    }

    /// True once after skills were added or removed, so the app rescans them.
    pub fn take_skills_changed(&mut self) -> bool {
        std::mem::take(&mut self.skills_changed)
    }
}

fn general_ui(ui: &mut egui::Ui, s: &mut Settings) {
    group(ui, "Projects", "", |ui| {
        chrome::field(ui, "Folder", &mut s.projects_dir);
    });
    group(ui, "Models", "", |ui| {
        chrome::switch_row(ui, "Open the program's window when the agents work in a model", &mut s.open_models);
        chrome::hint(ui, "Aedifex follows the agents live. Pondus, Tellus and Tellus3D open when the task ends; open them again from Models to see a newer state.");
    });
    group(ui, "Python", "", |ui| {
        chrome::field(ui, "Python interpreter", &mut s.python);
        let found = th_harness::tools::find_python(s);
        chrome::hint(ui, &match found {
            Some(p) if s.python.trim().is_empty() => format!("Empty uses the first python3 found: {}. The run_python tool runs scripts in the project's work/ folder; switch it on per agent in Team.", p.display()),
            Some(p) => format!("Using {}.", p.display()),
            None => "No Python 3 found. Install it or give the full path to the interpreter.".to_string(),
        });
    });
    group(ui, "Appearance", "", |ui| {
        chrome::switch_row(ui, "Dark theme", &mut s.dark);
    });
}
