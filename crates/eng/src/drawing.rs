//! 2D engineering drawings in world coordinates (y up), exported to SVG for the screen and the
//! report, to DXF R12 for CAD, and rasterised to PNG with resvg using the bundled fonts.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::{Arc, OnceLock};

use serde::{Deserialize, Serialize};

pub(crate) static INTER: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");
pub(crate) static INTER_SEMI: &[u8] = include_bytes!("../../../assets/fonts/Inter-SemiBold.ttf");
pub(crate) static INTER_MEDIUM: &[u8] = include_bytes!("../../../assets/fonts/Inter-Medium.ttf");
pub(crate) static MONO: &[u8] = include_bytes!("../../../assets/fonts/JetBrainsMono-Regular.ttf");
pub(crate) static THAI: &[u8] = include_bytes!("../../../assets/fonts/NotoSansThai-Regular.ttf");

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Drawing {
    pub title: String,
    /// "mm" or "m"; empty means "m".
    pub units: String,
    pub scale_note: String,
    pub layers: Vec<Layer>,
    pub entities: Vec<Entity>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Layer {
    pub name: String,
    /// "#rrggbb"
    pub color: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Entity {
    Line {
        layer: String,
        a: [f64; 2],
        b: [f64; 2],
    },
    Polyline {
        layer: String,
        points: Vec<[f64; 2]>,
        #[serde(default)]
        closed: bool,
        /// "concrete" | "soil" | "solid" | "water" | null
        #[serde(default)]
        hatch: Option<String>,
    },
    Circle {
        layer: String,
        c: [f64; 2],
        r: f64,
        #[serde(default)]
        filled: bool,
    },
    /// Counter-clockwise from start_deg to end_deg, degrees from +x.
    Arc {
        layer: String,
        c: [f64; 2],
        r: f64,
        start_deg: f64,
        end_deg: f64,
    },
    /// `at` is the baseline point; `height` is in drawing units.
    Text {
        layer: String,
        at: [f64; 2],
        text: String,
        height: f64,
        /// "left" (default) | "center" | "right"
        #[serde(default)]
        align: String,
    },
    /// Aligned linear dimension from a to b, the dimension line offset by `offset` to the
    /// left of a->b (negative: to the right). Text defaults to the measured length.
    Dim {
        layer: String,
        a: [f64; 2],
        b: [f64; 2],
        offset: f64,
        #[serde(default)]
        text: Option<String>,
    },
}

impl Drawing {
    pub fn new(title: &str, units: &str) -> Self {
        Drawing { title: title.into(), units: units.into(), ..Default::default() }
    }
    pub fn layer(&mut self, name: &str, color: &str) -> &mut Self {
        self.layers.push(Layer { name: name.into(), color: color.into() });
        self
    }
    pub fn add(&mut self, e: Entity) -> &mut Self {
        self.entities.push(e);
        self
    }
    fn units(&self) -> &str {
        if self.units.trim().is_empty() {
            "m"
        } else {
            self.units.trim()
        }
    }
    fn color_of(&self, layer: &str) -> String {
        self.layers
            .iter()
            .find(|l| l.name == layer)
            .and_then(|l| parse_hex(&l.color))
            .map(|c| format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]))
            .unwrap_or_else(|| "#000000".into())
    }
    /// Annotation size (dimension text height) scaled to the geometry, so dimensions read the
    /// same whether the drawing is in mm or m.
    /// Dimension text height: the median height of the drawing's own text, so dimensions read
    /// at the size of the labels beside them; without any text, the annotation size.
    fn text_h(&self) -> f64 {
        let mut h: Vec<f64> = self
            .entities
            .iter()
            .filter_map(|e| match e {
                Entity::Text { height, .. } if *height > 0.0 && height.is_finite() => Some(*height),
                _ => None,
            })
            .collect();
        if h.is_empty() {
            return self.anno();
        }
        h.sort_by(|a, b| a.total_cmp(b));
        h[h.len() / 2]
    }
    fn anno(&self) -> f64 {
        let mut b = Bounds::new();
        for e in &self.entities {
            match e {
                Entity::Text { .. } => {}
                Entity::Dim { a, b: bb, .. } => {
                    b.add(*a);
                    b.add(*bb);
                }
                _ => entity_bounds(e, 0.0, &mut b),
            }
        }
        if !b.valid() {
            return 1.0;
        }
        let ext = (b.x1 - b.x0).max(b.y1 - b.y0);
        if ext > 0.0 {
            0.032 * ext
        } else {
            1.0
        }
    }
}

// ------------------------------------------------------------------ geometry helpers

#[derive(Debug, Clone, Copy)]
struct Bounds {
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
}

impl Bounds {
    fn new() -> Self {
        Bounds { x0: f64::INFINITY, y0: f64::INFINITY, x1: f64::NEG_INFINITY, y1: f64::NEG_INFINITY }
    }
    fn add(&mut self, p: [f64; 2]) {
        if p[0].is_finite() && p[1].is_finite() {
            self.x0 = self.x0.min(p[0]);
            self.y0 = self.y0.min(p[1]);
            self.x1 = self.x1.max(p[0]);
            self.y1 = self.y1.max(p[1]);
        }
    }
    fn valid(&self) -> bool {
        self.x0 <= self.x1 && self.y0 <= self.y1
    }
}

/// Approximate horizontal extent of a text run, in units of its height.
fn text_width(s: &str, h: f64) -> f64 {
    0.56 * h * s.chars().count() as f64
}

fn text_left(x: f64, w: f64, align: &str) -> f64 {
    match align {
        "center" => x - w / 2.0,
        "right" => x - w,
        _ => x,
    }
}

fn entity_bounds(e: &Entity, anno: f64, b: &mut Bounds) {
    match e {
        Entity::Line { a, b: p, .. } => {
            b.add(*a);
            b.add(*p);
        }
        Entity::Polyline { points, .. } => points.iter().for_each(|p| b.add(*p)),
        Entity::Circle { c, r, .. } => {
            b.add([c[0] - r, c[1] - r]);
            b.add([c[0] + r, c[1] + r]);
        }
        Entity::Arc { c, r, start_deg, end_deg, .. } => {
            for p in arc_points(*c, *r, *start_deg, *end_deg, 24) {
                b.add(p);
            }
        }
        Entity::Text { at, text, height, align, .. } => {
            let w = text_width(text, *height);
            let x = text_left(at[0], w, align);
            b.add([x, at[1] - 0.25 * height]);
            b.add([x + w, at[1] + height]);
        }
        Entity::Dim { .. } => {
            if let Some(g) = dim_geometry(e, anno, "m") {
                for (p, q) in &g.lines {
                    b.add(*p);
                    b.add(*q);
                }
                let w = text_width(&g.text, g.text_h) / 2.0;
                b.add([g.text_at[0] - w, g.text_at[1] - w]);
                b.add([g.text_at[0] + w, g.text_at[1] + w]);
            }
        }
    }
}

/// Points along an arc, counter-clockwise from start to end (degrees).
fn arc_points(c: [f64; 2], r: f64, start: f64, end: f64, n: usize) -> Vec<[f64; 2]> {
    let mut sweep = (end - start) % 360.0;
    if sweep <= 0.0 {
        sweep += 360.0;
    }
    (0..=n)
        .map(|k| {
            let t = (start + sweep * k as f64 / n as f64).to_radians();
            [c[0] + r * t.cos(), c[1] + r * t.sin()]
        })
        .collect()
}

/// A dimension broken into plain primitives, shared by the SVG and DXF writers.
struct DimGeo {
    lines: Vec<([f64; 2], [f64; 2])>,
    arrows: Vec<[[f64; 2]; 3]>,
    text: String,
    text_at: [f64; 2],
    text_angle: f64, // degrees, counter-clockwise, in (-90, 90]
    text_h: f64,
}

fn dim_geometry(e: &Entity, th: f64, units: &str) -> Option<DimGeo> {
    let Entity::Dim { a, b, offset, text, .. } = e else { return None };
    let dx = b[0] - a[0];
    let dy = b[1] - a[1];
    let len = (dx * dx + dy * dy).sqrt();
    if !(len > 0.0) || !len.is_finite() {
        return None;
    }
    let u = [dx / len, dy / len];
    let n = [-u[1], u[0]];
    let sgn = if *offset < 0.0 { -1.0 } else { 1.0 };
    let pa = [a[0] + n[0] * offset, a[1] + n[1] * offset];
    let pb = [b[0] + n[0] * offset, b[1] + n[1] * offset];
    let gap = 0.35 * th;
    let ext = 0.5 * th;
    let mut lines = vec![];
    // extension lines (only when the offset is longer than the gap)
    if offset.abs() > gap {
        for (p, q) in [(a, pa), (b, pb)] {
            lines.push(([p[0] + n[0] * sgn * gap, p[1] + n[1] * sgn * gap], [q[0] + n[0] * sgn * ext, q[1] + n[1] * sgn * ext]));
        }
    }
    lines.push((pa, pb));
    let al = 0.7 * th;
    let aw = 0.22 * th;
    let arrow = |tip: [f64; 2], dir: f64| -> [[f64; 2]; 3] {
        // dir +1: arrow points backwards along u (tip at the start)
        let base = [tip[0] + u[0] * al * dir, tip[1] + u[1] * al * dir];
        [tip, [base[0] + n[0] * aw, base[1] + n[1] * aw], [base[0] - n[0] * aw, base[1] - n[1] * aw]]
    };
    // arrows inside if there is room, otherwise outside pointing in
    let inside = len > 2.6 * al;
    let arrows = if inside { vec![arrow(pa, 1.0), arrow(pb, -1.0)] } else { vec![arrow(pa, -1.0), arrow(pb, 1.0)] };
    let mut ang = u[1].atan2(u[0]).to_degrees();
    if ang > 90.0 + 1e-9 {
        ang -= 180.0;
    } else if ang <= -90.0 + 1e-9 {
        ang += 180.0;
    }
    // put the text on the outer side of the dimension line, clear of it
    let up = [-(ang.to_radians().sin()), ang.to_radians().cos()];
    let out = [n[0] * sgn, n[1] * sgn];
    let mid = [(pa[0] + pb[0]) / 2.0, (pa[1] + pb[1]) / 2.0];
    let along = if up[0] * out[0] + up[1] * out[1] >= 0.0 { 0.3 * th } else { 1.3 * th };
    let text_at = [mid[0] + out[0] * along, mid[1] + out[1] * along];
    let text = match text {
        Some(t) if !t.trim().is_empty() => t.clone(),
        _ => {
            if units == "mm" {
                format!("{len:.0}")
            } else {
                format!("{len:.2}")
            }
        }
    };
    Some(DimGeo { lines, arrows, text, text_at, text_angle: ang, text_h: th })
}

/// Parallel hatch lines clipped to a polygon, for DXF (R12 has no HATCH entity).
fn hatch_lines(poly: &[[f64; 2]], angle_deg: f64, spacing: f64) -> Vec<([f64; 2], [f64; 2])> {
    let mut out = vec![];
    if poly.len() < 3 || !(spacing > 0.0) {
        return out;
    }
    let t = angle_deg.to_radians();
    let d = [t.cos(), t.sin()];
    let nn = [-t.sin(), t.cos()];
    let proj = |p: &[f64; 2]| p[0] * nn[0] + p[1] * nn[1];
    let lo = poly.iter().map(proj).fold(f64::INFINITY, f64::min);
    let hi = poly.iter().map(proj).fold(f64::NEG_INFINITY, f64::max);
    let count = ((hi - lo) / spacing).floor() as usize;
    if count > 4000 {
        return out; // pathological spacing; skip rather than flood the file
    }
    for k in 0..=count {
        let c = lo + spacing * (k as f64 + 0.5);
        if c > hi {
            break;
        }
        let mut s = vec![];
        for i in 0..poly.len() {
            let p = poly[i];
            let q = poly[(i + 1) % poly.len()];
            let fp = proj(&p) - c;
            let fq = proj(&q) - c;
            if (fp > 0.0) != (fq > 0.0) {
                let l = fp / (fp - fq);
                let x = [p[0] + (q[0] - p[0]) * l, p[1] + (q[1] - p[1]) * l];
                s.push(x[0] * d[0] + x[1] * d[1]);
            }
        }
        s.sort_by(|a, b| a.total_cmp(b));
        for pair in s.chunks(2) {
            if let [s0, s1] = pair {
                let p0 = [d[0] * s0 + nn[0] * c, d[1] * s0 + nn[1] * c];
                let p1 = [d[0] * s1 + nn[0] * c, d[1] * s1 + nn[1] * c];
                out.push((p0, p1));
            }
        }
    }
    out
}

fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let h = s.trim().trim_start_matches('#');
    let v = |i: usize, n: usize| u8::from_str_radix(h.get(i..i + n)?, 16).ok();
    match h.len() {
        6 => Some([v(0, 2)?, v(2, 2)?, v(4, 2)?]),
        3 => Some([v(0, 1)? * 17, v(1, 1)? * 17, v(2, 1)? * 17]),
        _ => None,
    }
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

// ------------------------------------------------------------------ SVG

/// SVG of the drawing: world y-up mapped to SVG, fitted with a margin, white background, a
/// title strip at the bottom, hatches as patterns and one `<g>` per layer.
pub fn to_svg(d: &Drawing, width_px: u32) -> String {
    let wpx = width_px.max(200) as f64;
    let th = d.text_h();
    let mut b = Bounds::new();
    for e in &d.entities {
        entity_bounds(e, th, &mut b);
    }
    if !b.valid() {
        b = Bounds { x0: 0.0, y0: 0.0, x1: 1.0, y1: 1.0 };
    }
    let bw = (b.x1 - b.x0).max(1e-9);
    let bh = (b.y1 - b.y0).max(1e-9);
    let margin = (0.05 * wpx).max(16.0);
    let s = ((wpx - 2.0 * margin) / bw).min((1.3 * wpx - 2.0 * margin) / bh);
    let ox = (wpx - bw * s) / 2.0;
    let content_h = bh * s + 2.0 * margin;
    let strip = (0.045 * wpx).clamp(30.0, 60.0);
    let hpx = content_h + strip;
    let px = |p: [f64; 2]| -> (f64, f64) { (ox + (p[0] - b.x0) * s, margin + (b.y1 - p[1]) * s) };
    let lw = (wpx / 700.0).max(1.0);

    let mut o = String::new();
    let _ = write!(
        o,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{wpx:.0}" height="{hpx:.0}" viewBox="0 0 {wpx:.2} {hpx:.2}" font-family="Inter, Noto Sans Thai, sans-serif">"#
    );
    // hatch patterns
    let t = (wpx / 90.0).max(6.0);
    let _ = write!(
        o,
        r##"<defs><pattern id="h-concrete" patternUnits="userSpaceOnUse" width="{w:.1}" height="{w:.1}"><rect width="{w:.1}" height="{w:.1}" fill="#f1f1ee"/><circle cx="{a:.1}" cy="{a:.1}" r="{r:.2}" fill="#777"/><circle cx="{c:.1}" cy="{e:.1}" r="{r:.2}" fill="#777"/><path d="M{f:.1},{g:.1} l{k:.1},{k2:.1} l{k:.1},-{k2:.1} z" fill="none" stroke="#888" stroke-width="{sw:.2}"/></pattern><pattern id="h-soil" patternUnits="userSpaceOnUse" width="{t:.1}" height="{t:.1}" patternTransform="rotate(45)"><rect width="{t:.1}" height="{t:.1}" fill="#faf6ee"/><line x1="0" y1="0" x2="0" y2="{t:.1}" stroke="#a08560" stroke-width="{sw:.2}"/></pattern><pattern id="h-water" patternUnits="userSpaceOnUse" width="{t2:.1}" height="{t:.1}"><rect width="{t2:.1}" height="{t:.1}" fill="#e3f0fb"/><line x1="{q:.1}" y1="{hh:.1}" x2="{q3:.1}" y2="{hh:.1}" stroke="#3b82c4" stroke-width="{sw:.2}"/></pattern></defs>"##,
        w = 2.0 * t,
        a = 0.4 * t,
        c = 1.4 * t,
        e = 1.1 * t,
        r = 0.09 * t,
        f = 0.3 * t,
        g = 1.6 * t,
        k = 0.25 * t,
        k2 = 0.35 * t,
        sw = 0.08 * t,
        t2 = 2.0 * t,
        q = 0.4 * t,
        q3 = 1.4 * t,
        hh = 0.5 * t,
    );
    let _ = write!(o, r##"<rect x="0" y="0" width="{wpx:.2}" height="{hpx:.2}" fill="#ffffff"/>"##);

    // group by layer, keeping the order of first appearance (declared layers first)
    let mut order: Vec<String> = d.layers.iter().map(|l| l.name.clone()).collect();
    for e in &d.entities {
        let l = entity_layer(e).to_string();
        if !order.contains(&l) {
            order.push(l);
        }
    }
    for layer in &order {
        let col = d.color_of(layer);
        let ents: Vec<&Entity> = d.entities.iter().filter(|e| entity_layer(e) == layer).collect();
        if ents.is_empty() {
            continue;
        }
        let _ = write!(o, r#"<g id="layer-{}" stroke="{col}" fill="none" stroke-width="{lw:.2}" stroke-linejoin="round" stroke-linecap="round">"#, xml(&svg_id(layer)));
        for e in ents {
            match e {
                Entity::Line { a, b: q, .. } => {
                    let (x1, y1) = px(*a);
                    let (x2, y2) = px(*q);
                    let _ = write!(o, r#"<line x1="{x1:.2}" y1="{y1:.2}" x2="{x2:.2}" y2="{y2:.2}"/>"#);
                }
                Entity::Polyline { points, closed, hatch, .. } => {
                    if points.is_empty() {
                        continue;
                    }
                    let pts: Vec<String> = points
                        .iter()
                        .map(|p| {
                            let (x, y) = px(*p);
                            format!("{x:.2},{y:.2}")
                        })
                        .collect();
                    let fill = match hatch.as_deref() {
                        Some("concrete") => "url(#h-concrete)".to_string(),
                        Some("soil") => "url(#h-soil)".to_string(),
                        Some("water") => "url(#h-water)".to_string(),
                        Some("solid") => col.clone(),
                        _ => "none".to_string(),
                    };
                    let tag = if *closed || hatch.is_some() { "polygon" } else { "polyline" };
                    let op = if hatch.as_deref() == Some("solid") { r#" fill-opacity="0.35""# } else { "" };
                    let _ = write!(o, r#"<{tag} points="{}" fill="{fill}"{op}/>"#, pts.join(" "));
                    if hatch.is_some() && !*closed {
                        // a hatched open polyline: fill the area but stroke only the open path
                        let _ = write!(o, r#"<polyline points="{}"/>"#, pts.join(" "));
                    }
                }
                Entity::Circle { c, r, filled, .. } => {
                    let (x, y) = px(*c);
                    let f = if *filled { col.as_str() } else { "none" };
                    let _ = write!(o, r#"<circle cx="{x:.2}" cy="{y:.2}" r="{:.2}" fill="{f}"/>"#, r * s);
                }
                Entity::Arc { c, r, start_deg, end_deg, .. } => {
                    let mut sweep = (end_deg - start_deg) % 360.0;
                    if sweep <= 0.0 {
                        sweep += 360.0;
                    }
                    if sweep >= 359.999 {
                        let (x, y) = px(*c);
                        let _ = write!(o, r#"<circle cx="{x:.2}" cy="{y:.2}" r="{:.2}"/>"#, r * s);
                        continue;
                    }
                    let p0 = [c[0] + r * start_deg.to_radians().cos(), c[1] + r * start_deg.to_radians().sin()];
                    let p1 = [c[0] + r * end_deg.to_radians().cos(), c[1] + r * end_deg.to_radians().sin()];
                    let (x0, y0) = px(p0);
                    let (x1, y1) = px(p1);
                    let large = if sweep > 180.0 { 1 } else { 0 };
                    // counter-clockwise in y-up world is sweep-flag 0 on the y-down screen
                    let _ = write!(o, r#"<path d="M{x0:.2},{y0:.2} A{rr:.2},{rr:.2} 0 {large} 0 {x1:.2},{y1:.2}"/>"#, rr = r * s);
                }
                Entity::Text { at, text, height, align, .. } => {
                    let (x, y) = px(*at);
                    let anchor = match align.as_str() {
                        "center" => "middle",
                        "right" => "end",
                        _ => "start",
                    };
                    let _ = write!(
                        o,
                        r#"<text x="{x:.2}" y="{y:.2}" font-size="{:.2}" text-anchor="{anchor}" fill="{col}" stroke="none">{}</text>"#,
                        height * s,
                        xml(text)
                    );
                }
                Entity::Dim { .. } => {
                    let Some(g) = dim_geometry(e, th, d.units()) else { continue };
                    let dw = (lw * 0.6).max(0.6);
                    for (p, q) in &g.lines {
                        let (x1, y1) = px(*p);
                        let (x2, y2) = px(*q);
                        let _ = write!(o, r#"<line x1="{x1:.2}" y1="{y1:.2}" x2="{x2:.2}" y2="{y2:.2}" stroke-width="{dw:.2}"/>"#);
                    }
                    for tri in &g.arrows {
                        let pts: Vec<String> = tri
                            .iter()
                            .map(|p| {
                                let (x, y) = px(*p);
                                format!("{x:.2},{y:.2}")
                            })
                            .collect();
                        let _ = write!(o, r#"<polygon points="{}" fill="{col}" stroke="none"/>"#, pts.join(" "));
                    }
                    let (x, y) = px(g.text_at);
                    let _ = write!(
                        o,
                        r#"<text x="{x:.2}" y="{y:.2}" font-size="{:.2}" text-anchor="middle" fill="{col}" stroke="none" transform="rotate({:.2} {x:.2} {y:.2})">{}</text>"#,
                        g.text_h * s,
                        -g.text_angle,
                        xml(&g.text)
                    );
                }
            }
        }
        o.push_str("</g>");
    }

    // title block strip
    let y0 = content_h;
    let fs = strip * 0.36;
    let ty = y0 + strip * 0.62;
    let _ = write!(
        o,
        r##"<g id="title-block"><rect x="0" y="{y0:.2}" width="{wpx:.2}" height="{strip:.2}" fill="#f3f5f8" stroke="#2a2f3a" stroke-width="{lw:.2}"/><text x="{tx:.2}" y="{ty:.2}" font-size="{fs:.2}" font-weight="600" fill="#1a1f2b">{title}</text><text x="{mx:.2}" y="{ty:.2}" font-size="{fs2:.2}" text-anchor="middle" fill="#3a4150">{scale}</text><text x="{rx:.2}" y="{ty:.2}" font-size="{fs2:.2}" text-anchor="end" fill="#3a4150">Units: {units}</text></g>"##,
        tx = strip * 0.4,
        mx = wpx * 0.62,
        rx = wpx - strip * 0.4,
        fs2 = fs * 0.85,
        title = xml(&d.title),
        scale = xml(&d.scale_note),
        units = xml(d.units()),
    );
    o.push_str("</svg>");
    o
}

fn svg_id(s: &str) -> String {
    s.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect()
}

fn entity_layer(e: &Entity) -> &str {
    match e {
        Entity::Line { layer, .. }
        | Entity::Polyline { layer, .. }
        | Entity::Circle { layer, .. }
        | Entity::Arc { layer, .. }
        | Entity::Text { layer, .. }
        | Entity::Dim { layer, .. } => layer,
    }
}

// ------------------------------------------------------------------ DXF R12

/// ACI (AutoCAD Color Index) palette entry for index 1..=255, approximated from the standard
/// layout: 1-9 named colours, 10-249 24 hues x 10 shades, 250-255 greys.
fn aci_rgb(i: u8) -> [u8; 3] {
    match i {
        1 => [255, 0, 0],
        2 => [255, 255, 0],
        3 => [0, 255, 0],
        4 => [0, 255, 255],
        5 => [0, 0, 255],
        6 => [255, 0, 255],
        7 => [0, 0, 0], // white on dark screens, black on paper
        8 => [128, 128, 128],
        9 => [192, 192, 192],
        10..=249 => {
            let hue = ((i / 10) as f64 - 1.0) * 15.0;
            let shade = i % 10;
            let v = [1.0, 1.0, 0.65, 0.65, 0.5, 0.5, 0.3, 0.3, 0.15, 0.15][shade as usize];
            let sat = if shade % 2 == 0 { 1.0 } else { 0.5 };
            hsv(hue, sat, v)
        }
        250..=255 => {
            let g = [51u8, 80, 105, 130, 190, 255][(i - 250) as usize];
            [g, g, g]
        }
        _ => [0, 0, 0],
    }
}

fn hsv(h: f64, s: f64, v: f64) -> [u8; 3] {
    let c = v * s;
    let hp = (h / 60.0) % 6.0;
    let x = c * (1.0 - ((hp % 2.0) - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    [((r + m) * 255.0).round() as u8, ((g + m) * 255.0).round() as u8, ((b + m) * 255.0).round() as u8]
}

/// Nearest ACI colour to a hex colour. Black and white map to 7 (the paper-ink colour).
pub fn nearest_aci(hex: &str) -> u8 {
    let Some(c) = parse_hex(hex) else { return 7 };
    let mx = *c.iter().max().unwrap_or(&0) as i32;
    let mn = *c.iter().min().unwrap_or(&0) as i32;
    // black, near-white and dark greys are "ink": ACI 7 shows white on a dark CAD screen
    // and prints black, whereas a dark grey ACI colour vanishes on the dark background
    if (mx < 40) || (mn > 235) || (mx < 110 && mx - mn < 40) {
        return 7;
    }
    let mut best = (i64::MAX, 7u8);
    for i in 1..=255u8 {
        if i == 7 {
            continue;
        }
        let p = aci_rgb(i);
        let dist: i64 = (0..3).map(|k| (p[k] as i64 - c[k] as i64).pow(2)).sum();
        if dist < best.0 {
            best = (dist, i);
        }
    }
    best.1
}

/// DXF layer names: upper case letters, digits, '_', '-', '$'.
fn dxf_layer(s: &str) -> String {
    let n: String = s.trim().chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '$' { c.to_ascii_uppercase() } else { '_' }).collect();
    if n.is_empty() {
        "0".into()
    } else {
        n
    }
}

/// DXF R12 strings are 8-bit; anything outside ASCII goes as \U+XXXX, which AutoCAD reads.
fn dxf_text(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars() {
        if c == '\n' || c == '\r' {
            o.push(' ');
        } else if c.is_ascii() {
            o.push(c);
        } else {
            let _ = write!(o, "\\U+{:04X}", c as u32);
        }
    }
    o
}

struct Dxf(String);

impl Dxf {
    fn g(&mut self, code: i32, v: &str) {
        let _ = write!(self.0, "{code:>3}\n{v}\n");
    }
    fn f(&mut self, code: i32, v: f64) {
        let v = if v.is_finite() { v } else { 0.0 };
        let _ = write!(self.0, "{code:>3}\n{v:.6}\n");
    }
    fn i(&mut self, code: i32, v: i64) {
        let _ = write!(self.0, "{code:>3}\n{v:>6}\n");
    }
    fn xy(&mut self, base: i32, p: [f64; 2]) {
        self.f(base, p[0]);
        self.f(base + 10, p[1]);
        self.f(base + 20, 0.0);
    }
    fn line(&mut self, layer: &str, a: [f64; 2], b: [f64; 2]) {
        self.g(0, "LINE");
        self.g(8, layer);
        self.xy(10, a);
        self.xy(11, b);
    }
    fn text(&mut self, layer: &str, at: [f64; 2], h: f64, s: &str, rot: f64, just: i64) {
        self.g(0, "TEXT");
        self.g(8, layer);
        self.xy(10, at);
        self.f(40, h);
        self.g(1, &dxf_text(s));
        if rot != 0.0 {
            self.f(50, rot);
        }
        self.g(7, if s.chars().any(|c| ('\u{0E00}'..='\u{0E7F}').contains(&c)) { "THAI" } else { "STANDARD" });
        if just != 0 {
            self.i(72, just);
            self.xy(11, at);
        }
    }
    /// A polyline; `bulge` per vertex and a constant `width` (used for filled circles).
    fn polyline(&mut self, layer: &str, pts: &[[f64; 2]], closed: bool, width: f64, bulge: f64) {
        self.g(0, "POLYLINE");
        self.g(8, layer);
        self.i(66, 1);
        self.xy(10, [0.0, 0.0]);
        self.i(70, if closed { 1 } else { 0 });
        if width > 0.0 {
            self.f(40, width);
            self.f(41, width);
        }
        for p in pts {
            self.g(0, "VERTEX");
            self.g(8, layer);
            self.xy(10, *p);
            if bulge != 0.0 {
                self.f(42, bulge);
            }
        }
        self.g(0, "SEQEND");
        self.g(8, layer);
    }
}

/// ASCII DXF R12 (AC1009): HEADER, TABLES (LTYPE, LAYER, STYLE), BLOCKS, ENTITIES, EOF.
/// Dimensions are exploded into lines and text; hatches into clipped lines; a filled circle
/// is a circle plus a donut (a wide two-bulge polyline), the R12 way of filling a disc.
pub fn to_dxf(d: &Drawing) -> String {
    let anno = d.anno();
    let th = d.text_h();
    let mut b = Bounds::new();
    for e in &d.entities {
        entity_bounds(e, th, &mut b);
    }
    if !b.valid() {
        b = Bounds { x0: 0.0, y0: 0.0, x1: 1.0, y1: 1.0 };
    }
    // layers: declared ones plus any used but undeclared (black)
    let mut layers: BTreeMap<String, u8> = BTreeMap::new();
    layers.insert("0".into(), 7);
    for l in &d.layers {
        layers.insert(dxf_layer(&l.name), nearest_aci(&l.color));
    }
    for e in &d.entities {
        layers.entry(dxf_layer(entity_layer(e))).or_insert(7);
    }
    // pattern hatches go on their own grey layer, as in CAD practice, so the outlines,
    // bars and dimensions stay readable and the hatch can be switched off
    let has_pattern = d.entities.iter().any(|e| matches!(e, Entity::Polyline { hatch: Some(h), .. } if h != "solid"));
    if has_pattern {
        layers.entry("HATCH".into()).or_insert(8);
    }

    let mut x = Dxf(String::new());
    x.g(999, "th-eng drawing");
    x.g(0, "SECTION");
    x.g(2, "HEADER");
    x.g(9, "$ACADVER");
    x.g(1, "AC1009");
    x.g(9, "$INSBASE");
    x.xy(10, [0.0, 0.0]);
    x.g(9, "$EXTMIN");
    x.xy(10, [b.x0, b.y0]);
    x.g(9, "$EXTMAX");
    x.xy(10, [b.x1, b.y1]);
    x.g(9, "$LIMMIN");
    x.f(10, b.x0);
    x.f(20, b.y0);
    x.g(9, "$LIMMAX");
    x.f(10, b.x1);
    x.f(20, b.y1);
    x.g(9, "$TEXTSIZE");
    x.f(40, th);
    x.g(0, "ENDSEC");

    x.g(0, "SECTION");
    x.g(2, "TABLES");
    x.g(0, "TABLE");
    x.g(2, "LTYPE");
    x.i(70, 1);
    x.g(0, "LTYPE");
    x.g(2, "CONTINUOUS");
    x.i(70, 0);
    x.g(3, "Solid line");
    x.i(72, 65);
    x.i(73, 0);
    x.f(40, 0.0);
    x.g(0, "ENDTAB");
    x.g(0, "TABLE");
    x.g(2, "LAYER");
    x.i(70, layers.len() as i64);
    for (name, col) in &layers {
        x.g(0, "LAYER");
        x.g(2, name);
        x.i(70, 0);
        x.i(62, *col as i64);
        x.g(6, "CONTINUOUS");
    }
    x.g(0, "ENDTAB");
    x.g(0, "TABLE");
    x.g(2, "STYLE");
    x.i(70, 2);
    // TrueType fonts that AutoCAD and CADCraft both find on macOS and Windows, rather than
    // the txt.shx shape font, which CADCraft has to substitute; Tahoma carries Thai
    for (name, font) in [("STANDARD", "arial.ttf"), ("THAI", "tahoma.ttf")] {
        x.g(0, "STYLE");
        x.g(2, name);
        x.i(70, 0);
        x.f(40, 0.0);
        x.f(41, 1.0);
        x.f(50, 0.0);
        x.i(71, 0);
        x.f(42, th);
        x.g(3, font);
        x.g(4, "");
    }
    x.g(0, "ENDTAB");
    x.g(0, "ENDSEC");

    x.g(0, "SECTION");
    x.g(2, "BLOCKS");
    x.g(0, "ENDSEC");

    x.g(0, "SECTION");
    x.g(2, "ENTITIES");
    for e in &d.entities {
        let layer = dxf_layer(entity_layer(e));
        match e {
            Entity::Line { a, b: q, .. } => x.line(&layer, *a, *q),
            Entity::Polyline { points, closed, hatch, .. } => {
                if points.len() >= 2 {
                    x.polyline(&layer, points, *closed, 0.0, 0.0);
                }
                if let Some(h) = hatch {
                    let set: &[(f64, f64)] = match h.as_str() {
                        "concrete" => &[(45.0, 1.2), (135.0, 1.2)],
                        "soil" => &[(45.0, 0.5)],
                        "water" => &[(0.0, 0.4)],
                        "solid" => &[(45.0, 0.1)],
                        _ => &[],
                    };
                    let hl = if h == "solid" { layer.clone() } else { "HATCH".to_string() };
                    for (ang, sp) in set {
                        for (p, q) in hatch_lines(points, *ang, sp * anno) {
                            x.line(&hl, p, q);
                        }
                    }
                }
            }
            Entity::Circle { c, r, filled, .. } => {
                x.g(0, "CIRCLE");
                x.g(8, &layer);
                x.xy(10, *c);
                x.f(40, *r);
                if *filled && *r > 0.0 {
                    x.polyline(&layer, &[[c[0] - r / 2.0, c[1]], [c[0] + r / 2.0, c[1]]], true, *r, 1.0);
                }
            }
            Entity::Arc { c, r, start_deg, end_deg, .. } => {
                x.g(0, "ARC");
                x.g(8, &layer);
                x.xy(10, *c);
                x.f(40, *r);
                x.f(50, start_deg.rem_euclid(360.0));
                x.f(51, end_deg.rem_euclid(360.0));
            }
            Entity::Text { at, text, height, align, .. } => {
                let j = match align.as_str() {
                    "center" => 1,
                    "right" => 2,
                    _ => 0,
                };
                x.text(&layer, *at, *height, text, 0.0, j);
            }
            Entity::Dim { .. } => {
                if let Some(g) = dim_geometry(e, th, d.units()) {
                    for (p, q) in &g.lines {
                        x.line(&layer, *p, *q);
                    }
                    for t in &g.arrows {
                        x.line(&layer, t[0], t[1]);
                        x.line(&layer, t[1], t[2]);
                        x.line(&layer, t[2], t[0]);
                    }
                    // bottom-centre justified (72 = 1), rotated along the dimension
                    x.text(&layer, g.text_at, g.text_h, &g.text, g.text_angle, 1);
                }
            }
        }
    }
    x.g(0, "ENDSEC");
    x.g(0, "EOF");
    x.0
}

// ------------------------------------------------------------------ PNG

fn fontdb() -> Arc<resvg::usvg::fontdb::Database> {
    static DB: OnceLock<Arc<resvg::usvg::fontdb::Database>> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = resvg::usvg::fontdb::Database::new();
        for f in [INTER, INTER_SEMI, INTER_MEDIUM, MONO, THAI] {
            db.load_font_data(f.to_vec());
        }
        db.set_sans_serif_family("Inter");
        Arc::new(db)
    })
    .clone()
}

/// Rasterises an SVG to PNG at `width_px` (height follows the aspect ratio), on white, with
/// the bundled fonts. Returns (png bytes, width, height).
pub fn svg_to_png(svg: &str, width_px: u32) -> Result<(Vec<u8>, u32, u32), String> {
    use resvg::{tiny_skia, usvg};
    let opt = usvg::Options { font_family: "Inter".into(), fontdb: fontdb(), ..Default::default() };
    let tree = usvg::Tree::from_str(svg, &opt).map_err(|e| format!("svg parse: {e}"))?;
    let size = tree.size();
    if !(size.width() > 0.0) || !(size.height() > 0.0) {
        return Err("svg has zero size".into());
    }
    let w = width_px.clamp(16, 8000);
    let scale = w as f32 / size.width();
    let h = ((size.height() * scale).round() as u32).clamp(1, 16000);
    let mut pix = tiny_skia::Pixmap::new(w, h).ok_or("cannot allocate the image")?;
    pix.fill(tiny_skia::Color::WHITE);
    resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale), &mut pix.as_mut());
    let png = pix.encode_png().map_err(|e| format!("png encode: {e}"))?;
    Ok((png, w, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Drawing {
        let mut d = Drawing::new("Test", "mm");
        d.layer("concrete", "#444444").layer("dims", "#2060c0");
        d.add(Entity::Polyline { layer: "concrete".into(), points: vec![[0.0, 0.0], [300.0, 0.0], [300.0, 500.0], [0.0, 500.0]], closed: true, hatch: Some("concrete".into()) });
        d.add(Entity::Circle { layer: "bars".into(), c: [60.0, 60.0], r: 10.0, filled: true });
        d.add(Entity::Arc { layer: "concrete".into(), c: [150.0, 250.0], r: 40.0, start_deg: 0.0, end_deg: 270.0 });
        d.add(Entity::Text { layer: "dims".into(), at: [150.0, 520.0], text: "3-DB20 ทดสอบ".into(), height: 18.0, align: "center".into() });
        d.add(Entity::Dim { layer: "dims".into(), a: [0.0, 0.0], b: [300.0, 0.0], offset: -60.0, text: None });
        d.add(Entity::Dim { layer: "dims".into(), a: [0.0, 0.0], b: [0.0, 500.0], offset: 60.0, text: Some("h = 500".into()) });
        d
    }

    #[test]
    fn svg_dxf_png() {
        let d = sample();
        let svg = to_svg(&d, 800);
        assert!(svg.starts_with("<svg") && svg.contains("h-concrete") && svg.contains(">300<"));
        let dxf = to_dxf(&d);
        assert!(dxf.contains("AC1009") && dxf.trim_end().ends_with("EOF"));
        assert!(dxf.contains("\nBARS\n"), "undeclared layer must be in the LAYER table");
        let (png, w, h) = svg_to_png(&svg, 600).unwrap();
        assert_eq!(&png[1..4], b"PNG");
        assert_eq!(w, 600);
        assert!(h > 100);
    }

    #[test]
    fn aci_colours() {
        assert_eq!(nearest_aci("#ff0000"), 1);
        assert_eq!(nearest_aci("#0000ff"), 5);
        assert_eq!(nearest_aci("#000000"), 7);
        assert_eq!(nearest_aci("bogus"), 7);
        assert_eq!(nearest_aci("#808080"), 8);
    }

    #[test]
    fn hatch_clips_to_square() {
        let sq = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        let l = hatch_lines(&sq, 0.0, 1.0);
        assert_eq!(l.len(), 10);
        for (p, q) in l {
            assert!((p[0] - 0.0).abs() < 1e-9 && (q[0] - 10.0).abs() < 1e-9);
        }
    }
}

#[cfg(test)]
mod aci_tests {
    #[test]
    fn dark_ink_maps_to_7() {
        assert_eq!(super::nearest_aci("#333333"), 7);
        assert_eq!(super::nearest_aci("#202830"), 7);
        assert_eq!(super::nearest_aci("#000000"), 7);
        assert_ne!(super::nearest_aci("#c0392b"), 7);
    }
}
