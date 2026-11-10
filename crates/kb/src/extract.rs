//! Text extraction from PDF, TXT and MD files into page-tagged lines.

use std::collections::HashMap;
use std::path::Path;

use crate::text::clean_chars;

/// One line of source text with the page it came from (PDF only).
#[derive(Debug, Clone)]
pub struct Line {
    pub text: String,
    pub page: Option<u32>,
}

pub fn extract_file(path: &Path) -> Result<Vec<Line>, String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "pdf" => extract_pdf(path),
        "txt" | "md" | "markdown" | "text" => {
            let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
            let s = String::from_utf8_lossy(&bytes);
            Ok(text_to_lines(&s, None))
        }
        other => Err(format!(
            "unsupported file type '.{other}' for {} (supported: .pdf, .txt, .md)",
            path.display()
        )),
    }
}

/// Plain text to lines. A form-feed character starts a new page, so text
/// exported with `pdftotext` keeps page numbers too.
pub fn text_to_lines(s: &str, first_page: Option<u32>) -> Vec<Line> {
    let s = clean_chars(s);
    let has_ff = s.contains('\u{000C}');
    let mut page = if has_ff { Some(first_page.unwrap_or(1)) } else { first_page };
    let mut out = Vec::new();
    for (i, part) in s.split('\u{000C}').enumerate() {
        if i > 0 {
            page = page.map(|p| p + 1);
        }
        for l in part.lines() {
            out.push(Line { text: l.trim_end().to_string(), page });
        }
    }
    out
}

fn extract_pdf(path: &Path) -> Result<Vec<Line>, String> {
    let p = path.to_path_buf();
    // pdf-extract panics on some malformed fonts; turn that into an error.
    let res = std::panic::catch_unwind(move || pdf_extract::extract_text_by_pages(&p));
    let pages = match res {
        Ok(Ok(pages)) => pages,
        Ok(Err(e)) => {
            return Err(format!(
                "could not extract text from PDF {}: {e}. If the file is encrypted or damaged, export it to text first.",
                path.display()
            ))
        }
        Err(_) => {
            return Err(format!(
                "the PDF parser crashed on {} (unsupported font or structure). Export the PDF to text (e.g. `pdftotext -layout`) and ingest the .txt instead.",
                path.display()
            ))
        }
    };
    if pages.is_empty() {
        return Err(format!("PDF {} has no pages that could be read", path.display()));
    }
    let alnum: usize = pages
        .iter()
        .map(|p| p.chars().filter(|c| c.is_alphanumeric()).count())
        .sum();
    if alnum < 40 * pages.len().max(1) / 2 + 20 {
        return Err(format!(
            "PDF {} has almost no text layer ({} letters over {} pages); it is probably a scanned image. Run OCR first (e.g. `ocrmypdf in.pdf out.pdf`) and ingest the OCR'd file.",
            path.display(),
            alnum,
            pages.len()
        ));
    }
    let mut per_page: Vec<Vec<String>> = pages
        .iter()
        .map(|p| clean_chars(p).lines().map(|l| l.trim_end().to_string()).collect())
        .collect();
    strip_running_heads(&mut per_page);
    let mut out = Vec::new();
    for (i, lines) in per_page.into_iter().enumerate() {
        let page = Some(i as u32 + 1);
        for l in lines {
            out.push(Line { text: l, page });
        }
    }
    Ok(dehyphenate(out))
}

fn shape(l: &str) -> String {
    l.trim()
        .chars()
        .map(|c| if c.is_ascii_digit() { '#' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_page_number(l: &str) -> bool {
    let t = l.trim();
    if t.is_empty() {
        return false;
    }
    if t.chars().all(|c| c.is_ascii_digit()) && t.len() <= 4 {
        return true;
    }
    let low = t.to_ascii_lowercase();
    (low.starts_with("page ") && low.len() < 20) || (t.starts_with('-') && t.ends_with('-') && t.len() < 10)
}

/// Remove running headers/footers (lines that repeat on at least half of the
/// pages, digits ignored) and bare page numbers.
fn strip_running_heads(pages: &mut [Vec<String>]) {
    let n = pages.len();
    let mut counts: HashMap<String, usize> = HashMap::new();
    if n >= 4 {
        for p in pages.iter() {
            let nonempty: Vec<&String> = p.iter().filter(|l| !l.trim().is_empty()).collect();
            let mut seen = std::collections::HashSet::new();
            let m = nonempty.len();
            for (j, l) in nonempty.iter().enumerate() {
                if j < 2 || j + 2 >= m {
                    let s = shape(l);
                    if !s.is_empty() && seen.insert(s.clone()) {
                        *counts.entry(s).or_insert(0) += 1;
                    }
                }
            }
        }
    }
    let thresh = (n / 2).max(3);
    for p in pages.iter_mut() {
        let m = p.len();
        let mut keep = Vec::with_capacity(m);
        for (j, l) in p.iter().enumerate() {
            let edge = j < 4 || j + 4 >= m;
            if edge && is_page_number(l) {
                continue;
            }
            if n >= 4 && edge && counts.get(&shape(l)).copied().unwrap_or(0) >= thresh {
                continue;
            }
            keep.push(l.clone());
        }
        *p = keep;
    }
}

/// Join words broken across lines ("reinforce-" + "ment").
fn dehyphenate(lines: Vec<Line>) -> Vec<Line> {
    let mut out = lines;
    let mut merged: Vec<Line> = Vec::with_capacity(out.len());
    let mut i = 0;
    while i < out.len() {
        let cur = out[i].clone();
        let t = cur.text.trim_end().to_string();
        if t.ends_with('-') && i + 1 < out.len() {
            let next = out[i + 1].text.trim_start();
            let ok_prev = t.chars().rev().nth(1).map(|c| c.is_alphabetic()).unwrap_or(false);
            if ok_prev && next.chars().next().map(|c| c.is_lowercase()).unwrap_or(false) {
                let (word, rest) = match next.find(' ') {
                    Some(k) => (&next[..k], next[k + 1..].to_string()),
                    None => (next, String::new()),
                };
                merged.push(Line { text: format!("{}{}", &t[..t.len() - 1], word), page: cur.page });
                out[i + 1].text = rest;
                i += 1;
                continue;
            }
        }
        merged.push(cur);
        i += 1;
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn form_feed_pages() {
        let l = text_to_lines("a\nb\u{000C}c", None);
        assert_eq!(l[0].page, Some(1));
        assert_eq!(l[2].page, Some(2));
        assert_eq!(text_to_lines("a\nb", None)[0].page, None);
    }
    #[test]
    fn hyphen_join() {
        let v = vec![
            Line { text: "shear reinforce-".into(), page: Some(1) },
            Line { text: "ment shall be provided".into(), page: Some(1) },
        ];
        let m = dehyphenate(v);
        assert_eq!(m[0].text, "shear reinforcement");
        assert_eq!(m[1].text, "shall be provided");
    }
}
