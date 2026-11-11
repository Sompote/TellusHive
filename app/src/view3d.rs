//! A 3D view of a BIM model in the output panel: the triangle mesh Aedifex exports
//! (`models/aedifex.axmesh`), drawn with flat shading and an orbit camera. Drag to rotate,
//! right-drag or Shift-drag to pan, scroll or pinch to zoom, double-click to fit; a click
//! picks the element under the pointer.
//!
//! The model is rasterised on the CPU into an image with a depth buffer (egui's painter has
//! none), so hidden faces are always right, then shown as a texture. An element-id buffer
//! from the same pass gives exact picking and the thin outlines where elements meet. The
//! image is only redrawn when the camera, the size, the selection or the model change.

use std::path::Path;

use eframe::egui::{self, Color32, Pos2, Sense, Vec2};

/// One triangle of the mesh file.
#[derive(Clone, Copy)]
pub struct Tri {
    pub p: [[f32; 3]; 3],
    pub n: [f32; 3],
    pub rgba: [u8; 4],
    pub id: u32,
}

pub struct MeshModel {
    pub tris: Vec<Tri>,
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub elements: usize,
}

/// Reads an `AXMESH1` file: magic, u32 count, then per triangle 9 f32 corners, 3 f32 normal,
/// 4 u8 RGBA, u32 element id (little-endian, z up).
pub fn load(path: &Path) -> Result<MeshModel, String> {
    let b = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse(&b)
}

pub fn parse(b: &[u8]) -> Result<MeshModel, String> {
    if b.len() < 12 || &b[..8] != b"AXMESH1\0" {
        return Err("not an Aedifex mesh file".into());
    }
    let n = u32::from_le_bytes([b[8], b[9], b[10], b[11]]) as usize;
    const REC: usize = 12 * 4 + 4 + 4;
    if b.len() < 12 + n * REC {
        return Err(format!("the mesh file is cut short ({} of {} triangles)", (b.len() - 12) / REC, n));
    }
    let f = |o: usize| f32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
    let mut tris = Vec::with_capacity(n);
    let (mut min, mut max) = ([f32::MAX; 3], [f32::MIN; 3]);
    let mut ids = std::collections::HashSet::new();
    for k in 0..n {
        let o = 12 + k * REC;
        let v = |i: usize| [f(o + i * 12), f(o + i * 12 + 4), f(o + i * 12 + 8)];
        let t = Tri { p: [v(0), v(1), v(2)], n: v(3), rgba: [b[o + 48], b[o + 49], b[o + 50], b[o + 51]], id: u32::from_le_bytes([b[o + 52], b[o + 53], b[o + 54], b[o + 55]]) };
        for p in t.p {
            for a in 0..3 {
                min[a] = min[a].min(p[a]);
                max[a] = max[a].max(p[a]);
            }
        }
        ids.insert(t.id);
        tris.push(t);
    }
    if tris.is_empty() {
        min = [0.0; 3];
        max = [0.0; 3];
    }
    Ok(MeshModel { elements: ids.len(), tris, min, max })
}

/// The orbit camera; kept across reloads so a live model does not jump while it updates.
#[derive(Clone, Copy, Debug)]
pub struct Camera {
    /// Degrees from +x, counter-clockwise seen from above.
    pub yaw: f32,
    /// Degrees above the horizon.
    pub pitch: f32,
    /// Distance from the target, m.
    pub dist: f32,
    pub target: [f32; 3],
    /// False until the camera has been fitted to a model.
    pub fitted: bool,
}

impl Default for Camera {
    fn default() -> Self {
        Camera { yaw: -60.0, pitch: 28.0, dist: 30.0, target: [0.0; 3], fitted: false }
    }
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

impl Camera {
    pub fn fit(&mut self, m: &MeshModel) {
        self.target = [(m.min[0] + m.max[0]) / 2.0, (m.min[1] + m.max[1]) / 2.0, (m.min[2] + m.max[2]) / 2.0];
        let d = sub(m.max, m.min);
        let r = (dot(d, d).sqrt() / 2.0).max(0.5);
        self.dist = r * 2.6;
        self.yaw = -60.0;
        self.pitch = 28.0;
        self.fitted = true;
    }

    /// Camera axes: right, up, forward (towards the target).
    fn axes(&self) -> ([f32; 3], [f32; 3], [f32; 3]) {
        let (y, p) = (self.yaw.to_radians(), self.pitch.to_radians());
        // the eye sits at target - forward * dist
        let fwd = [-(p.cos() * y.cos()), -(p.cos() * y.sin()), -p.sin()];
        let right = [-y.sin(), y.cos(), 0.0];
        let up = [right[1] * fwd[2] - right[2] * fwd[1], right[2] * fwd[0] - right[0] * fwd[2], right[0] * fwd[1] - right[1] * fwd[0]];
        (right, up, fwd)
    }

    fn eye(&self) -> [f32; 3] {
        let (_, _, f) = self.axes();
        [self.target[0] - f[0] * self.dist, self.target[1] - f[1] * self.dist, self.target[2] - f[2] * self.dist]
    }
}

/// The last rendered image and its element-id buffer (picking), redrawn only when needed.
#[derive(Default)]
pub struct Drawn {
    texture: Option<egui::TextureHandle>,
    key: String,
    ids: Vec<u32>,
    w: usize,
    h: usize,
}

const NONE: u32 = u32::MAX;

/// Rasterises the model: flat-shaded triangles with a depth buffer, outlines where the
/// element changes or the depth jumps. Returns the colour image and the element ids.
fn render(m: &MeshModel, cam: &Camera, w: usize, h: usize, picked: Option<u32>, bg: Color32) -> (egui::ColorImage, Vec<u32>) {
    let (right, up, fwd) = cam.axes();
    let eye = cam.eye();
    let focal = h as f32 / 2.0 / (22.5f32).to_radians().tan();
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let near = cam.dist * 0.005;
    let light = {
        let l = [0.35 * right[0] + 0.6 * up[0] - 0.7 * fwd[0], 0.35 * right[1] + 0.6 * up[1] - 0.7 * fwd[1], 0.35 * right[2] + 0.6 * up[2] - 0.7 * fwd[2]];
        let n = dot(l, l).sqrt();
        [l[0] / n, l[1] / n, l[2] / n]
    };
    let mut col = vec![bg; w * h];
    let mut depth = vec![0.0f32; w * h]; // 1/z, larger is nearer
    let mut ids = vec![NONE; w * h];
    for t in &m.tris {
        if dot(t.n, t.n) > 0.5 && dot(t.n, sub(t.p[0], eye)) > 0.0 {
            continue; // back face of a closed solid
        }
        let mut v = [(0.0f32, 0.0f32, 0.0f32); 3];
        let mut behind = false;
        for (i, p) in t.p.iter().enumerate() {
            let d = sub(*p, eye);
            let z = dot(d, fwd);
            if z < near {
                behind = true;
                break;
            }
            v[i] = (cx + dot(d, right) * focal / z, cy - dot(d, up) * focal / z, 1.0 / z);
        }
        if behind {
            continue;
        }
        let area = (v[1].0 - v[0].0) * (v[2].1 - v[0].1) - (v[1].1 - v[0].1) * (v[2].0 - v[0].0);
        if area.abs() < 1e-6 {
            continue;
        }
        let lit = 0.45 + 0.55 * dot(t.n, light).abs();
        let c = if picked == Some(t.id) {
            Color32::from_rgb(55, 142, 240)
        } else {
            let [r, g, b, a] = t.rgba;
            let s = |x: u8| ((x as f32) * lit).min(255.0) as u8;
            // see-through glass blends with what is behind it later; drawn opaque but lighter
            if a < 250 {
                Color32::from_rgb(s(r).saturating_add(25), s(g).saturating_add(30), s(b).saturating_add(40))
            } else {
                Color32::from_rgb(s(r), s(g), s(b))
            }
        };
        let x0 = v.iter().map(|p| p.0).fold(f32::MAX, f32::min).floor().max(0.0) as usize;
        let x1 = (v.iter().map(|p| p.0).fold(f32::MIN, f32::max).ceil() as isize).min(w as isize - 1);
        let y0 = v.iter().map(|p| p.1).fold(f32::MAX, f32::min).floor().max(0.0) as usize;
        let y1 = (v.iter().map(|p| p.1).fold(f32::MIN, f32::max).ceil() as isize).min(h as isize - 1);
        if x1 < 0 || y1 < 0 {
            continue;
        }
        let inv = 1.0 / area;
        for y in y0..=y1 as usize {
            let py = y as f32 + 0.5;
            for x in x0..=x1 as usize {
                let px = x as f32 + 0.5;
                let w0 = ((v[2].0 - v[1].0) * (py - v[1].1) - (v[2].1 - v[1].1) * (px - v[1].0)) * inv;
                let w1 = ((v[0].0 - v[2].0) * (py - v[2].1) - (v[0].1 - v[2].1) * (px - v[2].0)) * inv;
                let w2 = 1.0 - w0 - w1;
                if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                    continue;
                }
                let iz = w0 * v[0].2 + w1 * v[1].2 + w2 * v[2].2;
                let k = y * w + x;
                if iz > depth[k] {
                    depth[k] = iz;
                    col[k] = c;
                    ids[k] = t.id;
                }
            }
        }
    }
    // outlines: where the element under the pixel changes, or the depth jumps
    let edge = Color32::from_rgb(40, 44, 50);
    let mut out = col.clone();
    for y in 0..h.saturating_sub(1) {
        for x in 0..w.saturating_sub(1) {
            let k = y * w + x;
            if ids[k] == NONE && ids[k + 1] == NONE && ids[k + w] == NONE {
                continue;
            }
            let jump = |a: f32, b: f32| a > 0.0 && b > 0.0 && (1.0 / a - 1.0 / b).abs() > 0.02 * (1.0 / a).max(1.0 / b);
            if ids[k] != ids[k + 1] || ids[k] != ids[k + w] || jump(depth[k], depth[k + 1]) || jump(depth[k], depth[k + w]) {
                out[k] = mix(out[k], edge, 0.55);
            }
        }
    }
    (egui::ColorImage { size: [w, h], pixels: out }, ids)
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (x as f32 * (1.0 - t) + y as f32 * t) as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

/// Draws the model in the space left and handles the pointer. Returns the element picked
/// this frame (`u32::MAX` for a click on empty space).
pub fn ui(ui: &mut egui::Ui, m: &MeshModel, cam: &mut Camera, drawn: &mut Drawn, picked: Option<u32>, background: Color32) -> Option<u32> {
    if !cam.fitted {
        cam.fit(m);
    }
    let size = ui.available_size().max(Vec2::new(200.0, 200.0));
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());

    // camera controls
    let shift = ui.input(|i| i.modifiers.shift);
    let d = resp.drag_delta();
    if resp.dragged_by(egui::PointerButton::Primary) && !shift {
        cam.yaw -= d.x * 0.35;
        cam.pitch = (cam.pitch + d.y * 0.35).clamp(-89.0, 89.0);
    } else if resp.dragged_by(egui::PointerButton::Secondary) || resp.dragged_by(egui::PointerButton::Middle) || (resp.dragged_by(egui::PointerButton::Primary) && shift) {
        let (r, u, _) = cam.axes();
        let k = cam.dist / rect.height().max(1.0) * 1.1;
        for a in 0..3 {
            cam.target[a] += -r[a] * d.x * k + u[a] * d.y * k;
        }
    }
    if resp.hovered() {
        let (scroll, zoom) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
        let f = (-scroll * 0.0025).exp() / zoom;
        if (f - 1.0).abs() > 1e-4 {
            cam.dist = (cam.dist * f).clamp(0.2, 50_000.0);
        }
    }
    if resp.double_clicked() {
        cam.fit(m);
    }

    // render at the screen's pixel density, at most about 4 megapixels
    let ppp = ui.ctx().pixels_per_point();
    let scale = ppp.min((4.0e6 / (rect.width() * rect.height()).max(1.0)).sqrt());
    let (w, h) = (((rect.width() * scale) as usize).max(1), ((rect.height() * scale) as usize).max(1));
    let key = format!("{:p}|{}|{:?}|{w}x{h}|{:?}|{:?}", m.tris.as_ptr(), m.tris.len(), (cam.yaw, cam.pitch, cam.dist, cam.target), picked, background);
    if drawn.key != key || drawn.texture.is_none() {
        let (img, ids) = render(m, cam, w, h, picked, background);
        match &mut drawn.texture {
            Some(t) => t.set(img, egui::TextureOptions::LINEAR),
            None => drawn.texture = Some(ui.ctx().load_texture("bim-3d-view", img, egui::TextureOptions::LINEAR)),
        }
        (drawn.ids, drawn.w, drawn.h, drawn.key) = (ids, w, h, key);
    }
    let painter = ui.painter_at(rect);
    if let Some(t) = &drawn.texture {
        painter.image(t.id(), rect, egui::Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
    }
    let (right, up, _) = cam.axes();

    // axis tripod
    let o = rect.left_bottom() + Vec2::new(34.0, -34.0);
    for (axis, colour, label) in [([1.0, 0.0, 0.0], Color32::from_rgb(220, 70, 60), "x"), ([0.0, 1.0, 0.0], Color32::from_rgb(70, 170, 80), "y"), ([0.0, 0.0, 1.0], Color32::from_rgb(60, 110, 220), "z")] {
        let e = o + Vec2::new(dot(axis, right), -dot(axis, up)) * 22.0;
        painter.line_segment([o, e], egui::Stroke::new(2.0, colour));
        painter.text(e + (e - o).normalized() * 7.0, egui::Align2::CENTER_CENTER, label, egui::FontId::proportional(11.0), colour);
    }

    if resp.clicked() {
        let p = resp.interact_pointer_pos()? - rect.min;
        let (x, y) = ((p.x / rect.width() * drawn.w as f32) as usize, (p.y / rect.height() * drawn.h as f32) as usize);
        return Some(drawn.ids.get(y.min(drawn.h.saturating_sub(1)) * drawn.w + x.min(drawn.w.saturating_sub(1))).copied().unwrap_or(NONE));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(tris: &[([[f32; 3]; 3], u32)]) -> Vec<u8> {
        let mut b = b"AXMESH1\0".to_vec();
        b.extend_from_slice(&(tris.len() as u32).to_le_bytes());
        for (p, id) in tris {
            for v in p.iter().chain(std::iter::once(&[0.0, 0.0, 1.0])) {
                for c in v {
                    b.extend_from_slice(&c.to_le_bytes());
                }
            }
            b.extend_from_slice(&[200, 100, 50, 255]);
            b.extend_from_slice(&id.to_le_bytes());
        }
        b
    }

    #[test]
    fn reads_the_aedifex_mesh_format() {
        let b = file(&[([[0.0, 0.0, 0.0], [4.0, 0.0, 0.0], [0.0, 3.0, 2.5]], 7), ([[1.0, 1.0, 0.0], [2.0, 1.0, 0.0], [1.0, 2.0, 0.0]], 9)]);
        let m = parse(&b).unwrap();
        assert_eq!(m.tris.len(), 2);
        assert_eq!((m.min, m.max), ([0.0, 0.0, 0.0], [4.0, 3.0, 2.5]));
        assert_eq!(m.elements, 2);
        assert_eq!(m.tris[1].id, 9);
        assert!(parse(&b[..b.len() - 4]).is_err());
        assert!(parse(b"nope").is_err());
        let mut cam = Camera::default();
        cam.fit(&m);
        assert_eq!(cam.target, [2.0, 1.5, 1.25]);
    }

    #[test]
    fn the_depth_buffer_hides_what_is_behind() {
        // a red square at z = 0 and a blue one above it at z = 2, seen from above
        let quad = |z: f32, rgba: [u8; 4], id: u32| {
            let n = [0.0, 0.0, 1.0];
            [Tri { p: [[-1.0, -1.0, z], [1.0, -1.0, z], [1.0, 1.0, z]], n, rgba, id }, Tri { p: [[-1.0, -1.0, z], [1.0, 1.0, z], [-1.0, 1.0, z]], n, rgba, id }]
        };
        let mut tris = quad(2.0, [0, 0, 255, 255], 2).to_vec();
        tris.extend(quad(0.0, [255, 0, 0, 255], 1)); // drawn last, but further away
        let m = MeshModel { tris, min: [-1.0, -1.0, 0.0], max: [1.0, 1.0, 2.0], elements: 2 };
        let cam = Camera { yaw: 0.0, pitch: 89.0, dist: 10.0, target: [0.0, 0.0, 1.0], fitted: true };
        let (img, ids) = render(&m, &cam, 64, 64, None, Color32::WHITE);
        let k = 32 * 64 + 32;
        assert_eq!(ids[k], 2, "the upper square is in front");
        assert!(img.pixels[k].b() > img.pixels[k].r());
        assert_eq!(ids[0], NONE, "the corner is background");
    }

    #[test]
    fn camera_axes_are_orthonormal_and_look_at_the_target() {
        let cam = Camera { yaw: -60.0, pitch: 28.0, dist: 10.0, target: [1.0, 2.0, 3.0], fitted: true };
        let (r, u, f) = cam.axes();
        for (a, b) in [(r, u), (u, f), (r, f)] {
            assert!(dot(a, b).abs() < 1e-5);
        }
        assert!((dot(u, u) - 1.0).abs() < 1e-5 && u[2] > 0.0, "up points up");
        let e = cam.eye();
        let to = sub(cam.target, e);
        assert!((dot(to, f) - 10.0).abs() < 1e-4);
    }
}
