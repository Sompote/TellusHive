//! The Bots workspace: the layer above the teams. A rail of bot pictures on the left, the
//! conversation with the chosen bot as a messenger in the middle, and its profile on the
//! right (picture, character, what it runs on, its files). A bot either fronts a saved team
//! or works alone as one agent; each has its own engine, conversation and folder, so bots
//! keep working while the engineer talks to another one.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText};
use th_harness::bots::{Bot, BotMode, CHARACTERS, LANGUAGES};
use th_harness::config::{Settings, TeamSpec};
use th_harness::skills::Skill;
use th_harness::{Artifact, Engine, Project};

use crate::chat::{BotLook, ChatView};
use crate::chrome::{self, hex_color, to_hex, tokens};
use crate::icons::{self, Icon};
use crate::theme::medium_font;

/// A round picture: the avatar file, or the initials on the bot's colour.
pub fn avatar(ui: &mut egui::Ui, name: &str, colour: Color32, path: Option<&Path>, size: f32) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::click());
    paint_avatar(ui, r, name, colour, path);
    resp
}

fn paint_avatar(ui: &egui::Ui, r: egui::Rect, name: &str, colour: Color32, path: Option<&Path>) {
    let size = r.width();
    if let Some(p) = path.filter(|p| p.is_file()) {
        egui::Image::new(format!("file://{}", p.display())).corner_radius(size / 2.0).paint_at(ui, r);
        return;
    }
    ui.painter().circle_filled(r.center(), size / 2.0, colour);
    ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, initials(name), medium_font(size * 0.38), Color32::WHITE);
}

/// Two letters for a name: the first letters of its first two words ("single-qwen" → SQ,
/// "Design office" → DO), or the first two letters of a single word ("single" → SI).
pub fn initials(name: &str) -> String {
    let words: Vec<&str> = name.split(|c: char| c.is_whitespace() || c == '-' || c == '_').filter(|w| !w.is_empty()).collect();
    let s: String = match words.as_slice() {
        [] => "?".into(),
        [one] => one.chars().take(2).collect(),
        [a, b, ..] => a.chars().take(1).chain(b.chars().take(1)).collect(),
    };
    s.to_uppercase()
}

fn look(b: &Bot) -> BotLook {
    BotLook { name: b.name.clone(), tagline: b.tagline.clone(), avatar: b.has_avatar().then(|| b.avatar_path()), color: hex_color(&b.color, Color32::from_rgb(123, 79, 214)) }
}

/// A bot that has been opened: its engine, conversation and project.
struct Slot {
    engine: Arc<Engine>,
    chat: ChatView,
    project: Project,
    artifacts: Vec<Artifact>,
    artifacts_at: Option<Instant>,
}

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Activity,
    Library,
    Setup,
}

/// The team the engineer talks to directly in the open project (the app's team chat).
pub struct Office {
    pub team: String,
    pub project: String,
    pub busy: bool,
}

pub struct BotsView {
    pub bots: Vec<Bot>,
    /// The design office is on screen instead of one of the bots.
    pub office: bool,
    /// A project a bot set up with its standards and plugins (the app reloads it if open).
    pub project_edited: Option<PathBuf>,
    slots: HashMap<String, Slot>,
    pub selected: Option<String>,
    profile: bool,
    tab: Tab,
    dirty: Option<Instant>,
    confirm_delete: bool,
    status: String,
    /// The saved teams: name, the lead's colour, number of agents.
    teams: Vec<(String, String, usize)>,
    teams_at: Option<Instant>,
    /// A team the engineer picked in the rail to talk to (the app switches to it).
    pub pick_team: Option<String>,
    /// The Teams section of the rail is open (hidden by default).
    teams_open: bool,
    /// "List models" of a bot's own API: (bot id, result).
    models: Option<(String, Result<Vec<String>, String>)>,
    models_rx: Option<std::sync::mpsc::Receiver<(String, Result<Vec<String>, String>)>>,
}

impl Default for BotsView {
    fn default() -> Self {
        let bots = Bot::list();
        let selected = bots.first().map(|b| b.id.clone());
        BotsView { office: bots.is_empty(), bots, project_edited: None, slots: HashMap::new(), selected, profile: false, tab: Tab::Activity, dirty: None, confirm_delete: false, status: String::new(), teams: vec![], teams_at: None, pick_team: None, teams_open: false, models: None, models_rx: None }
    }
}

impl BotsView {
    pub fn busy_count(&self) -> usize {
        self.slots.values().filter(|s| s.chat.busy()).count()
    }

    pub fn stop_all(&self) {
        for s in self.slots.values() {
            s.chat.stop();
        }
    }

    /// Saves every conversation and any unsaved bot.
    pub fn save_all(&mut self) {
        for s in self.slots.values() {
            s.chat.save(&s.project.dir);
        }
        self.flush(true);
    }

    /// The engines follow the app's settings (keys, providers, harness limits).
    pub fn sync_settings(&self, settings: &Settings) {
        let s = Bot::engine_settings(settings, &self.bots);
        for slot in self.slots.values() {
            if let Ok(mut e) = slot.engine.settings.write() {
                *e = s.clone();
            }
        }
    }

    /// Takes the workers' events into each bot's conversation, every frame, so a bot keeps
    /// working while another one is shown.
    pub fn poll_all(&mut self) {
        let mut synced = vec![];
        for (id, s) in self.slots.iter_mut() {
            if s.chat.poll() {
                s.chat.save(&s.project.dir);
                s.artifacts_at = None;
                // the bot may have set its project up from the chat: its bundle follows
                if let Ok(p) = Project::open(&s.project.dir) {
                    s.project.info = p.info.clone();
                    synced.push((id.clone(), p));
                }
            }
            if !s.chat.new_artifacts.is_empty() {
                s.chat.new_artifacts.clear();
                s.artifacts_at = None;
            }
            s.chat.model_saves.clear();
            if s.chat.open_model.take().is_some() {
                chrome::open_path(&s.project.dir.join("models"));
            }
            if let Some(p) = s.chat.open_request.take() {
                chrome::open_path(&p);
            }
        }
        for (id, p) in synced {
            if let Some(b) = self.bots.iter_mut().find(|b| b.id == id) {
                if b.take_bundle(&p) {
                    self.dirty = Some(Instant::now());
                    self.project_edited = Some(p.dir.clone());
                }
            }
        }
    }

    /// Files dropped on the window go to the bot on screen.
    pub fn add_attachment(&mut self, p: PathBuf) {
        if let Some(id) = self.selected.clone() {
            if let Some(s) = self.slots.get_mut(&id) {
                s.chat.add_attachment(p);
            }
        }
    }

    fn flush(&mut self, force: bool) {
        if self.dirty.is_some_and(|t| force || t.elapsed() > Duration::from_millis(700)) {
            for b in &self.bots {
                if let Err(e) = b.save() {
                    self.status = e;
                }
            }
            self.dirty = None;
        }
    }

    fn open_slot(&mut self, id: &str, base: &Arc<Engine>) -> Result<(), String> {
        if self.slots.contains_key(id) {
            return Ok(());
        }
        let b = self.bots.iter().find(|b| b.id == id).ok_or("no such bot")?;
        let project = b.project()?;
        let settings = Bot::engine_settings(&base.settings.read().map(|s| s.clone()).unwrap_or_default(), &self.bots);
        let engine = Arc::new(Engine::new(settings, b.team_spec(&self.bots), project.clone(), base.library.clone(), base.plugins.clone()));
        let mut chat = ChatView::new(ChatView::load(&project.dir));
        chat.bot = Some(look(b));
        self.slots.insert(id.to_string(), Slot { engine, chat, project, artifacts: vec![], artifacts_at: None });
        Ok(())
    }

    /// After the bot was edited: its team, project and look follow. A changed workspace
    /// folder needs a new engine, which waits until the bot is idle.
    fn apply(&mut self, id: &str) {
        self.dirty = Some(Instant::now());
        let Some(b) = self.bots.iter().find(|b| b.id == id).cloned() else { return };
        let Some(s) = self.slots.get_mut(id) else { return };
        s.chat.bot = Some(look(&b));
        if s.chat.busy() {
            self.status = "The changes apply from the next message.".into();
        }
        let want = match b.project() {
            Ok(p) => p,
            Err(e) => {
                self.status = e;
                return;
            }
        };
        if want.dir != s.project.dir {
            if s.chat.busy() {
                return;
            }
            s.chat.save(&s.project.dir);
            self.slots.remove(id);
            return;
        }
        self.project_edited = Some(want.dir.clone());
        s.project = want.clone();
        if let Ok(mut p) = s.engine.project.write() {
            *p = want;
        }
        // a bot's name, model, own API or calls change the bots that call it too
        let mut global = s.engine.settings.read().map(|x| x.clone()).unwrap_or_default();
        global.providers.retain(|p| !p.name.starts_with("bot:"));
        let es = Bot::engine_settings(&global, &self.bots);
        for (sid, slot) in self.slots.iter_mut() {
            if let Ok(mut e) = slot.engine.settings.write() {
                *e = es.clone();
            }
            if let Some(sb) = self.bots.iter().find(|x| &x.id == sid) {
                if let Ok(mut t) = slot.engine.team.write() {
                    *t = sb.team_spec(&self.bots);
                }
            }
        }
    }

    fn new_bot(&mut self, mode: BotMode, _settings: &Settings) {
        let n = self.bots.len() + 1;
        let (name, tagline, character) = match mode {
            BotMode::Solo => (format!("Bot {n}"), CHARACTERS[0].1, CHARACTERS[0].2),
            BotMode::Team => (format!("Manager {n}"), "Runs the design office", CHARACTERS[0].2),
        };
        let mut b = Bot::new(&name);
        b.mode = mode;
        b.tagline = tagline.to_string();
        b.character = character.to_string();
        let palette = ["#7b4fd6", "#2f74d0", "#2e9e4b", "#c25a35", "#1d8fb5", "#a87a1e", "#c23b2e"];
        b.color = palette[self.bots.len() % palette.len()].to_string();
        if mode == BotMode::Team {
            b.team = TeamSpec::list().first().map(|t| t.name.clone()).unwrap_or_else(|| "Design office".into());
        }
        if let Err(e) = b.save() {
            self.status = e;
            return;
        }
        self.selected = Some(b.id.clone());
        self.office = false;
        self.bots.push(b);
        self.profile = true;
        self.tab = Tab::Setup;
    }

    fn team_list(&mut self) -> Vec<(String, String, usize)> {
        if self.teams_at.is_none_or(|t| t.elapsed() > Duration::from_secs(3)) {
            self.teams = TeamSpec::list().into_iter().map(|t| (t.name.clone(), t.agent(&t.coordinator).map(|a| a.color.clone()).unwrap_or_default(), t.agents.len())).collect();
            self.teams_at = Some(Instant::now());
        }
        self.teams.clone()
    }

    fn team_names(&mut self) -> Vec<String> {
        self.team_list().into_iter().map(|t| t.0).collect()
    }

    // ------------------------------------------------------------------ the workspace

    /// Draws the rail and, for a bot, its chat and profile. Returns true when the design
    /// office is chosen, which the app draws beside the rail.
    pub fn ui(&mut self, ctx: &egui::Context, base: &Arc<Engine>, settings: &Settings, skills: &[Skill], current_project: &Path, office: &Office) -> bool {
        let t = crate::theme::Tokens::get(ctx);
        self.flush(false);
        if self.dirty.is_some() {
            ctx.request_repaint_after(Duration::from_millis(800));
        }
        // the rail
        let rail = egui::Frame::NONE.fill(t.chrome).inner_margin(egui::Margin { left: 10, right: 10, top: 14, bottom: 14 }).stroke(egui::Stroke::new(1.0_f32, t.separator));
        egui::SidePanel::left("bot_rail").exact_width(88.0).resizable(false).frame(rail).show(ctx, |ui| {
            egui::TopBottomPanel::bottom("bot_rail_add").frame(egui::Frame::NONE).show_separator_line(false).show_inside(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(6.0);
                    let add = icons::button(ui, Icon::Plus, "New bot", false);
                    let pop = ui.make_persistent_id("bot_new_menu");
                    if add.clicked() {
                        ui.memory_mut(|m| m.toggle_popup(pop));
                    }
                    egui::popup::popup_above_or_below_widget(ui, pop, &add, egui::AboveOrBelow::Above, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                        ui.set_min_width(260.0);
                        if ui.button("New bot that calls a team").on_hover_text("A project manager that hands the work to a team, led by the team's coordinator.").clicked() {
                            self.new_bot(BotMode::Team, settings);
                        }
                        if ui.button("New bot that works alone").on_hover_text("One agent with its own model and tools. It can call other bots later.").clicked() {
                            self.new_bot(BotMode::Solo, settings);
                        }
                    });
                });
            });
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.spacing_mut().item_spacing.y = 8.0;
                    let ids: Vec<String> = self.bots.iter().map(|b| b.id.clone()).collect();
                    for id in ids {
                        let Some(b) = self.bots.iter().find(|b| b.id == id) else { continue };
                        let on = !self.office && self.selected.as_deref() == Some(id.as_str());
                        let busy = self.slots.get(&id).is_some_and(|s| s.chat.busy());
                        let (r, resp) = ui.allocate_exact_size(egui::vec2(64.0, 64.0), egui::Sense::click());
                        let fill = if on { t.hover } else if resp.hovered() { t.hover.gamma_multiply(0.6) } else { Color32::TRANSPARENT };
                        ui.painter().rect_filled(r, 16.0, fill);
                        let a = egui::Rect::from_center_size(r.center(), egui::vec2(50.0, 50.0));
                        paint_avatar(ui, a, &b.name, hex_color(&b.color, t.accent), b.has_avatar().then(|| b.avatar_path()).as_deref());
                        // the badge: a team bot wears a small team mark, a working bot a pulsing dot
                        let badge = a.right_bottom() - egui::vec2(6.0, 6.0);
                        if busy {
                            let k = (ui.input(|i| i.time) * 3.0).sin() as f32 * 0.5 + 0.5;
                            ui.painter().circle_filled(badge, 7.5, t.chrome);
                            ui.painter().circle_filled(badge, 5.5, t.accent.gamma_multiply(0.55 + 0.45 * k));
                            ui.ctx().request_repaint_after(Duration::from_millis(60));
                        } else if b.calls_anyone() {
                            ui.painter().circle_filled(badge, 8.0, t.chrome);
                            ui.painter().circle_filled(badge, 6.5, Color32::from_rgb(232, 145, 40));
                            icons::paint(ui, Icon::Users, egui::Rect::from_center_size(badge, egui::vec2(9.0, 9.0)), Color32::WHITE);
                        }
                        let tip = format!("{}\n{}", b.name, if b.tagline.is_empty() { b.engine_line(settings, &self.bots) } else { b.tagline.clone() });
                        if resp.on_hover_text(tip).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            self.selected = Some(id.clone());
                            self.office = false;
                            self.confirm_delete = false;
                        }
                        if on {
                            ui.add_space(2.0);
                        }
                    }
                    // the saved teams, below the bots: talk to one directly
                    let teams = self.team_list();
                    if !teams.is_empty() {
                        if !self.bots.is_empty() {
                            let (r, _) = ui.allocate_exact_size(egui::vec2(48.0, 1.0), egui::Sense::hover());
                            ui.painter().hline(r.x_range(), r.center().y, egui::Stroke::new(1.0_f32, t.separator));
                        }
                        let label = format!("Teams {}", if self.teams_open { "▾" } else { "▸" });
                        let r = ui.add(egui::Label::new(RichText::new(label).size(11.0).color(if self.teams_open || self.office { t.text_dim } else { t.text_faint })).sense(egui::Sense::click()));
                        if r.on_hover_text(if self.teams_open { "Hide the teams" } else { "Show the teams, to talk to one directly" }).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            self.teams_open = !self.teams_open;
                        }
                    }
                    // hidden by default; the team on screen stays visible
                    let teams: Vec<(String, String, usize)> = if self.teams_open { teams } else { teams.into_iter().filter(|t| self.office && t.0 == office.team).collect() };
                    for (name, colour, n) in teams {
                        let on = self.office && name == office.team;
                        let (r, resp) = ui.allocate_exact_size(egui::vec2(64.0, 64.0), egui::Sense::click());
                        let fill = if on { t.hover } else if resp.hovered() { t.hover.gamma_multiply(0.6) } else { Color32::TRANSPARENT };
                        ui.painter().rect_filled(r, 16.0, fill);
                        let a = egui::Rect::from_center_size(r.center(), egui::vec2(50.0, 50.0));
                        ui.painter().rect_filled(a, 14.0, hex_color(&colour, t.accent));
                        ui.painter().text(a.center(), egui::Align2::CENTER_CENTER, initials(&name), medium_font(17.0), Color32::WHITE);
                        if office.busy && name == office.team {
                            let badge = a.right_bottom() - egui::vec2(4.0, 4.0);
                            let k = (ui.input(|i| i.time) * 3.0).sin() as f32 * 0.5 + 0.5;
                            ui.painter().circle_filled(badge, 7.5, t.chrome);
                            ui.painter().circle_filled(badge, 5.5, t.ok.gamma_multiply(0.55 + 0.45 * k));
                            ui.ctx().request_repaint_after(Duration::from_millis(60));
                        }
                        let tip = format!("{name}\n{n} agents · talk to the team directly on the project '{}'", office.project);
                        if resp.on_hover_text(tip).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            self.office = true;
                            self.confirm_delete = false;
                            if name != office.team {
                                self.pick_team = Some(name.clone());
                            }
                        }
                    }
                });
            });
        });

        if self.office {
            return true;
        }
        let Some(id) = self.selected.clone().filter(|id| self.bots.iter().any(|b| &b.id == id)) else {
            self.empty_ui(ctx, settings);
            return false;
        };
        if let Err(e) = self.open_slot(&id, base) {
            egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.field).inner_margin(40)).show(ctx, |ui| {
                ui.label(RichText::new(format!("This bot could not start: {e}")).color(t.error));
            });
            return false;
        }

        if self.profile {
            let max_w = (ctx.screen_rect().width() * 0.4).max(340.0);
            egui::SidePanel::right("bot_profile").resizable(true).default_width(380.0).min_width(330.0).max_width(max_w).frame(egui::Frame::NONE.fill(t.field).stroke(egui::Stroke::new(1.0_f32, t.separator)).inner_margin(egui::Margin { left: 20, right: 16, top: 12, bottom: 12 })).show(ctx, |ui| {
                self.profile_ui(ui, &id, settings, skills, base, current_project);
            });
        }

        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.field)).show(ctx, |ui| {
            let Some(b) = self.bots.iter().find(|b| b.id == id).cloned() else { return };
            // the name chip, top right: opens the profile
            egui::TopBottomPanel::top("bot_head").frame(egui::Frame::NONE.inner_margin(egui::Margin { left: 16, right: 16, top: 10, bottom: 4 })).show_separator_line(false).show_inside(ui, |ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let galley = ui.painter().layout_no_wrap(b.name.clone(), medium_font(14.0), t.text);
                    let w = galley.size().x + 56.0;
                    let (r, resp) = ui.allocate_exact_size(egui::vec2(w, 40.0), egui::Sense::click());
                    let fill = if self.profile || resp.hovered() { t.hover } else { t.card };
                    ui.painter().rect(r, 20.0, fill, egui::Stroke::new(1.0_f32, t.separator), egui::StrokeKind::Inside);
                    let a = egui::Rect::from_center_size(egui::pos2(r.left() + 22.0, r.center().y), egui::vec2(28.0, 28.0));
                    paint_avatar(ui, a, &b.name, hex_color(&b.color, t.accent), b.has_avatar().then(|| b.avatar_path()).as_deref());
                    ui.painter().galley(egui::pos2(r.left() + 44.0, r.center().y - galley.size().y / 2.0), galley, t.text);
                    if resp.on_hover_text(if self.profile { "Close the profile" } else { "Profile: picture, character, model, files" }).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        self.profile = !self.profile;
                    }
                    ui.add_space(8.0);
                    ui.label(RichText::new(b.engine_line(settings, &self.bots)).size(12.0).color(t.text_faint));
                    if !self.status.is_empty() {
                        ui.add_space(12.0);
                        ui.label(RichText::new(&self.status).size(12.0).color(t.text_dim));
                    }
                });
            });
            let Some(s) = self.slots.get_mut(&id) else { return };
            let colours: Vec<(String, Color32)> = s.engine.team.read().map(|tm| tm.agents.iter().map(|a| (a.name.clone(), hex_color(&a.color, Color32::from_rgb(47, 116, 208)))).collect()).unwrap_or_default();
            let f = move |n: &str| colours.iter().find(|(k, _)| k == n).map(|(_, c)| *c).unwrap_or(Color32::from_rgb(120, 120, 130));
            let engine = s.engine.clone();
            s.chat.ui(ui, &engine, &f, true);
        });
        false
    }

    /// No bot yet: the two ways to make one.
    fn empty_ui(&mut self, ctx: &egui::Context, settings: &Settings) {
        let t = crate::theme::Tokens::get(ctx);
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.field).inner_margin(40)).show(ctx, |ui| {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                ui.set_max_width(720.0);
                ui.label(RichText::new("Your bots").font(medium_font(26.0)));
                ui.add_space(4.0);
                ui.label(RichText::new("A bot is your project manager. It talks with you, works alone or calls other bots and teams, and sets up its project with its own standards and plugins.").size(14.0).color(t.text_dim));
                ui.add_space(24.0);
                ui.columns(2, |cols| {
                    let cards = [
                        (BotMode::Team, Icon::Users, "Calls a team", "A project manager for a team. You talk to one face; the team lead and the designer, checker, drafter and the others do the work behind it."),
                        (BotMode::Solo, Icon::Chat, "Works alone", "One agent with its own model and tools, for quick questions, checks and calculations. It can call other bots later."),
                    ];
                    for (k, (mode, icon, title, about)) in cards.into_iter().enumerate() {
                        let ui = &mut cols[k];
                        let r = egui::Frame::NONE.fill(t.card).stroke(egui::Stroke::new(1.0_f32, t.separator)).corner_radius(16).inner_margin(20).show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            ui.set_min_height(140.0);
                            let (r, _) = ui.allocate_exact_size(egui::vec2(28.0, 28.0), egui::Sense::hover());
                            icons::paint(ui, icon, r, t.accent_text);
                            ui.add_space(6.0);
                            ui.label(RichText::new(title).font(medium_font(16.0)));
                            ui.label(RichText::new(about).size(13.0).color(t.text_dim));
                        });
                        if r.response.interact(egui::Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            self.new_bot(mode, settings);
                        }
                    }
                });
            });
        });
    }

    // ------------------------------------------------------------------ the profile

    fn profile_ui(&mut self, ui: &mut egui::Ui, id: &str, settings: &Settings, skills: &[Skill], base: &Arc<Engine>, current_project: &Path) {
        let t = tokens(ui);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
            if icons::button(ui, Icon::Close, "Close", false).clicked() {
                self.profile = false;
            }
        });
        let Some(idx) = self.bots.iter().position(|b| b.id == id) else { return };
        let b = self.bots[idx].clone();
        ui.vertical_centered(|ui| {
            let resp = avatar(ui, &b.name, hex_color(&b.color, t.accent), b.has_avatar().then(|| b.avatar_path()).as_deref(), 112.0);
            if resp.hovered() {
                ui.painter().circle_filled(resp.rect.center(), 56.0, Color32::from_black_alpha(90));
                ui.painter().text(resp.rect.center(), egui::Align2::CENTER_CENTER, "Change", medium_font(13.0), Color32::WHITE);
            }
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                self.pick_avatar(ui.ctx(), idx);
            }
            ui.add_space(8.0);
            ui.label(RichText::new(&b.name).font(medium_font(22.0)));
            if !b.tagline.is_empty() {
                ui.label(RichText::new(&b.tagline).size(13.0).color(t.text_dim));
            }
            ui.add_space(10.0);
            let mut tab = self.tab;
            pill_tabs(ui, &mut tab, &[(Tab::Activity, "Activity"), (Tab::Library, "Library"), (Tab::Setup, "Setup")]);
            self.tab = tab;
        });
        ui.add_space(10.0);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| match self.tab {
            Tab::Activity => self.activity_ui(ui, id, settings),
            Tab::Library => self.library_ui(ui, id),
            Tab::Setup => {
                if self.setup_ui(ui, idx, settings, skills, base, current_project) {
                    self.apply(id);
                }
            }
        });
    }

    fn pick_avatar(&mut self, ctx: &egui::Context, idx: usize) {
        let Some(f) = rfd::FileDialog::new().add_filter("Picture", &["png", "jpg", "jpeg", "webp"]).pick_file() else { return };
        let b = &self.bots[idx];
        match b.set_avatar(&f) {
            Ok(()) => {
                ctx.forget_image(&format!("file://{}", b.avatar_path().display()));
                let id = b.id.clone();
                self.apply(&id);
            }
            Err(e) => self.status = format!("Picture: {e}"),
        }
    }

    fn activity_ui(&mut self, ui: &mut egui::Ui, id: &str, settings: &Settings) {
        let t = tokens(ui);
        let Some(b) = self.bots.iter().find(|b| b.id == id).cloned() else { return };
        let Some(s) = self.slots.get_mut(id) else { return };
        section_title(ui, "Now");
        card(ui, |ui| {
            if s.chat.busy() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    let secs = s.chat.busy_since.map(|b| b.elapsed().as_secs()).unwrap_or(0);
                    ui.label(RichText::new(format!("Working · {}:{:02}", secs / 60, secs % 60)).font(medium_font(14.0)));
                });
                ui.label(RichText::new(s.chat.current_activity()).size(13.0).color(t.text_dim));
                if ui.add(egui::Button::new(RichText::new("Stop").color(t.error)).corner_radius(8)).clicked() {
                    s.chat.stop();
                }
            } else {
                ui.label(RichText::new("Idle").font(medium_font(14.0)));
                let n = s.chat.saved.entries.iter().filter(|e| matches!(e, crate::chat::Entry::User { .. })).count();
                ui.label(RichText::new(if n == 0 { "No messages yet".to_string() } else { format!("{n} messages from you in this conversation") }).size(13.0).color(t.text_dim));
            }
        });
        section_title(ui, "Who works");
        card(ui, |ui| {
            ui.label(RichText::new(b.engine_line(settings, &self.bots)).size(14.0));
            let team = s.engine.team.read().map(|tm| tm.clone()).unwrap_or_default();
            if team.agents.len() > 1 {
                ui.add_space(4.0);
                // the chain of command, one level of indent per hand-over
                let mut rows: Vec<(usize, String)> = vec![];
                let mut stack = vec![(0usize, team.coordinator.clone())];
                let mut seen = std::collections::HashSet::new();
                while let Some((d, id)) = stack.pop() {
                    if !seen.insert(id.clone()) {
                        continue;
                    }
                    rows.push((d, id.clone()));
                    for (f, to) in team.links.iter().rev() {
                        if *f == id {
                            stack.push((d + 1, to.clone()));
                        }
                    }
                }
                for (d, id) in rows {
                    let Some(a) = team.agent(&id) else { continue };
                    ui.horizontal(|ui| {
                        ui.add_space(16.0 * d as f32);
                        let (r, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                        ui.painter().circle_filled(r.center(), 4.0, hex_color(&a.color, t.accent));
                        ui.label(RichText::new(&a.name).size(12.5).color(if d == 0 { t.text } else { t.text_dim }));
                    });
                }
            }
        });
        section_title(ui, "Project");
        card(ui, |ui| {
            let own = b.project.trim().is_empty();
            ui.label(RichText::new(if own { "Its own folder".to_string() } else { format!("Project: {}", s.project.info.name) }).size(14.0));
            ui.label(RichText::new(chrome::display_path(&s.project.dir).display().to_string()).size(11.5).color(t.text_faint));
            ui.horizontal(|ui| {
                if ui.button("Show in Finder").clicked() {
                    chrome::open_path(&s.project.dir);
                }
                if ui.add_enabled(!s.chat.busy(), egui::Button::new("New conversation")).on_hover_text("Clears this chat; the old conversation stays in the sessions folder").clicked() {
                    s.chat.clear();
                    s.engine.new_conversation();
                    s.chat.save(&s.project.dir);
                }
            });
        });
        let (p, c) = s.chat.saved.tokens;
        if p + c > 0 {
            section_title(ui, "Usage");
            card(ui, |ui| {
                ui.label(RichText::new(format!("{} tokens in · {} out", crate::chat::fmt_tokens(p), crate::chat::fmt_tokens(c))).size(13.0).color(t.text_dim));
            });
        }
    }

    fn library_ui(&mut self, ui: &mut egui::Ui, id: &str) {
        let t = tokens(ui);
        let Some(s) = self.slots.get_mut(id) else { return };
        if s.artifacts_at.is_none_or(|a| a.elapsed() > Duration::from_secs(4)) {
            s.artifacts = s.project.artifacts();
            s.artifacts_at = Some(Instant::now());
        }
        if s.artifacts.is_empty() {
            ui.add_space(20.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("Nothing here yet").font(medium_font(14.0)));
                ui.label(RichText::new("Calculations, drawings, reports and pictures the bot makes, and the files you send it, appear here.").size(12.5).color(t.text_dim));
            });
            return;
        }
        let mut last = String::new();
        for a in &s.artifacts {
            let kind = a.kind.label().to_string();
            if kind != last {
                section_title(ui, &kind);
                last = kind;
            }
            let icon = match a.kind {
                th_harness::ArtifactKind::Calculation => Icon::Calculator,
                th_harness::ArtifactKind::Drawing => Icon::Drafting,
                th_harness::ArtifactKind::Report => Icon::FileText,
                th_harness::ArtifactKind::Image => Icon::Picture,
                th_harness::ArtifactKind::Attachment => Icon::Paperclip,
                th_harness::ArtifactKind::Work => Icon::Edit,
            };
            let r = card(ui, |ui| {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
                    icons::paint(ui, icon, r, t.accent_text);
                    ui.label(RichText::new(&a.name).size(13.0));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(ago(a.modified)).size(11.5).color(t.text_faint));
                    });
                });
            });
            if r.interact(egui::Sense::click()).on_hover_text("Open").on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                if let Some(f) = a.files.iter().find(|f| f.extension().is_some_and(|e| e == "pdf")).or(a.files.first()) {
                    chrome::open_path(f);
                }
            }
        }
    }

    /// The bot's set-up. Returns true when something changed.
    fn setup_ui(&mut self, ui: &mut egui::Ui, idx: usize, settings: &Settings, skills: &[Skill], base: &Arc<Engine>, current_project: &Path) -> bool {
        let t = tokens(ui);
        let teams = self.team_names();
        let others: Vec<(String, String, Color32, Option<PathBuf>)> = self.bots.iter().map(|o| (o.id.clone(), o.name.clone(), hex_color(&o.color, t.accent), o.has_avatar().then(|| o.avatar_path()))).collect();
        let busy = self.slots.get(&self.bots[idx].id).is_some_and(|s| s.chat.busy());
        let mut pick_avatar = false;
        let mut changed_avatar = false;
        let mut delete = false;
        let before = self.bots[idx].clone();
        {
            let b = &mut self.bots[idx];
            section_title(ui, "Picture");
            ui.horizontal(|ui| {
                if ui.button("Choose picture…").clicked() {
                    pick_avatar = true;
                }
                if b.has_avatar() && ui.button("Use initials").clicked() {
                    b.remove_avatar();
                    ui.ctx().forget_image(&format!("file://{}", b.avatar_path().display()));
                    changed_avatar = true;
                }
                let mut c = hex_color(&b.color, t.accent);
                if ui.color_edit_button_srgba(&mut c).on_hover_text("Colour of the initials and the replies' dot").changed() {
                    b.color = to_hex(c);
                }
            });

            section_title(ui, "Name");
            ui.add(egui::TextEdit::singleline(&mut b.name).desired_width(f32::INFINITY));
            ui.label(RichText::new("Tagline").size(12.0).color(t.text_dim));
            ui.add(egui::TextEdit::singleline(&mut b.tagline).hint_text("one line under the name").desired_width(f32::INFINITY));

            section_title(ui, "Character");
            egui::ComboBox::from_id_salt("bot_character").selected_text("Start from a character…").width(ui.available_width()).show_ui(ui, |ui| {
                for (label, tag, text) in CHARACTERS {
                    if ui.selectable_label(false, format!("{label}  ·  {tag}")).clicked() {
                        b.character = text.to_string();
                        b.tagline = tag.to_string();
                    }
                }
            });
            ui.add(egui::TextEdit::multiline(&mut b.character).hint_text("Who the bot is and how it talks: its background, tone and habits").desired_rows(5).desired_width(f32::INFINITY));
            ui.horizontal(|ui| {
                chrome::row_label(ui, "Language");
                let shown = if b.language.is_empty() || b.language == "auto" { "Auto-detect".to_string() } else { b.language.clone() };
                egui::ComboBox::from_id_salt("bot_lang").selected_text(shown).width(170.0).show_ui(ui, |ui| {
                    for l in LANGUAGES {
                        ui.selectable_value(&mut b.language, l.to_string(), if *l == "auto" { "Auto-detect" } else { l });
                    }
                });
            });

            section_title(ui, "Model");
            ui.label(RichText::new("What it does").size(12.0).color(t.text_dim));
            ui.add(egui::TextEdit::multiline(&mut b.instructions).hint_text("Empty = manage the work: agree the scope, hand tasks to whom it calls, check and report (or do the work itself when it calls no one)").desired_rows(3).desired_width(f32::INFINITY));
            let mut own = usize::from(b.own_api.is_some());
            chrome::segmented(ui, &["Shared provider", "Own API"], &mut own);
            if own == 1 && b.own_api.is_none() {
                b.own_api = Some(th_llm::Provider { name: "own".into(), ..th_llm::Provider::openai() });
                b.model.clear();
            } else if own == 0 && b.own_api.is_some() {
                b.own_api = None;
                b.model.clear();
            }
            ui.add_space(2.0);
            if let Some(p) = b.own_api.as_mut() {
                chrome::hint(ui, "This bot calls its own LLM API with its own key; the other bots and teams are not affected. The key is kept in the bot's file, readable only by you.");
                ui.horizontal(|ui| {
                    chrome::row_label(ui, "Service");
                    let presets = [("OpenAI", th_llm::Provider::openai()), ("OpenRouter", th_llm::Provider::openrouter()), ("DeepSeek", th_llm::Provider::deepseek()), ("Ollama (this computer)", th_llm::Provider::ollama()), ("vLLM server", th_llm::Provider::vllm())];
                    let current = presets.iter().find(|(_, x)| x.base_url == p.base_url).map(|(l, _)| l.to_string()).unwrap_or_else(|| "Other (OpenAI-compatible)".into());
                    egui::ComboBox::from_id_salt("bot_api_preset").selected_text(current).width(220.0).show_ui(ui, |ui| {
                        for (label, preset) in &presets {
                            if ui.selectable_label(p.base_url == preset.base_url, *label).clicked() {
                                let key = std::mem::take(&mut p.api_key);
                                *p = th_llm::Provider { name: "own".into(), api_key: key, ..preset.clone() };
                            }
                        }
                    });
                });
                ui.horizontal(|ui| {
                    chrome::row_label(ui, "Base URL");
                    ui.add_sized([ui.available_width(), 26.0], egui::TextEdit::singleline(&mut p.base_url).hint_text("https://…/v1").vertical_align(egui::Align::Center));
                });
                ui.horizontal(|ui| {
                    chrome::row_label(ui, "API key");
                    let hint = if p.key_env.is_empty() { "the key".to_string() } else { format!("the key, or leave empty to use ${}", p.key_env) };
                    ui.add_sized([ui.available_width(), 26.0], egui::TextEdit::singleline(&mut p.api_key).password(true).hint_text(hint).vertical_align(egui::Align::Center));
                });
                ui.horizontal(|ui| {
                    chrome::row_label(ui, "");
                    ui.checkbox(&mut p.vision, "Models accept images");
                    let ready = p.ready() && !p.unconfigured();
                    ui.label(RichText::new(if ready { "ready" } else { "needs a key" }).size(11.5).color(if ready { t.ok } else { t.warn }));
                    if ui.add_enabled(self.models_rx.is_none(), egui::Button::new("List models")).on_hover_text("Asks the API which models it serves, which also checks the URL and the key").clicked() {
                        let (tx, rx) = std::sync::mpsc::channel();
                        self.models_rx = Some(rx);
                        let pc = p.clone();
                        let bid = b.id.clone();
                        let ctx = ui.ctx().clone();
                        std::thread::spawn(move || {
                            let _ = tx.send((bid, th_llm::list_models(&pc)));
                            ctx.request_repaint();
                        });
                    }
                    if self.models_rx.is_some() {
                        ui.spinner();
                    }
                });
            } else {
                ui.horizontal(|ui| {
                    chrome::row_label(ui, "Provider");
                    let label = if b.provider.is_empty() { format!("default ({})", settings.default_provider) } else { b.provider.clone() };
                    egui::ComboBox::from_id_salt("bot_provider").selected_text(label).width(190.0).show_ui(ui, |ui| {
                        ui.selectable_value(&mut b.provider, String::new(), "default");
                        for p in &settings.providers {
                            ui.selectable_value(&mut b.provider, p.name.clone(), &p.name);
                        }
                    });
                });
            }
            let default_model = b.own_api.as_ref().map(|p| p.default_model.clone()).filter(|m| !m.is_empty());
            ui.horizontal(|ui| {
                chrome::row_label(ui, "Model");
                let hint = default_model.map(|m| format!("empty = {m}")).unwrap_or_else(|| "empty = provider default".into());
                ui.add_sized([ui.available_width(), 26.0], egui::TextEdit::singleline(&mut b.model).hint_text(hint).vertical_align(egui::Align::Center));
            });
            if let Some(rx) = &self.models_rx {
                if let Ok(r) = rx.try_recv() {
                    self.models = Some(r);
                    self.models_rx = None;
                }
            }
            match &self.models {
                Some((bid, Ok(list))) if *bid == b.id && b.own_api.is_some() => {
                    ui.horizontal(|ui| {
                        chrome::row_label(ui, "");
                        egui::ComboBox::from_id_salt("bot_api_models").selected_text(format!("{} models: pick one", list.len())).height(320.0).width(260.0).show_ui(ui, |ui| {
                            for m in list {
                                if ui.selectable_label(b.model == *m, m).clicked() {
                                    b.model = m.clone();
                                }
                            }
                        });
                    });
                }
                Some((bid, Err(e))) if *bid == b.id && b.own_api.is_some() => {
                    ui.label(RichText::new(e).size(11.5).color(t.error));
                }
                _ => {}
            }
            ui.horizontal(|ui| {
                chrome::row_label(ui, "Thinking");
                let shown = if b.thinking.is_empty() { "provider default".to_string() } else { b.thinking.clone() };
                egui::ComboBox::from_id_salt("bot_thinking").selected_text(shown).width(190.0).show_ui(ui, |ui| {
                    for (v, l) in [("", "provider default"), ("off", "off (fastest)"), ("low", "low"), ("medium", "medium"), ("high", "high")] {
                        ui.selectable_value(&mut b.thinking, v.to_string(), l);
                    }
                });
            });
            ui.horizontal(|ui| {
                chrome::row_label(ui, "Temperature");
                ui.add(egui::Slider::new(&mut b.temperature, 0.0..=1.0));
            });
            ui.horizontal(|ui| {
                chrome::row_label(ui, "Max turns");
                ui.add_sized([70.0, 26.0], egui::DragValue::new(&mut b.max_turns).range(1..=200));
            });
            egui::CollapsingHeader::new(RichText::new(format!("Built-in tools · {}", if b.tools.iter().any(|x| x == "*") { "all".to_string() } else { b.tools.len().to_string() })).font(medium_font(13.0))).id_salt("bot_tools").show(ui, |ui| {
                let mut all = b.tools.iter().any(|x| x == "*");
                if chrome::switch_row(ui, "All tools", &mut all).changed() {
                    b.tools = if all { vec!["*".into()] } else { th_harness::bots::SOLO_TOOLS.iter().map(|s| s.to_string()).collect() };
                }
                if !all {
                    ui.columns(2, |cols| {
                        for (k, spec) in th_harness::tools::builtin_specs().into_iter().enumerate() {
                            let ui = &mut cols[k % 2];
                            let mut on = b.tools.contains(&spec.name);
                            if ui.checkbox(&mut on, RichText::new(&spec.name).size(12.0)).on_hover_text(&spec.description).changed() {
                                if on {
                                    b.tools.push(spec.name.clone());
                                } else {
                                    b.tools.retain(|x| x != &spec.name);
                                }
                            }
                        }
                    });
                }
            });
            egui::CollapsingHeader::new(RichText::new(format!("Skills · {} found", skills.len())).font(medium_font(13.0))).id_salt("bot_skills").show(ui, |ui| {
                let mut all = b.skills.iter().any(|x| x == "*");
                if chrome::switch_row(ui, "All skills", &mut all).changed() {
                    b.skills = if all { vec!["*".into()] } else { vec![] };
                }
                if !all {
                    for s in skills {
                        let mut on = b.skills.contains(&s.name);
                        if ui.checkbox(&mut on, RichText::new(&s.name).size(12.5)).on_hover_text(&s.description).changed() {
                            if on {
                                b.skills.push(s.name.clone());
                            } else {
                                b.skills.retain(|x| x != &s.name);
                            }
                        }
                    }
                }
            });
            chrome::switch_row(ui, "May use Claude Code / Codex", &mut b.can_use_external);

            section_title(ui, "Calls");
            chrome::hint(ui, "Whom the bot hands work to. A team joins through its coordinator, who leads it like another bot. With nothing to call, the bot works alone.");
            let mut team_on = b.mode == BotMode::Team;
            ui.horizontal(|ui| {
                if chrome::switch(ui, &mut team_on).changed() {
                    b.mode = if team_on { BotMode::Team } else { BotMode::Solo };
                }
                ui.label("A team");
                ui.add_enabled_ui(team_on, |ui| {
                    egui::ComboBox::from_id_salt("bot_team").selected_text(&b.team).width(190.0).show_ui(ui, |ui| {
                        for n in &teams {
                            ui.selectable_value(&mut b.team, n.clone(), n);
                        }
                    });
                });
            });
            let others: Vec<(String, String, Color32, Option<PathBuf>)> = others.iter().filter(|o| o.0 != b.id).cloned().collect();
            if others.is_empty() {
                chrome::hint(ui, "Other bots appear here once you make them.");
            }
            for (oid, oname, ocol, oav) in &others {
                ui.horizontal(|ui| {
                    let mut on = b.calls.contains(oid);
                    if chrome::switch(ui, &mut on).changed() {
                        if on {
                            b.calls.push(oid.clone());
                        } else {
                            b.calls.retain(|x| x != oid);
                        }
                    }
                    let (r, _) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::hover());
                    paint_avatar(ui, r, oname, *ocol, oav.as_deref());
                    ui.label(oname);
                });
            }

            section_title(ui, "Standards and plugins");
            chrome::hint(ui, "The bot's bundle. It sets its project up with these, so every agent it calls cites these standards and drives these programs.");
            let lib = base.library.list();
            egui::CollapsingHeader::new(RichText::new(format!("Standards · {} of {}", b.standards.len(), lib.len())).font(medium_font(13.0))).id_salt("bot_std").show(ui, |ui| {
                if lib.is_empty() {
                    chrome::hint(ui, "The standards library is empty: import standards under Project › Standards.");
                }
                for m in &lib {
                    let mut on = b.standards.contains(&m.id);
                    if ui.checkbox(&mut on, RichText::new(format!("{}  {}", m.code, m.name)).size(12.5)).changed() {
                        if on {
                            b.standards.push(m.id.clone());
                        } else {
                            b.standards.retain(|x| x != &m.id);
                        }
                    }
                }
            });
            let enabled: Vec<(String, String)> = settings.plugins.iter().filter(|p| p.enabled).map(|p| (p.name.clone(), p.description.clone())).collect();
            let all_on = b.plugins.iter().any(|x| x == "*");
            let used = if all_on { enabled.len() } else { enabled.iter().filter(|(n, _)| b.plugins.contains(n)).count() };
            egui::CollapsingHeader::new(RichText::new(format!("Plugins · {used} of {}", enabled.len())).font(medium_font(13.0))).id_salt("bot_plug").show(ui, |ui| {
                if enabled.is_empty() {
                    chrome::hint(ui, "No plugins switched on: add MCP programs under Project › Plugins.");
                }
                for (name, about) in &enabled {
                    let mut on = all_on || b.plugins.contains(name);
                    if chrome::switch_row(ui, name, &mut on).on_hover_text(about).changed() {
                        let mut list: Vec<String> = if all_on { enabled.iter().map(|(n, _)| n.clone()).collect() } else { b.plugins.clone() };
                        if on {
                            list.push(name.clone());
                        } else {
                            list.retain(|x| x != name);
                        }
                        b.plugins = list;
                    }
                }
            });

            section_title(ui, "Project");
            let own = b.project.trim().is_empty();
            ui.label(RichText::new(if own { "Its own folder".to_string() } else { b.project.clone() }).size(12.5).color(t.text_dim));
            ui.horizontal_wrapped(|ui| {
                if !own && ui.add_enabled(!busy, egui::Button::new("Use its own folder")).clicked() {
                    b.project.clear();
                }
                if ui.add_enabled(!busy, egui::Button::new("Use the open project")).on_hover_text(current_project.display().to_string()).clicked() {
                    b.project = current_project.display().to_string();
                }
                if ui.add_enabled(!busy, egui::Button::new("Choose project…")).clicked() {
                    if let Some(d) = rfd::FileDialog::new().set_directory(&settings.projects_dir).pick_folder() {
                        b.project = d.display().to_string();
                    }
                }
            });
            if busy {
                chrome::hint(ui, "The project can change when the bot is idle.");
            } else if !own {
                chrome::hint(ui, "The bot's standards and plugins replace the project's own.");
            }

            ui.add_space(14.0);
            ui.separator();
            if self.confirm_delete {
                ui.label(RichText::new(format!("Delete {}? Its conversation and own folder go too; a linked project is kept.", b.name)).color(t.error));
                ui.horizontal(|ui| {
                    if ui.add(egui::Button::new(RichText::new("Delete").color(t.error))).clicked() {
                        delete = true;
                    }
                    if ui.button("Keep").clicked() {
                        self.confirm_delete = false;
                    }
                });
            } else if ui.add_enabled(!busy, egui::Button::new(RichText::new("Delete bot").color(t.error)).frame(false)).clicked() {
                self.confirm_delete = true;
            }
        }
        if pick_avatar {
            self.pick_avatar(ui.ctx(), idx);
        }
        if delete {
            let b = self.bots.remove(idx);
            self.slots.remove(&b.id);
            if let Err(e) = b.delete() {
                self.status = e;
            }
            self.selected = self.bots.first().map(|b| b.id.clone());
            self.office = self.selected.is_none();
            self.confirm_delete = false;
            self.profile = false;
            return false;
        }
        if self.bots[idx].name.trim().is_empty() {
            self.bots[idx].name = before.name.clone();
        }
        self.bots[idx] != before || changed_avatar
    }
}

fn section_title(ui: &mut egui::Ui, s: &str) {
    let t = tokens(ui);
    ui.add_space(12.0);
    ui.label(RichText::new(s).size(14.0).color(t.text_dim));
    ui.add_space(4.0);
}

/// A rounded white card, as in the profile.
fn card(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) -> egui::Response {
    let t = tokens(ui);
    egui::Frame::NONE
        .fill(t.card)
        .stroke(egui::Stroke::new(1.0_f32, t.separator))
        .corner_radius(16)
        .inner_margin(egui::Margin::symmetric(16, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        })
        .response
}

/// Text tabs; the current one sits on a grey pill.
fn pill_tabs(ui: &mut egui::Ui, current: &mut Tab, tabs: &[(Tab, &str)]) {
    let t = tokens(ui);
    let font = egui::FontId::proportional(15.0);
    let widths: Vec<f32> = tabs.iter().map(|(_, l)| ui.painter().layout_no_wrap(l.to_string(), font.clone(), t.text).size().x + 28.0).collect();
    let total: f32 = widths.iter().sum::<f32>() + 6.0 * (tabs.len() - 1) as f32;
    ui.allocate_ui_with_layout(egui::vec2(total, 36.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        for ((tab, label), w) in tabs.iter().zip(widths) {
            let (r, resp) = ui.allocate_exact_size(egui::vec2(w, 34.0), egui::Sense::click());
            let on = *current == *tab;
            if on || resp.hovered() {
                ui.painter().rect_filled(r, 17.0, if on { t.hover } else { t.hover.gamma_multiply(0.5) });
            }
            ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, *label, font.clone(), if on { t.text } else { t.text_dim });
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                *current = *tab;
            }
        }
    });
}

fn ago(secs: u64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let s = now.saturating_sub(secs);
    match s {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", s / 60),
        3600..=86_399 => format!("{} h ago", s / 3600),
        _ => format!("{} d ago", s / 86_400),
    }
}
