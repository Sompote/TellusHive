//! Widgets shared by the Input and Output workspaces. Layout follows PLAXIS 2D
//! (mode tabs, explorers, tool strip, drawing area, command line; recovered with REA),
//! drawn with the theme tokens in `theme.rs`.

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Margin, Pos2, Rect, Sense, Shape, Stroke, Vec2};

use crate::icons::{self, Icon};
use crate::theme::{medium_font, Tokens, RADIUS, RADIUS_SM, ROW_H, TAB_STRIP_H};

/// Linear world → screen map: `screen = (ox + s·x, oy − s·y)`.
#[derive(Clone, Copy)]
pub struct View2D {
    pub ox: f64,
    pub oy: f64,
    pub s: f64,
}

impl View2D {
    pub fn to_screen(&self, p: [f64; 2]) -> Pos2 {
        Pos2::new((self.ox + self.s * p[0]) as f32, (self.oy - self.s * p[1]) as f32)
    }

    pub fn to_world(&self, p: Pos2) -> [f64; 2] {
        [(p.x as f64 - self.ox) / self.s, (self.oy - p.y as f64) / self.s]
    }
}

pub fn tokens(ui: &egui::Ui) -> Tokens {
    Tokens::get(ui.ctx())
}

/// Frame of the side panels and bars.
pub fn panel_frame(ctx: &egui::Context, margin: i8) -> egui::Frame {
    egui::Frame::NONE.fill(Tokens::get(ctx).chrome).inner_margin(margin)
}

/// Frame of the menu / tab bar at the top.
pub fn dock_frame(ctx: &egui::Context) -> egui::Frame {
    let t = Tokens::get(ctx);
    egui::Frame::NONE.fill(t.dock).inner_margin(Margin::symmetric(8, 4)).stroke(Stroke::new(1.0_f32, t.separator))
}

/// Body of an explorer, joined to the `header` above it (square top corners).
pub fn card(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    let t = tokens(ui);
    let corners = CornerRadius { nw: 0, ne: 0, sw: RADIUS, se: RADIUS };
    egui::Frame::NONE.fill(t.card).stroke(Stroke::new(1.0_f32, t.separator)).corner_radius(corners).inner_margin(8).show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        add(ui);
    });
}

/// Flat tab strip; the active tab is lifted onto the card colour with an accent bar.
/// Tabs are numbered as steps when `steps` is set (PLAXIS' modes are done in order).
pub fn mode_tabs(ui: &mut egui::Ui, labels: &[&str], active: usize, steps: bool) -> Option<usize> {
    let t = tokens(ui);
    let mut clicked = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (i, label) in labels.iter().enumerate() {
            let font = medium_font(12.5);
            let tw = ui.fonts(|f| f.layout_no_wrap(label.to_string(), font.clone(), t.text).size().x);
            let badge = if steps { 22.0 } else { 0.0 };
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(tw + badge + 28.0, TAB_STRIP_H + 4.0), Sense::click());
            let on = i == active;
            let fill = if on { t.card } else if resp.hovered() { t.hover } else { Color32::TRANSPARENT };
            ui.painter().rect_filled(rect, CornerRadius { nw: RADIUS, ne: RADIUS, sw: 0, se: 0 }, fill);
            if on {
                ui.painter().rect_filled(Rect::from_min_max(Pos2::new(rect.left(), rect.bottom() - 2.0), rect.right_bottom()), 0.0, t.accent);
            }
            let mut x = rect.left() + 14.0;
            if steps {
                let c = Pos2::new(x + 8.0, rect.center().y);
                let (bg, fg) = if on { (t.accent, Color32::WHITE) } else if i < active { (t.accent_soft, t.accent_text) } else { (t.hover, t.text_dim) };
                ui.painter().circle_filled(c, 8.5, bg);
                ui.painter().text(c, Align2::CENTER_CENTER, format!("{}", i + 1), FontId::proportional(10.5), fg);
                x += badge;
            }
            ui.painter().text(Pos2::new(x, rect.center().y), Align2::LEFT_CENTER, *label, font, if on { t.text } else { t.text_dim });
            if resp.clicked() {
                clicked = Some(i);
            }
        }
    });
    clicked
}

/// Explorer title bar on the steel-blue header tint, joined to the `card` below it. The
/// `context` (the mode, phase or selected object the explorer is showing) follows the
/// title in the accent colour; pass "" for none.
pub fn header(ui: &mut egui::Ui, title: &str, context: &str) {
    let t = tokens(ui);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), TAB_STRIP_H + 2.0), Sense::hover());
    let corners = CornerRadius { nw: RADIUS, ne: RADIUS, sw: 0, se: 0 };
    let p = ui.painter();
    p.rect_filled(rect, corners, t.header);
    p.rect_stroke(rect, corners, Stroke::new(1.0_f32, t.header_edge), egui::StrokeKind::Inside);
    let title_rect = p.text(rect.left_center() + Vec2::new(10.0, 0.0), Align2::LEFT_CENTER, title, medium_font(12.5), t.text);
    if !context.is_empty() {
        p.text(Pos2::new(title_rect.right() + 7.0, rect.center().y), Align2::LEFT_CENTER, context, FontId::proportional(12.0), t.accent_text);
    }
    // the card's top border overlaps the bar's bottom edge
    ui.add_space(-ui.spacing().item_spacing.y - 1.0);
}

/// The command key as shortcuts show it: ⌘ on macOS, Ctrl+ elsewhere.
pub const CMD: &str = if cfg!(target_os = "macos") { "⌘" } else { "Ctrl+" };

/// What a step of the workflow stepper shows.
#[derive(Clone, Copy, PartialEq)]
pub enum StepState {
    Todo,
    Current,
    /// Needs doing again (the mesh after an edit): an amber dot after the label.
    Attention,
}

/// One step of the top bar's workflow stepper: a numbered circle (or an icon) and the label;
/// the current step sits on the selection tint with its number on the accent.
pub fn step_button(ui: &mut egui::Ui, number: Option<usize>, icon: Option<Icon>, label: &str, state: StepState) -> egui::Response {
    let t = tokens(ui);
    let on = state == StepState::Current;
    let font = if on { medium_font(13.0) } else { FontId::proportional(13.0) };
    let tw = ui.fonts(|f| f.layout_no_wrap(label.to_string(), font.clone(), t.text).size().x);
    let dot = if state == StepState::Attention { 13.0 } else { 0.0 };
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(12.0 + 18.0 + 8.0 + tw + dot + 12.0, 32.0), Sense::click());
    let fill = if on { t.accent_soft } else if resp.hovered() { t.hover } else { Color32::TRANSPARENT };
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(7), fill);
    let c = Pos2::new(rect.left() + 21.0, rect.center().y);
    if let Some(n) = number {
        if on {
            p.circle_filled(c, 9.0, t.accent);
        } else {
            p.circle_stroke(c, 8.5, Stroke::new(1.0_f32, t.text_faint));
        }
        p.text(c, Align2::CENTER_CENTER, n.to_string(), FontId::proportional(11.0), if on { Color32::WHITE } else { t.text_dim });
    } else if let Some(i) = icon {
        icons::paint(ui, i, Rect::from_center_size(c, Vec2::splat(16.0)), if on { t.accent_text } else { t.text_dim });
    }
    let x = rect.left() + 38.0;
    p.text(Pos2::new(x, rect.center().y), Align2::LEFT_CENTER, label, font, if on { t.text } else { t.text_dim });
    if dot > 0.0 {
        p.circle_filled(Pos2::new(x + tw + 9.0, rect.center().y), 3.5, t.warn);
    }
    resp
}

/// The frame of a segmented strip (the stepper, tabs inside a panel).
pub fn segmented_frame(ctx: &egui::Context) -> egui::Frame {
    let t = Tokens::get(ctx);
    egui::Frame::NONE.fill(t.field).stroke(Stroke::new(1.0_f32, t.separator)).corner_radius(CornerRadius::same(10)).inner_margin(3)
}

/// Tabs inside a panel as a segmented control (Water: Pore pressure | Flow and drainage).
pub fn segmented(ui: &mut egui::Ui, labels: &[&str], current: &mut usize) {
    let t = tokens(ui);
    segmented_frame(ui.ctx()).corner_radius(CornerRadius::same(9)).show(ui, |ui| {
        let w = (ui.available_width() - 2.0 * (labels.len() - 1) as f32) / labels.len() as f32;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (i, l) in labels.iter().enumerate() {
                let (r, resp) = ui.allocate_exact_size(Vec2::new(w, 30.0), Sense::click());
                let on = *current == i;
                let fill = if on { t.accent_soft } else if resp.hovered() { t.hover } else { Color32::TRANSPARENT };
                ui.painter().rect_filled(r, CornerRadius::same(6), fill);
                ui.painter().text(r.center(), Align2::CENTER_CENTER, *l, if on { medium_font(13.0) } else { FontId::proportional(13.0) }, if on { t.text } else { t.text_dim });
                if resp.clicked() {
                    *current = i;
                }
            }
        });
    });
}

/// A small rounded label in a colour (a phase's kind): the colour on a tint of itself.
pub fn chip(ui: &mut egui::Ui, text: &str, colour: Color32) {
    let t = tokens(ui);
    let font = FontId::proportional(11.0);
    let w = ui.fonts(|f| f.layout_no_wrap(text.to_string(), font.clone(), colour).size().x) + 14.0;
    let (r, _) = ui.allocate_exact_size(Vec2::new(w, 20.0), Sense::hover());
    ui.painter().rect_filled(r, CornerRadius::same(10), colour.gamma_multiply(if t.dark { 0.18 } else { 0.14 }));
    ui.painter().text(r.center(), Align2::CENTER_CENTER, text, font, colour);
}

/// A count with thousands separators: 17,640.
pub fn thousands(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The app icon as a texture (loaded once), for the top bar and the start page.
pub fn app_icon(ctx: &egui::Context) -> egui::TextureHandle {
    let id = egui::Id::new("app_icon_texture");
    if let Some(t) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(id)) {
        return t;
    }
    let img = image::load_from_memory(include_bytes!("../assets/app_icon.png")).expect("app icon is a valid PNG").resize(96, 96, image::imageops::FilterType::Lanczos3).to_rgba8();
    let tex = ctx.load_texture("app_icon", egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw()), egui::TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(id, tex.clone()));
    tex
}

/// An on/off switch (what is in place in a phase).
pub fn switch(ui: &mut egui::Ui, on: &mut bool) -> egui::Response {
    let t = tokens(ui);
    let (r, mut resp) = ui.allocate_exact_size(Vec2::new(32.0, 18.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let k = ui.ctx().animate_bool(resp.id, *on);
    let track = if *on { t.accent } else { t.field_border };
    ui.painter().rect_filled(r, 9.0, track);
    let x = egui::lerp(r.left() + 9.0..=r.right() - 9.0, k);
    ui.painter().circle_filled(Pos2::new(x, r.center().y), 6.5, Color32::WHITE);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A row of a phase's "in this phase" card: its colour, its name (dim when off), its value
/// (a load's, a drain's vacuum) and the switch.
pub fn place_row(ui: &mut egui::Ui, colour: Color32, name: &str, on: &mut bool, value: impl FnOnce(&mut egui::Ui, bool)) {
    let t = tokens(ui);
    ui.horizontal(|ui| {
        ui.set_min_height(32.0);
        let (r, _) = ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
        if *on {
            ui.painter().rect_filled(r, 3.0, colour);
        } else {
            ui.painter().rect_stroke(r, 3.0, Stroke::new(1.0_f32, t.text_faint), egui::StrokeKind::Inside);
        }
        ui.label(egui::RichText::new(name).color(if *on { t.text } else { t.text_dim }));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            switch(ui, on);
            value(ui, *on);
        });
    });
}

/// The card that holds rows (a phase's "in this phase", a choice of modes).
pub fn rows_card(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    let t = tokens(ui);
    egui::Frame::NONE.stroke(Stroke::new(1.0_f32, t.separator)).corner_radius(CornerRadius::same(8)).inner_margin(Margin::symmetric(10, 4)).show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        add(ui);
    });
}

/// The Assistant button: a speech bubble and the label, outlined.
pub fn assistant_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    let t = tokens(ui);
    let font = medium_font(13.0);
    let tw = ui.fonts(|f| f.layout_no_wrap(label.to_string(), font.clone(), t.text).size().x);
    let (r, resp) = ui.allocate_exact_size(Vec2::new(tw + 46.0, 34.0), Sense::click());
    let p = ui.painter();
    p.rect_filled(r, 8.0, if resp.hovered() { t.hover } else { Color32::TRANSPARENT });
    p.rect_stroke(r, 8.0, Stroke::new(1.0_f32, t.field_border), egui::StrokeKind::Inside);
    bubble(p, Pos2::new(r.left() + 20.0, r.center().y), t.accent_text);
    p.text(Pos2::new(r.left() + 34.0, r.center().y), Align2::LEFT_CENTER, label, font, t.text);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A speech bubble 16 px wide centred on `c` (the assistant's mark).
pub fn bubble(p: &egui::Painter, c: Pos2, colour: Color32) {
    let s = Stroke::new(1.6_f32, colour);
    let r = Rect::from_center_size(c + Vec2::new(0.0, -1.5), Vec2::new(15.0, 11.0));
    p.rect_stroke(r, 3.0, s, egui::StrokeKind::Middle);
    p.line_segment([Pos2::new(r.left() + 3.0, r.bottom()), Pos2::new(r.left() + 2.0, r.bottom() + 4.0)], s);
    p.line_segment([Pos2::new(r.left() + 2.0, r.bottom() + 4.0), Pos2::new(r.left() + 7.0, r.bottom())], s);
}

/// A label and its switch on the right, a row of a list of options.
pub fn switch_row(ui: &mut egui::Ui, label: &str, on: &mut bool) -> egui::Response {
    ui.horizontal(|ui| {
        ui.set_min_height(30.0);
        ui.label(label);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| switch(ui, on)).inner
    })
    .inner
}

/// The model as a small box with its drained faces green: the top, the four sides and the
/// bottom as `open` says (x min, x max, y min, y max, bottom); the top always drains.
pub fn drained_box(ui: &mut egui::Ui, open: [bool; 5]) {
    let t = tokens(ui);
    let (r, _) = ui.allocate_exact_size(Vec2::new(112.0, 96.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(r, 8.0, t.field);
    let at = |x: f32, y: f32| Pos2::new(r.left() + x, r.top() + y);
    let green = t.ok;
    let face = |pts: Vec<Pos2>, on: bool, shade: f32| {
        let fill = if on { green.gamma_multiply(0.35) } else { t.card.gamma_multiply(shade) };
        p.add(Shape::convex_polygon(pts, fill, Stroke::new(if on { 1.5 } else { 1.0 }, if on { green } else { t.field_border })));
    };
    // the hidden faces first (x max at the back right, y max at the back left, bottom), as outlines
    if open[1] {
        p.add(Shape::closed_line(vec![at(56.0, 18.0), at(96.0, 34.0), at(96.0, 68.0), at(56.0, 52.0)], Stroke::new(1.0_f32, green)));
    }
    if open[3] {
        p.add(Shape::closed_line(vec![at(16.0, 34.0), at(56.0, 18.0), at(56.0, 52.0), at(16.0, 68.0)], Stroke::new(1.0_f32, green)));
    }
    // the visible faces: x min (front left), y min (front right), the top
    face(vec![at(16.0, 34.0), at(56.0, 50.0), at(56.0, 84.0), at(16.0, 68.0)], open[0], 0.9);
    face(vec![at(56.0, 50.0), at(96.0, 34.0), at(96.0, 68.0), at(56.0, 84.0)], open[2], 0.75);
    face(vec![at(16.0, 34.0), at(56.0, 18.0), at(96.0, 34.0), at(56.0, 50.0)], true, 1.0);
    if open[4] {
        p.add(Shape::line(vec![at(16.0, 68.0), at(56.0, 84.0), at(96.0, 68.0)], Stroke::new(3.0_f32, green)));
    }
}

/// The card of a toolbar floating on a view.
pub fn float_frame(ctx: &egui::Context) -> egui::Frame {
    let t = Tokens::get(ctx);
    egui::Frame::NONE.fill(t.card).stroke(Stroke::new(1.0_f32, t.field_border)).corner_radius(CornerRadius::same(10)).inner_margin(Margin::symmetric(6, 4)).shadow(egui::Shadow { offset: [0, 4], blur: 14, spread: 0, color: Color32::from_black_alpha(if t.dark { 70 } else { 22 }) })
}

/// A thin vertical divider between groups of a strip.
pub fn strip_divider(ui: &mut egui::Ui) {
    let t = tokens(ui);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(13.0, 20.0), Sense::hover());
    ui.painter().vline(rect.center().x, rect.y_range(), Stroke::new(1.0_f32, t.field_border));
}

/// The primary action of a screen: white text on the accent.
pub fn primary_button(text: &str) -> egui::Button<'static> {
    egui::Button::new(egui::RichText::new(text.to_string()).color(Color32::WHITE).font(medium_font(13.0))).fill(Color32::from_rgb(47, 116, 208)).corner_radius(CornerRadius::same(7)).min_size(Vec2::new(0.0, 32.0))
}

/// A status dot and its text, for the status bar.
pub fn status_dot(ui: &mut egui::Ui, colour: Color32, text: impl Into<String>) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let (r, _) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
        ui.painter().circle_filled(r.center(), 3.5, colour);
        ui.label(egui::RichText::new(text.into()).size(12.0));
    });
}

/// A section label with a `?` after it that explains the section: hovering shows the text,
/// a click pins it in a popup until the next click elsewhere (the design language's rule:
/// no paragraphs of hint text in a panel).
pub fn section_help(ui: &mut egui::Ui, title: &str, help: &str) {
    let t = tokens(ui);
    ui.add_space(14.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.label(egui::RichText::new(title).font(medium_font(13.0)).color(t.text));
        help_button(ui, title, help);
    });
    ui.add_space(2.0);
}

/// The `?` of `section_help`, for use after any label.
pub fn help_button(ui: &mut egui::Ui, title: &str, help: &str) -> egui::Response {
    let t = tokens(ui);
    let id = ui.make_persistent_id(("help", title));
    let open = ui.memory(|m| m.is_popup_open(id));
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::click());
    let c = rect.center();
    let p = ui.painter();
    if open {
        p.circle_filled(c, 8.0, t.accent);
    } else {
        p.circle_stroke(c, 7.5, Stroke::new(1.0_f32, if resp.hovered() { t.text_dim } else { t.field_border }));
    }
    p.text(c, Align2::CENTER_CENTER, "?", medium_font(10.5), if open { Color32::WHITE } else { t.text_dim });
    let resp = if open { resp } else { resp.on_hover_ui(|ui| {
        ui.set_max_width(320.0);
        ui.label(help);
    }) };
    if resp.clicked() {
        ui.memory_mut(|m| m.toggle_popup(id));
    }
    egui::popup::popup_below_widget(ui, id, &resp, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
        ui.set_width(320.0);
        ui.label(egui::RichText::new(title).font(medium_font(13.0)));
        ui.add_space(2.0);
        ui.label(help);
    });
    resp.on_hover_cursor(egui::CursorIcon::Help)
}

/// Small section label inside a panel, followed by a hairline.
pub fn section(ui: &mut egui::Ui, title: &str) {
    let t = tokens(ui);
    ui.add_space(14.0);
    ui.label(egui::RichText::new(title).font(medium_font(13.0)).color(t.text));
    ui.add_space(2.0);
}

/// An object as a one-line row: its colour, its name, what it is, a chevron; the row of the
/// selected object sits on the selection tint. The caller opens `inspector` under it.
pub fn object_row(ui: &mut egui::Ui, colour: Color32, icon: Option<Icon>, title: &str, what: &str, selected: bool) -> egui::Response {
    let t = tokens(ui);
    let (r, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 38.0), Sense::click());
    let p = ui.painter();
    if selected {
        p.rect_filled(r, 7.0, t.accent_soft);
        p.rect_stroke(r, 7.0, Stroke::new(1.0_f32, t.header_edge), egui::StrokeKind::Inside);
    } else if resp.hovered() {
        p.rect_filled(r, 7.0, t.hover);
    }
    let mark = Rect::from_center_size(Pos2::new(r.left() + 17.0, r.center().y), Vec2::new(12.0, 12.0));
    match icon {
        Some(i) => icons::paint(ui, i, mark.expand(3.0), colour),
        None => {
            ui.painter().rect_filled(mark, 3.0, colour);
        }
    }
    let p = ui.painter();
    p.text(Pos2::new(r.left() + 34.0, r.center().y), Align2::LEFT_CENTER, title, medium_font(13.0), t.text);
    p.text(Pos2::new(r.right() - 30.0, r.center().y), Align2::RIGHT_CENTER, what, FontId::proportional(11.5), t.text_faint);
    let c = Pos2::new(r.right() - 15.0, r.center().y);
    let chevron = if selected { [c + Vec2::new(-4.0, -2.0), c + Vec2::new(0.0, 2.0), c + Vec2::new(4.0, -2.0)] } else { [c + Vec2::new(-2.0, -4.0), c + Vec2::new(2.0, 0.0), c + Vec2::new(-2.0, 4.0)] };
    p.add(Shape::line(chevron.to_vec(), Stroke::new(1.5_f32, t.text_dim)));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The card the selected object's fields open in, under its row.
pub fn inspector(ui: &mut egui::Ui, body: impl FnOnce(&mut egui::Ui)) {
    let t = tokens(ui);
    egui::Frame::NONE.fill(t.card).stroke(Stroke::new(1.0_f32, t.separator)).corner_radius(8).inner_margin(12).outer_margin(Margin { left: 0, right: 0, top: 2, bottom: 6 }).show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        body(ui);
    });
}

/// A small × that removes a row (painted: the fonts have no ×).
pub fn remove_button(ui: &mut egui::Ui, tip: &str) -> egui::Response {
    let t = tokens(ui);
    let (r, resp) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(r, 6.0, t.hover);
    }
    let c = r.center();
    let s = Stroke::new(1.5_f32, if resp.hovered() { t.error } else { t.text_faint });
    ui.painter().line_segment([c + Vec2::new(-4.0, -4.0), c + Vec2::new(4.0, 4.0)], s);
    ui.painter().line_segment([c + Vec2::new(-4.0, 4.0), c + Vec2::new(4.0, -4.0)], s);
    resp.on_hover_text(tip)
}

/// A text-only action in the accent colour ("+ Layer").
pub fn ghost_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let t = tokens(ui);
    ui.add(egui::Button::new(egui::RichText::new(text).font(medium_font(12.5)).color(t.accent_text)).frame(false))
}

/// A name field, as tall as the value boxes beside it, its text centred in the box.
pub fn name_field(ui: &mut egui::Ui, s: &mut String, width: f32) -> egui::Response {
    ui.add_sized([width, 28.0], egui::TextEdit::singleline(s).vertical_align(egui::Align::Center).margin(egui::Margin::symmetric(8, 0)))
}

/// A number field with its unit inside, right-aligned, the design language's value box.
pub fn number(ui: &mut egui::Ui, v: &mut f64, unit: &str, speed: f64, width: f32) -> egui::Response {
    let d = egui::DragValue::new(v).speed(speed).max_decimals(3);
    let d = if unit.is_empty() { d } else { d.suffix(format!(" {unit}")) };
    ui.add_sized([width, 28.0], d)
}

pub fn hairline(ui: &mut egui::Ui) {
    let t = tokens(ui);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, t.separator);
}

/// Hint text under a group of fields.
pub fn hint(ui: &mut egui::Ui, text: &str) {
    let t = tokens(ui);
    ui.label(egui::RichText::new(text).small().color(t.text_faint));
}

/// Grid row: dim label, monospace value box and a faint unit.
pub fn value_row(ui: &mut egui::Ui, label: &str, unit: &str, v: &mut f64, speed: f64) {
    let t = tokens(ui);
    ui.label(egui::RichText::new(label).color(t.text_dim));
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.add_sized([92.0, ROW_H], egui::DragValue::new(v).speed(speed).custom_formatter(|x, _| fmt_value(x)));
        ui.label(egui::RichText::new(unit).font(FontId::monospace(11.0)).color(t.text_faint));
    });
    ui.end_row();
}

/// Short, readable numbers: 30000, 0.3, 7.5e6.
pub fn fmt_value(x: f64) -> String {
    let a = x.abs();
    if a != 0.0 && !(1e-3..1e6).contains(&a) {
        format!("{x:.3e}").replace(".000e", "e").replace("00e", "e").replace("0e", "e")
    } else {
        let s = format!("{x:.4}");
        let s = s.trim_end_matches('0').trim_end_matches('.');
        if s == "-0" { "0".into() } else { s.into() }
    }
}

/// Model-explorer tree node.
pub fn tree_node(ui: &mut egui::Ui, id: &str, label: &str, add_body: impl FnOnce(&mut egui::Ui)) {
    egui::CollapsingHeader::new(egui::RichText::new(label).size(12.0)).id_salt(id).default_open(false).show(ui, add_body);
}

/// Tree item whose body edits the object; returns true when its header was clicked.
pub fn tree_item(ui: &mut egui::Ui, id: impl std::hash::Hash, label: impl Into<egui::WidgetText>, add_body: impl FnOnce(&mut egui::Ui)) -> bool {
    egui::CollapsingHeader::new(label).id_salt(id).default_open(false).show(ui, add_body).header_response.clicked()
}

/// Drawing-area background: canvas colour with a dot grid every `pitch` pixels.
pub fn canvas_background(painter: &egui::Painter, rect: Rect, t: &Tokens) {
    painter.rect_filled(rect, 0.0, t.canvas);
    let pitch = 22.0;
    let mut y = rect.top() + pitch / 2.0;
    while y < rect.bottom() {
        let mut x = rect.left() + pitch / 2.0;
        while x < rect.right() {
            painter.circle_filled(Pos2::new(x, y), 0.9, t.canvas_dot);
            x += pitch;
        }
        y += pitch;
    }
}

/// Soft drop shadow under a model rectangle.
pub fn soft_shadow(painter: &egui::Painter, rect: Rect, t: &Tokens) {
    let shadow = egui::epaint::Shadow { offset: [0, 6], blur: 24, spread: 0, color: Color32::from_black_alpha(if t.dark { 120 } else { 45 }) };
    painter.add(shadow.as_shape(rect, CornerRadius::ZERO));
}

pub fn nice_step(range: f64, target_ticks: f64) -> f64 {
    // a squeezed canvas (a side panel opened) can ask for no or negative ticks
    let raw = (range.abs() / target_ticks.max(1.0)).max(1e-9);
    if !raw.is_finite() {
        return 1.0;
    }
    let p = 10f64.powf(raw.log10().floor());
    for m in [1.0, 2.0, 5.0, 10.0] {
        if raw <= m * p {
            return m * p;
        }
    }
    10.0 * p
}

fn tick_label(v: f64, step: f64) -> String {
    let d = if step >= 1.0 { 0 } else { (-step.log10()).ceil() as usize };
    let v = if v.abs() < step * 1e-6 { 0.0 } else { v };
    format!("{v:.d$}")
}

/// Rulers along the top and left of `rect`; returns the inner plotting rect.
pub fn rulers(painter: &egui::Painter, rect: Rect, view: &View2D, t: &Tokens) -> Rect {
    let th = 20.0;
    let tw = 34.0;
    let top = Rect::from_min_max(Pos2::new(rect.left() + tw, rect.top()), Pos2::new(rect.right(), rect.top() + th));
    let left = Rect::from_min_max(Pos2::new(rect.left(), rect.top() + th), Pos2::new(rect.left() + tw, rect.bottom()));
    painter.rect_filled(top, 0.0, t.chrome);
    painter.rect_filled(left, 0.0, t.chrome);
    painter.rect_filled(Rect::from_min_max(rect.min, Pos2::new(rect.left() + tw, rect.top() + th)), 0.0, t.chrome);
    let ink = Stroke::new(1.0_f32, t.text_faint);
    let font = FontId::monospace(9.5);
    let w0 = view.to_world(top.left_center())[0];
    let w1 = view.to_world(top.right_center())[0];
    let major = nice_step(w1 - w0, (top.width() / 80.0) as f64);
    let minor = major / 5.0;
    let mut x = (w0 / minor).ceil() * minor;
    // at most a few hundred ticks, whatever the view
    let ok = |a: f64, b: f64, step: f64| a.is_finite() && b.is_finite() && step > 0.0 && (b - a) / step < 2000.0;
    if !ok(w0, w1, minor) {
        x = f64::INFINITY;
    }
    while x <= w1 {
        let sx = view.to_screen([x, 0.0]).x;
        let is_major = ((x / major).round() * major - x).abs() < minor * 0.01;
        painter.line_segment([Pos2::new(sx, top.bottom()), Pos2::new(sx, top.bottom() - if is_major { 7.0 } else { 3.0 })], ink);
        if is_major {
            painter.text(Pos2::new(sx + 3.0, top.top() + 2.0), Align2::LEFT_TOP, tick_label(x, major), font.clone(), t.text_dim);
        }
        x += minor;
    }
    let h0 = view.to_world(left.center_bottom())[1];
    let h1 = view.to_world(left.center_top())[1];
    let major = nice_step(h1 - h0, (left.height() / 60.0) as f64);
    let minor = major / 5.0;
    let mut y = (h0 / minor).ceil() * minor;
    if !ok(h0, h1, minor) {
        y = f64::INFINITY;
    }
    while y <= h1 {
        let sy = view.to_screen([0.0, y]).y;
        let is_major = ((y / major).round() * major - y).abs() < minor * 0.01;
        painter.line_segment([Pos2::new(left.right(), sy), Pos2::new(left.right() - if is_major { 7.0 } else { 3.0 }, sy)], ink);
        if is_major {
            painter.text(Pos2::new(left.right() - 9.0, sy), Align2::RIGHT_CENTER, tick_label(y, major), font.clone(), t.text_dim);
        }
        y += minor;
    }
    let edge = Stroke::new(1.0_f32, t.separator);
    painter.line_segment([Pos2::new(rect.left(), top.bottom()), top.right_bottom()], edge);
    painter.line_segment([Pos2::new(left.right(), rect.top()), left.right_bottom()], edge);
    Rect::from_min_max(Pos2::new(left.right(), top.bottom()), rect.max)
}

/// Global axes at the origin: green y and red x arrows.
pub fn origin_axes(painter: &egui::Painter, view: &View2D, t: &Tokens) {
    let o = view.to_screen([0.0, 0.0]);
    let arrow = |dir: Vec2, col: Color32, label: &str| {
        let tip = o + dir * 28.0;
        painter.line_segment([o, tip], Stroke::new(1.6_f32, col));
        let n = Vec2::new(-dir.y, dir.x);
        painter.add(Shape::convex_polygon(vec![tip + dir * 7.0, tip + n * 3.5, tip - n * 3.5], col, Stroke::NONE));
        painter.text(tip + dir * 14.0, Align2::CENTER_CENTER, label, medium_font(11.0), col);
    };
    arrow(Vec2::new(0.0, -1.0), Color32::from_rgb(70, 190, 90), "y");
    arrow(Vec2::new(1.0, 0.0), Color32::from_rgb(235, 80, 70), "x");
    painter.circle_filled(o, 3.0, t.ink);
}

/// Text with a translucent backing so it stays legible over soil colours.
pub fn label_on_canvas(painter: &egui::Painter, pos: Pos2, align: Align2, text: &str, col: Color32, t: &Tokens) {
    let galley = painter.layout_no_wrap(text.to_string(), FontId::proportional(11.0), col);
    let r = align.anchor_size(pos, galley.size()).expand2(Vec2::new(4.0, 1.5));
    let bg = if t.dark { Color32::from_black_alpha(150) } else { Color32::from_white_alpha(200) };
    painter.rect_filled(r, RADIUS_SM, bg);
    painter.galley(r.min + Vec2::new(4.0, 1.5), galley, col);
}

/// One entry of the command-line session log.
pub struct LogLine {
    pub text: String,
    pub kind: LogKind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LogKind {
    Command,
    Reply,
    Info,
    Error,
}

/// PLAXIS command-line panel: "Session" / "Model history" tabs over a log, and a command entry.
/// Returns a command typed by the user (Enter or the run button).
pub fn command_line(ui: &mut egui::Ui, tab: &mut usize, session: &[LogLine], history: &[String], entry: &mut String, enabled: bool, hint: &str) -> Option<String> {
    let t = tokens(ui);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Command line").font(medium_font(12.0)));
        ui.add_space(8.0);
        ui.selectable_value(tab, 0, "Session");
        ui.selectable_value(tab, 1, "Model history");
    });
    // the entry row, spacing and the frame's margins and border below the log; reserving
    // less makes the resizable panel grow every frame
    let avail = ui.available_height() - ROW_H - 2.0 * ui.spacing().item_spacing.y - 12.0 - 2.0 - 4.0;
    egui::Frame::NONE.fill(t.field).stroke(Stroke::new(1.0_f32, t.separator)).corner_radius(RADIUS).inner_margin(6).show(ui, |ui| {
        egui::ScrollArea::vertical().max_height(avail.max(40.0)).stick_to_bottom(true).auto_shrink(false).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            let mono = |s: &str, c: Color32| egui::RichText::new(s).monospace().size(11.5).color(c);
            if *tab == 0 {
                let mut n = 0;
                for l in session {
                    let line = match l.kind {
                        LogKind::Command => {
                            n += 1;
                            mono(&format!("{n:04}> {}", l.text), t.log_command)
                        }
                        LogKind::Reply => mono(&format!("      {}", l.text), t.log_reply),
                        LogKind::Info => mono(&l.text, t.text_dim),
                        LogKind::Error => mono(&l.text, t.error),
                    };
                    ui.label(line);
                }
            } else {
                for (i, c) in history.iter().enumerate() {
                    ui.label(mono(&format!("{:04}> {c}", i + 1), t.log_command));
                }
            }
        });
    });
    let mut out = None;
    ui.horizontal(|ui| {
        let w = ui.available_width() - 40.0;
        let resp = ui.add_enabled(enabled, egui::TextEdit::singleline(entry).font(FontId::monospace(12.0)).desired_width(w).hint_text(if enabled { "Type a command" } else { hint }));
        let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let run = ui.add_enabled_ui(enabled, |ui| icons::button(ui, Icon::Play, "Run command", false)).inner.clicked();
        if (run || enter) && !entry.trim().is_empty() {
            out = Some(std::mem::take(entry));
        }
    });
    out
}

/// Status bar cell.
pub fn status_cell(ui: &mut egui::Ui, text: impl Into<String>, width: f32) {
    let t = tokens(ui);
    egui::Frame::NONE.fill(t.field).corner_radius(RADIUS_SM).inner_margin(Margin::symmetric(8, 2)).show(ui, |ui| {
        ui.set_min_width(width);
        ui.label(egui::RichText::new(text.into()).font(FontId::monospace(11.0)).color(t.text_dim));
    });
}

/// Tool strip button.
pub fn tool_button(ui: &mut egui::Ui, icon: Icon, tip: &str, selected: bool) -> bool {
    icons::button(ui, icon, tip, selected).clicked()
}


// ------------------------------------------------------------------ TellusHive additions

/// A titled group in a step panel, as Tellus3D lays them out: the title (and a faint context
/// after it) above a rounded card of rows.
pub fn explorer(ui: &mut egui::Ui, title: &str, context: &str, add: impl FnOnce(&mut egui::Ui)) {
    let t = tokens(ui);
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(title).font(medium_font(13.0)).color(t.text));
        if !context.is_empty() {
            ui.label(egui::RichText::new(context).size(12.0).color(t.text_faint));
        }
    });
    ui.add_space(2.0);
    rows_card(ui, add);
}

/// A labelled text field: the label on the left in the dim colour, a 28 px box filling the row.
pub fn field(ui: &mut egui::Ui, label: &str, s: &mut String) -> egui::Response {
    ui.horizontal(|ui| {
        ui.add_sized([96.0, 28.0], egui::Label::new(egui::RichText::new(label).color(tokens(ui).text_dim)));
        ui.add_sized([ui.available_width(), 28.0], egui::TextEdit::singleline(s).vertical_align(egui::Align::Center).margin(Margin::symmetric(8, 0)))
    })
    .inner
}

pub fn password(ui: &mut egui::Ui, label: &str, s: &mut String) -> egui::Response {
    ui.horizontal(|ui| {
        ui.add_sized([96.0, 28.0], egui::Label::new(egui::RichText::new(label).color(tokens(ui).text_dim)));
        ui.add_sized([ui.available_width(), 28.0], egui::TextEdit::singleline(s).password(true).vertical_align(egui::Align::Center).margin(Margin::symmetric(8, 0)))
    })
    .inner
}

/// A label in the left column of a form row (same width as `field`'s).
pub fn row_label(ui: &mut egui::Ui, label: &str) {
    ui.add_sized([96.0, 28.0], egui::Label::new(egui::RichText::new(label).color(tokens(ui).text_dim)));
}

pub fn hex_color(s: &str, fallback: Color32) -> Color32 {
    let h = s.trim().trim_start_matches('#');
    if h.len() == 6 {
        if let Ok(v) = u32::from_str_radix(h, 16) {
            return Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8);
        }
    }
    fallback
}

pub fn to_hex(c: Color32) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b())
}

pub fn open_path(p: &std::path::Path) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(p).spawn();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("cmd").args(["/C", "start", ""]).arg(p).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(p).spawn();
}

/// A button with a Lucide icon and a label, in the floating toolbars.
pub fn icon_text_button(ui: &mut egui::Ui, icon: Icon, label: &str) -> egui::Response {
    let t = tokens(ui);
    let font = FontId::proportional(13.0);
    let tw = ui.fonts(|f| f.layout_no_wrap(label.to_string(), font.clone(), t.text).size().x);
    let (r, resp) = ui.allocate_exact_size(Vec2::new(tw + 40.0, 32.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(r, 7.0, t.hover);
    }
    icons::paint(ui, icon, Rect::from_center_size(Pos2::new(r.left() + 17.0, r.center().y), Vec2::splat(16.0)), t.text_dim);
    ui.painter().text(Pos2::new(r.left() + 31.0, r.center().y), Align2::LEFT_CENTER, label, font, t.text);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The picture to show for a file: SVG drawings are rasterised with the engine's renderer
/// (which has the fonts) into a cached PNG, because egui's SVG loader draws them small and
/// without text. Other files are returned unchanged.
pub fn display_path(p: &std::path::Path) -> std::path::PathBuf {
    if !p.extension().is_some_and(|e| e.eq_ignore_ascii_case("svg")) {
        return p.to_path_buf();
    }
    use std::hash::{Hash, Hasher};
    let mtime = p.metadata().and_then(|m| m.modified()).ok();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    p.hash(&mut h);
    mtime.hash(&mut h);
    let dir = std::env::temp_dir().join("tellushive-previews");
    let out = dir.join(format!("{:016x}.png", h.finish()));
    if out.exists() {
        return out;
    }
    let Ok(svg) = std::fs::read_to_string(p) else { return p.to_path_buf() };
    match th_eng::drawing::svg_to_png(&svg, 1600) {
        Ok((png, _, _)) => {
            let _ = std::fs::create_dir_all(&dir);
            if std::fs::write(&out, png).is_ok() {
                out
            } else {
                p.to_path_buf()
            }
        }
        Err(_) => p.to_path_buf(),
    }
}
