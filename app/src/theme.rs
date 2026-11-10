//! Look and feel: one set of colour tokens mapped onto egui's `Visuals` and `Style`,
//! with Inter / JetBrains Mono. The approach follows PhotoCraft's `ui-egui` theme
//! (github.com/storytold/photocraft, MIT OR Apache-2.0); fonts are OFL, see assets/fonts.

use std::sync::Arc;

use eframe::egui::{self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Id, Margin, Shadow, Stroke, TextStyle, Vec2};

/// Semantic colours; every widget reads these instead of hard-coding a colour.
#[derive(Clone, Copy)]
pub struct Tokens {
    pub dark: bool,
    /// Panels and bars.
    pub chrome: Color32,
    /// Menu bar / tab strips, a step darker than `chrome`.
    pub dock: Color32,
    /// Raised content: explorer bodies, popups.
    pub card: Color32,
    /// Explorer title bar and its outline: a steel-blue tint, so the titles stand apart
    /// from the grey panels by hue, not only by lightness.
    pub header: Color32,
    pub header_edge: Color32,
    /// Text fields and value boxes.
    pub field: Color32,
    pub field_border: Color32,
    pub hover: Color32,
    pub pressed: Color32,
    pub separator: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub text_faint: Color32,
    pub accent: Color32,
    pub accent_soft: Color32,
    pub accent_text: Color32,
    /// Drawing area around the model, its dot grid and the ink of rulers / labels on it.
    pub canvas: Color32,
    pub canvas_dot: Color32,
    pub ink: Color32,
    pub ok: Color32,
    pub warn: Color32,
    pub error: Color32,
    pub log_command: Color32,
    pub log_reply: Color32,
    pub tools: ToolHues,
}

/// Stroke colours of the tool-strip icons: each tool takes the colour of what it makes, as
/// drawn on the canvas (loads blue like their arrows). Select and view tools stay neutral.
#[derive(Clone, Copy)]
pub struct ToolHues {
    /// Boreholes, soil layers and soil polygons.
    pub soil: Color32,
    pub plate: Color32,
    pub load: Color32,
    pub mesh: Color32,
    pub water: Color32,
    /// Calculate.
    pub run: Color32,
    /// View the results (Output).
    pub view: Color32,
    pub fixity: Color32,
    pub spring: Color32,
    pub geogrid: Color32,
    pub pile: Color32,
}

pub const RADIUS_SM: u8 = 4;
pub const RADIUS: u8 = 6;
/// Height of value fields, buttons and tab strips.
pub const ROW_H: f32 = 24.0;
pub const TAB_STRIP_H: f32 = 26.0;

impl Tokens {
    pub const DARK: Tokens = Tokens {
        dark: true,
        chrome: Color32::from_rgb(31, 32, 36),
        dock: Color32::from_rgb(24, 25, 28),
        card: Color32::from_rgb(35, 37, 42),
        header: Color32::from_rgb(42, 47, 58),
        header_edge: Color32::from_rgb(61, 111, 184),
        field: Color32::from_rgb(24, 25, 28),
        field_border: Color32::from_rgb(52, 54, 60),
        hover: Color32::from_rgb(44, 46, 51),
        pressed: Color32::from_rgb(52, 54, 60),
        separator: Color32::from_rgb(42, 43, 48),
        text: Color32::from_rgb(236, 237, 239),
        text_dim: Color32::from_rgb(169, 172, 180),
        text_faint: Color32::from_rgb(140, 143, 152),
        accent: Color32::from_rgb(47, 116, 208),
        accent_soft: Color32::from_rgb(42, 47, 58),
        accent_text: Color32::from_rgb(141, 184, 245),
        canvas: Color32::from_rgb(28, 29, 32),
        canvas_dot: Color32::from_rgb(44, 46, 51),
        ink: Color32::from_rgb(190, 190, 196),
        ok: Color32::from_rgb(110, 200, 120),
        warn: Color32::from_rgb(232, 170, 70),
        error: Color32::from_rgb(240, 138, 126),
        log_command: Color32::from_rgb(140, 190, 255),
        log_reply: Color32::from_rgb(120, 200, 190),
        tools: ToolHues {
            soil: Color32::from_rgb(226, 184, 94),
            plate: Color32::from_rgb(157, 180, 211),
            load: Color32::from_rgb(76, 155, 255),
            mesh: Color32::from_rgb(180, 140, 255),
            water: Color32::from_rgb(79, 195, 232),
            run: Color32::from_rgb(110, 200, 120),
            view: Color32::from_rgb(240, 153, 123),
            // as the canvas draws them
            fixity: Color32::from_rgb(210, 90, 200),
            spring: Color32::from_rgb(235, 150, 40),
            geogrid: Color32::from_rgb(70, 200, 110),
            pile: Color32::from_rgb(170, 110, 60),
        },
    };

    pub const LIGHT: Tokens = Tokens {
        dark: false,
        chrome: Color32::from_rgb(246, 247, 249),
        dock: Color32::from_rgb(255, 255, 255),
        card: Color32::from_rgb(255, 255, 255),
        header: Color32::from_rgb(227, 237, 251),
        header_edge: Color32::from_rgb(138, 180, 238),
        field: Color32::from_rgb(255, 255, 255),
        field_border: Color32::from_rgb(205, 209, 216),
        hover: Color32::from_rgb(233, 235, 239),
        pressed: Color32::from_rgb(218, 221, 226),
        separator: Color32::from_rgb(225, 227, 232),
        text: Color32::from_rgb(27, 29, 34),
        text_dim: Color32::from_rgb(90, 95, 105),
        text_faint: Color32::from_rgb(107, 112, 122),
        accent: Color32::from_rgb(47, 116, 208),
        accent_soft: Color32::from_rgb(227, 237, 251),
        accent_text: Color32::from_rgb(31, 99, 194),
        canvas: Color32::from_rgb(238, 240, 243),
        canvas_dot: Color32::from_rgb(213, 216, 222),
        ink: Color32::from_rgb(70, 74, 82),
        ok: Color32::from_rgb(46, 158, 75),
        warn: Color32::from_rgb(154, 106, 10),
        error: Color32::from_rgb(194, 59, 46),
        log_command: Color32::from_rgb(20, 70, 170),
        log_reply: Color32::from_rgb(20, 120, 120),
        tools: ToolHues {
            soil: Color32::from_rgb(168, 122, 30),
            plate: Color32::from_rgb(91, 122, 166),
            load: Color32::from_rgb(47, 116, 208),
            mesh: Color32::from_rgb(123, 79, 214),
            water: Color32::from_rgb(29, 143, 181),
            run: Color32::from_rgb(46, 139, 62),
            view: Color32::from_rgb(194, 90, 53),
            fixity: Color32::from_rgb(168, 60, 160),
            spring: Color32::from_rgb(196, 112, 16),
            geogrid: Color32::from_rgb(36, 146, 76),
            pile: Color32::from_rgb(140, 86, 40),
        },
    };

    pub fn get(ctx: &egui::Context) -> Tokens {
        ctx.data(|d| d.get_temp::<Tokens>(id())).unwrap_or(Tokens::DARK)
    }
}

fn id() -> Id {
    Id::new("tellushive-theme")
}

/// Stores the tokens and maps them onto egui's visuals and spacing.
pub fn apply(ctx: &egui::Context, t: Tokens) {
    ctx.data_mut(|d| d.insert_temp(id(), t));
    let mut v = if t.dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    let r = CornerRadius::same(RADIUS);
    v.panel_fill = t.chrome;
    v.window_fill = t.card;
    v.window_stroke = Stroke::new(1.0_f32, t.separator);
    v.window_corner_radius = CornerRadius::same(10);
    v.menu_corner_radius = CornerRadius::same(8);
    v.extreme_bg_color = t.field;
    v.faint_bg_color = t.card;
    v.code_bg_color = t.field;
    v.override_text_color = Some(t.text);
    v.hyperlink_color = t.accent_text;
    v.warn_fg_color = t.warn;
    v.error_fg_color = t.error;
    v.selection.bg_fill = t.accent_soft;
    v.selection.stroke = Stroke::new(1.0_f32, t.accent_text);
    v.window_shadow = Shadow { offset: [0, 10], blur: 32, spread: 0, color: Color32::from_black_alpha(if t.dark { 110 } else { 40 }) };
    v.popup_shadow = Shadow { offset: [0, 6], blur: 20, spread: 0, color: Color32::from_black_alpha(if t.dark { 100 } else { 36 }) };
    v.handle_shape = egui::style::HandleShape::Circle;
    v.slider_trailing_fill = true;
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = t.chrome;
    w.noninteractive.weak_bg_fill = t.chrome;
    w.noninteractive.bg_stroke = Stroke::new(1.0_f32, t.separator);
    w.noninteractive.fg_stroke = Stroke::new(1.0_f32, t.text_dim);
    for (s, fill) in [(&mut w.inactive, t.field), (&mut w.hovered, t.hover), (&mut w.active, t.pressed), (&mut w.open, t.hover)] {
        s.bg_fill = fill;
        s.weak_bg_fill = fill;
        s.bg_stroke = Stroke::new(1.0_f32, t.field_border);
        s.fg_stroke = Stroke::new(1.0_f32, t.text);
        s.corner_radius = r;
        s.expansion = 0.0;
    }
    w.inactive.fg_stroke = Stroke::new(1.0_f32, t.text_dim);
    w.hovered.bg_stroke = Stroke::new(1.0_f32, t.text_faint);
    w.active.bg_stroke = Stroke::new(1.0_f32, t.accent);
    w.noninteractive.corner_radius = r;

    // eframe follows the system light/dark setting, so both theme slots get this style
    let mut s = (*ctx.style()).clone();
    s.visuals = v;
    s.text_styles = [
        (TextStyle::Small, FontId::proportional(11.0)),
        (TextStyle::Body, FontId::proportional(13.0)),
        (TextStyle::Button, FontId::proportional(13.0)),
        (TextStyle::Heading, FontId::new(15.0, medium())),
        (TextStyle::Monospace, FontId::monospace(12.0)),
    ]
    .into();
    let sp = &mut s.spacing;
    sp.item_spacing = Vec2::new(8.0, 6.0);
    sp.button_padding = Vec2::new(10.0, 4.0);
    sp.interact_size = Vec2::new(40.0, ROW_H);
    sp.menu_margin = Margin::same(6);
    sp.window_margin = Margin::same(10);
    sp.indent = 16.0;
    sp.combo_width = 120.0;
    sp.scroll = egui::style::ScrollStyle::thin();
    s.interaction.tooltip_delay = 0.35;
    ctx.set_style_of(egui::Theme::Dark, s.clone());
    ctx.set_style_of(egui::Theme::Light, s);
    ctx.set_theme(if t.dark { egui::ThemePreference::Dark } else { egui::ThemePreference::Light });
}

pub fn medium() -> FontFamily {
    FontFamily::Name("medium".into())
}

/// Medium-weight Inter for headers and tabs.
pub fn medium_font(size: f32) -> FontId {
    FontId::new(size, medium())
}

pub fn install_fonts(ctx: &egui::Context) {
    let mut f = FontDefinitions::default();
    let add = |f: &mut FontDefinitions, name: &str, bytes: &'static [u8]| {
        f.font_data.insert(name.into(), Arc::new(FontData::from_static(bytes)));
    };
    add(&mut f, "Inter", include_bytes!("../../assets/fonts/Inter-Regular.ttf"));
    add(&mut f, "InterMedium", include_bytes!("../../assets/fonts/Inter-Medium.ttf"));
    add(&mut f, "JetBrainsMono", include_bytes!("../../assets/fonts/JetBrainsMono-Regular.ttf"));
    // Thai script (Inter and JetBrains Mono have no Thai glyphs), tried after the main font
    add(&mut f, "NotoSansThai", include_bytes!("../../assets/fonts/NotoSansThai-Regular.ttf"));
    // egui's own fonts stay as fallbacks (symbols, emoji)
    let fallback = f.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    f.families.entry(FontFamily::Proportional).or_default().splice(0..0, ["Inter".to_string(), "NotoSansThai".to_string()]);
    f.families.entry(FontFamily::Monospace).or_default().splice(0..0, ["JetBrainsMono".to_string(), "NotoSansThai".to_string()]);
    let mut med = vec!["InterMedium".to_string(), "Inter".to_string(), "NotoSansThai".to_string()];
    med.extend(fallback);
    f.families.insert(medium(), med);
    ctx.set_fonts(f);
}
