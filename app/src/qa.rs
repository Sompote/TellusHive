//! A test robot for the window (TELLUSHIVE_QA=<script>, TELLUSHIVE_QA_OUT=<folder>): it plays a
//! script of real input events (clicks, typing, keys, scrolling) into the window and saves a
//! screenshot and the status line where the script asks, so the user interface can be tested
//! like a person uses it, with no operating-system permission. One command per line:
//!
//! ```text
//! click X Y        left click at a point (window coordinates, points)
//! rclick X Y       right click
//! move X Y         pointer to a point
//! scroll X Y DY    wheel at a point (DY > 0 scrolls the content down)
//! type TEXT        text typed into the focused field
//! key NAME         a key, with modifiers: key Enter, key cmd+A, key Escape
//! wait MS          pause
//! shot NAME        screenshot to <out>/NAME.png
//! note TEXT        a line in the log
//! quit             close the window
//! ```
//! The app adds commands of its own (ws, waitidle, status); see `App::qa_command`.

use std::collections::VecDeque;
use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui::{self, Event, Key, Modifiers, PointerButton, Pos2};

pub struct Qa {
    pub steps: VecDeque<String>,
    /// Input events still to play, one frame's worth per entry.
    frames: VecDeque<Vec<Event>>,
    pub wait_until: Instant,
    pub out: PathBuf,
    /// The screenshot asked for and not yet received.
    pub shot: Option<String>,
    log: std::fs::File,
    pointer: Pos2,
    pub n: usize,
}

impl Qa {
    pub fn from_env() -> Option<Qa> {
        let script = std::env::var("TELLUSHIVE_QA").ok()?;
        let out = PathBuf::from(std::env::var("TELLUSHIVE_QA_OUT").unwrap_or_else(|_| "qa_out".into()));
        let _ = std::fs::create_dir_all(&out);
        let text = std::fs::read_to_string(&script).ok()?;
        let steps = text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).map(String::from).collect();
        let log = std::fs::File::create(out.join("log.txt")).ok()?;
        Some(Qa { steps, frames: VecDeque::new(), wait_until: Instant::now() + Duration::from_millis(1500), out, shot: None, log, pointer: Pos2::new(10.0, 10.0), n: 0 })
    }

    pub fn log(&mut self, line: &str) {
        let _ = writeln!(self.log, "{line}");
        let _ = self.log.flush();
    }

    /// Plays one frame of queued input.
    pub fn input(&mut self, raw: &mut egui::RawInput) {
        if let Some(evs) = self.frames.pop_front() {
            raw.events.extend(evs);
        }
        raw.focused = true;
    }

    pub fn busy(&self) -> bool {
        !self.frames.is_empty() || self.shot.is_some() || Instant::now() < self.wait_until
    }

    fn pause(&mut self, ms: u64) {
        self.wait_until = Instant::now() + Duration::from_millis(ms);
    }

    /// Runs a robot command. Returns false when the command is not one of the robot's own.
    pub fn command(&mut self, cmd: &str, rest: &str, ctx: &egui::Context) -> bool {
        let nums: Vec<f32> = rest.split_whitespace().filter_map(|x| x.parse().ok()).collect();
        let at = |i: usize| Pos2::new(*nums.get(i).unwrap_or(&0.0), *nums.get(i + 1).unwrap_or(&0.0));
        match cmd {
            "click" | "rclick" => {
                let p = at(0);
                let button = if cmd == "rclick" { PointerButton::Secondary } else { PointerButton::Primary };
                self.pointer = p;
                self.frames.push_back(vec![Event::PointerMoved(p)]);
                self.frames.push_back(vec![Event::PointerButton { pos: p, button, pressed: true, modifiers: Modifiers::NONE }]);
                self.frames.push_back(vec![Event::PointerButton { pos: p, button, pressed: false, modifiers: Modifiers::NONE }]);
                self.pause(350);
            }
            "move" => {
                self.pointer = at(0);
                self.frames.push_back(vec![Event::PointerMoved(self.pointer)]);
                self.pause(250);
            }
            "scroll" => {
                let p = at(0);
                let dy = -nums.get(2).copied().unwrap_or(0.0);
                self.frames.push_back(vec![Event::PointerMoved(p)]);
                self.frames.push_back(vec![Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: egui::vec2(0.0, dy), modifiers: Modifiers::NONE }]);
                self.pause(300);
            }
            "type" => {
                self.frames.push_back(vec![Event::Text(rest.to_string())]);
                self.pause(250);
            }
            "key" => {
                let mut mods = Modifiers::NONE;
                let mut name = rest.trim();
                while let Some((m, k)) = name.split_once('+') {
                    match m.to_ascii_lowercase().as_str() {
                        "cmd" | "command" => {
                            mods.command = true;
                            mods.mac_cmd = cfg!(target_os = "macos");
                            mods.ctrl = !cfg!(target_os = "macos");
                        }
                        "shift" => mods.shift = true,
                        "alt" => mods.alt = true,
                        "ctrl" => mods.ctrl = true,
                        _ => {}
                    }
                    name = k;
                }
                match Key::from_name(name) {
                    Some(key) => {
                        self.frames.push_back(vec![Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: mods }]);
                        self.frames.push_back(vec![Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers: mods }]);
                    }
                    None => self.log(&format!("  ! unknown key {name}")),
                }
                self.pause(250);
            }
            "wait" => self.pause(nums.first().copied().unwrap_or(500.0) as u64),
            "shot" => {
                self.shot = Some(rest.trim().to_string());
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            }
            "note" => self.log(&format!("# {rest}")),
            "quit" => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            _ => return false,
        }
        true
    }

    /// Saves a screenshot that arrived for `shot`.
    pub fn take_screenshot(&mut self, ctx: &egui::Context) {
        let Some(name) = self.shot.clone() else { return };
        let shots: Vec<std::sync::Arc<egui::ColorImage>> = ctx.input(|i| i.events.iter().filter_map(|e| if let Event::Screenshot { image, .. } = e { Some(image.clone()) } else { None }).collect());
        if let Some(img) = shots.into_iter().next() {
            let buf: Vec<u8> = img.pixels.iter().flat_map(|c| [c.r(), c.g(), c.b(), c.a()]).collect();
            if let Some(rgba) = image::RgbaImage::from_raw(img.size[0] as u32, img.size[1] as u32, buf) {
                let _ = rgba.save(self.out.join(format!("{name}.png")));
            }
            self.shot = None;
            self.pause(100);
        }
    }
}
