//! Step 2, Standards: the library (switch on what the project follows), importing a standard
//! (PDF / text / Markdown) into the vector database, office rules as notes (step panel); a
//! search to try and a browser of a standard's clauses (canvas).

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;

use th_kb::{EmbedderConfig, Hit, Library, NewStandard, StandardMeta};
use eframe::egui::{self, RichText};

use crate::chrome::{self, tokens};
use crate::icons::Icon;
use crate::theme::medium_font;

enum ImportMsg {
    Progress(f32, String),
    Done(Result<StandardMeta, String>),
    /// Re-indexing finished: how many standards, or the first error.
    Reindexed(Result<usize, String>),
}

pub struct StandardsView {
    new: NewStandard,
    files: Vec<PathBuf>,
    import_rx: Option<Receiver<ImportMsg>>,
    progress: (f32, String),
    pub status: String,
    query: String,
    hits: Vec<Hit>,
    searched: bool,
    browse: Option<String>,
    browse_chunks: Vec<Hit>,
    note: (String, String, String),
    list: Vec<StandardMeta>,
    stale: bool,
    show_import: bool,
}

impl Default for StandardsView {
    fn default() -> Self {
        StandardsView {
            new: NewStandard { discipline: "structural".into(), ..Default::default() },
            files: vec![],
            import_rx: None,
            progress: (0.0, String::new()),
            status: String::new(),
            query: String::new(),
            hits: vec![],
            searched: false,
            browse: None,
            browse_chunks: vec![],
            note: Default::default(),
            list: vec![],
            stale: true,
            show_import: false,
        }
    }
}

impl StandardsView {
    pub fn refresh(&mut self) {
        self.stale = true;
    }

    /// The step panel. Returns true when the project's chosen standards changed.
    pub fn left(&mut self, ui: &mut egui::Ui, lib: &Arc<Library>, chosen: &mut Vec<String>, embedder: &EmbedderConfig) -> bool {
        let t = tokens(ui);
        let mut changed = false;
        if self.stale {
            self.list = lib.list();
            self.stale = false;
        }
        if let Some(rx) = &self.import_rx {
            for m in rx.try_iter().collect::<Vec<_>>() {
                match m {
                    ImportMsg::Progress(f, s) => self.progress = (f, s),
                    ImportMsg::Done(r) => {
                        self.status = match r {
                            Ok(m) => {
                                if !chosen.contains(&m.id) {
                                    chosen.push(m.id.clone());
                                    changed = true;
                                }
                                self.show_import = false;
                                format!("Imported {} ({} clauses and pieces); it is switched on for this project.", m.code, m.chunks)
                            }
                            Err(e) => format!("Import failed: {e}"),
                        };
                        self.import_rx = None;
                        self.files.clear();
                        self.stale = true;
                    }
                    ImportMsg::Reindexed(r) => {
                        self.status = match r {
                            Ok(n) => format!("Re-indexed {n} standard(s) with the current embedding setting."),
                            Err(e) => format!("Re-indexing failed: {e}"),
                        };
                        self.import_rx = None;
                        self.stale = true;
                    }
                }
            }
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
        }

        chrome::explorer(ui, "Library", &format!("{} standards", self.list.len()), |ui| {
            if self.list.is_empty() {
                ui.add_space(4.0);
                chrome::hint(ui, "The library is empty. Import the standards you design to (your own licensed copies); agents cite only what is here.");
                ui.add_space(4.0);
            }
            let mut delete = None;
            for m in &self.list {
                ui.horizontal(|ui| {
                    ui.set_min_height(44.0);
                    let mut on = chosen.contains(&m.id);
                    if chrome::switch(ui, &mut on).on_hover_text("Follow this standard in the project").changed() {
                        if on {
                            chosen.push(m.id.clone());
                        } else {
                            chosen.retain(|x| x != &m.id);
                        }
                        changed = true;
                    }
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.label(RichText::new(&m.code).font(medium_font(13.0)));
                        ui.label(RichText::new(format!("{} · {} · {} pieces", m.name, m.discipline, m.chunks)).size(11.5).color(t.text_dim));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if chrome::remove_button(ui, "Delete from the library").clicked() {
                            delete = Some(m.id.clone());
                        }
                        if chrome::ghost_button(ui, "Browse").clicked() {
                            self.browse = Some(m.id.clone());
                            self.browse_chunks = lib.chunks(&m.id);
                        }
                    });
                });
            }
            if let Some(id) = delete {
                self.status = lib.delete(&id).map(|_| format!("Deleted {id}")).unwrap_or_else(|e| e);
                chosen.retain(|x| x != &id);
                changed = true;
                self.stale = true;
            }
        });
        // standards indexed with another method than the current setting
        let want = embedder.signature();
        let other: Vec<String> = self.list.iter().filter(|m| m.embedder != want).map(|m| m.id.clone()).collect();
        if !other.is_empty() {
            let how = if embedder.is_openrouter() { "the OpenRouter embedding model" } else { "the simple built-in method" };
            chrome::hint(ui, &format!("{} standard(s) were indexed another way than the current setting ({how}). Re-index them so that search uses it.", other.len()));
            ui.horizontal(|ui| {
                if ui.add_enabled(self.import_rx.is_none(), chrome::primary_button("Re-index")).clicked() {
                    let (tx, rx) = mpsc::channel();
                    self.import_rx = Some(rx);
                    let (lib, emb, ctx) = (lib.clone(), embedder.clone(), ui.ctx().clone());
                    std::thread::spawn(move || {
                        let n = other.len();
                        for (k, id) in other.iter().enumerate() {
                            let tx2 = tx.clone();
                            let c2 = ctx.clone();
                            let mut progress = |f: f32, s: &str| {
                                let _ = tx2.send(ImportMsg::Progress((k as f32 + f) / n as f32, format!("{id}: {s}")));
                                c2.request_repaint();
                            };
                            if let Err(e) = lib.reindex(id, &emb, &mut progress) {
                                let _ = tx.send(ImportMsg::Reindexed(Err(format!("{id}: {e}"))));
                                ctx.request_repaint();
                                return;
                            }
                        }
                        let _ = tx.send(ImportMsg::Reindexed(Ok(n)));
                        ctx.request_repaint();
                    });
                }
            });
        }
        if self.import_rx.is_some() && !self.show_import {
            ui.add(egui::ProgressBar::new(self.progress.0).text(&self.progress.1));
        }
        if !self.show_import && chrome::ghost_button(ui, "+ Import a standard").clicked() {
            self.show_import = true;
        }
        if self.show_import || self.import_rx.is_some() {
            chrome::explorer(ui, "Import a standard", "PDF, text or Markdown", |ui| {
                chrome::field(ui, "Code", &mut self.new.code).on_hover_text("e.g. ACI 318-19, EN 1997-1:2004, AISC 360-22, มยผ. 1301");
                chrome::field(ui, "Name", &mut self.new.name);
                ui.horizontal(|ui| {
                    chrome::row_label(ui, "Discipline");
                    egui::ComboBox::from_id_salt("disc").selected_text(&self.new.discipline).width(180.0).show_ui(ui, |ui| {
                        for d in ["structural", "geotechnical", "steel", "loads", "seismic", "highway", "hydraulic", "other"] {
                            ui.selectable_value(&mut self.new.discipline, d.to_string(), d);
                        }
                    });
                });
                chrome::field(ui, "Country", &mut self.new.country);
                ui.horizontal(|ui| {
                    chrome::row_label(ui, "Year");
                    let mut y = self.new.year.unwrap_or(0) as i32;
                    if ui.add_sized([90.0, 28.0], egui::DragValue::new(&mut y).range(0..=2100)).changed() {
                        self.new.year = if y > 0 { Some(y as u32) } else { None };
                    }
                });
                ui.horizontal(|ui| {
                    chrome::row_label(ui, "Files");
                    if ui.button("Choose…").clicked() {
                        if let Some(f) = rfd::FileDialog::new().add_filter("Standards", &["pdf", "txt", "md"]).pick_files() {
                            self.files = f;
                        }
                    }
                    ui.label(RichText::new(format!("{} chosen", self.files.len())).color(t.text_dim));
                });
                for f in &self.files {
                    ui.label(RichText::new(f.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()).size(11.5).color(t.text_faint));
                }
                let embed_label = match embedder {
                    EmbedderConfig::Hashed { .. } => "the simple built-in method (offline)".to_string(),
                    EmbedderConfig::Api { base_url, model, .. } if base_url.contains("openrouter.ai") => format!("{model} through OpenRouter"),
                    EmbedderConfig::Api { model, .. } => format!("the embedding model {model}"),
                };
                chrome::hint(ui, &format!("Indexed with {embed_label}; change it in Settings."));
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    let can = self.import_rx.is_none() && !self.files.is_empty() && !self.new.code.trim().is_empty();
                    if ui.add_enabled(can, chrome::primary_button("Import")).clicked() {
                        let (tx, rx) = mpsc::channel();
                        self.import_rx = Some(rx);
                        let (lib, files, meta, emb) = (lib.clone(), self.files.clone(), self.new.clone(), embedder.clone());
                        let ctx = ui.ctx().clone();
                        std::thread::spawn(move || {
                            let tx2 = tx.clone();
                            let mut progress = |f: f32, s: &str| {
                                let _ = tx2.send(ImportMsg::Progress(f, s.to_string()));
                                ctx.request_repaint();
                            };
                            let r = lib.ingest(&files, &meta, &emb, &mut progress);
                            let _ = tx.send(ImportMsg::Done(r));
                        });
                    }
                    if self.import_rx.is_none() && ui.button("Cancel").clicked() {
                        self.show_import = false;
                    }
                });
                if self.import_rx.is_some() {
                    ui.add(egui::ProgressBar::new(self.progress.0).text(&self.progress.1));
                }
                ui.add_space(4.0);
            });
        }
        if !self.status.is_empty() {
            ui.add_space(6.0);
            ui.label(RichText::new(&self.status).size(12.0).color(t.text_dim));
        }

        chrome::explorer(ui, "Office rule", "searchable like a clause", |ui| {
            let target = self.browse.clone().or_else(|| self.list.first().map(|m| m.id.clone()));
            ui.horizontal(|ui| {
                chrome::row_label(ui, "Clause");
                ui.add_sized([ui.available_width(), 28.0], egui::TextEdit::singleline(&mut self.note.0).hint_text("e.g. 9.6.3").vertical_align(egui::Align::Center));
            });
            chrome::field(ui, "Title", &mut self.note.1);
            ui.add(egui::TextEdit::multiline(&mut self.note.2).hint_text("The rule, e.g. 'Use cover 40 mm to stirrups for all beams exposed to weather.'").desired_rows(3).desired_width(f32::INFINITY));
            match target {
                Some(id) => {
                    if ui.add_enabled(!self.note.2.trim().is_empty(), egui::Button::new(format!("Add to {id}"))).clicked() {
                        self.status = lib.add_note(&id, &self.note.0, &self.note.1, &self.note.2).map(|_| "Office rule added.".to_string()).unwrap_or_else(|e| e);
                        self.note = Default::default();
                        self.stale = true;
                    }
                }
                None => chrome::hint(ui, "Import a standard first; rules are attached to one."),
            }
            ui.add_space(4.0);
        });
        changed
    }

    /// The canvas: a floating search bar, then the results or the clauses being browsed.
    pub fn center(&mut self, ui: &mut egui::Ui, lib: &Arc<Library>, chosen: &[String]) {
        let t = tokens(ui);
        chrome::float_frame(ui.ctx()).show(ui, |ui| {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(egui::vec2(28.0, 28.0), egui::Sense::hover());
                crate::icons::paint(ui, Icon::ZoomDrag, r.shrink(5.0), t.text_dim);
                let resp = ui.add_sized([ui.available_width() - 100.0, 28.0], egui::TextEdit::singleline(&mut self.query).hint_text("Try a search: 'minimum shear reinforcement' or '22.5.5.1'").frame(false).vertical_align(egui::Align::Center));
                if ui.add(chrome::primary_button("Search")).clicked() || (resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) {
                    match lib.search(&self.query, chosen, 10) {
                        Ok(h) => {
                            self.hits = h;
                            self.searched = true;
                            self.browse = None;
                        }
                        Err(e) => self.status = e,
                    }
                }
            });
        });
        ui.add_space(4.0);
        chrome::hint(ui, if chosen.is_empty() { "Searching the whole library: no standard is switched on for the project." } else { "Searching the standards switched on for the project, as the agents do." });
        ui.add_space(8.0);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.set_max_width(980.0);
            let list = if self.browse.is_some() { &self.browse_chunks } else { &self.hits };
            if let Some(b) = &self.browse {
                ui.label(RichText::new(format!("{b} · {} pieces", list.len())).font(medium_font(15.0)));
                ui.add_space(6.0);
            } else if self.searched && list.is_empty() {
                ui.label(RichText::new("No clause matches.").color(t.text_dim));
            } else if !self.searched {
                ui.add_space(30.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("Search the standards as the agents do").font(medium_font(17.0)));
                    ui.label(RichText::new("Each result is a clause with its number, title and page; the agents cite these numbers in the calculations.").color(t.text_dim));
                });
            }
            for h in list.iter().take(400) {
                egui::Frame::NONE.fill(t.card).stroke(egui::Stroke::new(1.0, t.separator)).corner_radius(10).inner_margin(12).show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.horizontal(|ui| {
                        chrome::chip(ui, &h.standard_code, t.accent_text);
                        ui.label(RichText::new(format!("{}  {}", h.clause, h.title)).font(medium_font(13.0)));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if self.browse.is_none() {
                                ui.label(RichText::new(format!("score {:.3}", h.score)).size(11.0).color(t.text_faint));
                            }
                            if let Some(p) = h.page {
                                ui.label(RichText::new(format!("p. {p}")).size(11.5).color(t.text_dim));
                            }
                        });
                    });
                    let text: String = h.text.chars().take(1400).collect();
                    ui.label(RichText::new(text).size(12.5).color(t.text_dim));
                });
                ui.add_space(8.0);
            }
        });
    }
}
