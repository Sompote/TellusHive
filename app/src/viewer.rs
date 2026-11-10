//! The artifact viewer beside the chat: a calculation sheet as tables with PASS/FAIL, a
//! drawing (SVG) with its DXF, a report (Markdown preview + open the PDF), a picture, or a
//! text file.

use std::path::{Path, PathBuf};

use th_eng::calc::Sheet;
use eframe::egui::{self, Color32, RichText};

use crate::chrome::{self, tokens};
use crate::icons::{self, Icon};
use crate::markdown;
use crate::theme::medium_font;

#[derive(Default)]
pub struct Viewer {
    pub path: Option<PathBuf>,
    /// Set when the engineer asks to open a DXF in CADCraft (the app does it).
    pub open_in_cad: Option<PathBuf>,
    pub cad_status: String,
    cache: Option<(PathBuf, std::time::SystemTime, Content)>,
    zoom: f32,
}

enum Content {
    Sheet(Box<Sheet>),
    Text(String),
    Image,
    Other,
}

fn ext(p: &Path) -> String {
    p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

fn sibling(p: &Path, e: &str) -> PathBuf {
    p.with_extension(e)
}

impl Viewer {
    pub fn show(&mut self, p: PathBuf) {
        self.path = Some(p);
        self.zoom = 1.0;
    }

    fn load(p: &Path) -> Content {
        match ext(p).as_str() {
            "json" if p.parent().is_some_and(|d| d.ends_with("calcs")) => match std::fs::read_to_string(p).ok().and_then(|s| serde_json::from_str::<Sheet>(&s).ok()) {
                Some(s) => Content::Sheet(Box::new(s)),
                None => Content::Text(std::fs::read_to_string(p).unwrap_or_default()),
            },
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" => Content::Image,
            "pdf" => {
                let md = sibling(p, "md");
                match std::fs::read_to_string(&md) {
                    Ok(s) => Content::Text(s),
                    Err(_) => Content::Other,
                }
            }
            "md" | "txt" | "csv" | "json" | "py" | "rs" | "dxf" | "log" | "tex" | "html" => {
                let s = std::fs::read_to_string(p).unwrap_or_default();
                Content::Text(s.chars().take(200_000).collect())
            }
            _ => Content::Other,
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        let t = tokens(ui);
        let Some(p) = self.path.clone() else {
            ui.add_space(60.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("Nothing to show yet").font(medium_font(17.0)));
                ui.label(RichText::new("Calculations, drawings and reports the agents make appear here.").color(t.text_dim));
            });
            return;
        };
        let mtime = p.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
        let fresh = matches!(&self.cache, Some((cp, ct, _)) if *cp == p && *ct == mtime);
        if !fresh {
            // a regenerated picture must not come from egui's image cache
            ui.ctx().forget_image(&format!("file://{}", p.display()));
            self.cache = Some((p.clone(), mtime, Self::load(&p)));
        }
        let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let e = ext(&p);
        let is_image = matches!(self.cache, Some((_, _, Content::Image)));
        chrome::float_frame(ui.ctx()).show(ui, |ui| {
            ui.horizontal(|ui| {
                let icon = match e.as_str() {
                    "json" => Icon::Calculator,
                    "svg" | "dxf" => Icon::Drafting,
                    "pdf" | "md" => Icon::FileText,
                    "png" | "jpg" | "jpeg" => Icon::Picture,
                    _ => Icon::FileText,
                };
                let (r, _) = ui.allocate_exact_size(egui::vec2(24.0, 28.0), egui::Sense::hover());
                icons::paint(ui, icon, r.shrink2(egui::vec2(4.0, 6.0)), t.accent_text);
                ui.label(RichText::new(&name).font(medium_font(13.5)));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if icons::button(ui, Icon::Close, "Close", false).clicked() {
                        self.path = None;
                    }
                    if icons::button(ui, Icon::Folder, "Show in folder", false).clicked() {
                        if let Some(d) = p.parent() {
                            chrome::open_path(d);
                        }
                    }
                    chrome::strip_divider(ui);
                    if e == "json" && sibling(&p, "md").exists() && ui.add(egui::Button::new("Markdown").frame(false)).clicked() {
                        chrome::open_path(&sibling(&p, "md"));
                    }
                    if (e == "md" || e == "pdf") && sibling(&p, "pdf").exists() && ui.add(chrome::primary_button("Open PDF")).clicked() {
                        chrome::open_path(&sibling(&p, "pdf"));
                    }
                    if (e == "svg" && sibling(&p, "dxf").exists()) || e == "dxf" {
                        if ui.add(egui::Button::new("DXF").frame(false)).on_hover_text("Open the DXF with the default CAD program").clicked() {
                            chrome::open_path(&sibling(&p, "dxf"));
                        }
                        if ui.add(chrome::primary_button("Open in CADCraft")).on_hover_text("Open the DXF in the CADCraft window").clicked() {
                            self.open_in_cad = Some(sibling(&p, "dxf"));
                        }
                    }
                    if is_image {
                        chrome::strip_divider(ui);
                        if icons::button(ui, Icon::ZoomIn, "Zoom in", false).clicked() {
                            self.zoom = (self.zoom * 1.25).min(4.0);
                        }
                        ui.label(RichText::new(format!("{:.0} %", self.zoom * 100.0)).size(12.0).color(t.text_dim));
                        if icons::button(ui, Icon::Fit, "Fit", false).clicked() {
                            self.zoom = 1.0;
                        }
                    }
                });
            });
        });
        if !self.cad_status.is_empty() {
            ui.label(RichText::new(&self.cad_status).size(11.5).color(t.text_dim));
        }
        ui.add_space(8.0);
        let Some((_, _, content)) = &self.cache else { return };
        match content {
            Content::Sheet(s) => {
                egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
                    egui::Frame::NONE.fill(t.card).stroke(egui::Stroke::new(1.0_f32, t.separator)).corner_radius(10).inner_margin(16).show(ui, |ui| {
                        ui.set_min_width(ui.available_width() - 34.0);
                        sheet_ui(ui, s);
                    });
                });
            }
            Content::Image => {
                let shown = chrome::display_path(&p);
                egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
                    let w = (ui.available_width() - 8.0) * self.zoom;
                    ui.add(egui::Image::new(format!("file://{}", shown.display())).fit_to_exact_size(egui::vec2(w, w * 4.0)).maintain_aspect_ratio(true).bg_fill(Color32::WHITE).corner_radius(4));
                });
            }
            Content::Text(s) => {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    if matches!(ext(&p).as_str(), "md" | "pdf") {
                        if ext(&p) == "pdf" {
                            if ui.add(chrome::primary_button("Open the PDF")).clicked() {
                                chrome::open_path(&p);
                            }
                            ui.add_space(6.0);
                        }
                        markdown::show(ui, s, &t);
                    } else {
                        ui.label(RichText::new(s).monospace().size(11.5));
                    }
                });
            }
            Content::Other => {
                chrome::hint(ui, "No preview for this file type.");
                if ui.add(chrome::primary_button("Open")).clicked() {
                    chrome::open_path(&p);
                }
            }
        }
    }
}

fn fmt(v: f64) -> String {
    if v == 0.0 {
        return "0".into();
    }
    let a = v.abs();
    if !(1e-3..1e6).contains(&a) {
        format!("{v:.3e}")
    } else if a >= 100.0 {
        format!("{v:.1}")
    } else if a >= 1.0 {
        format!("{v:.3}")
    } else {
        format!("{v:.4}")
    }
}

pub fn sheet_ui(ui: &mut egui::Ui, s: &Sheet) {
    let t = tokens(ui);
    ui.label(RichText::new(&s.title).font(medium_font(15.0)));
    ui.horizontal(|ui| {
        if !s.standard.is_empty() {
            chrome::chip(ui, &s.standard, t.accent_text);
        }
        if s.all_pass {
            chrome::chip(ui, "ALL CHECKS PASS", t.ok);
        } else {
            chrome::chip(ui, "CHECK FAILS", t.error);
        }
    });
    ui.add_space(6.0);
    chrome::section(ui, "Inputs");
    egui::Grid::new("inputs").striped(true).num_columns(4).spacing([14.0, 4.0]).show(ui, |ui| {
        for v in &s.inputs {
            ui.label(RichText::new(&v.symbol).monospace());
            ui.label(fmt(v.value));
            ui.label(RichText::new(&v.unit).color(t.text_dim));
            ui.label(RichText::new(&v.description).color(t.text_dim));
            ui.end_row();
        }
    });
    chrome::section(ui, "Calculation");
    for st in &s.steps {
        egui::Frame::NONE.fill(t.card).stroke(egui::Stroke::new(1.0, t.separator)).corner_radius(6).inner_margin(8).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(&st.description).color(t.text));
                if !st.reference.is_empty() {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(&st.reference).size(11.0).color(t.accent_text));
                    });
                }
            });
            ui.label(RichText::new(format!("{} = {}", st.symbol, st.expr)).monospace().size(11.5).color(t.text_dim));
            if st.substituted != st.expr {
                ui.label(RichText::new(format!("  = {}", st.substituted)).monospace().size(11.5).color(t.text_faint));
            }
            ui.label(RichText::new(format!("{} = {} {}", st.symbol, fmt(st.value), st.unit)).monospace().strong());
            if let Some(c) = &st.check {
                let col = if c.pass { t.ok } else { t.error };
                ui.label(RichText::new(format!("{}: {} ≤ {}  (ratio {:.3}) {}", c.label, fmt(c.demand), fmt(c.capacity), c.ratio, if c.pass { "PASS" } else { "FAIL" })).color(col));
            }
        });
        ui.add_space(4.0);
    }
    if !s.notes.is_empty() {
        chrome::section(ui, "Notes");
        for n in &s.notes {
            ui.label(RichText::new(format!("• {n}")).color(t.text_dim));
        }
    }
}
