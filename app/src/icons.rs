//! Toolbar icons: Lucide SVGs (ISC, assets/icons/LICENSE-lucide.txt) rendered white by
//! egui_extras' SVG loader and tinted per state, plus a few PLAXIS-specific glyphs
//! (plate, loads) painted directly.

use eframe::egui::{self, Color32, Pos2, Rect, Sense, Shape, Stroke, Vec2};

use crate::theme::{Tokens, RADIUS};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Select,
    Play,
    Eye,
    Ruler,
    ZoomIn,
    /// Orbit (3D view).
    Rotate,
    /// Pan (3D view).
    Pan,
    /// Zoom by dragging (3D view).
    ZoomDrag,
    ZoomOut,
    Fit,
    Mesh,
    Water,
    Moon,
    Sun,
    Undo,
    Redo,
    // painted
    Plate,
    LineLoad,
    PointLoad,
    Polygon,
    Rectangle,
    Arrows,
    Contours,
    Points,
    Curves,
    Fixity,
    Spring,
    Geogrid,
    Pile,
    Tunnel,
    Move,
    Borehole,
    Drains,
    Refine,
    Slab,
    /// A cross-section line through a block (Output).
    Section,
    Home,
    Plus,
    Folder,
    Users,
    Plug,
    Settings,
    Chat,
    Paperclip,
    Send,
    FileText,
    Drafting,
    Calculator,
    Library,
    Picture,
    Close,
    Book,
    Edit,
    Dashboard,
}

const SVGS: &[(&str, &[u8])] = &[
    ("mouse-pointer-2", include_bytes!("../assets/icons/mouse-pointer-2.svg")),
    ("play", include_bytes!("../assets/icons/play.svg")),
    ("eye", include_bytes!("../assets/icons/eye.svg")),
    ("ruler", include_bytes!("../assets/icons/ruler.svg")),
    ("zoom-in", include_bytes!("../assets/icons/zoom-in.svg")),
    ("rotate-3d", include_bytes!("../assets/icons/rotate-3d.svg")),
    ("hand", include_bytes!("../assets/icons/hand.svg")),
    ("search", include_bytes!("../assets/icons/search.svg")),
    ("maximize-2", include_bytes!("../assets/icons/maximize-2.svg")),
    ("triangle", include_bytes!("../assets/icons/triangle.svg")),
    ("droplet", include_bytes!("../assets/icons/droplet.svg")),
    ("moon", include_bytes!("../assets/icons/moon.svg")),
    ("sun", include_bytes!("../assets/icons/sun.svg")),
    ("undo-2", include_bytes!("../assets/icons/undo-2.svg")),
    ("redo-2", include_bytes!("../assets/icons/redo-2.svg")),
    ("house", include_bytes!("../assets/icons/house.svg")),
    ("plus", include_bytes!("../assets/icons/plus.svg")),
    ("folder-open", include_bytes!("../assets/icons/folder-open.svg")),
    ("users", include_bytes!("../assets/icons/users.svg")),
    ("plug", include_bytes!("../assets/icons/plug.svg")),
    ("settings", include_bytes!("../assets/icons/settings.svg")),
    ("message-square", include_bytes!("../assets/icons/message-square.svg")),
    ("paperclip", include_bytes!("../assets/icons/paperclip.svg")),
    ("send", include_bytes!("../assets/icons/send.svg")),
    ("file-text", include_bytes!("../assets/icons/file-text.svg")),
    ("pencil-ruler", include_bytes!("../assets/icons/pencil-ruler.svg")),
    ("calculator", include_bytes!("../assets/icons/calculator.svg")),
    ("library", include_bytes!("../assets/icons/library.svg")),
    ("image", include_bytes!("../assets/icons/image.svg")),
    ("x", include_bytes!("../assets/icons/x.svg")),
    ("book-open", include_bytes!("../assets/icons/book-open.svg")),
    ("square-pen", include_bytes!("../assets/icons/square-pen.svg")),
    ("layout-dashboard", include_bytes!("../assets/icons/layout-dashboard.svg")),
];

impl Icon {
    fn svg(self) -> Option<&'static str> {
        Some(match self {
            Icon::Select => "mouse-pointer-2",
            Icon::Play => "play",
            Icon::Eye => "eye",
            Icon::Ruler => "ruler",
            Icon::ZoomIn => "zoom-in",
            Icon::Rotate => "rotate-3d",
            Icon::Pan => "hand",
            Icon::ZoomDrag => "search",
            Icon::Fit => "maximize-2",
            Icon::Mesh => "triangle",
            Icon::Water => "droplet",
            Icon::Moon => "moon",
            Icon::Sun => "sun",
            Icon::Undo => "undo-2",
            Icon::Redo => "redo-2",
            Icon::Home => "house",
            Icon::Plus => "plus",
            Icon::Folder => "folder-open",
            Icon::Users => "users",
            Icon::Plug => "plug",
            Icon::Settings => "settings",
            Icon::Chat => "message-square",
            Icon::Paperclip => "paperclip",
            Icon::Send => "send",
            Icon::FileText => "file-text",
            Icon::Drafting => "pencil-ruler",
            Icon::Calculator => "calculator",
            Icon::Library => "library",
            Icon::Picture => "image",
            Icon::Close => "x",
            Icon::Book => "book-open",
            Icon::Edit => "square-pen",
            Icon::Dashboard => "layout-dashboard",
            Icon::ZoomOut | Icon::Plate | Icon::LineLoad | Icon::PointLoad | Icon::Polygon | Icon::Rectangle | Icon::Arrows | Icon::Contours | Icon::Points | Icon::Curves | Icon::Fixity | Icon::Spring | Icon::Geogrid | Icon::Pile | Icon::Tunnel | Icon::Move | Icon::Borehole | Icon::Drains | Icon::Refine | Icon::Slab | Icon::Section => return None,
        })
    }
}

impl Icon {
    /// The tool-strip colour of the icon (see `ToolHues`); none for neutral icons.
    fn hue(self, t: &Tokens) -> Option<Color32> {
        let h = &t.tools;
        Some(match self {
            Icon::Ruler | Icon::Polygon | Icon::Rectangle | Icon::Borehole => h.soil,
            Icon::Plate | Icon::Slab | Icon::Tunnel => h.plate,
            Icon::Drains => h.water,
            Icon::Refine => h.mesh,
            Icon::LineLoad | Icon::PointLoad => h.load,
            Icon::Mesh => h.mesh,
            Icon::Water => h.water,
            Icon::Play => h.run,
            Icon::Eye => h.view,
            Icon::Fixity => h.fixity,
            Icon::Spring => h.spring,
            Icon::Geogrid => h.geogrid,
            Icon::Pile => h.pile,
            _ => return None,
        })
    }
}

/// Registers the SVG loader and the icon bytes (white strokes, slightly thinner).
pub fn install(ctx: &egui::Context) {
    egui_extras::install_image_loaders(ctx);
    for (name, bytes) in SVGS {
        let svg = String::from_utf8_lossy(bytes).replace("currentColor", "#ffffff").replace("stroke-width=\"2\"", "stroke-width=\"1.75\"");
        ctx.include_bytes(format!("bytes://icons/{name}.svg"), svg.into_bytes());
    }
}

/// Paints `icon` centred in `rect`.
pub fn paint(ui: &egui::Ui, icon: Icon, rect: Rect, tint: Color32) {
    if let Some(name) = icon.svg() {
        egui::Image::new(format!("bytes://icons/{name}.svg")).fit_to_exact_size(rect.size()).tint(tint).paint_at(ui, rect);
        return;
    }
    let p = ui.painter();
    let s = Stroke::new(1.6_f32, tint);
    let (c, h) = (rect.center(), rect.width() / 2.0);
    let at = |x: f32, y: f32| Pos2::new(c.x + x * h, c.y + y * h);
    let arrow = |tip: Pos2, len: f32| {
        p.line_segment([tip - Vec2::new(0.0, len), tip], s);
        p.add(Shape::convex_polygon(vec![tip, tip + Vec2::new(-0.28 * h, -0.4 * h), tip + Vec2::new(0.28 * h, -0.4 * h)], tint, Stroke::NONE));
    };
    match icon {
        Icon::Plate => {
            p.line_segment([at(0.0, -0.95), at(0.0, 0.95)], Stroke::new(3.0_f32, tint));
            for y in [-0.6, 0.0, 0.6] {
                p.line_segment([at(-0.55, y), at(-0.25, y + 0.25)], Stroke::new(1.0_f32, tint));
            }
        }
        Icon::LineLoad => {
            p.line_segment([at(-0.9, -0.85), at(0.9, -0.85)], s);
            for x in [-0.75, 0.0, 0.75] {
                arrow(at(x, 0.85), 1.6 * h);
            }
        }
        Icon::PointLoad => arrow(at(0.0, 0.9), 1.8 * h),
        Icon::Polygon => {
            let pts = [at(-0.85, 0.75), at(-0.55, -0.7), at(0.35, -0.9), at(0.9, 0.1), at(0.2, 0.85)];
            p.add(Shape::closed_line(pts.to_vec(), s));
            for q in pts {
                p.circle_filled(q, 1.8, tint);
            }
        }
        Icon::Rectangle => {
            p.rect_stroke(Rect::from_two_pos(at(-0.85, -0.65), at(0.85, 0.65)), 1.0, s, egui::StrokeKind::Middle);
            p.circle_filled(at(-0.85, -0.65), 1.8, tint);
            p.circle_filled(at(0.85, 0.65), 1.8, tint);
        }
        Icon::Arrows => {
            // a small displacement field: three diagonal arrows
            for (x, y) in [(-0.55, -0.35), (0.25, -0.55), (-0.15, 0.45)] {
                let (a, b) = (at(x - 0.3, y - 0.3), at(x + 0.3, y + 0.3));
                p.line_segment([a, b], s);
                let d = (b - a).normalized() * 0.35 * h;
                let n = Vec2::new(-d.y, d.x) * 0.55;
                p.add(Shape::convex_polygon(vec![b, b - d + n, b - d - n], tint, Stroke::NONE));
            }
        }
        Icon::Points => {
            // a picked node with its label flag
            p.circle_stroke(at(-0.2, 0.3), 0.32 * h, s);
            p.circle_filled(at(-0.2, 0.3), 0.1 * h, tint);
            p.line_segment([at(0.05, 0.05), at(0.55, -0.45)], s);
            p.add(Shape::convex_polygon(vec![at(0.55, -0.45), at(0.95, -0.7), at(0.55, -0.95)], tint, Stroke::NONE));
        }
        Icon::Curves => {
            // axes and a settlement-like curve
            p.line_segment([at(-0.85, -0.9), at(-0.85, 0.85)], s);
            p.line_segment([at(-0.85, 0.85), at(0.9, 0.85)], s);
            let pts: Vec<Pos2> = (0..=12).map(|i| {
                let x = i as f32 / 12.0;
                at(-0.75 + 1.6 * x, -0.75 + 1.3 * (1.0 - (-3.5 * x).exp()))
            }).collect();
            p.add(Shape::line(pts, s));
        }
        Icon::Section => {
            // a block cut by a vertical plane, with the section's end marks A and A*
            p.add(Shape::closed_line(vec![at(-0.9, -0.35), at(-0.35, -0.85), at(0.9, -0.85), at(0.9, 0.45), at(0.35, 0.95), at(-0.9, 0.95)], Stroke::new(1.0_f32, tint.gamma_multiply(0.6))));
            p.add(Shape::dashed_line(&[at(-0.55, 0.95), at(0.55, -0.85)], s, 3.0, 2.0));
            for (q, d) in [(at(-0.55, 0.95), Vec2::new(-0.35, -0.2)), (at(0.55, -0.85), Vec2::new(0.35, 0.2))] {
                p.line_segment([q, q + d * h], Stroke::new(2.2_f32, tint));
            }
        }
        Icon::Contours => {
            // nested contour bands
            for r in [0.9, 0.6, 0.3] {
                p.circle_stroke(at(0.0, 0.0), r * h, s);
            }
            p.circle_filled(at(0.0, 0.0), 0.12 * h, tint);
        }
        Icon::Fixity => {
            // a fixed line with hatching below
            p.line_segment([at(-0.9, -0.1), at(0.9, -0.1)], Stroke::new(2.4_f32, tint));
            for x in [-0.75, -0.3, 0.15, 0.6] {
                p.line_segment([at(x, -0.1), at(x - 0.3, 0.55)], Stroke::new(1.1_f32, tint));
            }
        }
        Icon::Spring => {
            // a node, a zig-zag spring and a hatched fixed end
            p.circle_filled(at(-0.85, 0.0), 0.16 * h, tint);
            let mut pts = vec![at(-0.85, 0.0), at(-0.5, 0.0)];
            for (k, x) in [-0.4, -0.2, 0.0, 0.2, 0.4].iter().enumerate() {
                pts.push(at(*x, if k % 2 == 0 { -0.35 } else { 0.35 }));
            }
            pts.push(at(0.6, 0.0));
            p.add(Shape::line(pts, s));
            p.line_segment([at(0.7, -0.6), at(0.7, 0.6)], Stroke::new(2.0_f32, tint));
            for y in [-0.45, 0.0, 0.45] {
                p.line_segment([at(0.7, y), at(0.95, y + 0.25)], Stroke::new(1.0_f32, tint));
            }
        }
        Icon::Geogrid => {
            // a horizontal line with cross ticks
            p.line_segment([at(-0.9, 0.0), at(0.9, 0.0)], Stroke::new(2.0_f32, tint));
            for x in [-0.7, -0.35, 0.0, 0.35, 0.7] {
                p.line_segment([at(x, -0.3), at(x, 0.3)], Stroke::new(1.2_f32, tint));
            }
        }
        Icon::Pile => {
            // a vertical bar with a base plate, and a load on its head
            p.line_segment([at(0.0, -0.55), at(0.0, 0.8)], Stroke::new(3.2_f32, tint));
            p.line_segment([at(-0.35, 0.85), at(0.35, 0.85)], Stroke::new(2.2_f32, tint));
            arrow(at(0.0, -0.6), 0.45 * h);
        }
        Icon::Tunnel => {
            // a bore in section with its lining and the ground line above
            p.line_segment([at(-0.95, -0.8), at(0.95, -0.8)], Stroke::new(1.2_f32, tint));
            p.circle_stroke(at(0.0, 0.2), 0.68 * h, Stroke::new(2.4_f32, tint));
            p.circle_stroke(at(0.0, 0.2), 0.4 * h, Stroke::new(1.0_f32, tint));
        }
        Icon::Move => {
            // four arrows from the centre
            for d in [Vec2::new(1.0, 0.0), Vec2::new(-1.0, 0.0), Vec2::new(0.0, 1.0), Vec2::new(0.0, -1.0)] {
                let tip = c + d * 0.92 * h;
                p.line_segment([c, tip], s);
                let n = Vec2::new(-d.y, d.x) * 0.26 * h;
                p.add(Shape::convex_polygon(vec![tip, tip - d * 0.36 * h + n, tip - d * 0.36 * h - n], tint, Stroke::NONE));
            }
        }
        Icon::Borehole => {
            // a log of three layers with the ground line
            let col = Rect::from_two_pos(at(-0.3, -0.75), at(0.3, 0.95));
            p.rect_stroke(col, 0.0, s, egui::StrokeKind::Middle);
            for y in [-0.2, 0.35] {
                p.line_segment([at(-0.3, y), at(0.3, y)], s);
            }
            p.line_segment([at(-0.95, -0.75), at(0.95, -0.75)], Stroke::new(1.2_f32, tint));
        }
        Icon::Drains => {
            // a row of vertical drains under a blanket
            p.line_segment([at(-0.95, -0.75), at(0.95, -0.75)], Stroke::new(2.0_f32, tint));
            for x in [-0.6, 0.0, 0.6] {
                let mut y = -0.6;
                while y < 0.9 {
                    p.line_segment([at(x, y), at(x, (y + 0.22).min(0.95))], s);
                    y += 0.34;
                }
            }
        }
        Icon::Refine => {
            // a coarse grid with a fine corner
            let r = Rect::from_two_pos(at(-0.85, -0.85), at(0.85, 0.85));
            p.rect_stroke(r, 0.0, s, egui::StrokeKind::Middle);
            p.line_segment([at(0.0, -0.85), at(0.0, 0.85)], s);
            p.line_segment([at(-0.85, 0.0), at(0.85, 0.0)], s);
            let thin = Stroke::new(0.9_f32, tint);
            for f in [0.28, 0.57] {
                p.line_segment([at(f, 0.0), at(f, 0.85)], thin);
                p.line_segment([at(0.0, f), at(0.85, f)], thin);
            }
        }
        Icon::Slab => {
            // a horizontal plate with hatching
            p.line_segment([at(-0.95, 0.0), at(0.95, 0.0)], Stroke::new(3.0_f32, tint));
            for x in [-0.6, 0.0, 0.6] {
                p.line_segment([at(x, 0.25), at(x + 0.25, 0.55)], Stroke::new(1.0_f32, tint));
            }
        }
        Icon::ZoomOut => {
            p.circle_stroke(at(-0.15, -0.15), 0.6 * h, s);
            p.line_segment([at(0.3, 0.3), at(0.9, 0.9)], s);
            p.line_segment([at(-0.45, -0.15), at(0.15, -0.15)], s);
        }
        _ => {}
    }
}

/// Square toolbar button: transparent, hover wash, accent fill when selected. A tool's
/// icon keeps its colour (`Icon::hue`) in every state; neutral icons brighten on hover.
pub fn button(ui: &mut egui::Ui, icon: Icon, tip: &str, selected: bool) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let size = 30.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    let hue = icon.hue(&t);
    let (fill, stroke, tint) = if selected {
        (t.accent_soft, Stroke::new(1.0_f32, t.accent), hue.unwrap_or(t.accent_text))
    } else if resp.hovered() {
        (t.hover, Stroke::NONE, hue.unwrap_or(t.text))
    } else {
        (Color32::TRANSPARENT, Stroke::NONE, hue.unwrap_or(t.text_dim))
    };
    ui.painter().rect(rect, RADIUS, fill, stroke, egui::StrokeKind::Inside);
    let g = (size * 0.56).round();
    paint(ui, icon, Rect::from_center_size(rect.center(), Vec2::splat(g)), tint);
    resp.on_hover_text(tip)
}
