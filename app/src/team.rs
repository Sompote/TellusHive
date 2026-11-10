//! Step 3, Team: the agents as nodes on the dotted canvas (drag to arrange) with delegation
//! arrows and floating toolbars; the selected agent's inspector in the step panel: role,
//! instructions, model, built-in tools, skills (Claude-style SKILL.md folders) and plugins.

use th_harness::config::{AgentSpec, Settings, TeamSpec};
use th_harness::skills::Skill;
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, Vec2};

use crate::chrome::{self, hex_color, to_hex, tokens};
use crate::icons::{self, Icon};
use crate::theme::medium_font;

const NODE: Vec2 = Vec2::new(210.0, 74.0);

pub struct TeamView {
    pub selected: Option<String>,
    drag: Option<(String, Vec2)>,
    /// A link being drawn from this agent's port to the pointer.
    linking: Option<String>,
    /// The link picked on the canvas (from, to), removed with Delete or its × button.
    selected_link: Option<(String, String)>,
    /// What the context menu was opened on.
    menu_target: Option<Target>,
    pan: Vec2,
    pub status: String,
    new_team_name: String,
}

#[derive(Clone, PartialEq)]
enum Target {
    Agent(String),
    Link(String, String),
}

impl Default for TeamView {
    fn default() -> Self {
        TeamView {
            selected: Some("coordinator".into()),
            drag: None,
            linking: None,
            selected_link: None,
            menu_target: None,
            pan: Vec2::new(60.0, 80.0),
            status: String::new(),
            new_team_name: String::new(),
        }
    }
}

fn effective_links(team: &TeamSpec) -> Vec<(String, String)> {
    team.effective_links()
}

/// Switch the team to drawn links, keeping the links now in force.
fn materialise_links(team: &mut TeamSpec) {
    if !team.uses_links() {
        team.links = team.effective_links();
    }
    team.custom_links = true;
}

/// Add the link `from` → `to`. Returns a message when the link is refused.
fn add_link(team: &mut TeamSpec, from: &str, to: &str) -> Result<String, String> {
    if from == to {
        return Err("An agent cannot delegate to itself".into());
    }
    let links = team.effective_links();
    if links.iter().any(|(f, t)| f == from && t == to) {
        return Err("That link already exists".into());
    }
    // refuse a loop: `to` must not already reach `from`
    let mut stack = vec![to.to_string()];
    let mut seen: Vec<String> = vec![];
    while let Some(n) = stack.pop() {
        if n == from {
            return Err("That link would make a delegation loop".into());
        }
        if seen.contains(&n) {
            continue;
        }
        seen.push(n.clone());
        stack.extend(links.iter().filter(|(f, _)| *f == n).map(|(_, t)| t.clone()));
    }
    materialise_links(team);
    team.links.push((from.to_string(), to.to_string()));
    if let Some(a) = team.agents.iter_mut().find(|a| a.id == from) {
        a.can_delegate = true;
    }
    let name = |id: &str| team.agent(id).map(|a| a.name.clone()).unwrap_or_default();
    Ok(format!("{} can now assign tasks to {}", name(from), name(to)))
}

fn remove_link(team: &mut TeamSpec, from: &str, to: &str) {
    materialise_links(team);
    team.links.retain(|(f, t)| !(f == from && t == to));
}

/// Remove an agent and every link to or from it. The coordinator cannot be removed.
fn delete_agent(team: &mut TeamSpec, id: &str) -> bool {
    if id == team.coordinator {
        return false;
    }
    team.agents.retain(|a| a.id != id);
    team.links.retain(|(f, t)| f != id && t != id);
    true
}

fn link_points(ra: Rect, rb: Rect) -> [Pos2; 4] {
    let s = Pos2::new(ra.right(), ra.center().y);
    let e = Pos2::new(rb.left(), rb.center().y);
    let k = ((e.x - s.x).abs() * 0.5).max(70.0);
    [s, s + Vec2::new(k, 0.0), e - Vec2::new(k, 0.0), e]
}

fn bezier(p: &[Pos2; 4], t: f32) -> Pos2 {
    let u = 1.0 - t;
    let v = p[0].to_vec2() * (u * u * u) + p[1].to_vec2() * (3.0 * u * u * t) + p[2].to_vec2() * (3.0 * u * t * t) + p[3].to_vec2() * (t * t * t);
    v.to_pos2()
}

/// Distance from `q` to the curve, sampled as a polyline.
fn curve_distance(p: &[Pos2; 4], q: Pos2) -> f32 {
    let mut best = f32::INFINITY;
    let mut prev = p[0];
    for k in 1..=24 {
        let cur = bezier(p, k as f32 / 24.0);
        let d = cur - prev;
        let t = if d.length_sq() > 0.0 { ((q - prev).dot(d) / d.length_sq()).clamp(0.0, 1.0) } else { 0.0 };
        best = best.min((prev + d * t).distance(q));
        prev = cur;
    }
    best
}

fn out_port(r: Rect) -> Pos2 {
    Pos2::new(r.right(), r.center().y)
}

fn close_button(r: Rect) -> Rect {
    Rect::from_center_size(r.right_top() + Vec2::new(-14.0, 14.0), Vec2::splat(18.0))
}

fn new_agent(team: &TeamSpec, pan: Vec2) -> AgentSpec {
    let mut n = team.agents.len() + 1;
    while team.agents.iter().any(|a| a.id == format!("agent{n}")) {
        n += 1;
    }
    AgentSpec {
        id: format!("agent{n}"),
        name: format!("Agent {n}"),
        role: "Describe what this agent does.".into(),
        tools: vec!["search_standards".into(), "get_clause".into(), "calc_sheet".into(), "list_files".into(), "read_file".into()],
        pos: [460.0 - pan.x * 0.0, 40.0 + 110.0 * (n % 5) as f32],
        color: "#1d8fb5".into(),
        ..Default::default()
    }
}

impl TeamView {
    /// Add a new agent; under drawn links the coordinator gets a link to it.
    fn push_agent(&mut self, team: &mut TeamSpec) {
        let a = new_agent(team, self.pan);
        self.selected = Some(a.id.clone());
        self.selected_link = None;
        if team.uses_links() {
            team.links.push((team.coordinator.clone(), a.id.clone()));
        }
        team.agents.push(a);
    }

    /// The canvas. Returns true when the team changed.
    pub fn canvas(&mut self, ui: &mut egui::Ui, team: &mut TeamSpec, settings: &Settings) -> bool {
        let mut changed = false;
        let t = tokens(ui);
        let full = ui.max_rect();
        let (rect, resp) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        let p = ui.painter_at(rect);
        let step = 24.0;
        let mut x = rect.left() + (self.pan.x.rem_euclid(step));
        while x < rect.right() {
            let mut y = rect.top() + (self.pan.y.rem_euclid(step));
            while y < rect.bottom() {
                p.circle_filled(Pos2::new(x, y), 1.0, t.canvas_dot);
                y += step;
            }
            x += step;
        }
        let origin = rect.min + self.pan;
        let node_rect = |a: &AgentSpec| Rect::from_min_size(origin + Vec2::new(a.pos[0], a.pos[1]), NODE);
        let pointer = ui.input(|i| i.pointer.interact_pos());
        let links = effective_links(team);
        // what is under the pointer: an agent's port, its × button, the agent, or a link
        let mut hovered: Option<String> = None;
        let mut hover_port: Option<String> = None;
        let mut hover_close: Option<String> = None;
        if let Some(pp) = pointer.filter(|pp| rect.contains(*pp)) {
            for a in team.agents.iter().rev() {
                let r = node_rect(a);
                if out_port(r).distance(pp) < 11.0 {
                    hover_port = Some(a.id.clone());
                    hovered = Some(a.id.clone());
                    break;
                }
                if r.contains(pp) {
                    if a.id != team.coordinator && close_button(r).contains(pp) {
                        hover_close = Some(a.id.clone());
                    }
                    hovered = Some(a.id.clone());
                    break;
                }
            }
        }
        let mut hover_link: Option<(String, String)> = None;
        if hovered.is_none() {
            if let Some(pp) = pointer.filter(|pp| rect.contains(*pp)) {
                let mut best = 7.0;
                for (from, to) in &links {
                    let (Some(a), Some(b)) = (team.agent(from), team.agent(to)) else { continue };
                    let d = curve_distance(&link_points(node_rect(a), node_rect(b)), pp);
                    if d < best {
                        best = d;
                        hover_link = Some((from.clone(), to.clone()));
                    }
                }
            }
        }
        // the selected link's × button, at the middle of the curve
        let link_close = self.selected_link.as_ref().and_then(|(f, tt)| {
            let (a, b) = (team.agent(f)?, team.agent(tt)?);
            Some(Rect::from_center_size(bezier(&link_points(node_rect(a), node_rect(b)), 0.5), Vec2::splat(20.0)))
        });
        let on_link_close = link_close.zip(pointer).is_some_and(|(r, pp)| r.contains(pp));

        for (from, to) in &links {
            let (Some(a), Some(b)) = (team.agent(from), team.agent(to)) else { continue };
            let pts = link_points(node_rect(a), node_rect(b));
            let key = Some((from.clone(), to.clone()));
            let picked = self.selected_link == key;
            let hot = hover_link == key || picked;
            let col = if picked { t.accent } else { hex_color(&b.color, t.accent).gamma_multiply(if hot { 1.0 } else { 0.85 }) };
            let w = if hot { 3.0 } else { 1.8 };
            p.add(egui::epaint::CubicBezierShape::from_points_stroke(pts, false, Color32::TRANSPARENT, Stroke::new(w, col)));
            let e = pts[3];
            p.add(egui::Shape::convex_polygon(vec![e, e + Vec2::new(-9.0, -5.0), e + Vec2::new(-9.0, 5.0)], col, Stroke::NONE));
        }
        // the link being drawn follows the pointer
        if let (Some(from), Some(pp)) = (&self.linking, pointer) {
            if let Some(a) = team.agent(from) {
                let ra = node_rect(a);
                let target = hovered.as_ref().filter(|h| *h != from).and_then(|h| team.agent(h)).map(|b| node_rect(b));
                let pts = match target {
                    Some(rb) => link_points(ra, rb),
                    None => {
                        let s = out_port(ra);
                        let k = ((pp.x - s.x).abs() * 0.5).max(40.0);
                        [s, s + Vec2::new(k, 0.0), pp - Vec2::new(k, 0.0), pp]
                    }
                };
                p.add(egui::epaint::CubicBezierShape::from_points_stroke(pts, false, Color32::TRANSPARENT, Stroke::new(2.2, t.accent)));
                p.circle_filled(pts[3], 4.0, t.accent);
            }
        }

        for a in &team.agents {
            let r = node_rect(a);
            let col = hex_color(&a.color, t.accent);
            let sel = self.selected.as_deref() == Some(a.id.as_str());
            let drop = self.linking.is_some() && hovered.as_deref() == Some(a.id.as_str()) && self.linking.as_deref() != Some(a.id.as_str());
            chrome::soft_shadow(&p, r, &t);
            p.rect_filled(r, 10.0, t.card);
            let edge = if sel || drop { Stroke::new(2.0, t.accent) } else { Stroke::new(1.0, t.field_border) };
            p.rect_stroke(r, 10.0, edge, egui::StrokeKind::Inside);
            // the agent's mark: a coloured disc with its initial
            let c = r.left_top() + Vec2::new(26.0, 26.0);
            p.circle_filled(c, 14.0, col.gamma_multiply(if t.dark { 0.35 } else { 0.18 }));
            p.text(c, Align2::CENTER_CENTER, a.name.chars().next().unwrap_or('A').to_string(), medium_font(13.0), col);
            let title = if a.id == team.coordinator { format!("{}  ★", a.name) } else { a.name.clone() };
            p.text(r.left_top() + Vec2::new(48.0, 18.0), Align2::LEFT_CENTER, title, medium_font(13.5), t.text);
            let prov = if a.provider.is_empty() { settings.default_provider.as_str() } else { a.provider.as_str() };
            let model = if a.model.is_empty() { format!("{prov} · default model") } else { a.model.clone() };
            p.text(r.left_top() + Vec2::new(48.0, 36.0), Align2::LEFT_CENTER, model, FontId::proportional(11.5), t.text_dim);
            let n = |v: &Vec<String>| if v.iter().any(|x| x == "*") { "all".to_string() } else { v.len().to_string() };
            p.text(r.left_top() + Vec2::new(48.0, 56.0), Align2::LEFT_CENTER, format!("{} tools · {} skills · {} plugins", n(&a.tools), n(&a.skills), a.plugins.len()), FontId::proportional(11.0), t.text_faint);
            // the output port: drag from it to another agent to let this one assign tasks to it
            let node_hot = hovered.as_deref() == Some(a.id.as_str()) || sel;
            let port_hot = hover_port.as_deref() == Some(a.id.as_str()) || self.linking.as_deref() == Some(a.id.as_str());
            let pc = out_port(r);
            p.circle_filled(pc, if port_hot { 7.0 } else { 5.0 }, t.card);
            p.circle_stroke(pc, if port_hot { 7.0 } else { 5.0 }, Stroke::new(if node_hot || port_hot { 2.0 } else { 1.2 }, if port_hot { t.accent } else { col }));
            if port_hot {
                p.circle_filled(pc, 3.0, t.accent);
            }
            // the delete button, shown on hover (the coordinator stays)
            if hovered.as_deref() == Some(a.id.as_str()) && a.id != team.coordinator && self.linking.is_none() && self.drag.is_none() {
                let cb = close_button(r);
                let on = hover_close.as_deref() == Some(a.id.as_str());
                if on {
                    p.circle_filled(cb.center(), 9.0, t.error.gamma_multiply(0.15));
                }
                let cc = if on { t.error } else { t.text_faint };
                let d = 3.5;
                p.line_segment([cb.center() + Vec2::new(-d, -d), cb.center() + Vec2::new(d, d)], Stroke::new(1.6, cc));
                p.line_segment([cb.center() + Vec2::new(-d, d), cb.center() + Vec2::new(d, -d)], Stroke::new(1.6, cc));
            }
        }
        if let Some(r) = link_close {
            p.circle_filled(r.center(), 10.0, t.card);
            p.circle_stroke(r.center(), 10.0, Stroke::new(1.4, if on_link_close { t.error } else { t.accent }));
            let d = 3.5;
            let cc = if on_link_close { t.error } else { t.text };
            p.line_segment([r.center() + Vec2::new(-d, -d), r.center() + Vec2::new(d, d)], Stroke::new(1.6, cc));
            p.line_segment([r.center() + Vec2::new(-d, d), r.center() + Vec2::new(d, -d)], Stroke::new(1.6, cc));
        }

        if hover_port.is_some() || self.linking.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        } else if hover_close.is_some() || on_link_close || hover_link.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }

        if resp.drag_started() {
            if let Some(id) = hover_port.clone() {
                self.linking = Some(id.clone());
                self.selected = Some(id);
                self.selected_link = None;
            } else if let Some(id) = hovered.clone() {
                let a = team.agent(&id).map(|a| a.pos).unwrap_or_default();
                let grab = pointer.map(|pp| pp - (origin + Vec2::new(a[0], a[1]))).unwrap_or_default();
                self.drag = Some((id.clone(), grab));
                self.selected = Some(id);
            }
        }
        if resp.dragged() && self.linking.is_none() {
            match (&self.drag, pointer) {
                (Some((id, grab)), Some(pp)) => {
                    let np = pp - origin - *grab;
                    if let Some(a) = team.agents.iter_mut().find(|a| &a.id == id) {
                        a.pos = [np.x.round(), np.y.round()];
                        changed = true;
                    }
                }
                _ => self.pan += resp.drag_delta(),
            }
        }
        if resp.drag_stopped() {
            if let Some(from) = self.linking.take() {
                if let Some(to) = hovered.clone().filter(|h| *h != from) {
                    match add_link(team, &from, &to) {
                        Ok(msg) => {
                            self.status = msg;
                            self.selected_link = Some((from, to));
                            changed = true;
                        }
                        Err(e) => self.status = e,
                    }
                }
            }
            self.drag = None;
        }
        if resp.clicked() {
            if on_link_close {
                if let Some((f, tt)) = self.selected_link.take() {
                    remove_link(team, &f, &tt);
                    self.status = "Link removed".into();
                    changed = true;
                }
            } else if let Some(id) = hover_close.clone() {
                let name = team.agent(&id).map(|a| a.name.clone()).unwrap_or_default();
                if delete_agent(team, &id) {
                    self.status = format!("Deleted {name}");
                    self.selected = Some(team.coordinator.clone());
                    self.selected_link = None;
                    changed = true;
                }
            } else if hovered.is_some() {
                self.selected = hovered.clone();
                self.selected_link = None;
            } else {
                self.selected_link = hover_link.clone();
            }
        }
        if resp.secondary_clicked() {
            self.menu_target = match (&hovered, &hover_link) {
                (Some(id), _) => Some(Target::Agent(id.clone())),
                (None, Some((f, tt))) => Some(Target::Link(f.clone(), tt.clone())),
                _ => None,
            };
        }
        let mut menu_action: Option<Box<dyn FnOnce(&mut TeamSpec, &mut TeamView)>> = None;
        let target = self.menu_target.clone();
        let coord = team.coordinator.clone();
        resp.context_menu(|ui| match &target {
            Some(Target::Agent(id)) => {
                if *id != coord {
                    if ui.button("Make coordinator").clicked() {
                        let id = id.clone();
                        menu_action = Some(Box::new(move |team, _| {
                            team.coordinator = id.clone();
                            if let Some(a) = team.agents.iter_mut().find(|a| a.id == id) {
                                a.can_delegate = true;
                            }
                            team.links.clear();
                            team.custom_links = false;
                        }));
                        ui.close_menu();
                    }
                    if ui.button(RichText::new("Delete agent").color(t.error)).clicked() {
                        let id = id.clone();
                        menu_action = Some(Box::new(move |team, view| {
                            if delete_agent(team, &id) {
                                view.selected = Some(team.coordinator.clone());
                                view.status = "Agent deleted".into();
                            }
                        }));
                        ui.close_menu();
                    }
                } else {
                    ui.label(RichText::new("The coordinator cannot be deleted").color(t.text_dim));
                }
            }
            Some(Target::Link(f, tt)) => {
                if ui.button(RichText::new("Remove link").color(t.error)).clicked() {
                    let (f, tt) = (f.clone(), tt.clone());
                    menu_action = Some(Box::new(move |team, view| {
                        remove_link(team, &f, &tt);
                        view.selected_link = None;
                        view.status = "Link removed".into();
                    }));
                    ui.close_menu();
                }
            }
            None => {
                if ui.button("Add agent").clicked() {
                    menu_action = Some(Box::new(|team, view| view.push_agent(team)));
                    ui.close_menu();
                }
            }
        });
        if let Some(f) = menu_action {
            f(team, self);
            changed = true;
        }
        // Delete / Backspace removes the picked link, else the selected agent, unless typing
        let typing = ui.ctx().memory(|m| m.focused().is_some());
        if !typing && ui.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace)) {
            if let Some((f, tt)) = self.selected_link.take() {
                remove_link(team, &f, &tt);
                self.status = "Link removed".into();
                changed = true;
            } else if let Some(id) = self.selected.clone() {
                if delete_agent(team, &id) {
                    self.status = "Agent deleted".into();
                    self.selected = Some(team.coordinator.clone());
                    changed = true;
                }
            }
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.linking = None;
            self.selected_link = None;
        }

        // floating toolbars, as on Tellus3D's model view
        let bar = Rect::from_min_size(full.min + Vec2::new(16.0, 16.0), Vec2::new((full.width() - 32.0).max(200.0), 44.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(bar).layout(egui::Layout::left_to_right(egui::Align::Min)));
        chrome::float_frame(child.ctx()).show(&mut child, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Team").color(t.text_dim));
                let mut pick: Option<TeamSpec> = None;
                egui::ComboBox::from_id_salt("team_pick").selected_text(&team.name).width(180.0).show_ui(ui, |ui| {
                    for tm in TeamSpec::list() {
                        if ui.selectable_label(tm.name == team.name, &tm.name).clicked() {
                            pick = Some(tm);
                        }
                    }
                });
                if let Some(p) = pick {
                    *team = p;
                    self.selected = Some(team.coordinator.clone());
                    changed = true;
                }
                chrome::strip_divider(ui);
                if chrome::icon_text_button(ui, Icon::Plus, "Add agent").clicked() {
                    self.push_agent(team);
                    changed = true;
                }
                chrome::strip_divider(ui);
                ui.menu_button("More", |ui| {
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.new_team_name).hint_text("new team name").desired_width(150.0));
                        if ui.button("Save as").clicked() && !self.new_team_name.trim().is_empty() {
                            team.name = self.new_team_name.trim().to_string();
                            self.status = team.save().map(|_| format!("Saved '{}'", team.name)).unwrap_or_else(|e| e);
                            self.new_team_name.clear();
                            changed = true;
                            ui.close_menu();
                        }
                    });
                    if ui.button("Reset to the default design office").clicked() {
                        *team = th_harness::config::default_team();
                        changed = true;
                        ui.close_menu();
                    }
                });
            });
        });
        let hint = Rect::from_min_size(Pos2::new(full.left() + 16.0, full.bottom() - 34.0), Vec2::new(full.width() - 32.0, 24.0));
        ui.painter().text(hint.left_center(), Align2::LEFT_CENTER, "Drag an agent to move it · drag from its right-hand port to another agent to let it assign tasks · click a link or hover an agent for × · right-click for more · Delete removes the selection", FontId::proportional(11.5), t.text_faint);
        if !self.status.is_empty() {
            ui.painter().text(hint.right_center(), Align2::RIGHT_CENTER, &self.status, FontId::proportional(11.5), t.text_dim);
        }
        changed
    }

    /// The step panel: the agents as rows, the selected one's inspector under its row.
    pub fn left(&mut self, ui: &mut egui::Ui, team: &mut TeamSpec, settings: &Settings, skills: &[Skill]) -> bool {
        let t = tokens(ui);
        let mut changed = false;
        chrome::section(ui, &format!("Agents  ·  {}", team.agents.len()));
        let ids: Vec<String> = team.agents.iter().map(|a| a.id.clone()).collect();
        for id in ids {
            let Some(a) = team.agents.iter().find(|a| a.id == id) else { continue };
            let sel = self.selected.as_deref() == Some(id.as_str());
            let what = if id == team.coordinator { "coordinator".to_string() } else { a.role.split(['.', ':', ',']).next().unwrap_or("").chars().take(26).collect() };
            if chrome::object_row(ui, hex_color(&a.color, t.accent), None, &a.name.clone(), &what, sel).clicked() {
                self.selected = if sel { None } else { Some(id.clone()) };
            }
            if sel {
                chrome::inspector(ui, |ui| {
                    if self.inspector(ui, team, settings, skills) {
                        changed = true;
                    }
                });
            }
        }
        if chrome::ghost_button(ui, "+ Agent").clicked() {
            self.push_agent(team);
            changed = true;
        }
        changed
    }

    fn inspector(&mut self, ui: &mut egui::Ui, team: &mut TeamSpec, settings: &Settings, skills: &[Skill]) -> bool {
        let t = tokens(ui);
        let mut changed = false;
        let Some(sel) = self.selected.clone() else { return false };
        let names: Vec<(String, String)> = team.agents.iter().map(|a| (a.id.clone(), a.name.clone())).collect();
        let coordinator = team.coordinator.clone();
        let mut links = effective_links(team);
        let Some(idx) = team.agents.iter().position(|a| a.id == sel) else { return false };
        let mut delete = false;
        let mut make_coord = false;
        let mut links_edited = false;
        {
            let a = &mut team.agents[idx];
            let before = a.clone();
            chrome::field(ui, "Name", &mut a.name);
            ui.horizontal(|ui| {
                chrome::row_label(ui, "Colour");
                let mut c = hex_color(&a.color, t.accent);
                if ui.color_edit_button_srgba(&mut c).changed() {
                    a.color = to_hex(c);
                }
                ui.label(RichText::new(format!("id {}", a.id)).size(11.0).color(t.text_faint));
            });
            ui.label(RichText::new("Role (what the coordinator sees)").color(t.text_dim));
            ui.add(egui::TextEdit::multiline(&mut a.role).desired_rows(2).desired_width(f32::INFINITY));
            ui.label(RichText::new("Instructions").color(t.text_dim));
            ui.add(egui::TextEdit::multiline(&mut a.instructions).desired_rows(6).desired_width(f32::INFINITY));

            chrome::section(ui, "Model");
            ui.horizontal(|ui| {
                chrome::row_label(ui, "Provider");
                let label = if a.provider.is_empty() { format!("default ({})", settings.default_provider) } else { a.provider.clone() };
                egui::ComboBox::from_id_salt("agent_provider").selected_text(label).width(170.0).show_ui(ui, |ui| {
                    ui.selectable_value(&mut a.provider, String::new(), "default");
                    for p in &settings.providers {
                        ui.selectable_value(&mut a.provider, p.name.clone(), &p.name);
                    }
                });
            });
            ui.horizontal(|ui| {
                chrome::row_label(ui, "Model");
                ui.add_sized([ui.available_width(), 28.0], egui::TextEdit::singleline(&mut a.model).hint_text("empty = provider default").vertical_align(egui::Align::Center));
            });
            ui.horizontal(|ui| {
                chrome::row_label(ui, "Temperature");
                ui.add(egui::Slider::new(&mut a.temperature, 0.0..=1.0));
            });
            ui.horizontal(|ui| {
                chrome::row_label(ui, "Max turns");
                ui.add_sized([70.0, 28.0], egui::DragValue::new(&mut a.max_turns).range(1..=200));
            });

            chrome::section(ui, "Built-in tools");
            let mut all = a.tools.iter().any(|x| x == "*");
            if chrome::switch_row(ui, "All tools", &mut all).changed() {
                a.tools = if all { vec!["*".into()] } else { vec![] };
            }
            if !all {
                ui.columns(2, |cols| {
                    for (k, spec) in th_harness::tools::builtin_specs().into_iter().enumerate() {
                        let ui = &mut cols[k % 2];
                        let mut on = a.tools.contains(&spec.name);
                        if ui.checkbox(&mut on, RichText::new(&spec.name).size(12.0)).on_hover_text(&spec.description).changed() {
                            if on {
                                a.tools.push(spec.name.clone());
                            } else {
                                a.tools.retain(|x| x != &spec.name);
                            }
                        }
                    }
                });
            }

            chrome::section(ui, &format!("Skills  ·  {} found", skills.len()));
            let mut all = a.skills.iter().any(|x| x == "*");
            if chrome::switch_row(ui, "All skills", &mut all).changed() {
                a.skills = if all { vec!["*".into()] } else { vec![] };
            }
            if !all {
                if skills.is_empty() {
                    chrome::hint(ui, "No skills found. Add skill folders (each with SKILL.md) in Settings.");
                }
                for s in skills {
                    let mut on = a.skills.contains(&s.name);
                    if ui.checkbox(&mut on, RichText::new(&s.name).size(12.5)).on_hover_text(&s.description).changed() {
                        if on {
                            a.skills.push(s.name.clone());
                        } else {
                            a.skills.retain(|x| x != &s.name);
                        }
                    }
                }
            }

            chrome::section(ui, "Plugins");
            if settings.plugins.is_empty() {
                chrome::hint(ui, "No plugins yet: add MCP servers in step 4.");
            }
            for p in &settings.plugins {
                let mut on = a.plugins.contains(&p.name);
                let label = if p.enabled { p.name.clone() } else { format!("{}  (off)", p.name) };
                if chrome::switch_row(ui, &label, &mut on).on_hover_text(&p.description).changed() {
                    if on {
                        a.plugins.push(p.name.clone());
                    } else {
                        a.plugins.retain(|x| x != &p.name);
                    }
                }
            }

            chrome::section(ui, "Coordination");
            chrome::switch_row(ui, "May delegate to other agents", &mut a.can_delegate);
            if a.can_delegate {
                ui.label(RichText::new("Delegates to").color(t.text_dim));
                for (id, name) in &names {
                    if *id == a.id {
                        continue;
                    }
                    let mut on = links.iter().any(|(f, tt)| *f == a.id && tt == id);
                    if ui.checkbox(&mut on, name).changed() {
                        links_edited = true;
                        if on {
                            links.push((a.id.clone(), id.clone()));
                        } else {
                            links.retain(|(f, tt)| !(*f == a.id && tt == id));
                        }
                        changed = true;
                    }
                }
            }
            chrome::switch_row(ui, "May use Claude Code / Codex", &mut a.can_use_external);
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if a.id != coordinator && chrome::ghost_button(ui, "Make coordinator").on_hover_text("The coordinator is the agent you talk with").clicked() {
                    make_coord = true;
                }
                if a.id != coordinator && ui.add(egui::Button::new(RichText::new("Delete agent").color(t.error)).frame(false)).clicked() {
                    delete = true;
                }
            });
            if *a != before {
                changed = true;
            }
        }
        if links_edited {
            team.links = links;
            team.custom_links = true;
        }
        if make_coord {
            team.coordinator = sel.clone();
            if let Some(a) = team.agents.iter_mut().find(|a| a.id == sel) {
                a.can_delegate = true;
            }
            team.links.clear();
            team.custom_links = false;
            changed = true;
        }
        if delete && delete_agent(team, &sel) {
            self.selected = Some(team.coordinator.clone());
            self.selected_link = None;
            changed = true;
        }
        let _ = icons::Icon::Users;
        changed
    }
}
