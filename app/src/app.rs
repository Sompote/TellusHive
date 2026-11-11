//! The window, laid out as Tellus3D: a top bar (the project and its menu on the left, the
//! workflow as a centred pill of numbered steps with the primary action, Assistant and theme
//! on the right), a step panel on the left with the step's title and "Next", the work area
//! on the dotted canvas, the assistant docked on the right, and the status bar.
//!
//! Workflow: 1 Project, 2 Standards, 3 Team, 4 Plugins, then the Design office (the chat with
//! the agents, the primary action) and Output (the deliverables).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use th_harness::config::{data_dir, read_json, write_json, Settings, TeamSpec};
use th_harness::{Artifact, ArtifactKind, Engine, PluginState, Plugins, Project, ProjectInfo, Template};
use eframe::egui::{self, Color32, RichText};
use serde::{Deserialize, Serialize};

use crate::chat::ChatView;
use crate::chrome::{self, hex_color};
use crate::icons::{self, Icon};
use crate::plugins::PluginsView;
use crate::settings_view::SettingsView;
use crate::standards::StandardsView;
use crate::team::TeamView;
use crate::theme::{self, medium_font, Tokens};
use crate::viewer::Viewer;

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct AppState {
    last_project: String,
}

fn state_path() -> PathBuf {
    data_dir().join("app_state.json")
}

pub fn default_project(settings: &Settings) -> Result<Project, String> {
    let st: AppState = read_json(&state_path()).unwrap_or_default();
    if !st.last_project.is_empty() {
        if let Ok(p) = Project::open(std::path::Path::new(&st.last_project)) {
            return Ok(p);
        }
    }
    let dir = PathBuf::from(&settings.projects_dir).join("First project");
    if dir.join("project.json").is_file() {
        return Project::open(&dir);
    }
    Project::create(&dir, ProjectInfo { name: "First project".into(), ..Default::default() })
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ws {
    Home,
    Project,
    Standards,
    Team,
    Plugins,
    Design,
    Output,
    Settings,
}

impl Ws {
    const STEPS: [Ws; 4] = [Ws::Project, Ws::Standards, Ws::Team, Ws::Plugins];
    fn title(self) -> &'static str {
        match self {
            Ws::Home => "Home",
            Ws::Project => "Project",
            Ws::Standards => "Standards",
            Ws::Team => "Team",
            Ws::Plugins => "Plugins",
            Ws::Design => "Design office",
            Ws::Output => "Output",
            Ws::Settings => "Settings",
        }
    }
    fn number(self) -> Option<usize> {
        Ws::STEPS.iter().position(|s| *s == self).map(|i| i + 1)
    }
    fn about(self) -> &'static str {
        match self {
            Ws::Project => "What the project is and the notes every agent reads.",
            Ws::Standards => "The design standards the agents follow and cite, from your library.",
            Ws::Team => "The agents of the design office: their roles, models, skills and plugins.",
            Ws::Plugins => "Programs the agents drive through MCP: the Tellus family, Pondus, Aedifex, Granum, ContinuaX, CADCraft.",
            _ => "",
        }
    }
    fn help(self) -> &'static str {
        match self {
            Ws::Project => "Each project is a folder with its attachments, calculations, drawings and reports. The description and notes are added to every agent's instructions: put the client, the site, the units and any office rule here.",
            Ws::Standards => "Import your own copies of the standards (PDF, text or Markdown) into the library once; then switch on the ones this project follows. Agents search only the chosen standards and cite clause numbers from what they find. Office rules can be added as notes.",
            Ws::Team => "The coordinator talks with you and delegates to the others along the arrows. Each agent has its own instructions, model, built-in tools, skills (folders with SKILL.md) and plugins. Changes are saved automatically.",
            Ws::Plugins => "A plugin is an MCP server: a program started with arguments (stdio) or an HTTP endpoint. Equip agents with plugins in the Team step. Tellus, Tellus3D and CADCraft are found automatically.",
            _ => "",
        }
    }
    fn next(self) -> Option<Ws> {
        match self {
            Ws::Project => Some(Ws::Standards),
            Ws::Standards => Some(Ws::Team),
            Ws::Team => Some(Ws::Plugins),
            Ws::Plugins => Some(Ws::Design),
            _ => None,
        }
    }
}

pub struct App {
    engine: Arc<Engine>,
    settings: Settings,
    team: TeamSpec,
    project: Project,
    ws: Ws,
    chat: ChatView,
    chat_open: bool,
    viewer: Viewer,
    team_view: TeamView,
    std_view: StandardsView,
    plug_view: PluginsView,
    set_view: SettingsView,
    artifacts: Vec<Artifact>,
    artifacts_checked: Instant,
    settings_dirty: Option<Instant>,
    team_dirty: Option<Instant>,
    project_dirty: Option<Instant>,
    new_project: String,
    status: String,
    theme_dark: bool,
    home_ask: String,
    home_filter: String,
    /// The "Save as template" form in the Project step.
    tpl_name: String,
    tpl_desc: String,
    tpl_prompt: String,
    cad_rx: Option<std::sync::mpsc::Receiver<String>>,
    /// The analysis programs' windows this app opened on the project's models, and when.
    windows: std::collections::HashMap<String, (std::process::Child, std::time::SystemTime)>,
    /// The programs whose model changed during the running task.
    run_models: Vec<String>,
    /// TELLUSHIVE_SHOT=<dir>: saves a PNG of every workspace, then quits (UI checks).
    shot: Option<(PathBuf, usize, u32)>,
}

const SHOT_ORDER: [Ws; 8] = [Ws::Home, Ws::Project, Ws::Standards, Ws::Team, Ws::Plugins, Ws::Design, Ws::Output, Ws::Settings];

impl App {
    pub fn new(cc: &eframe::CreationContext) -> Result<App, String> {
        let settings = Settings::load();
        let library = Arc::new(th_kb::Library::open(&th_kb::Library::default_root())?);
        let had_project = read_json::<AppState>(&state_path()).map(|s| !s.last_project.is_empty()).unwrap_or(false);
        let project = default_project(&settings)?;
        let team = TeamSpec::load(&project.info.team);
        let engine = Arc::new(Engine::new(settings.clone(), team.clone(), project.clone(), library, Arc::new(Plugins::default())));
        theme::install_fonts(&cc.egui_ctx);
        theme::apply(&cc.egui_ctx, if settings.dark { Tokens::DARK } else { Tokens::LIGHT });
        icons::install(&cc.egui_ctx);
        let mut chat = ChatView::new(ChatView::load(&project.dir));
        if std::env::var("TELLUSHIVE_SHOT_BUSY").is_ok() {
            chat.demo_busy(&team.agents.iter().map(|a| a.name.clone()).collect::<Vec<_>>());
        }
        let artifacts = project.artifacts();
        let dark = settings.dark;
        let tpl_name = project.info.name.clone();
        let _ = write_json(&state_path(), &AppState { last_project: project.dir.display().to_string() }, false);
        let shot = std::env::var("TELLUSHIVE_SHOT").ok().map(|d| (PathBuf::from(d), 0, 0));
        Ok(App {
            engine,
            settings,
            team,
            project,
            ws: if had_project { Ws::Design } else { Ws::Home },
            chat,
            chat_open: false,
            viewer: Viewer::default(),
            team_view: TeamView::default(),
            std_view: StandardsView::default(),
            plug_view: PluginsView::default(),
            set_view: SettingsView::default(),
            artifacts,
            artifacts_checked: Instant::now(),
            settings_dirty: None,
            team_dirty: None,
            project_dirty: None,
            new_project: String::new(),
            status: String::new(),
            theme_dark: dark,
            home_ask: String::new(),
            home_filter: String::new(),
            tpl_name,
            tpl_desc: String::new(),
            tpl_prompt: String::new(),
            cad_rx: None,
            windows: Default::default(),
            run_models: vec![],
            shot,
        })
    }

    fn switch_project(&mut self, p: Project) {
        if self.chat.busy() {
            self.status = "Stop the running task before switching projects.".into();
            return;
        }
        self.chat.save(&self.project.dir);
        // the programs' windows stay open, but they show the other project's models
        self.windows.clear();
        self.run_models.clear();
        self.project = p;
        self.team = TeamSpec::load(&self.project.info.team);
        if let Ok(mut e) = self.engine.project.write() {
            *e = self.project.clone();
        }
        if let Ok(mut e) = self.engine.team.write() {
            *e = self.team.clone();
        }
        self.chat = ChatView::new(ChatView::load(&self.project.dir));
        self.tpl_name = self.project.info.name.clone();
        self.viewer = Viewer::default();
        self.artifacts = self.project.artifacts();
        let _ = write_json(&state_path(), &AppState { last_project: self.project.dir.display().to_string() }, false);
        self.status = format!("Opened {}", self.project.info.name);
    }

    /// A folder in the projects folder that does not exist yet.
    fn unique_dir(&self, name: &str) -> PathBuf {
        let base = th_harness::project::slug(name);
        let mut dir = PathBuf::from(&self.settings.projects_dir).join(&base);
        let mut n = 2;
        while dir.exists() {
            dir = PathBuf::from(&self.settings.projects_dir).join(format!("{base}_{n}"));
            n += 1;
        }
        dir
    }

    /// The current set-up (team, standards, notes, models, plugins, skills) as a template.
    fn capture_template(&self) -> Template {
        let lib = self.engine.library.list();
        let codes = |id: &str| lib.iter().find(|m| m.id == id).map(|m| m.code.clone());
        Template::capture(&self.tpl_name, &self.tpl_desc, &self.settings, &self.team, &self.project.info, &codes, &self.tpl_prompt)
    }

    /// A new project made from a template, opened (with the starter prompt in the chat box).
    /// Picks a template file and starts a new project from it (the file is not added to the
    /// template library).
    fn open_template_dialog(&mut self) {
        let mut dialog = rfd::FileDialog::new().add_filter("TellusHive template", &["json"]);
        if Template::dir().is_dir() {
            dialog = dialog.set_directory(Template::dir());
        }
        let Some(f) = dialog.pick_file() else { return };
        match Template::read(&f).and_then(|tpl| self.new_project_from_template(&tpl).map(|_| tpl.name)) {
            Ok(name) => {
                self.status = format!("New project from the template '{name}'");
                self.ws = Ws::Design;
            }
            Err(e) => self.status = e,
        }
    }

    fn new_project_from_template(&mut self, tpl: &Template) -> Result<(), String> {
        if self.chat.busy() {
            return Err("Stop the running task before starting a new project.".into());
        }
        let lib = self.engine.library.list();
        let mut settings = self.settings.clone();
        let applied = tpl.apply(&tpl.name, &mut settings, &|id| lib.iter().any(|m| m.id == id));
        if applied.settings_changed {
            self.settings = settings;
            self.apply_settings();
        }
        let p = Project::create(&self.unique_dir(&tpl.name), applied.info)?;
        self.switch_project(p);
        if !tpl.starter_prompt.is_empty() {
            self.chat.set_input(&tpl.starter_prompt);
        }
        self.status = if applied.notes.is_empty() { format!("Started from template '{}'", tpl.name) } else { format!("Started from template '{}': {}", tpl.name, applied.notes.join("; ")) };
        Ok(())
    }

    /// A new project in the projects folder (a unique folder name), opened.
    fn new_project_named(&mut self, name: &str) -> Result<(), String> {
        let dir = self.unique_dir(name);
        let info = ProjectInfo { name: name.to_string(), standards: self.project.info.standards.clone(), team: self.team.name.clone(), ..Default::default() };
        let p = Project::create(&dir, info)?;
        self.switch_project(p);
        Ok(())
    }

    fn flush(&mut self, force: bool) {
        let due = |t: &Option<Instant>| t.is_some_and(|t| force || t.elapsed() > Duration::from_millis(800));
        if due(&self.settings_dirty) {
            if let Err(e) = self.settings.save() {
                self.status = e;
            }
            self.settings_dirty = None;
        }
        if due(&self.team_dirty) {
            let _ = self.team.save();
            self.team_dirty = None;
        }
        if due(&self.project_dirty) {
            let _ = self.project.save();
            self.project_dirty = None;
        }
    }

    fn apply_settings(&mut self) {
        let skills_changed = self.engine.settings.read().map(|s| s.skill_dirs != self.settings.skill_dirs).unwrap_or(true);
        if let Ok(mut s) = self.engine.settings.write() {
            *s = self.settings.clone();
        }
        if skills_changed {
            self.engine.rescan_skills();
        }
        self.settings_dirty = Some(Instant::now());
    }

    fn apply_project(&mut self) {
        if let Ok(mut p) = self.engine.project.write() {
            p.info = self.project.info.clone();
        }
        self.project_dirty = Some(Instant::now());
    }

    fn agent_colours(&self) -> Vec<(String, Color32)> {
        self.team.agents.iter().map(|a| (a.name.clone(), hex_color(&a.color, Color32::from_rgb(47, 116, 208)))).collect()
    }

    fn refresh_artifacts(&mut self) {
        if self.artifacts_checked.elapsed() > Duration::from_secs(3) {
            self.artifacts = self.project.artifacts();
            self.artifacts_checked = Instant::now();
        }
    }

    // ---------------------------------------------------------------- top bar

    fn top_bar(&mut self, ctx: &egui::Context) {
        let t = Tokens::get(ctx);
        let bar = egui::Frame::NONE.fill(t.chrome).inner_margin(egui::Margin::symmetric(12, 8)).stroke(egui::Stroke::new(1.0_f32, t.separator));
        egui::TopBottomPanel::top("top").frame(bar).show(ctx, |ui| {
            ui.horizontal(|ui| {
                // the app's mark and the project; it opens the project menu
                let mut title = egui::text::LayoutJob::default();
                title.append(&self.project.info.name, 0.0, egui::TextFormat::simple(medium_font(13.5), t.text));
                title.append("\nTellusHive", 0.0, egui::TextFormat::simple(egui::FontId::proportional(11.0), t.text_faint));
                let logo = egui::Image::new(&chrome::app_icon(ctx)).fit_to_exact_size(egui::vec2(28.0, 28.0));
                let button = egui::Button::image_and_text(logo, title).frame(false).min_size(egui::vec2(0.0, 36.0));
                let menu = egui::menu::menu_custom_button(ui, button, |ui| {
                    ui.set_min_width(260.0);
                    if ui.button("Home").on_hover_text("Start a project, examples and recent projects").clicked() {
                        self.ws = Ws::Home;
                        ui.close_menu();
                    }
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.new_project).hint_text("new project name").desired_width(170.0));
                        if ui.button("Create").clicked() && !self.new_project.trim().is_empty() {
                            let name = std::mem::take(&mut self.new_project);
                            if let Err(e) = self.new_project_named(name.trim()) {
                                self.status = e;
                            }
                            self.ws = Ws::Project;
                            ui.close_menu();
                        }
                    });
                    if ui.button("Save as template…").on_hover_text("Save the team, standards, notes, models and plugins so new projects can start from them").clicked() {
                        self.ws = Ws::Project;
                        ui.close_menu();
                    }
                    if ui.button("Open project folder…").clicked() {
                        ui.close_menu();
                        if let Some(d) = rfd::FileDialog::new().set_directory(&self.settings.projects_dir).pick_folder() {
                            match Project::open_or_create(&d) {
                                Ok(p) => self.switch_project(p),
                                Err(e) => self.status = e,
                            }
                        }
                    }
                    if ui.button("Open template…").on_hover_text("Start a new project from a template file (.json): its team, standards, plugins and notes").clicked() {
                        ui.close_menu();
                        self.open_template_dialog();
                    }
                    if ui.button("Show project in Finder").clicked() {
                        chrome::open_path(&self.project.dir);
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.add_enabled(!self.chat.busy(), egui::Button::new("Clear the conversation")).clicked() {
                        self.chat.clear();
                        self.engine.new_conversation();
                        self.chat.save(&self.project.dir);
                        ui.close_menu();
                    }
                    if ui.button("Settings").clicked() {
                        self.ws = Ws::Settings;
                        ui.close_menu();
                    }
                });
                let r = menu.response.on_hover_text("Project: new, open, settings");
                let c = egui::pos2(r.rect.right() + 10.0, r.rect.center().y);
                ui.painter().add(egui::Shape::line(vec![c + egui::vec2(-4.0, -2.0), c + egui::vec2(0.0, 2.0), c + egui::vec2(4.0, -2.0)], egui::Stroke::new(1.5_f32, t.text_dim)));
                ui.add_space(20.0);

                // the workflow, centred
                let font = egui::FontId::proportional(13.0);
                let labels: f32 = Ws::STEPS.iter().map(|s| s.title()).chain(["Output", "Design office"]).map(|l| ui.fonts(|f| f.layout_no_wrap(l.to_string(), font.clone(), t.text).size().x)).sum();
                let busy_w = if self.chat.busy() { 190.0 } else { 0.0 };
                let strip_w = labels + 6.0 * 50.0 + 2.0 * 13.0 + 24.0 + busy_w;
                let right_w = 240.0;
                ui.add_space(((ui.available_width() - right_w - strip_w) / 2.0).max(8.0));
                chrome::segmented_frame(ctx).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    for step in Ws::STEPS {
                        let state = if self.ws == step { chrome::StepState::Current } else if step == Ws::Standards && self.project.info.standards.is_empty() { chrome::StepState::Attention } else { chrome::StepState::Todo };
                        let r = chrome::step_button(ui, step.number(), None, step.title(), state);
                        let r = if state == chrome::StepState::Attention { r.on_hover_text("No standard chosen for this project yet") } else { r };
                        if r.clicked() {
                            self.ws = step;
                            if step == Ws::Standards {
                                self.std_view.refresh();
                            }
                        }
                    }
                    chrome::strip_divider(ui);
                    if self.chat.busy() {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        ui.add_space(6.0);
                        ui.spinner();
                        let secs = self.chat.busy_since.map(|b| b.elapsed().as_secs()).unwrap_or(0);
                        ui.label(RichText::new(self.chat.current_agent()).font(medium_font(13.0)));
                        ui.label(RichText::new(format!("{}:{:02}", secs / 60, secs % 60)).monospace().color(t.text_dim));
                        let (_, tp, tc) = self.chat.run_usage.totals();
                        if tp + tc > 0 && self.chat.saved.details {
                            ui.label(RichText::new(format!("{} tokens", crate::chat::fmt_tokens(tp + tc))).monospace().color(t.text_dim)).on_hover_text(self.chat.run_usage.table());
                        }
                        if ui.add(egui::Button::new(RichText::new("Stop").color(t.error)).frame(false)).on_hover_text("Stops the agents now, including a running program calculation or script").clicked() {
                            self.chat.stop();
                        }
                        if ui.add(egui::Button::new("Watch").frame(false)).clicked() {
                            self.ws = Ws::Design;
                        }
                    } else {
                        let on = self.ws == Ws::Design;
                        let b = chrome::primary_button("Design office").fill(if on { t.accent } else { t.accent.gamma_multiply(0.92) });
                        if ui.add(b).on_hover_text("Talk with the coordinator and its team").clicked() {
                            self.ws = Ws::Design;
                        }
                    }
                    chrome::strip_divider(ui);
                    let state = if self.ws == Ws::Output { chrome::StepState::Current } else { chrome::StepState::Todo };
                    if chrome::step_button(ui, None, Some(Icon::FileText), "Output", state).on_hover_text("Calculations, drawings and reports").clicked() {
                        self.ws = Ws::Output;
                    }
                });

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (icon, tip) = if t.dark { (Icon::Sun, "Light theme") } else { (Icon::Moon, "Dark theme") };
                    if icons::button(ui, icon, tip, false).clicked() {
                        self.settings.dark = !self.settings.dark;
                        self.apply_settings();
                    }
                    if icons::button(ui, Icon::Settings, "Settings", self.ws == Ws::Settings).clicked() {
                        self.ws = if self.ws == Ws::Settings { Ws::Design } else { Ws::Settings };
                    }
                    if self.ws != Ws::Design {
                        let label = if self.chat_open { "Close assistant" } else { "Assistant" };
                        if chrome::assistant_button(ui, label).on_hover_text("The design office's coordinator, beside any step").clicked() {
                            self.chat_open = !self.chat_open;
                        }
                    }
                });
            });
        });
    }

    fn status_bar(&mut self, ctx: &egui::Context) {
        let t = Tokens::get(ctx);
        let status = egui::Frame::NONE.fill(t.dock).inner_margin(egui::Margin::symmetric(12, 4)).stroke(egui::Stroke::new(1.0_f32, t.separator));
        egui::TopBottomPanel::bottom("status").exact_height(28.0).frame(status).show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 18.0;
                let key = self.settings.provider("").map(|p| p.key().is_some() || p.base_url.contains("localhost") || p.is_cli()).unwrap_or(false);
                if self.chat.busy() {
                    chrome::status_dot(ui, t.accent, self.chat.current_activity());
                } else if let Some(e) = self.chat.last_error() {
                    chrome::status_dot(ui, t.error, e.chars().take(120).collect::<String>());
                } else if !key {
                    chrome::status_dot(ui, t.warn, format!("No API key for {}", self.settings.default_provider));
                } else {
                    chrome::status_dot(ui, t.ok, "Ready");
                }
                let lib = self.engine.library.list();
                let chosen = lib.iter().filter(|m| self.project.info.standards.contains(&m.id)).count();
                let ready = self.settings.plugins.iter().filter(|p| matches!(self.engine.plugins.state_of(&p.name), PluginState::Ready { .. })).count();
                ui.label(RichText::new(format!("{chosen} of {} standards · {} team members · {ready} plugins connected", lib.len(), self.team.agents.len())).size(12.0).color(t.text_dim));
                if !self.status.is_empty() {
                    ui.label(RichText::new(&self.status).size(12.0).color(t.text_faint));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.chat.saved.details {
                        let (p, c) = self.chat.saved.tokens;
                        ui.label(RichText::new(format!("{} · {} in · {} out", self.settings.default_model, crate::chat::fmt_tokens(p), crate::chat::fmt_tokens(c))).size(12.0).color(t.text_faint)).on_hover_text("Default model, and the tokens this project's conversation has used so far");
                    }
                    ui.label(RichText::new("kN · m · MPa").size(12.0).color(t.text_dim));
                });
            });
        });
    }

    // ---------------------------------------------------------------- step panel

    /// The left panel of a step: title, "Step n of 4", help, about, the body, "Next".
    fn step_panel(&mut self, ctx: &egui::Context, body: impl FnOnce(&mut Self, &mut egui::Ui)) {
        let t = Tokens::get(ctx);
        let ws = self.ws;
        let edge = egui::Stroke::new(1.0_f32, t.separator);
        egui::SidePanel::left("steps").resizable(true).default_width(400.0).min_width(330.0).frame(egui::Frame::NONE.fill(t.chrome).stroke(edge)).show(ctx, |ui| {
            egui::TopBottomPanel::top("step_head").frame(egui::Frame::NONE.inner_margin(egui::Margin { left: 20, right: 16, top: 18, bottom: 8 })).show_separator_line(false).show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(ws.title()).font(medium_font(17.0)));
                    if let Some(n) = ws.number() {
                        ui.label(RichText::new(format!("Step {n} of 4")).size(12.0).color(t.text_faint));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if !ws.help().is_empty() {
                            chrome::help_button(ui, ws.title(), ws.help());
                        }
                    });
                });
                if !ws.about().is_empty() {
                    ui.label(RichText::new(ws.about()).size(12.5).color(t.text_dim));
                }
            });
            if let Some(next) = ws.next() {
                egui::TopBottomPanel::bottom("step_next").frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(20, 10))).show_separator_line(true).show_inside(ui, |ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let clicked = if next == Ws::Design {
                            ui.add(chrome::primary_button("Design office →").fill(t.accent)).clicked()
                        } else {
                            ui.add(egui::Button::new(RichText::new(format!("Next: {} →", next.title())).font(medium_font(13.0))).min_size(egui::vec2(0.0, 32.0)).corner_radius(8)).clicked()
                        };
                        if clicked {
                            self.ws = next;
                            if next == Ws::Standards {
                                self.std_view.refresh();
                            }
                        }
                    });
                });
            }
            egui::CentralPanel::default().frame(egui::Frame::NONE.inner_margin(egui::Margin { left: 20, right: 14, top: 0, bottom: 8 })).show_inside(ui, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| body(self, ui));
            });
        });
    }

    fn canvas_frame(ctx: &egui::Context) -> egui::Frame {
        egui::Frame::NONE.fill(Tokens::get(ctx).canvas)
    }

    /// Paints the canvas dot grid behind the work area.
    fn dots(ui: &egui::Ui) {
        let t = Tokens::get(ui.ctx());
        let r = ui.max_rect().expand(16.0);
        let p = ui.painter();
        let step = 24.0;
        let mut x = r.left() + 12.0;
        while x < r.right() {
            let mut y = r.top() + 12.0;
            while y < r.bottom() {
                p.circle_filled(egui::pos2(x, y), 1.0, t.canvas_dot);
                y += step;
            }
            x += step;
        }
    }

    // ---------------------------------------------------------------- workspaces

    fn project_ws(&mut self, ctx: &egui::Context) {
        self.step_panel(ctx, |app, ui| {
            let t = Tokens::get(ui.ctx());
            let mut changed = false;
            chrome::explorer(ui, "Project", "", |ui| {
                changed |= chrome::field(ui, "Name", &mut app.project.info.name).changed();
                ui.add_space(4.0);
                ui.label(RichText::new("Description").color(t.text_dim));
                changed |= ui.add(egui::TextEdit::multiline(&mut app.project.info.description).hint_text("e.g. 4-storey RC office building on soft clay, Bangkok").desired_rows(3).desired_width(f32::INFINITY)).changed();
                ui.add_space(4.0);
            });
            chrome::explorer(ui, "Notes every agent reads", "", |ui| {
                ui.add_space(4.0);
                changed |= ui.add(egui::TextEdit::multiline(&mut app.project.info.notes).hint_text("Client, site, load assumptions, units, office rules…").desired_rows(6).desired_width(f32::INFINITY)).changed();
                ui.add_space(4.0);
            });
            chrome::explorer(ui, "Save as template", "", |ui| {
                ui.label(RichText::new(format!("Saves the team '{}' ({} agents with their roles, instructions, models, tools, skills and plugins), the chosen standards, the description and notes, the default and vision models, and the plugin and skill set-up. API keys and plugin secrets are left out; the chat and the outputs stay with this project.", app.team.name, app.team.agents.len())).size(11.5).color(t.text_dim));
                ui.add_space(4.0);
                chrome::field(ui, "Name", &mut app.tpl_name);
                ui.add(egui::TextEdit::singleline(&mut app.tpl_desc).hint_text("What it is for, e.g. RC buildings to ACI 318-19").desired_width(f32::INFINITY));
                ui.add(egui::TextEdit::multiline(&mut app.tpl_prompt).hint_text("Starter message for new projects (optional)").desired_rows(2).desired_width(f32::INFINITY));
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    let exists = Template::path_for(&app.tpl_name).is_file();
                    let label = if exists { "Replace template" } else { "Save template" };
                    if ui.add_enabled(!app.tpl_name.trim().is_empty(), chrome::primary_button(label).fill(t.accent)).on_hover_text("Saved templates appear on the Home page").clicked() {
                        app.status = match app.capture_template().save() {
                            Ok(_) => format!("Saved template '{}'. Start a project from it on the Home page.", app.tpl_name.trim()),
                            Err(e) => e,
                        };
                    }
                    if ui.add_enabled(!app.tpl_name.trim().is_empty(), egui::Button::new("Export…")).on_hover_text("Write the template to a file to share it").clicked() {
                        let tpl = app.capture_template();
                        if let Some(to) = rfd::FileDialog::new().set_file_name(th_harness::TeamSpec::file_name(&tpl.name)).add_filter("TellusHive template", &["json"]).save_file() {
                            app.status = tpl.export(&to).map(|_| format!("Exported to {}", to.display())).unwrap_or_else(|e| e);
                        }
                    }
                });
            });
            chrome::explorer(ui, "Folder", "", |ui| {
                ui.label(RichText::new(app.project.dir.display().to_string()).monospace().size(11.0).color(t.text_dim));
                if chrome::ghost_button(ui, "Show in Finder").clicked() {
                    chrome::open_path(&app.project.dir);
                }
            });
            if changed {
                app.apply_project();
            }
        });
        egui::CentralPanel::default().frame(Self::canvas_frame(ctx)).show(ctx, |ui| {
            Self::dots(ui);
            let t = Tokens::get(ctx);
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(&self.project.info.name).font(medium_font(24.0)));
                ui.label(RichText::new("The design office for this project").color(t.text_dim));
                ui.add_space(20.0);
            });
            let lib = self.engine.library.list();
            let chosen: Vec<String> = lib.iter().filter(|m| self.project.info.standards.contains(&m.id)).map(|m| m.code.clone()).collect();
            let ready = self.settings.plugins.iter().filter(|p| p.enabled).count();
            let counts = |k: ArtifactKind| self.artifacts.iter().filter(|a| a.kind == k).count();
            let cards: [(Ws, Icon, String, String); 5] = [
                (Ws::Standards, Icon::Library, "Standards".into(), if chosen.is_empty() { "None chosen yet".into() } else { chosen.join(", ") }),
                (Ws::Team, Icon::Users, "Team".into(), format!("{}: {}", self.team.name, self.team.agents.iter().map(|a| a.name.clone()).collect::<Vec<_>>().join(", "))),
                (Ws::Plugins, Icon::Plug, "Plugins".into(), format!("{ready} available: {}", self.settings.plugins.iter().filter(|p| p.enabled).map(|p| p.name.clone()).collect::<Vec<_>>().join(", "))),
                (Ws::Design, Icon::Chat, "Design office".into(), format!("{} messages in the conversation", self.chat.saved.entries.iter().filter(|e| matches!(e, crate::chat::Entry::User { .. })).count())),
                (Ws::Output, Icon::FileText, "Output".into(), format!("{} calculations · {} drawings · {} reports", counts(ArtifactKind::Calculation), counts(ArtifactKind::Drawing), counts(ArtifactKind::Report))),
            ];
            let w = 300.0;
            let avail = ui.available_width();
            let cols = ((avail - 40.0) / (w + 16.0)).floor().clamp(1.0, 3.0) as usize;
            let left = ((avail - cols as f32 * (w + 16.0)) / 2.0).max(16.0);
            let mut go = None;
            for row in cards.chunks(cols) {
                ui.horizontal(|ui| {
                    ui.add_space(left);
                    ui.spacing_mut().item_spacing.x = 16.0;
                    for (ws, icon, title, sub) in row {
                        let (r, resp) = ui.allocate_exact_size(egui::vec2(w, 100.0), egui::Sense::click());
                        chrome::soft_shadow(ui.painter(), r, &t);
                        ui.painter().rect_filled(r, 10.0, t.card);
                        ui.painter().rect_stroke(r, 10.0, egui::Stroke::new(1.0_f32, if resp.hovered() { t.text_faint } else { t.separator }), egui::StrokeKind::Inside);
                        icons::paint(ui, *icon, egui::Rect::from_center_size(r.left_top() + egui::vec2(30.0, 30.0), egui::Vec2::splat(20.0)), t.accent_text);
                        ui.painter().text(r.left_top() + egui::vec2(52.0, 30.0), egui::Align2::LEFT_CENTER, title, medium_font(14.0), t.text);
                        let g = ui.painter().layout(sub.clone(), egui::FontId::proportional(12.0), t.text_dim, w - 40.0);
                        ui.painter().galley(r.left_top() + egui::vec2(20.0, 52.0), g, t.text_dim);
                        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            go = Some(*ws);
                        }
                    }
                });
                ui.add_space(16.0);
            }
            if let Some(ws) = go {
                self.ws = ws;
                if ws == Ws::Standards {
                    self.std_view.refresh();
                }
            }
        });
    }

    fn standards_ws(&mut self, ctx: &egui::Context) {
        let lib = self.engine.library.clone();
        let emb = self.settings.embedder.clone();
        let mut changed = false;
        self.step_panel(ctx, |app, ui| {
            changed |= app.std_view.left(ui, &lib, &mut app.project.info.standards, &emb);
        });
        egui::CentralPanel::default().frame(Self::canvas_frame(ctx).inner_margin(16)).show(ctx, |ui| {
            Self::dots(ui);
            self.std_view.center(ui, &lib, &self.project.info.standards);
        });
        if changed {
            self.apply_project();
        }
    }

    fn team_ws(&mut self, ctx: &egui::Context) {
        let skills = self.engine.skills.read().map(|s| s.clone()).unwrap_or_default();
        let mut changed = false;
        self.step_panel(ctx, |app, ui| {
            let settings = app.settings.clone();
            changed |= app.team_view.left(ui, &mut app.team, &settings, &skills);
        });
        egui::CentralPanel::default().frame(Self::canvas_frame(ctx)).show(ctx, |ui| {
            changed |= self.team_view.canvas(ui, &mut self.team, &self.settings);
        });
        if changed {
            if let Ok(mut e) = self.engine.team.write() {
                *e = self.team.clone();
            }
            if self.project.info.team != self.team.name {
                self.project.info.team = self.team.name.clone();
                self.apply_project();
            }
            self.team_dirty = Some(Instant::now());
        }
    }

    fn plugins_ws(&mut self, ctx: &egui::Context) {
        let plugins = self.engine.plugins.clone();
        let mut changed = false;
        self.step_panel(ctx, |app, ui| {
            changed |= app.plug_view.left(ui, &mut app.settings.plugins, &plugins);
        });
        egui::CentralPanel::default().frame(Self::canvas_frame(ctx).inner_margin(16)).show(ctx, |ui| {
            Self::dots(ui);
            changed |= self.plug_view.center(ui, &mut self.settings.plugins, &plugins);
        });
        if changed {
            self.apply_settings();
        }
    }

    /// True while the program's window that this app opened is still running.
    fn window_open(&mut self, plugin: &str) -> bool {
        let alive = self.windows.get_mut(plugin).is_some_and(|(c, _)| matches!(c.try_wait(), Ok(None)));
        if !alive {
            self.windows.remove(plugin);
        }
        alive
    }

    /// Opens the project's model in the program's own window. `refresh` reopens a window
    /// that does not follow its file when the model changed since it was opened.
    fn open_model_window(&mut self, plugin: &str, refresh: bool) {
        let Some(s) = th_harness::live::spec(plugin) else { return };
        let program = th_harness::live::program_name(plugin);
        let model = th_harness::live::model_path(&self.project.dir, s);
        if !model.is_file() {
            self.status = format!("There is no {} yet: it appears when an agent starts one in {program}.", s.label.to_lowercase());
            return;
        }
        let Some(cmd) = self.settings.plugins.iter().find(|p| p.name == plugin).map(|p| PathBuf::from(&p.command)).filter(|c| c.is_file()) else {
            self.status = format!("{program} was not found on this computer (see Plugins).");
            return;
        };
        if self.window_open(plugin) {
            let opened = self.windows.get(plugin).map(|w| w.1);
            let newer = std::fs::metadata(&model).and_then(|m| m.modified()).ok().zip(opened).is_some_and(|(m, o)| m > o);
            if s.follows_file || !refresh || !newer {
                self.status = format!("{program} already has the {} open in its own window.", s.label.to_lowercase());
                return;
            }
            if let Some((mut c, _)) = self.windows.remove(plugin) {
                let _ = c.kill();
                let _ = c.wait();
            }
        }
        let spawned = std::process::Command::new(&cmd)
            .arg(&model)
            .current_dir(model.parent().unwrap_or(&self.project.dir))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        match spawned {
            Ok(c) => {
                self.windows.insert(plugin.to_string(), (c, std::time::SystemTime::now()));
                self.status = format!("Opened the {} in {program}.", s.label.to_lowercase());
            }
            Err(e) => self.status = format!("Could not start {program}: {e}"),
        }
    }

    /// The project's models in the analysis programs: open one in its program's window.
    fn models_list(&mut self, ui: &mut egui::Ui) {
        let t = Tokens::get(ui.ctx());
        let found: Vec<(&'static th_harness::live::LiveSpec, PathBuf, std::time::SystemTime)> = th_harness::live::LIVE
            .iter()
            .filter_map(|s| {
                let p = th_harness::live::model_path(&self.project.dir, s);
                let m = std::fs::metadata(&p).and_then(|m| m.modified()).ok()?;
                Some((s, p, m))
            })
            .collect();
        if found.is_empty() {
            return;
        }
        chrome::section(ui, &format!("Models  ·  {}", found.len()));
        for (s, _, modified) in found {
            let program = th_harness::live::program_name(s.plugin);
            let open = self.window_open(s.plugin);
            let opened = self.windows.get(s.plugin).map(|w| w.1);
            let newer = opened.is_some_and(|o| modified > o);
            let state = match (open, s.follows_file, newer) {
                (true, true, _) => "open · follows live".to_string(),
                (true, false, true) => "newer model ready: click to show it".to_string(),
                (true, false, false) => "open".to_string(),
                (false, _, _) => format!("updated {}", ago(modified)),
            };
            let tip = if s.follows_file {
                format!("Opens the model in {program}'s window, which follows the agents' changes as they make them. Rotate, cut and inspect it there.")
            } else {
                format!("Opens the model in {program}'s window. That window shows the model as it was when opened: click again after the agents change it (the window this app opened is closed and reopened, so save any edits you made in it first).")
            };
            let colour = if newer { t.warn } else { t.accent };
            if chrome::object_row(ui, colour, Some(Icon::Rotate), &format!("{} · {program}", s.label), &state, false).on_hover_text(tip).clicked() {
                self.open_model_window(s.plugin, true);
            }
            if s.plugin == "aedifex" {
                let mesh = th_harness::live::model_path(&self.project.dir, s).with_extension("axmesh");
                let stale = std::fs::metadata(&mesh).and_then(|m| m.modified()).map(|m| m < modified).unwrap_or(true);
                let sel = self.viewer.path.as_ref() == Some(&mesh);
                let label = if stale { "make it" } else { "rotate, zoom" };
                if chrome::object_row(ui, t.accent, Some(Icon::Rotate), "3D view", label, sel).on_hover_text("Shows the Aedifex model here: drag to rotate, scroll to zoom, right-drag to pan, click to pick an element.").clicked() {
                    if stale {
                        match self.engine.export_bim_mesh() {
                            Ok(p) => self.viewer.show(p),
                            Err(e) => self.status = e,
                        }
                    } else {
                        self.viewer.show(mesh);
                    }
                }
            }
        }
    }

    fn deliverables(&mut self, ui: &mut egui::Ui) {
        let t = Tokens::get(ui.ctx());
        self.refresh_artifacts();
        self.models_list(ui);
        if self.artifacts.is_empty() {
            ui.add_space(8.0);
            chrome::hint(ui, "Calculations, drawings and reports the agents make appear here.");
        }
        for kind in ArtifactKind::all() {
            let items: Vec<Artifact> = self.artifacts.iter().filter(|a| a.kind == kind).cloned().collect();
            if items.is_empty() {
                continue;
            }
            chrome::section(ui, &format!("{}  ·  {}", kind.label(), items.len()));
            let (icon, colour) = match kind {
                ArtifactKind::Calculation => (Icon::Calculator, t.tools.run),
                ArtifactKind::Drawing => (Icon::Drafting, t.tools.soil),
                ArtifactKind::Report => (Icon::FileText, t.tools.view),
                ArtifactKind::Image => (Icon::Picture, t.tools.water),
                ArtifactKind::Attachment => (Icon::Paperclip, t.text_dim),
                ArtifactKind::Work => (Icon::Edit, t.tools.mesh),
            };
            for a in items {
                let pref = ["pdf", "svg", "json", "md"];
                let file = pref.iter().find_map(|e| a.files.iter().find(|f| f.extension().is_some_and(|x| x == *e))).or(a.files.first()).cloned();
                let Some(file) = file else { continue };
                let exts: Vec<String> = a.files.iter().filter_map(|f| f.extension().map(|e| e.to_string_lossy().to_string())).collect();
                let sel = self.viewer.path.as_ref() == Some(&file);
                if chrome::object_row(ui, colour, Some(icon), &a.name, &exts.join(" · "), sel).clicked() {
                    self.viewer.show(file);
                }
            }
        }
    }

    fn design_ws(&mut self, ctx: &egui::Context) {
        let t = Tokens::get(ctx);
        let edge = egui::Stroke::new(1.0_f32, t.separator);
        egui::SidePanel::left("deliverables").resizable(true).default_width(300.0).min_width(240.0).frame(egui::Frame::NONE.fill(t.chrome).stroke(edge).inner_margin(egui::Margin { left: 16, right: 12, top: 18, bottom: 8 })).show(ctx, |ui| {
            ui.label(RichText::new("Design office").font(medium_font(17.0)));
            let std = if self.project.info.standards.is_empty() { "no standard chosen".to_string() } else { format!("{} standards", self.project.info.standards.len()) };
            ui.label(RichText::new(format!("{} · {std}", self.team.name)).size(12.5).color(t.text_dim));
            ui.add_space(6.0);
            let working = self.chat.live.iter().filter(|l| !l.waiting).count();
            ui.horizontal(|ui| {
                ui.label(RichText::new("Team").font(medium_font(13.0)));
                let note = if !self.chat.busy() { "ready".to_string() } else if working == 1 { "1 working now".to_string() } else { format!("{working} working now") };
                ui.label(RichText::new(note).size(11.5).color(if self.chat.busy() { t.accent_text } else { t.text_faint }));
            });
            let colours = self.agent_colours();
            self.chat.roster_ui(ui, &colours);
            ui.add_space(6.0);
            ui.separator();
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.deliverables(ui));
        });
        if self.viewer.path.is_some() {
            egui::SidePanel::right("viewer").resizable(true).default_width(560.0).min_width(300.0).frame(egui::Frame::NONE.fill(t.canvas).stroke(edge).inner_margin(12)).show(ctx, |ui| self.viewer.ui(ui));
        }
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.field)).show(ctx, |ui| {
            let colours = self.agent_colours();
            let f = move |n: &str| colours.iter().find(|(k, _)| k == n).map(|(_, c)| *c).unwrap_or(Color32::from_rgb(120, 120, 130));
            let engine = self.engine.clone();
            self.chat.ui(ui, &engine, &f, true);
        });
    }

    fn output_ws(&mut self, ctx: &egui::Context) {
        let t = Tokens::get(ctx);
        let edge = egui::Stroke::new(1.0_f32, t.separator);
        egui::SidePanel::left("out_list").resizable(true).default_width(340.0).min_width(260.0).frame(egui::Frame::NONE.fill(t.chrome).stroke(edge).inner_margin(egui::Margin { left: 16, right: 12, top: 18, bottom: 8 })).show(ctx, |ui| {
            ui.label(RichText::new("Output").font(medium_font(17.0)));
            ui.label(RichText::new("The calculations, drawings and reports of this project.").size(12.5).color(t.text_dim));
            ui.add_space(6.0);
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.deliverables(ui));
        });
        egui::CentralPanel::default().frame(Self::canvas_frame(ctx).inner_margin(16)).show(ctx, |ui| {
            Self::dots(ui);
            if self.viewer.path.is_none() {
                if let Some(first) = self.artifacts.iter().find(|a| a.kind == ArtifactKind::Drawing || a.kind == ArtifactKind::Calculation).and_then(|a| a.files.iter().find(|f| f.extension().is_some_and(|e| e == "svg" || e == "json")).cloned()) {
                    self.viewer.show(first);
                }
            }
            self.viewer.ui(ui);
        });
    }

    fn settings_ws(&mut self, ctx: &egui::Context) {
        let t = Tokens::get(ctx);
        let edge = egui::Stroke::new(1.0_f32, t.separator);
        egui::SidePanel::left("set_nav").resizable(false).exact_width(260.0).frame(egui::Frame::NONE.fill(t.chrome).stroke(edge).inner_margin(egui::Margin { left: 16, right: 12, top: 18, bottom: 8 })).show(ctx, |ui| {
            ui.label(RichText::new("Settings").font(medium_font(17.0)));
            ui.label(RichText::new("Models, keys and tools of the design office.").size(12.5).color(t.text_dim));
            ui.add_space(12.0);
            self.set_view.nav(ui);
        });
        let mut changed = false;
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.field).inner_margin(egui::Margin { left: 40, right: 40, top: 24, bottom: 16 })).show(ctx, |ui| {
            changed = self.set_view.ui(ui, &mut self.settings);
        });
        if changed {
            self.apply_settings();
        }
    }

    // ---------------------------------------------------------------- home

    fn home_ws(&mut self, ctx: &egui::Context) {
        let t = Tokens::get(ctx);
        let rail = egui::Frame::NONE.fill(t.chrome).inner_margin(egui::Margin { left: 16, right: 16, top: 28, bottom: 20 }).stroke(egui::Stroke::new(1.0_f32, t.separator));
        egui::SidePanel::left("home_rail").exact_width(248.0).resizable(false).frame(rail).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.add_space(6.0);
                ui.add(egui::Image::new(&chrome::app_icon(ctx)).fit_to_exact_size(egui::vec2(38.0, 38.0)));
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.label(RichText::new("TellusHive").font(medium_font(18.0)));
                    ui.label(RichText::new("An AI design team for civil engineers").size(12.0).color(t.text_dim));
                });
            });
            ui.add_space(24.0);
            ui.spacing_mut().item_spacing.y = 2.0;
            rail_button(ui, Icon::Home, "Home", true);
            if rail_button(ui, Icon::Plus, "New project", false).clicked() {
                let n = format!("Project {}", th_harness::tools::today());
                if let Err(e) = self.new_project_named(&n) {
                    self.status = e;
                }
                self.ws = Ws::Project;
            }
            if rail_button(ui, Icon::Folder, "Open…", false).clicked() {
                if let Some(d) = rfd::FileDialog::new().set_directory(&self.settings.projects_dir).pick_folder() {
                    match Project::open_or_create(&d) {
                        Ok(p) => {
                            self.switch_project(p);
                            self.ws = Ws::Design;
                        }
                        Err(e) => self.status = e,
                    }
                }
            }
            if rail_button(ui, Icon::Users, "Open template…", false).on_hover_text("Start a new project from a template file (.json): its team, standards, plugins and notes").clicked() {
                self.open_template_dialog();
            }
            if rail_button(ui, Icon::Library, "Standards library", false).clicked() {
                self.ws = Ws::Standards;
                self.std_view.refresh();
            }
            if rail_button(ui, Icon::Redo, "Back to the project", false).clicked() {
                self.ws = Ws::Design;
            }
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                ui.label(RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION"))).size(12.0).color(t.text_faint));
                ui.add_space(8.0);
                if rail_button(ui, Icon::Settings, "Settings", false).clicked() {
                    self.ws = Ws::Settings;
                }
            });
        });

        let page = egui::Frame::NONE.fill(t.field).inner_margin(egui::Margin { left: 56, right: 56, top: 40, bottom: 24 });
        egui::CentralPanel::default().frame(page).show(ctx, |ui| {
            let visible = (ui.clip_rect().right() - 56.0 - ui.cursor().left()).clamp(320.0, 1280.0);
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.set_max_width(visible);
                ui.set_min_width(visible);
                ui.label(RichText::new("Start a project").font(medium_font(26.0)));
                ui.add_space(14.0);
                let templates: [(&str, &str, Thumb, &str); 4] = [
                    ("Empty project", "Your own design brief", Thumb::Empty, ""),
                    ("RC beam", "ACI 318-19 flexure and shear", Thumb::Beam, "Design a simply supported RC beam, span 6 m, 300 x 500 mm, wD = 15 kN/m (including self-weight), wL = 10 kN/m, f'c = 28 MPa, fy = 420 MPa, to ACI 318-19. Have it checked, draw the section and write the calculation report."),
                    ("Footing", "Bearing resistance, EC7 DA1", Thumb::Footing, "Check the bearing resistance of a 2.0 x 2.0 m square pad footing founded at 1.5 m on sand (phi'k = 32 deg, c'k = 0, gamma = 18 kN/m3, water table deep), Gk = 900 kN, Qk = 300 kN, to EN 1997-1 Annex D, Design Approach 1. Have it checked and write the report."),
                    ("Retaining wall", "Cantilever wall stability", Thumb::Wall, "Design a 4 m high cantilever retaining wall with level backfill (phi = 30 deg, gamma = 18 kN/m3, c = 0) and a 10 kPa surcharge, founded on clayey sand (phi = 28 deg, c = 10 kPa, gamma = 19 kN/m3), embedment 1.0 m. Check overturning, sliding and bearing, draw the section and write the report."),
                ];
                let gap = 16.0;
                let w = ((visible - gap * 3.0) / 4.0).clamp(150.0, 260.0);
                let mut chosen: Option<(&str, &str)> = None;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for (title, sub, th, prompt) in templates {
                        let (r, resp) = ui.allocate_exact_size(egui::vec2(w, w * 0.55 + 58.0), egui::Sense::click());
                        let p = ui.painter_at(r);
                        p.rect_filled(r, 10.0, t.card);
                        let img = egui::Rect::from_min_size(r.min, egui::vec2(w, w * 0.55));
                        thumb(&p, img, th, &t);
                        p.text(egui::pos2(r.left() + 14.0, img.bottom() + 18.0), egui::Align2::LEFT_CENTER, title, medium_font(13.5), t.text);
                        p.text(egui::pos2(r.left() + 14.0, img.bottom() + 38.0), egui::Align2::LEFT_CENTER, sub, egui::FontId::proportional(12.0), t.text_dim);
                        p.rect_stroke(r, 10.0, egui::Stroke::new(1.0_f32, if resp.hovered() { t.text_faint } else { t.separator }), egui::StrokeKind::Inside);
                        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            chosen = Some((title, prompt));
                        }
                    }
                });
                if let Some((title, prompt)) = chosen {
                    let name = if prompt.is_empty() { format!("Project {}", th_harness::tools::today()) } else { title.to_string() };
                    match self.new_project_named(&name) {
                        Ok(()) => {
                            if prompt.is_empty() {
                                self.ws = Ws::Project;
                            } else {
                                self.chat.set_input(prompt);
                                self.ws = Ws::Design;
                            }
                        }
                        Err(e) => self.status = e,
                    }
                }

                ui.add_space(26.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Your templates").font(medium_font(17.0)));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(egui::Button::new("Import template…").min_size(egui::vec2(0.0, 32.0))).clicked() {
                            if let Some(f) = rfd::FileDialog::new().add_filter("TellusHive template", &["json"]).pick_file() {
                                self.status = Template::import(&f).map(|t| format!("Imported template '{}'", t.name)).unwrap_or_else(|e| e);
                            }
                        }
                    });
                });
                ui.add_space(8.0);
                let templates = Template::list();
                if templates.is_empty() {
                    ui.label(RichText::new("None yet. Set up a project's team, standards and notes, then use Save as template in the Project step; new projects can then start from it here.").size(12.5).color(t.text_dim));
                }
                let lib = self.engine.library.list();
                let mut use_tpl: Option<Template> = None;
                let mut delete: Option<String> = None;
                egui::Frame::NONE.stroke(egui::Stroke::new(if templates.is_empty() { 0.0 } else { 1.0_f32 }, t.separator)).corner_radius(10).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for (i, tpl) in templates.iter().enumerate() {
                        let (r, resp) = ui.allocate_exact_size(egui::vec2(visible - 2.0, 60.0), egui::Sense::click());
                        let painter = ui.painter_at(r);
                        if resp.hovered() {
                            painter.rect_filled(r, 0.0, t.hover);
                        }
                        if i > 0 {
                            painter.hline(r.x_range(), r.top(), egui::Stroke::new(1.0_f32, t.separator));
                        }
                        let tile = egui::Rect::from_center_size(egui::pos2(r.left() + 34.0, r.center().y), egui::vec2(32.0, 32.0));
                        painter.rect_filled(tile, 6.0, t.accent_soft);
                        icons::paint(ui, Icon::Users, tile.shrink(7.0), t.accent_text);
                        painter.text(egui::pos2(r.left() + 68.0, r.center().y - 10.0), egui::Align2::LEFT_CENTER, &tpl.name, medium_font(13.5), t.text);
                        let missing = tpl.standards.iter().filter(|id| !lib.iter().any(|m| &m.id == *id)).count();
                        let stds = if tpl.standard_codes.is_empty() { "no standards".to_string() } else { tpl.standard_codes.join(", ") };
                        let sub = format!("{}{} agents · {}{} · {}", if tpl.description.is_empty() { String::new() } else { format!("{} · ", tpl.description) }, tpl.team.agents.len(), stds, if missing > 0 { format!(" ({missing} not in library)") } else { String::new() }, tpl.created);
                        painter.text(egui::pos2(r.left() + 68.0, r.center().y + 10.0), egui::Align2::LEFT_CENTER, sub, egui::FontId::proportional(12.0), t.text_dim);
                        // the row's buttons, drawn over its right end
                        let btns = egui::Rect::from_min_max(egui::pos2(r.right() - 230.0, r.top()), r.max);
                        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(btns).layout(egui::Layout::right_to_left(egui::Align::Center)));
                        child.add_space(12.0);
                        if child.button("Delete").on_hover_text("Remove this template (projects made from it are kept)").clicked() {
                            delete = Some(tpl.name.clone());
                        }
                        if child.button("Export…").clicked() {
                            if let Some(to) = rfd::FileDialog::new().set_file_name(th_harness::TeamSpec::file_name(&tpl.name)).add_filter("TellusHive template", &["json"]).save_file() {
                                self.status = tpl.export(&to).map(|_| format!("Exported to {}", to.display())).unwrap_or_else(|e| e);
                            }
                        }
                        if child.add(chrome::primary_button("New project").fill(t.accent)).clicked() {
                            use_tpl = Some(tpl.clone());
                        }
                        let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
                        let resp = if tpl.notes.trim().is_empty() { resp } else { resp.on_hover_text(format!("Notes every agent reads:\n{}", tpl.notes)) };
                        if resp.clicked() {
                            use_tpl = Some(tpl.clone());
                        }
                    }
                });
                if let Some(name) = delete {
                    self.status = Template::delete(&name).map(|_| format!("Deleted template '{name}'")).unwrap_or_else(|e| e);
                }
                if let Some(tpl) = use_tpl {
                    match self.new_project_from_template(&tpl) {
                        Ok(()) => self.ws = Ws::Design,
                        Err(e) => self.status = e,
                    }
                }

                ui.add_space(26.0);
                ui.label(RichText::new("Or describe it, and the design office takes it from there").size(13.0).color(t.text_dim));
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    let field = egui::TextEdit::singleline(&mut self.home_ask).hint_text("e.g. a 3 m embankment on 12 m of soft Bangkok clay: stability and settlement").desired_width(visible - 150.0).margin(egui::Margin::symmetric(12, 10));
                    let r = ui.add(field);
                    let go = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if (ui.add_enabled(!self.home_ask.trim().is_empty(), chrome::primary_button("Start design").fill(t.accent).min_size(egui::vec2(128.0, 38.0))).clicked() || go) && !self.home_ask.trim().is_empty() {
                        let ask = std::mem::take(&mut self.home_ask);
                        let name: String = ask.split_whitespace().take(5).collect::<Vec<_>>().join(" ");
                        match self.new_project_named(&name) {
                            Ok(()) => {
                                self.chat.set_input(&ask);
                                self.ws = Ws::Design;
                            }
                            Err(e) => self.status = e,
                        }
                    }
                });

                ui.add_space(30.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Recent").font(medium_font(17.0)));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(egui::Button::new("Open other…").min_size(egui::vec2(0.0, 32.0))).clicked() {
                            if let Some(d) = rfd::FileDialog::new().set_directory(&self.settings.projects_dir).pick_folder() {
                                if let Ok(p) = Project::open_or_create(&d) {
                                    self.switch_project(p);
                                    self.ws = Ws::Design;
                                }
                            }
                        }
                        ui.add(egui::TextEdit::singleline(&mut self.home_filter).hint_text("Find a project").desired_width(240.0));
                    });
                });
                ui.add_space(8.0);
                let mut recent: Vec<(PathBuf, String, Option<std::time::SystemTime>)> = std::fs::read_dir(&self.settings.projects_dir)
                    .map(|rd| {
                        rd.flatten()
                            .map(|e| e.path())
                            .filter(|p| p.join("project.json").is_file())
                            .map(|p| {
                                let name = read_json::<ProjectInfo>(&p.join("project.json")).map(|i| i.name).unwrap_or_default();
                                let m = std::fs::metadata(p.join("chat.json")).or_else(|_| std::fs::metadata(p.join("project.json"))).and_then(|m| m.modified()).ok();
                                (p, name, m)
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                if !recent.iter().any(|r| r.0 == self.project.dir) {
                    recent.push((self.project.dir.clone(), self.project.info.name.clone(), None));
                }
                let f = self.home_filter.trim().to_lowercase();
                recent.retain(|r| f.is_empty() || r.1.to_lowercase().contains(&f) || r.0.display().to_string().to_lowercase().contains(&f));
                recent.sort_by(|a, b| b.2.cmp(&a.2));
                let mut open = None;
                egui::Frame::NONE.stroke(egui::Stroke::new(1.0_f32, t.separator)).corner_radius(10).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for (i, (p, name, m)) in recent.iter().enumerate() {
                        let (r, resp) = ui.allocate_exact_size(egui::vec2(visible - 2.0, 52.0), egui::Sense::click());
                        let painter = ui.painter_at(r);
                        if resp.hovered() {
                            painter.rect_filled(r, 0.0, t.hover);
                        }
                        if i > 0 {
                            painter.hline(r.x_range(), r.top(), egui::Stroke::new(1.0_f32, t.separator));
                        }
                        let tile = egui::Rect::from_center_size(egui::pos2(r.left() + 34.0, r.center().y), egui::vec2(32.0, 32.0));
                        painter.rect_filled(tile, 6.0, t.accent_soft);
                        icons::paint(ui, Icon::Drafting, tile.shrink(7.0), t.accent_text);
                        painter.text(egui::pos2(r.left() + 68.0, r.center().y), egui::Align2::LEFT_CENTER, name, medium_font(13.5), t.text);
                        let x0 = r.left() + r.width() * 0.36;
                        let room = r.right() - 150.0 - x0;
                        let mut path = p.display().to_string();
                        let font = egui::FontId::monospace(11.5);
                        let mut cut = 0;
                        while ui.fonts(|f| f.layout_no_wrap(path.clone(), font.clone(), t.text_dim).size().x) > room && path.chars().count() > 8 {
                            cut += 1;
                            let full: String = p.display().to_string();
                            path = format!("…{}", full.chars().skip(cut * 4).collect::<String>());
                        }
                        painter.text(egui::pos2(x0, r.center().y), egui::Align2::LEFT_CENTER, path, font, t.text_dim);
                        if let Some(m) = m {
                            painter.text(egui::pos2(r.right() - 16.0, r.center().y), egui::Align2::RIGHT_CENTER, ago(*m), egui::FontId::proportional(12.5), t.text_dim);
                        }
                        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            open = Some(p.clone());
                        }
                    }
                });
                if let Some(p) = open {
                    if p != self.project.dir {
                        if let Ok(pr) = Project::open(&p) {
                            self.switch_project(pr);
                        }
                    }
                    self.ws = Ws::Design;
                }
            });
        });
    }

    /// Opens a DXF in CADCraft through its MCP bridge (starting CADCraft when needed).
    fn open_in_cadcraft(&mut self, ctx: &egui::Context, dxf: PathBuf) {
        let (tx, rx) = std::sync::mpsc::channel();
        self.cad_rx = Some(rx);
        self.viewer.cad_status = "Opening in CADCraft…".into();
        let plugins = self.engine.plugins.clone();
        let mut settings = self.settings.clone();
        // the viewer button works even when the plugin is switched off for the agents
        if let Some(p) = settings.plugins.iter_mut().find(|p| p.name == "cadcraft") {
            p.enabled = true;
        }
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let msg = match plugins.ensure(&settings, "cadcraft").and_then(|c| c.call("open", serde_json::json!({"path": dxf.display().to_string()}))) {
                Ok(r) if !r.is_error => format!("Opened {} in CADCraft.", dxf.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()),
                Ok(r) => format!("CADCraft: {}", r.text),
                Err(e) => format!("CADCraft: {e}. If CADCraft is already open without its control port, quit it and try again (the plugin starts it with --control 7979)."),
            };
            let _ = tx.send(msg);
            ctx.request_repaint();
        });
    }

    fn shot_step(&mut self, ctx: &egui::Context) {
        let Some((dir, k, frames)) = self.shot.as_mut() else { return };
        if *k >= SHOT_ORDER.len() {
            return;
        }
        let shots: Vec<std::sync::Arc<egui::ColorImage>> = ctx.input(|i| i.events.iter().filter_map(|e| if let egui::Event::Screenshot { image, .. } = e { Some(image.clone()) } else { None }).collect());
        for img in shots {
            let name = format!("{:02}_{}", *k, format!("{:?}", SHOT_ORDER[*k]).to_lowercase());
            let _ = std::fs::create_dir_all(&*dir);
            let buf: Vec<u8> = img.pixels.iter().flat_map(|c| [c.r(), c.g(), c.b(), c.a()]).collect();
            if let Some(rgba) = image::RgbaImage::from_raw(img.size[0] as u32, img.size[1] as u32, buf) {
                let _ = rgba.save(dir.join(format!("{name}.png")));
            }
            *k += 1;
            *frames = 0;
            if *k >= SHOT_ORDER.len() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }
        }
        let k = *k;
        let first = *frames == 0;
        *frames += 1;
        let fire = *frames == 50;
        if first {
            let ws = SHOT_ORDER[k];
            self.ws = ws;
            self.chat_open = ws == Ws::Team;
            if ws == Ws::Standards {
                self.std_view.refresh();
            }
            if ws == Ws::Design {
                if let Some(f) = self.artifacts.iter().find(|a| a.kind == ArtifactKind::Drawing).and_then(|a| a.files.iter().find(|f| f.extension().is_some_and(|e| e == "svg")).cloned()) {
                    self.viewer.show(f);
                }
            }
            let mesh = self.project.dir.join("models/aedifex.axmesh");
            if ws == Ws::Output && mesh.is_file() {
                self.viewer.show(mesh);
            } else if ws == Ws::Output {
                if let Some(f) = self.artifacts.iter().find(|a| a.kind == ArtifactKind::Calculation).and_then(|a| a.files.iter().find(|f| f.extension().is_some_and(|e| e == "json")).cloned()) {
                    self.viewer.show(f);
                }
            }
        }
        if fire {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        ctx.request_repaint();
    }
}

/// A row of the home rail: its icon and its label.
fn rail_button(ui: &mut egui::Ui, icon: Icon, label: &str, on: bool) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 40.0), egui::Sense::click());
    let fill = if on { t.accent_soft } else if resp.hovered() { t.hover } else { Color32::TRANSPARENT };
    ui.painter().rect_filled(r, 8.0, fill);
    icons::paint(ui, icon, egui::Rect::from_center_size(egui::pos2(r.left() + 22.0, r.center().y), egui::Vec2::splat(17.0)), if on { t.accent_text } else { t.text_dim });
    ui.painter().text(egui::pos2(r.left() + 42.0, r.center().y), egui::Align2::LEFT_CENTER, label, if on { medium_font(13.5) } else { egui::FontId::proportional(13.5) }, if on { t.text } else { t.text_dim });
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

#[derive(Clone, Copy)]
enum Thumb {
    Empty,
    Beam,
    Footing,
    Wall,
}

const CONCRETE: Color32 = Color32::from_rgb(196, 200, 206);
const SAND: Color32 = Color32::from_rgb(224, 184, 94);
const CLAY: Color32 = Color32::from_rgb(124, 104, 56);
const STEEL: Color32 = Color32::from_rgb(194, 59, 46);

/// The little picture on a template card.
fn thumb(p: &egui::Painter, r: egui::Rect, th: Thumb, t: &Tokens) {
    let top = egui::CornerRadius { nw: 10, ne: 10, sw: 0, se: 0 };
    p.rect_filled(r, top, t.hover);
    let at = |x: f32, y: f32| egui::pos2(r.left() + r.width() * x, r.top() + r.height() * y);
    match th {
        Thumb::Empty => {
            let b = egui::Rect::from_center_size(r.center(), egui::vec2(r.width() * 0.66, r.height() * 0.6));
            p.rect_stroke(b, 6.0, egui::Stroke::new(1.5_f32, t.field_border), egui::StrokeKind::Inside);
            let s = egui::Stroke::new(3.0_f32, t.accent);
            p.line_segment([r.center() - egui::vec2(0.0, 22.0), r.center() + egui::vec2(0.0, 22.0)], s);
            p.line_segment([r.center() - egui::vec2(22.0, 0.0), r.center() + egui::vec2(22.0, 0.0)], s);
        }
        Thumb::Beam => {
            let sec = egui::Rect::from_min_max(at(0.38, 0.12), at(0.62, 0.88));
            p.rect_filled(sec, 2.0, CONCRETE);
            p.rect_stroke(sec.shrink(7.0), 2.0, egui::Stroke::new(1.6_f32, t.accent), egui::StrokeKind::Inside);
            for k in 0..3 {
                let x = sec.left() + 14.0 + k as f32 * (sec.width() - 28.0) / 2.0;
                p.circle_filled(egui::pos2(x, sec.bottom() - 14.0), 4.5, STEEL);
            }
            for k in 0..2 {
                let x = sec.left() + 14.0 + k as f32 * (sec.width() - 28.0);
                p.circle_filled(egui::pos2(x, sec.top() + 14.0), 3.0, STEEL);
            }
        }
        Thumb::Footing => {
            p.rect_filled(egui::Rect::from_min_max(at(0.0, 0.45), at(1.0, 0.72)), 0.0, SAND);
            p.rect_filled(egui::Rect::from_min_max(at(0.0, 0.72), r.right_bottom()), 0.0, CLAY);
            p.rect_filled(egui::Rect::from_min_max(at(0.36, 0.5), at(0.64, 0.62)), 1.0, CONCRETE);
            p.rect_filled(egui::Rect::from_min_max(at(0.46, 0.18), at(0.54, 0.5)), 1.0, CONCRETE);
            let s = egui::Stroke::new(2.0_f32, t.accent);
            p.line_segment([at(0.5, 0.04), at(0.5, 0.16)], s);
            p.add(egui::Shape::convex_polygon(vec![at(0.5, 0.18), at(0.48, 0.12), at(0.52, 0.12)], t.accent, egui::Stroke::NONE));
        }
        Thumb::Wall => {
            p.rect_filled(egui::Rect::from_min_max(at(0.0, 0.72), r.right_bottom()), 0.0, CLAY);
            p.add(egui::Shape::convex_polygon(vec![at(0.52, 0.16), at(1.0, 0.16), at(1.0, 0.72), at(0.52, 0.72)], SAND, egui::Stroke::NONE));
            p.rect_filled(egui::Rect::from_min_max(at(0.46, 0.14), at(0.53, 0.72)), 0.0, CONCRETE);
            p.rect_filled(egui::Rect::from_min_max(at(0.30, 0.72), at(0.78, 0.8)), 0.0, CONCRETE);
            p.rect_filled(egui::Rect::from_min_max(at(0.0, 0.62), at(0.46, 0.72)), 0.0, SAND.gamma_multiply(0.8));
        }
    }
}

fn ago(t: std::time::SystemTime) -> String {
    let s = std::time::SystemTime::now().duration_since(t).map(|d| d.as_secs()).unwrap_or(0);
    match s {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} minutes ago", s / 60),
        3600..=86_399 => format!("{} hours ago", s / 3600),
        _ => format!("{} days ago", s / 86_400),
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.shot_step(ctx);
        let saves: Vec<(String, PathBuf)> = self.chat.model_saves.drain(..).collect();
        for (plugin, path) in saves {
            if !self.run_models.contains(&plugin) {
                self.run_models.push(plugin.clone());
                // the first change to the BIM model in this task: show it in 3D beside the chat
                let mesh = path.with_extension("axmesh");
                if plugin == "aedifex" && mesh.is_file() {
                    self.viewer.show(mesh);
                }
            }
            // a window that follows its file shows the work as it happens
            if self.settings.open_models && th_harness::live::spec(&plugin).is_some_and(|s| s.follows_file) && !self.window_open(&plugin) {
                self.open_model_window(&plugin, false);
            }
        }
        if self.chat.poll() {
            self.chat.save(&self.project.dir);
            self.artifacts = self.project.artifacts();
            // the other programs show the finished model; an open window is left alone
            for plugin in std::mem::take(&mut self.run_models) {
                if self.settings.open_models && !self.window_open(&plugin) {
                    self.open_model_window(&plugin, false);
                }
            }
        }
        if let Some(p) = self.chat.open_model.take() {
            self.open_model_window(&p, true);
        }
        for (_, p) in self.chat.new_artifacts.drain(..) {
            ctx.forget_image(&format!("file://{}", p.display()));
            self.viewer.show(p);
            self.artifacts_checked = Instant::now() - Duration::from_secs(10);
        }
        if let Some(p) = self.chat.open_request.take() {
            self.viewer.show(p);
        }
        if let Some(dxf) = self.viewer.open_in_cad.take() {
            self.open_in_cadcraft(ctx, dxf);
        }
        if let Some(rx) = &self.cad_rx {
            if let Ok(msg) = rx.try_recv() {
                self.viewer.cad_status = msg;
                self.cad_rx = None;
            }
        }
        // files dropped on the window go to the chat
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        if !dropped.is_empty() {
            for p in dropped {
                self.chat.add_attachment(p);
            }
            if self.ws != Ws::Design {
                self.chat_open = true;
            }
        }
        if self.settings.dark != self.theme_dark {
            self.theme_dark = self.settings.dark;
            theme::apply(ctx, if self.settings.dark { Tokens::DARK } else { Tokens::LIGHT });
        }

        if self.ws == Ws::Home {
            self.home_ws(ctx);
        } else {
            self.top_bar(ctx);
            self.status_bar(ctx);
            // the assistant docked on the right of any step
            if self.chat_open && self.ws != Ws::Design {
                let t = Tokens::get(ctx);
                let max_w = (ctx.screen_rect().width() * 0.42).max(320.0);
                egui::SidePanel::right("assistant").resizable(true).default_width(420.0).min_width(300.0).max_width(max_w).frame(chrome::dock_frame(ctx)).show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Assistant").font(medium_font(14.0)));
                        ui.label(RichText::new(format!("{} · {}", self.team.name, self.settings.default_model)).small().color(t.text_dim));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("Close").clicked() {
                                self.chat_open = false;
                            }
                            if ui.small_button("Full view").clicked() {
                                self.ws = Ws::Design;
                            }
                        });
                    });
                    let colours = self.agent_colours();
                    let f = move |n: &str| colours.iter().find(|(k, _)| k == n).map(|(_, c)| *c).unwrap_or(Color32::from_rgb(120, 120, 130));
                    let engine = self.engine.clone();
                    self.chat.ui(ui, &engine, &f, false);
                });
            }
            match self.ws {
                Ws::Project => self.project_ws(ctx),
                Ws::Standards => self.standards_ws(ctx),
                Ws::Team => self.team_ws(ctx),
                Ws::Plugins => self.plugins_ws(ctx),
                Ws::Design => self.design_ws(ctx),
                Ws::Output => self.output_ws(ctx),
                Ws::Settings => self.settings_ws(ctx),
                Ws::Home => {}
            }
        }
        self.flush(false);
        if self.settings_dirty.is_some() || self.team_dirty.is_some() || self.project_dirty.is_some() {
            ctx.request_repaint_after(Duration::from_millis(900));
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.chat.stop();
        self.flush(true);
        self.chat.save(&self.project.dir);
    }
}
