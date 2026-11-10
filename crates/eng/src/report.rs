//! Calculation report as an A4 PDF (krilla, Inter with Noto Sans Thai so Thai prints) and as
//! Markdown. The layout engine (word-wrapping metrics, a flow of headings, paragraphs,
//! tables and images with page breaks, and a running header "page k of n") is ported from
//! the Pondus report writer.

use std::path::Path;
use std::sync::Arc;

use krilla::color::rgb;
use krilla::geom::{PathBuilder, Point, Size, Transform};
use krilla::image::Image as PdfImage;
use krilla::page::PageSettings;
use krilla::paint::{Fill, Stroke};
use krilla::text::{Font, TextDirection};
use krilla::Document;
use serde::{Deserialize, Serialize};

use crate::calc::Sheet;
use crate::drawing::{self, Drawing, INTER, INTER_SEMI, THAI};
use crate::expr::fmt4;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Report {
    pub title: String,
    pub project: String,
    pub engineer: String,
    pub checker: String,
    pub date: String,
    pub standards: Vec<String>,
    pub sections: Vec<Section>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Section {
    pub heading: String,
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Block {
    Paragraph {
        text: String,
    },
    Note {
        text: String,
    },
    Table {
        headers: Vec<String>,
        rows: Vec<Vec<String>>,
    },
    Sheet {
        sheet: Sheet,
    },
    Drawing {
        drawing: Drawing,
        #[serde(default)]
        caption: String,
    },
    /// A PNG or JPEG file on disk.
    Image {
        path: String,
        #[serde(default)]
        caption: String,
    },
    Svg {
        svg: String,
        #[serde(default)]
        caption: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ReportSummary {
    pub path: String,
    pub pages: usize,
    pub sections: Vec<String>,
}

// ------------------------------------------------------------------ layout (from Pondus)

const W: f32 = 595.0;
const H: f32 = 842.0;
const MARGIN: f32 = 48.0;
const TOP: f32 = 64.0;
const BOTTOM: f32 = 790.0;
/// Raster width for drawings and SVG blocks.
const RASTER_PX: u32 = 1600;

#[derive(Clone, Copy, PartialEq)]
enum Face {
    Regular,
    Semi,
}

#[derive(Clone)]
enum Op {
    Text { x: f32, y: f32, size: f32, face: Face, color: [u8; 3], s: String },
    Line { a: [f32; 2], b: [f32; 2], width: f32, color: [u8; 3] },
    Rect { x: f32, y: f32, w: f32, h: f32, color: [u8; 3] },
    Image { x: f32, y: f32, w: f32, h: f32, img: PdfImage },
}

struct Metrics {
    regular: ttf_parser::Face<'static>,
    semi: ttf_parser::Face<'static>,
    thai: ttf_parser::Face<'static>,
}

impl Metrics {
    fn new() -> Result<Self, String> {
        let p = |b: &'static [u8]| ttf_parser::Face::parse(b, 0).map_err(|e| format!("font: {e}"));
        Ok(Metrics { regular: p(INTER)?, semi: p(INTER_SEMI)?, thai: p(THAI)? })
    }
    fn face(&self, f: Face) -> &ttf_parser::Face<'static> {
        match f {
            Face::Regular => &self.regular,
            Face::Semi => &self.semi,
        }
    }
    fn width(&self, s: &str, size: f32, f: Face) -> f32 {
        let face = self.face(f);
        let upm = face.units_per_em() as f32;
        s.chars()
            .map(|c| match face.glyph_index(c) {
                Some(g) => face.glyph_hor_advance(g).unwrap_or(0) as f32 / upm,
                None => self.thai.glyph_index(c).and_then(|g| self.thai.glyph_hor_advance(g)).map(|a| a as f32 / self.thai.units_per_em() as f32).unwrap_or(0.5),
            })
            .sum::<f32>()
            * size
    }
    /// Greedy word wrap to `w` points; a single word wider than the column breaks by characters.
    fn wrap(&self, s: &str, size: f32, f: Face, w: f32) -> Vec<String> {
        let mut out = vec![];
        for para in s.split('\n') {
            let mut line = String::new();
            for word in para.split(' ') {
                let cand = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
                if self.width(&cand, size, f) <= w || line.is_empty() {
                    line = cand;
                    while self.width(&line, size, f) > w && line.chars().count() > 1 {
                        let mut cut = String::new();
                        for c in line.chars() {
                            if self.width(&format!("{cut}{c}"), size, f) > w && !cut.is_empty() {
                                break;
                            }
                            cut.push(c);
                        }
                        let rest: String = line.chars().skip(cut.chars().count()).collect();
                        out.push(cut);
                        line = rest;
                    }
                } else {
                    out.push(std::mem::take(&mut line));
                    line = word.to_string();
                }
            }
            out.push(line);
        }
        out
    }
}

struct Flow {
    pages: Vec<Vec<Op>>,
    y: f32,
    m: Metrics,
    sections: Vec<String>,
}

const INK: [u8; 3] = [24, 28, 36];
const DIM: [u8; 3] = [96, 104, 118];
const RULE: [u8; 3] = [200, 205, 214];
const BAND: [u8; 3] = [238, 241, 246];
const ACCENT: [u8; 3] = [38, 99, 210];
const BAD: [u8; 3] = [196, 40, 40];
const GOOD: [u8; 3] = [30, 120, 60];

impl Flow {
    fn new() -> Result<Self, String> {
        Ok(Flow { pages: vec![vec![]], y: TOP, m: Metrics::new()?, sections: vec![] })
    }
    fn ops(&mut self) -> &mut Vec<Op> {
        if self.pages.is_empty() {
            self.pages.push(vec![]);
        }
        let n = self.pages.len() - 1;
        &mut self.pages[n]
    }
    fn new_page(&mut self) {
        self.pages.push(vec![]);
        self.y = TOP;
    }
    fn need(&mut self, h: f32) {
        if self.y + h > BOTTOM {
            self.new_page();
        }
    }
    fn text(&mut self, x: f32, y: f32, size: f32, face: Face, color: [u8; 3], s: &str) {
        self.ops().push(Op::Text { x, y, size, face, color, s: s.into() });
    }
    fn title(&mut self, s: &str) {
        let lines = self.m.wrap(s, 18.0, Face::Semi, W - 2.0 * MARGIN);
        for l in lines {
            let y = self.y;
            self.text(MARGIN, y + 18.0, 18.0, Face::Semi, INK, &l);
            self.y += 24.0;
        }
        self.y += 6.0;
    }
    fn heading(&mut self, s: &str) {
        self.need(60.0);
        self.y += 10.0;
        self.sections.push(s.into());
        let y = self.y;
        self.text(MARGIN, y + 13.0, 14.0, Face::Semi, INK, s);
        self.ops().push(Op::Line { a: [MARGIN, y + 19.0], b: [W - MARGIN, y + 19.0], width: 0.8, color: ACCENT });
        self.y += 28.0;
    }
    fn sub(&mut self, s: &str) {
        self.need(40.0);
        self.y += 4.0;
        let y = self.y;
        self.text(MARGIN, y + 10.0, 10.5, Face::Semi, INK, s);
        self.y += 16.0;
    }
    fn para(&mut self, s: &str) {
        self.para_with(s, 9.0, INK, Face::Regular);
    }
    fn note(&mut self, s: &str) {
        self.para_with(s, 7.8, DIM, Face::Regular);
    }
    fn para_with(&mut self, s: &str, size: f32, color: [u8; 3], face: Face) {
        let lines = self.m.wrap(s, size, face, W - 2.0 * MARGIN);
        let lh = size * 1.42;
        for l in lines {
            self.need(lh);
            let y = self.y;
            self.text(MARGIN, y + size, size, face, color, &l);
            self.y += lh;
        }
        self.y += size * 0.5;
    }
    /// A table: column width shares, the header, the rows; a row flagged true prints red.
    /// Rows break across pages with the header repeated.
    fn table(&mut self, widths: &[f32], head: &[String], rows: &[(Vec<String>, bool)]) {
        let ncol = head.len().max(rows.iter().map(|r| r.0.len()).max().unwrap_or(0)).max(1);
        let mut widths: Vec<f32> = widths.to_vec();
        widths.resize(ncol, 1.0);
        let total: f32 = widths.iter().sum();
        let ws: Vec<f32> = widths.iter().map(|w| w / total * (W - 2.0 * MARGIN)).collect();
        let size = 7.6;
        let lh = size * 1.32;
        let pad = 3.0;
        let mut head = head.to_vec();
        head.resize(ncol, String::new());
        let head_lines: Vec<Vec<String>> = head.iter().zip(&ws).map(|(h, w)| self.m.wrap(h, size, Face::Semi, w - 2.0 * pad)).collect();
        let hh = head_lines.iter().map(|l| l.len()).max().unwrap_or(1) as f32 * lh + 2.0 * pad;
        let draw_head = |f: &mut Flow| {
            let y = f.y;
            f.ops().push(Op::Rect { x: MARGIN, y, w: W - 2.0 * MARGIN, h: hh, color: BAND });
            let mut x = MARGIN;
            for (k, lines) in head_lines.iter().enumerate() {
                for (i, l) in lines.iter().enumerate() {
                    f.text(x + pad, y + pad + size + i as f32 * lh, size, Face::Semi, INK, l);
                }
                x += ws[k];
            }
            f.y += hh;
        };
        self.need(hh + lh * 2.0 + 2.0 * pad);
        draw_head(self);
        for (cells, bad) in rows {
            let mut cells = cells.clone();
            cells.resize(ncol, String::new());
            let lines: Vec<Vec<String>> = cells.iter().zip(&ws).map(|(c, w)| self.m.wrap(c, size, Face::Regular, w - 2.0 * pad)).collect();
            let rh = lines.iter().map(|l| l.len()).max().unwrap_or(1) as f32 * lh + 2.0 * pad;
            if self.y + rh > BOTTOM {
                self.new_page();
                draw_head(self);
            }
            let y = self.y;
            let mut x = MARGIN;
            for (k, ls) in lines.iter().enumerate() {
                for (i, l) in ls.iter().enumerate() {
                    self.text(x + pad, y + pad + size + i as f32 * lh, size, Face::Regular, if *bad { BAD } else { INK }, l);
                }
                x += ws[k];
            }
            self.ops().push(Op::Line { a: [MARGIN, y + rh], b: [W - MARGIN, y + rh], width: 0.3, color: RULE });
            self.y += rh;
        }
        self.y += 8.0;
    }
    /// An image scaled to the text width (and to at most 60 % of the page height).
    fn image(&mut self, img: PdfImage, w: u32, h: u32, caption: &str) {
        let mut dw = W - 2.0 * MARGIN;
        let mut dh = dw * h.max(1) as f32 / w.max(1) as f32;
        let max_h = 0.6 * (BOTTOM - TOP);
        if dh > max_h {
            dw *= max_h / dh;
            dh = max_h;
        }
        self.need(dh + 24.0);
        let y = self.y;
        let x = MARGIN + (W - 2.0 * MARGIN - dw) / 2.0;
        self.ops().push(Op::Image { x, y, w: dw, h: dh, img });
        self.y += dh + 4.0;
        if !caption.is_empty() {
            self.note(caption);
        } else {
            self.y += 6.0;
        }
    }
}

fn fill(c: [u8; 3]) -> Fill {
    Fill { paint: rgb::Color::new(c[0], c[1], c[2]).into(), ..Default::default() }
}

/// Splits a string into runs that Inter can set and runs that need the Thai face.
fn runs(s: &str, inter: &ttf_parser::Face) -> Vec<(bool, String)> {
    let mut out: Vec<(bool, String)> = vec![];
    for c in s.chars() {
        let thai = inter.glyph_index(c).is_none() && ('\u{0E00}'..='\u{0E7F}').contains(&c);
        match out.last_mut() {
            Some((t, r)) if *t == thai => r.push(c),
            _ => out.push((thai, c.to_string())),
        }
    }
    out
}

fn render(pages: &[Vec<Op>], header: &str, footer: &str, title: &str) -> Result<Vec<u8>, String> {
    let mut doc = Document::new();
    doc.set_metadata(krilla::metadata::Metadata::new().title(title.into()).creator("th-eng".into()));
    let reg = Font::new(INTER.into(), 0).ok_or("font")?;
    let semi = Font::new(INTER_SEMI.into(), 0).ok_or("font")?;
    let thai = Font::new(THAI.into(), 0).ok_or("font")?;
    let m = Metrics::new()?;
    let n = pages.len();
    for (k, ops) in pages.iter().enumerate() {
        let mut page = doc.start_page_with(PageSettings::from_wh(W, H).ok_or("page")?);
        let mut s = page.surface();
        let text = |s: &mut krilla::surface::Surface, x: f32, y: f32, size: f32, face: Face, color: [u8; 3], t: &str| {
            s.set_fill(Some(fill(color)));
            let mut cx = x;
            for (is_thai, r) in runs(t, m.face(face)) {
                let font = if is_thai {
                    thai.clone()
                } else if face == Face::Semi {
                    semi.clone()
                } else {
                    reg.clone()
                };
                s.draw_text(Point::from_xy(cx, y), font, size, &r, false, TextDirection::Auto);
                cx += m.width(&r, size, face);
            }
        };
        // running header and footer
        text(&mut s, MARGIN, 36.0, 8.0, Face::Semi, DIM, header);
        let pg = format!("page {} of {n}", k + 1);
        let pw = m.width(&pg, 8.0, Face::Regular);
        text(&mut s, W - MARGIN - pw, 36.0, 8.0, Face::Regular, DIM, &pg);
        text(&mut s, MARGIN, 816.0, 7.0, Face::Regular, DIM, footer);
        for op in ops {
            match op {
                Op::Text { x, y, size, face, color, s: t } => text(&mut s, *x, *y, *size, *face, *color, t),
                Op::Rect { x, y, w, h, color } => {
                    let mut pb = PathBuilder::new();
                    pb.move_to(*x, *y);
                    pb.line_to(x + w, *y);
                    pb.line_to(x + w, y + h);
                    pb.line_to(*x, y + h);
                    pb.close();
                    if let Some(p) = pb.finish() {
                        s.set_stroke(None);
                        s.set_fill(Some(fill(*color)));
                        s.draw_path(&p);
                    }
                }
                Op::Line { a, b, width, color } => {
                    let mut pb = PathBuilder::new();
                    pb.move_to(a[0], a[1]);
                    pb.line_to(b[0], b[1]);
                    if let Some(p) = pb.finish() {
                        s.set_fill(None);
                        s.set_stroke(Some(Stroke { paint: rgb::Color::new(color[0], color[1], color[2]).into(), width: *width, ..Default::default() }));
                        s.draw_path(&p);
                        s.set_stroke(None);
                    }
                }
                Op::Image { x, y, w, h, img } => {
                    s.push_transform(&Transform::from_translate(*x, *y));
                    s.draw_image(img.clone(), Size::from_wh(*w, *h).ok_or("image size")?);
                    s.pop();
                }
            }
        }
        s.finish();
        page.finish();
    }
    doc.finish().map_err(|e| format!("pdf: {e:?}"))
}

// ------------------------------------------------------------------ content

/// Inputs, steps and checks of a sheet as table rows, shared by the PDF and Markdown writers.
fn sheet_tables(sh: &Sheet) -> (Vec<Vec<String>>, Vec<Vec<String>>, Vec<(Vec<String>, bool)>) {
    let inputs = sh.inputs.iter().map(|v| vec![v.symbol.clone(), fmt4(v.value), v.unit.clone(), v.description.clone()]).collect();
    let steps = sh
        .steps
        .iter()
        .map(|s| {
            let mut work = s.expr.clone();
            if s.substituted != s.expr {
                work.push_str(&format!(" → {}", s.substituted));
            }
            work.push_str(&format!(" → {} {}", fmt4(s.value), s.unit));
            vec![s.description.clone(), s.symbol.clone(), work.trim().to_string(), s.reference.clone()]
        })
        .collect();
    let checks = sh
        .checks()
        .iter()
        .map(|c| (vec![c.label.clone(), fmt4(c.demand), fmt4(c.capacity), format!("{:.3}", c.ratio), if c.pass { "PASS".into() } else { "FAIL".into() }], !c.pass))
        .collect();
    (inputs, steps, checks)
}

fn sheet_pdf(f: &mut Flow, sh: &Sheet) {
    if !sh.title.is_empty() {
        f.sub(&sh.title);
    }
    if !sh.standard.is_empty() {
        f.note(&format!("Standard: {}", sh.standard));
    }
    let (inputs, steps, checks) = sheet_tables(sh);
    if !inputs.is_empty() {
        let rows: Vec<(Vec<String>, bool)> = inputs.into_iter().map(|r| (r, false)).collect();
        f.table(&[1.2, 1.0, 0.9, 3.5], &["Symbol".into(), "Value".into(), "Unit".into(), "Description".into()], &rows);
    }
    if !steps.is_empty() {
        let rows: Vec<(Vec<String>, bool)> = steps.into_iter().map(|r| (r, false)).collect();
        f.table(&[2.0, 0.9, 4.2, 1.7], &["Description".into(), "Symbol".into(), "Expression → substituted → value".into(), "Reference".into()], &rows);
    }
    if !checks.is_empty() {
        f.table(&[3.4, 1.0, 1.0, 0.8, 0.8], &["Check".into(), "Demand".into(), "Capacity".into(), "Ratio".into(), "Result".into()], &checks);
    }
    for n in &sh.notes {
        f.note(&format!("Note: {n}"));
    }
    let (s, c) = if sh.all_pass { ("Result: all checks PASS.", GOOD) } else { ("Result: one or more checks FAIL.", BAD) };
    f.para_with(s, 9.0, c, Face::Semi);
}

/// Width and height from a PNG (IHDR) or JPEG (SOFn) header.
fn image_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() > 24 && &bytes[1..4] == b"PNG" {
        let w = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
        let h = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
        return Some((w, h));
    }
    if bytes.len() > 4 && bytes[0] == 0xFF && bytes[1] == 0xD8 {
        let mut i = 2;
        while i + 9 < bytes.len() {
            if bytes[i] != 0xFF {
                i += 1;
                continue;
            }
            let marker = bytes[i + 1];
            let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
            if (0xC0..=0xCF).contains(&marker) && marker != 0xC4 && marker != 0xC8 && marker != 0xCC {
                let h = u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]) as u32;
                let w = u16::from_be_bytes([bytes[i + 7], bytes[i + 8]]) as u32;
                return Some((w, h));
            }
            i += 2 + len;
        }
    }
    None
}

fn png_image(f: &mut Flow, png: Vec<u8>, w: u32, h: u32, caption: &str) -> Result<(), String> {
    let img = PdfImage::from_png(Arc::new(png).into(), true).map_err(|e| format!("png: {e}"))?;
    f.image(img, w, h, caption);
    Ok(())
}

/// Writes the report as an A4 PDF.
pub fn write_pdf(r: &Report, path: &Path) -> Result<ReportSummary, String> {
    let mut f = Flow::new()?;
    // cover block on page 1
    f.title(if r.title.is_empty() { "Calculation report" } else { &r.title });
    let mut cover: Vec<(Vec<String>, bool)> = vec![];
    for (k, v) in [("Project", &r.project), ("Engineer", &r.engineer), ("Checker", &r.checker), ("Date", &r.date)] {
        if !v.is_empty() {
            cover.push((vec![k.into(), v.clone()], false));
        }
    }
    if !r.standards.is_empty() {
        cover.push((vec!["Standards".into(), r.standards.join("\n")], false));
    }
    if !cover.is_empty() {
        f.table(&[1.0, 4.0], &["Item".into(), "".into()], &cover);
    }
    for sec in &r.sections {
        f.heading(&sec.heading);
        for b in &sec.blocks {
            match b {
                Block::Paragraph { text } => f.para(text),
                Block::Note { text } => f.note(text),
                Block::Table { headers, rows } => {
                    let n = headers.len().max(rows.iter().map(|r| r.len()).max().unwrap_or(0)).max(1);
                    let rows: Vec<(Vec<String>, bool)> = rows.iter().map(|r| (r.clone(), false)).collect();
                    f.table(&vec![1.0; n], headers, &rows);
                }
                Block::Sheet { sheet } => sheet_pdf(&mut f, sheet),
                Block::Drawing { drawing, caption } => {
                    let svg = drawing::to_svg(drawing, RASTER_PX);
                    let (png, w, h) = drawing::svg_to_png(&svg, RASTER_PX)?;
                    let cap = if caption.is_empty() { drawing.title.clone() } else { caption.clone() };
                    png_image(&mut f, png, w, h, &cap)?;
                }
                Block::Svg { svg, caption } => {
                    let (png, w, h) = drawing::svg_to_png(svg, RASTER_PX)?;
                    png_image(&mut f, png, w, h, caption)?;
                }
                Block::Image { path, caption } => {
                    let bytes = std::fs::read(path).map_err(|e| format!("image {path}: {e}"))?;
                    let (w, h) = image_size(&bytes).ok_or_else(|| format!("image {path}: only PNG and JPEG files are supported"))?;
                    let img = if bytes.get(1..4) == Some(b"PNG") {
                        PdfImage::from_png(Arc::new(bytes).into(), true)
                    } else {
                        PdfImage::from_jpeg(Arc::new(bytes).into(), true)
                    }
                    .map_err(|e| format!("image {path}: {e}"))?;
                    f.image(img, w, h, caption);
                }
            }
        }
    }
    let header = if r.project.is_empty() { r.title.clone() } else { format!("{} · {}", r.title, r.project) };
    let mut foot = vec![];
    if !r.engineer.is_empty() {
        foot.push(format!("Engineer: {}", r.engineer));
    }
    if !r.checker.is_empty() {
        foot.push(format!("Checker: {}", r.checker));
    }
    if !r.date.is_empty() {
        foot.push(r.date.clone());
    }
    foot.push("Calculation report generated by th-eng; to be checked by a qualified engineer".into());
    let pdf = render(&f.pages, &header, &foot.join("   ·   "), &r.title)?;
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
        }
    }
    std::fs::write(path, pdf).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(ReportSummary { path: path.display().to_string(), pages: f.pages.len(), sections: f.sections })
}

fn md_cell(s: &str) -> String {
    s.replace('|', "\\|").replace('\n', "<br>")
}

fn md_table(head: &[String], rows: &[Vec<String>]) -> String {
    let n = head.len().max(rows.iter().map(|r| r.len()).max().unwrap_or(0)).max(1);
    let mut h = head.to_vec();
    h.resize(n, String::new());
    let mut o = format!("| {} |\n|{}\n", h.iter().map(|c| md_cell(c)).collect::<Vec<_>>().join(" | "), "---|".repeat(n));
    for r in rows {
        let mut r = r.clone();
        r.resize(n, String::new());
        o.push_str(&format!("| {} |\n", r.iter().map(|c| md_cell(c)).collect::<Vec<_>>().join(" | ")));
    }
    o.push('\n');
    o
}

/// The report as Markdown. Drawings and SVG blocks are referred to by caption (the PDF
/// carries the pictures); images link to their file.
pub fn to_markdown(r: &Report) -> String {
    let mut o = format!("# {}\n\n", if r.title.is_empty() { "Calculation report" } else { &r.title });
    for (k, v) in [("Project", &r.project), ("Engineer", &r.engineer), ("Checker", &r.checker), ("Date", &r.date)] {
        if !v.is_empty() {
            o.push_str(&format!("- **{k}:** {v}\n"));
        }
    }
    if !r.standards.is_empty() {
        o.push_str(&format!("- **Standards:** {}\n", r.standards.join("; ")));
    }
    o.push('\n');
    for sec in &r.sections {
        o.push_str(&format!("## {}\n\n", sec.heading));
        for b in &sec.blocks {
            match b {
                Block::Paragraph { text } => o.push_str(&format!("{text}\n\n")),
                Block::Note { text } => o.push_str(&format!("> {text}\n\n")),
                Block::Table { headers, rows } => o.push_str(&md_table(headers, rows)),
                Block::Sheet { sheet } => {
                    o.push_str(&sheet.to_markdown());
                    o.push('\n');
                }
                Block::Drawing { drawing, caption } => {
                    let cap = if caption.is_empty() { &drawing.title } else { caption };
                    o.push_str(&format!("*Figure: {cap}* (drawing, see the PDF or the SVG/DXF export)\n\n"));
                }
                Block::Svg { caption, .. } => o.push_str(&format!("*Figure: {caption}*\n\n")),
                Block::Image { path, caption } => o.push_str(&format!("![{caption}]({path})\n\n")),
            }
        }
    }
    o
}
