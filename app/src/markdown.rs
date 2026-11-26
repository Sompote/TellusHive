//! A small Markdown renderer for the chat replies: headings, lines with **bold**, *italic*,
//! `code` and [links](reports/beam.pdf), bullet and numbered lists, tables, code blocks, quotes
//! and rules. Lines stay lines (a reply's single line breaks are kept), as the plain text
//! showed them before. A link is drawn underlined in the accent colour; clicking it hands its
//! target back to the caller, which opens the file or the web page.

use eframe::egui::{self, text::LayoutJob, Color32, FontFamily, FontId, TextFormat};

use crate::latex;
use crate::theme::Tokens;

/// Shows `text` as Markdown in the theme's colours. Returns the target of a link clicked
/// this frame.
pub fn show(ui: &mut egui::Ui, text: &str, t: &Tokens) -> Option<String> {
    let mut clicked = None;
    let size = ui.style().text_styles.get(&egui::TextStyle::Body).map_or(13.0, |f| f.size);
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;
    let mut first = true;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();
        // code block
        if trimmed.starts_with("```") {
            let mut code = vec![];
            i += 1;
            while i < lines.len() && !lines[i].trim_start().starts_with("```") {
                code.push(lines[i]);
                i += 1;
            }
            i += 1;
            egui::Frame::new().fill(t.field).stroke(egui::Stroke::new(1.0, t.field_border)).corner_radius(6.0).inner_margin(egui::Margin::same(6)).show(ui, |ui| {
                ui.add(egui::Label::new(egui::RichText::new(code.join("\n")).monospace().size(size - 1.0).color(t.text)).wrap());
            });
            first = false;
            continue;
        }
        // display math: $$…$$ or \[…\] on one line, or opened on this line and closed later
        if let Some(body) = latex::display_line(trimmed) {
            i += 1;
            math_block(ui, body, size, t);
            first = false;
            continue;
        }
        if let Some(close) = latex::display_open(trimmed) {
            let mut body = vec![trimmed.trim_start_matches("$$").trim_start_matches("\\[").to_string()];
            let mut j = i + 1;
            let mut closed = false;
            while j < lines.len() {
                let l = lines[j].trim();
                j += 1;
                if let Some(head) = l.strip_suffix(close) {
                    body.push(head.to_string());
                    closed = true;
                    break;
                }
                body.push(l.to_string());
            }
            // an opener that is never closed is text
            if closed {
                i = j;
                math_block(ui, &body.join("\n"), size, t);
                first = false;
                continue;
            }
        }
        // table: consecutive lines that start with '|'
        if trimmed.starts_with('|') && trimmed.matches('|').count() >= 2 {
            let mut rows = vec![];
            while i < lines.len() && lines[i].trim().starts_with('|') {
                rows.push(lines[i].trim());
                i += 1;
            }
            table(ui, &rows, size, t, i);
            first = false;
            continue;
        }
        i += 1;
        if trimmed.is_empty() {
            if !first {
                ui.add_space(size * 0.4);
            }
            continue;
        }
        first = false;
        if matches!(trimmed, "---" | "***" | "___") {
            ui.separator();
            continue;
        }
        let hashes = trimmed.chars().take_while(|&c| c == '#').count();
        if (1..=6).contains(&hashes) && trimmed[hashes..].starts_with(' ') {
            let grow = match hashes {
                1 => 5.0,
                2 => 3.0,
                _ => 1.5,
            };
            ui.add_space(size * 0.3);
            text_line(ui, &trimmed[hashes + 1..], size + grow, t.text, true, t, &mut clicked);
            ui.add_space(size * 0.15);
            continue;
        }
        if let Some(q) = trimmed.strip_prefix("> ").or_else(|| (trimmed == ">").then_some("")) {
            ui.horizontal_top(|ui| {
                let (r, _) = ui.allocate_exact_size(egui::vec2(3.0, size * 1.4), egui::Sense::hover());
                ui.painter().rect_filled(r, 1.0, t.accent);
                text_line(ui, q, size, t.text_dim, false, t, &mut clicked);
            });
            continue;
        }
        // lists, indented by the leading spaces
        let indent = (line.len() - line.trim_start().len()) as f32 / 2.0 * size;
        let bullet = ["- ", "* ", "+ "].iter().find_map(|b| trimmed.strip_prefix(b)).map(|rest| ("•".to_string(), rest));
        let numbered = trimmed.split_once(". ").filter(|(n, _)| !n.is_empty() && n.len() <= 3 && n.chars().all(|c| c.is_ascii_digit())).map(|(n, rest)| (format!("{n}."), rest));
        if let Some((mark, rest)) = bullet.or(numbered) {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.add_space(indent + 2.0);
                ui.add_sized([size * 1.2, size * 1.3], egui::Label::new(egui::RichText::new(mark).size(size).color(t.text_dim)));
                text_line(ui, rest, size, t.text, false, t, &mut clicked);
            });
            continue;
        }
        text_line(ui, trimmed, size, t.text, false, t, &mut clicked);
    }
    clicked
}

/// A display formula, on its own indented line, a little larger than the text.
fn math_block(ui: &mut egui::Ui, src: &str, size: f32, t: &Tokens) {
    let mut j = LayoutJob::default();
    append_math(&mut j, src, size + 1.0, t.text, false, t);
    j.wrap.max_width = f32::INFINITY;
    ui.add_space(size * 0.2);
    ui.horizontal(|ui| {
        ui.add_space(size * 1.5);
        ui.add(egui::Label::new(j).wrap());
    });
    ui.add_space(size * 0.2);
}

/// A formula's text into `job`: symbols as characters, exponents raised and indices lowered
/// in a smaller size.
fn append_math(job: &mut LayoutJob, src: &str, size: f32, color: Color32, bold: bool, t: &Tokens) {
    for (s, k) in latex::render(src) {
        let small = size * 0.72;
        let font = match (k, bold) {
            (latex::Script::Normal, true) => crate::theme::medium_font(size),
            (latex::Script::Normal, false) => FontId::new(size, FontFamily::Proportional),
            (_, true) => crate::theme::medium_font(small),
            (_, false) => FontId::new(small, FontFamily::Proportional),
        };
        let mut f = TextFormat::simple(font, color);
        if bold && color == t.text {
            f.color = if t.dark { Color32::WHITE } else { Color32::BLACK };
        }
        f.valign = match k {
            latex::Script::Normal => egui::Align::Center,
            latex::Script::Sup => egui::Align::TOP,
            latex::Script::Sub => egui::Align::BOTTOM,
        };
        job.append(&s, 0.0, f);
    }
}

/// One wrapped line. With links in it, the line is clickable, and the pointer turns into a
/// hand over a link.
fn text_line(ui: &mut egui::Ui, s: &str, size: f32, color: Color32, strong: bool, t: &Tokens, clicked: &mut Option<String>) {
    let (mut j, links) = job_links(s, size, color, strong, t);
    if links.is_empty() {
        ui.add(egui::Label::new(j).wrap());
        return;
    }
    j.wrap.max_width = ui.available_width();
    let galley = ui.fonts(|f| f.layout_job(j));
    let resp = ui.add(egui::Label::new(galley.clone()).sense(egui::Sense::click()));
    let Some(pos) = resp.hover_pos() else { return };
    let at = galley.cursor_from_pos(pos - resp.rect.min).ccursor.index;
    if let Some(l) = links.iter().find(|l| (l.start..=l.end).contains(&at)) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        let target = l.target.clone();
        if resp.on_hover_text(&target).clicked() {
            *clicked = Some(target);
        }
    }
}

/// A link in a laid-out line: its label's characters (not bytes) and where it points.
struct Link {
    start: usize,
    end: usize,
    target: String,
}

/// `[label](target)` starting at `k`: the label, the target and the index after the link.
fn link_at(chars: &[char], k: usize) -> Option<(String, String, usize)> {
    let close = (k + 1..chars.len()).find(|&i| chars[i] == ']' || chars[i] == '[')?;
    if chars[close] != ']' || chars.get(close + 1) != Some(&'(') {
        return None;
    }
    let end = (close + 2..chars.len()).find(|&i| chars[i] == ')')?;
    let label: String = chars[k + 1..close].iter().filter(|c| !matches!(c, '`' | '*')).collect();
    let target: String = chars[close + 2..end].iter().collect();
    let target = target.trim().trim_start_matches('<').trim_end_matches('>').trim().to_string();
    if label.trim().is_empty() || target.is_empty() {
        return None;
    }
    Some((label, target, end + 1))
}

/// A table: the first row is the header, separator rows (`|---|:--:|`) are skipped.
fn table(ui: &mut egui::Ui, rows: &[&str], size: f32, t: &Tokens, id: usize) {
    let cells = |r: &str| -> Vec<String> { split_row(r) };
    let sep = |r: &str| cells(r).iter().all(|c| !c.is_empty() && c.chars().all(|ch| matches!(ch, '-' | ':' | ' ')));
    let rows: Vec<Vec<String>> = rows.iter().filter(|r| !sep(r)).map(|r| cells(r)).collect();
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
    if cols == 0 {
        return;
    }
    // right-align a column whose body cells are all numbers
    let numeric: Vec<bool> = (0..cols).map(|c| rows.len() > 1 && rows[1..].iter().all(|r| r.get(c).is_none_or(|s| s.is_empty() || is_number(s)))).collect();
    let cell = |k: usize, c: usize| rows[k].get(c).map(String::as_str).unwrap_or("");
    let cell_job = |k: usize, c: usize, wrap: f32| {
        let mut j = job(cell(k, c), size - 0.5, if k == 0 { t.accent_text } else { t.text }, k == 0, t);
        j.wrap.max_width = wrap;
        j
    };
    // each column as wide as its widest cell on one line, but never narrower than its longest
    // word (so no word is broken); when that does not fit, the space is shared out so that
    // narrow columns keep their natural width and the wide ones wrap
    let (gap, pad) = (12.0_f32, 6.0_f32);
    let mut natural = vec![0.0_f32; cols];
    let mut least = vec![0.0_f32; cols];
    ui.fonts(|f| {
        for k in 0..rows.len() {
            for c in 0..cols {
                natural[c] = natural[c].max(f.layout_job(cell_job(k, c, f32::INFINITY)).size().x);
                for w in cell(k, c).split_whitespace() {
                    least[c] = least[c].max(f.layout_job(job(w, size - 0.5, t.text, k == 0, t)).size().x);
                }
            }
        }
    });
    let room = (ui.available_width() - 2.0 * pad - 12.0 - gap * (cols as f32 - 1.0)).max(cols as f32 * 40.0);
    let mut width: Vec<f32> = natural.iter().zip(&least).map(|(n, l)| n.max(*l).ceil()).collect();
    if width.iter().sum::<f32>() > room {
        let mut fixed = vec![false; cols];
        loop {
            let open: Vec<usize> = (0..cols).filter(|c| !fixed[*c]).collect();
            if open.is_empty() {
                break;
            }
            let left = room - (0..cols).filter(|c| fixed[*c]).map(|c| width[c]).sum::<f32>();
            let share = left / open.len() as f32;
            let small: Vec<usize> = open.iter().copied().filter(|c| natural[*c].max(least[*c]) <= share).collect();
            if small.is_empty() {
                for c in open {
                    width[c] = share.max(least[c]).ceil();
                }
                break;
            }
            for c in small {
                fixed[c] = true;
                width[c] = natural[c].max(least[c]).ceil();
            }
        }
    }
    let total: f32 = width.iter().sum::<f32>() + gap * (cols as f32 - 1.0) + 2.0 * pad;
    ui.add_space(size * 0.2);
    egui::Frame::new().stroke(egui::Stroke::new(1.0, t.field_border)).corner_radius(6.0).inner_margin(egui::Margin::same(4)).show(ui, |ui| {
        egui::ScrollArea::horizontal().id_salt(("md_table", id)).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for k in 0..rows.len() {
                // the row as tall as its tallest cell, every cell drawn inside its own column
                let galleys: Vec<_> = (0..cols).map(|c| ui.fonts(|f| f.layout_job(cell_job(k, c, width[c])))).collect();
                let h = galleys.iter().map(|g| g.size().y).fold(size, f32::max) + 2.0 * pad;
                let (rect, _) = ui.allocate_exact_size(egui::vec2(total, h), egui::Sense::hover());
                let p = ui.painter();
                if k % 2 == 0 && k > 0 {
                    p.rect_filled(rect, 0.0, ui.visuals().faint_bg_color);
                }
                let mut x = rect.min.x + pad;
                for (c, g) in galleys.into_iter().enumerate() {
                    let dx = if numeric[c] && k > 0 { width[c] - g.size().x } else { 0.0 };
                    p.galley(egui::pos2(x + dx, rect.min.y + pad), g, t.text);
                    x += width[c] + gap;
                }
                if k == 0 {
                    p.hline(rect.x_range(), rect.max.y, egui::Stroke::new(1.0, t.field_border));
                }
            }
        });
    });
    ui.add_space(size * 0.2);
}

/// The cells of a table row: split at the pipes that are not escaped (`\|` is a pipe in a
/// cell, as in `\|u\|`), without the outer ones.
fn split_row(r: &str) -> Vec<String> {
    let r = r.trim();
    let r = r.strip_prefix('|').unwrap_or(r);
    let r = if r.ends_with('|') && !r.ends_with("\\|") { &r[..r.len() - 1] } else { r };
    let mut cells = vec![String::new()];
    let mut chars = r.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'|') => {
                // keep the escape for the inline pass, which turns it into a plain pipe
                cells.last_mut().unwrap().push_str("\\|");
                chars.next();
            }
            '|' => cells.push(String::new()),
            _ => cells.last_mut().unwrap().push(c),
        }
    }
    cells.into_iter().map(|c| c.trim().to_string()).collect()
}

fn is_number(s: &str) -> bool {
    let s = s.trim().trim_end_matches(['%', ' ']).replace([',', '−'], "");
    let s = s.split_whitespace().next().unwrap_or("");
    !s.is_empty() && s.trim_start_matches(['-', '+', '≈', '~']).parse::<f64>().is_ok()
}

/// One line with inline **bold**, *italic* and `code` (underscores are left alone: names such
/// as Borehole_1 use them). Links show their label.
fn job(s: &str, size: f32, color: Color32, strong: bool, t: &Tokens) -> LayoutJob {
    job_links(s, size, color, strong, t).0
}

/// `job`, with the links it holds.
fn job_links(s: &str, size: f32, color: Color32, strong: bool, t: &Tokens) -> (LayoutJob, Vec<Link>) {
    let mut job = LayoutJob::default();
    let mut links = vec![];
    let (mut bold, mut italic, mut code) = (strong, false, false);
    let mut buf = String::new();
    let flush = |job: &mut LayoutJob, buf: &mut String, bold: bool, italic: bool, code: bool| {
        if buf.is_empty() {
            return;
        }
        let font = if code {
            FontId::new(size - 1.0, FontFamily::Monospace)
        } else if bold {
            crate::theme::medium_font(size)
        } else {
            FontId::new(size, FontFamily::Proportional)
        };
        let mut f = TextFormat::simple(font, color);
        if bold && color == t.text {
            f.color = if t.dark { Color32::WHITE } else { Color32::BLACK };
        }
        f.italics = italic;
        if code {
            f.background = t.field;
        }
        job.append(buf, 0.0, f);
        buf.clear();
    };
    let chars: Vec<char> = s.chars().collect();
    let mut k = 0;
    while k < chars.len() {
        let c = chars[k];
        // math: $…$, $$…$$, \(…\) and \[…\] within the line
        if !code && (c == '$' || c == '\\') {
            if let Some((body, next, _)) = latex::span_at(&chars, k) {
                flush(&mut job, &mut buf, bold, italic, code);
                append_math(&mut job, &body, size, color, bold, t);
                k = next;
                continue;
            }
        }
        // a backslash escapes Markdown punctuation: \| \* \` \_ \# and the like
        if c == '\\' && !code && chars.get(k + 1).is_some_and(|n| n.is_ascii_punctuation()) {
            buf.push(chars[k + 1]);
            k += 2;
            continue;
        }
        if !code && c == '[' {
            if let Some((label, target, next)) = link_at(&chars, k) {
                flush(&mut job, &mut buf, bold, italic, code);
                let start = job.text.chars().count();
                let mut f = TextFormat::simple(if bold { crate::theme::medium_font(size) } else { FontId::new(size, FontFamily::Proportional) }, t.accent_text);
                f.underline = egui::Stroke::new(1.0, t.accent_text);
                f.italics = italic;
                job.append(&label, 0.0, f);
                links.push(Link { start, end: start + label.chars().count(), target });
                k = next;
                continue;
            }
        }
        if c == '`' {
            flush(&mut job, &mut buf, bold, italic, code);
            code = !code;
        } else if !code && c == '*' && chars.get(k + 1) == Some(&'*') {
            flush(&mut job, &mut buf, bold, italic, code);
            bold = strong || !bold;
            k += 1;
        } else if !code && c == '*' && chars.get(k + 1).is_some_and(|n| !n.is_whitespace()) || !code && c == '*' && italic {
            flush(&mut job, &mut buf, bold, italic, code);
            italic = !italic;
        } else {
            buf.push(c);
        }
        k += 1;
    }
    flush(&mut job, &mut buf, bold, italic, code);
    job.wrap.max_width = f32::INFINITY;
    (job, links)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_and_numbers() {
        let t = Tokens::get(&egui::Context::default());
        let j = job("Settlement **12.4 mm** at `Phase_1`, *drained*", 13.0, Color32::GRAY, false, &t);
        let text: String = j.text.clone();
        assert_eq!(text, "Settlement 12.4 mm at Phase_1, drained");
        assert_eq!(j.sections.len(), 6);
        assert!(j.sections[3].format.font_id.family == FontFamily::Monospace);
        assert!(j.sections[5].format.italics);
        // a lone star (a multiplication) stays
        assert_eq!(job("2 * 3", 13.0, Color32::GRAY, false, &t).text, "2 * 3");
        assert!(is_number("-12.4") && is_number("1,530") && is_number("8.9 mm") && !is_number("Sand"));
        // escaped pipes stay in their cell and show as pipes
        assert_eq!(split_row("| Drawdown | plastic | max \\|u\\| = 19.5 mm |"), ["Drawdown", "plastic", "max \\|u\\| = 19.5 mm"]);
        assert_eq!(job("max \\|u\\| = 19.5 mm, 2 \\* 3", 13.0, Color32::GRAY, false, &t).text, "max |u| = 19.5 mm, 2 * 3");
    }

    #[test]
    fn math_in_a_line() {
        let t = Tokens::get(&egui::Context::default());
        let j = job(r"Height $H = 3\text{ m}$, so $\phi M_n \ge M_u$ costs $5 and $10.", 13.0, Color32::GRAY, false, &t);
        assert_eq!(j.text, "Height H = 3 m, so ϕMn ≥ Mu costs $5 and $10.");
        // the index is lowered and smaller
        let n = j.sections.iter().find(|s| &j.text[s.byte_range.clone()] == "n").expect("index n");
        assert_eq!(n.format.valign, egui::Align::BOTTOM);
        assert!(n.format.font_id.size < 13.0);
        assert_eq!(job(r"\(\theta = 30^\circ\)", 13.0, Color32::GRAY, false, &t).text, "θ = 30°");
        // an escaped dollar stays a dollar
        assert_eq!(job(r"\$20 and \$30", 13.0, Color32::GRAY, false, &t).text, "$20 and $30");
    }

    #[test]
    fn math_symbols_have_glyphs() {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let _ = ctx.run(Default::default(), |_| {});
        let all = "αβγδϵεζηθϑικλμνξπϖρϱσςτυϕφχψωΓΔΘΛΞΠΣΥΦΨΩℓℏ∂∇∞′°∘Å×·÷±∓∗⋆•≤≥≪≫≠≈∼≃≅≡∝→←⇒⇐↔⇔↑↓↦Σ∏∫∬∮∈∉⊂⊆∪∩∅∀∃¬∧∨∠⊥∥∴∵…⋮⋱⟨⟩⌈⌉⌊⌋‖√−ℝℕℤℚℂ✓";
        let font = FontId::new(13.0, FontFamily::Proportional);
        let missing: String = ctx.fonts(|f| all.chars().filter(|c| !f.has_glyphs(&font, &c.to_string())).collect());
        assert!(missing.is_empty(), "no glyph for: {missing}");
    }

    #[test]
    fn links() {
        let t = Tokens::get(&egui::Context::default());
        let (j, l) = job_links("Here it is: [reports/Slope stability.pdf](reports/Slope%20stability.pdf), and [the code](<https://example.org/a b>).", 13.0, Color32::GRAY, false, &t);
        assert_eq!(j.text, "Here it is: reports/Slope stability.pdf, and the code.");
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].target, "reports/Slope%20stability.pdf");
        assert_eq!(j.text.chars().skip(l[0].start).take(l[0].end - l[0].start).collect::<String>(), "reports/Slope stability.pdf");
        assert_eq!(l[1].target, "https://example.org/a b");
        // square brackets that are not a link stay as they are
        let (j, l) = job_links("a [note] and x[i] (b)", 13.0, Color32::GRAY, false, &t);
        assert_eq!((j.text.as_str(), l.len()), ("a [note] and x[i] (b)", 0));
    }
}
