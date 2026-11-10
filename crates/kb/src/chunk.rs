//! Structure-aware chunking of design standards.
//!
//! A heading detector recognises numbered clauses (`22.5.5.1`, `9.6.1.2`,
//! `6.5.2.2(1)P`, `D.4`), tables and figures (`Table 22.5.5.1`), sections,
//! chapters and annexes (`Section 4.3`, `Annex D`) and Thai clauses
//! (`ข้อ 3.2`). Each clause becomes one chunk carrying its number, title, page
//! and the titles of its parent clauses (the breadcrumb). Long clauses are
//! split into overlapping pieces; heading-only parents are folded into their
//! first child so that no chunk is just a title.

use std::collections::HashMap;
use std::sync::OnceLock;

use regex::Regex;

use crate::extract::Line;

pub const TARGET: usize = 1200;
pub const OVERLAP: usize = 200;
const MAX_SINGLE: usize = 1450;
const TINY: usize = 160;

#[derive(Debug, Clone, PartialEq)]
pub struct Heading {
    pub clause: String,
    pub title: String,
}

#[derive(Debug, Clone)]
pub struct RawChunk {
    pub clause: String,
    pub title: String,
    pub page: Option<u32>,
    pub text: String,
    /// Titles of parent clauses, outermost first, joined with " > ".
    pub context: String,
    /// 0 for the first piece of a clause, 1.. for continuation pieces.
    pub part: u32,
}

struct Rx {
    toc: Regex,
    thai: Regex,
    table: Regex,
    section: Regex,
    numeric: Regex,
}

fn rx() -> &'static Rx {
    static R: OnceLock<Rx> = OnceLock::new();
    R.get_or_init(|| Rx {
        toc: Regex::new(r"(\.\s*){4,}\s*\d+\s*$|…+\s*\d+\s*$").unwrap(),
        thai: Regex::new(r"^ข้อ\s*(\d{1,3}(?:\.\d{1,3})*)\s*[.)]?\s*(.*)$").unwrap(),
        table: Regex::new(
            r"^(?i:(table|figure|fig\.))\s+([A-Z]?\d{1,3}(?:[.\-]\d{1,3})*[a-z]?|[A-Z]\.\d{1,3}(?:\.\d{1,3})*)\s*[-–—:.]?\s*(.*)$",
        )
        .unwrap(),
        section: Regex::new(
            r"^(Section|SECTION|Chapter|CHAPTER|Clause|Article|ARTICLE|Annex|ANNEX|Appendix|APPENDIX|Part|PART|§)\s*((?:[A-Z]\.)?\d{1,3}(?:\.\d{1,3})*|[A-Z])\b\s*(\((?i:normative|informative)\))?\s*[-–—:.]?\s*(.*)$",
        )
        .unwrap(),
        numeric: Regex::new(r"^((?:[A-Z]\.)?\d{1,3}(?:\.\d{1,3})*|[A-Z]\.\d{1,3}(?:\.\d{1,3})*)(\(\d{1,2}\)P?)?\.?(?:\s+(.*))?$")
            .unwrap(),
    })
}

const UNITS: &[&str] = &[
    "mm", "m", "cm", "km", "MPa", "GPa", "kPa", "Pa", "kN", "N", "MN", "kN/m", "kN/m2", "kN/m3", "psi", "ksi", "in.",
    "in", "ft", "lb", "kg", "t", "%", "°", "times", "x", "×", "and", "or", "to", "percent", "MN/m2", "kNm", "kN·m",
];

fn titleish(rest: &str) -> bool {
    let r = rest.trim();
    !r.is_empty() && r.chars().count() <= 100 && !r.ends_with('.') && !r.ends_with(',') && !r.ends_with(';') && r.split_whitespace().count() <= 14
}

fn first_sentence(rest: &str) -> String {
    let r = rest.trim();
    let end = r.find(". ").map(|i| i + 1).unwrap_or(r.len());
    let s = &r[..end];
    if s.chars().count() <= 90 {
        return s.trim_end_matches('.').to_string();
    }
    let mut out = String::new();
    for w in s.split_whitespace() {
        if out.chars().count() + w.chars().count() > 80 {
            break;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(w);
    }
    format!("{out}…")
}

fn title_of(rest: &str) -> String {
    if titleish(rest) {
        rest.trim().to_string()
    } else if rest.trim().is_empty() {
        String::new()
    } else {
        first_sentence(rest)
    }
}

/// Detect a clause heading at the start of `line`. `next` is the following
/// non-empty line, used when a clause number stands alone on its line.
pub fn detect_heading(line: &str, next: Option<&str>) -> Option<Heading> {
    let l = line.trim();
    if l.is_empty() || l.chars().count() > 400 {
        return None;
    }
    let r = rx();
    if r.toc.is_match(l) {
        return None;
    }
    if let Some(c) = r.thai.captures(l) {
        return Some(Heading { clause: format!("ข้อ {}", &c[1]), title: title_of(&c[2]) });
    }
    if let Some(c) = r.table.captures(l) {
        let kind = c[1].to_ascii_lowercase();
        let kind = if kind.starts_with("tab") { "Table" } else { "Figure" };
        let rest = c.get(3).map(|m| m.as_str()).unwrap_or("");
        // "Table 5 shows ..." in running text is a reference, not a caption
        if rest.chars().next().map(|ch| ch.is_lowercase()).unwrap_or(false) {
            return None;
        }
        return Some(Heading { clause: format!("{kind} {}", &c[2]), title: title_of(rest) });
    }
    if let Some(c) = r.section.captures(l) {
        let word = &c[1];
        let rest = c.get(4).map(|m| m.as_str()).unwrap_or("");
        if rest.chars().next().map(|ch| ch.is_lowercase()).unwrap_or(false) {
            return None; // "Section 4 applies to ..." mid-sentence
        }
        let w = match word.to_ascii_lowercase().as_str() {
            "section" => "Section",
            "chapter" => "Chapter",
            "clause" => "Clause",
            "article" => "Article",
            "annex" => "Annex",
            "appendix" => "Appendix",
            "part" => "Part",
            _ => "§",
        };
        let clause = if w == "§" { c[2].to_string() } else { format!("{w} {}", &c[2]) };
        return Some(Heading { clause, title: title_of(rest) });
    }
    if let Some(c) = r.numeric.captures(l) {
        let num = &c[1];
        let para = c.get(2).map(|m| m.as_str()).unwrap_or("");
        let rest = c.get(3).map(|m| m.as_str()).unwrap_or("").trim();
        let comps: Vec<&str> = num.split('.').collect();
        let numeric_comps: Vec<&str> = comps.iter().copied().filter(|s| s.chars().all(|c| c.is_ascii_digit())).collect();
        let lettered = comps[0].chars().all(|c| c.is_ascii_uppercase());
        if numeric_comps.first().map(|s| s.starts_with('0')).unwrap_or(false) && !lettered {
            return None; // 0.75 ...
        }
        if numeric_comps.iter().any(|s| s.len() > 1 && s.starts_with('0')) {
            return None; // 1.05, 2.025: decimals, not clauses
        }
        let levels = comps.len();
        if rest.is_empty() {
            // number alone on its line: accept only for multi-level numbers followed by a capitalised line
            if levels >= 2 && para.is_empty() {
                if let Some(n) = next {
                    let n = n.trim();
                    if n.chars().next().map(|ch| ch.is_uppercase()).unwrap_or(false) && n.chars().count() > 3 {
                        return Some(Heading { clause: num.to_string(), title: String::new() });
                    }
                }
            }
            return None;
        }
        let first = rest.chars().next().unwrap();
        if first.is_ascii_lowercase() || first.is_ascii_digit() || matches!(first, '=' | '+' | '-' | '×' | '*' | '/' | ',' | ';' | ')' | '<' | '>' | '≤' | '≥') {
            return None;
        }
        let first_word = rest.split_whitespace().next().unwrap_or("");
        if UNITS.contains(&first_word) {
            return None;
        }
        if levels == 1 && !lettered {
            // "4 General": chapter-level heading. Strict, because body lines often start with a number.
            let n: u32 = num.parse().unwrap_or(999);
            if n == 0 || n > 60 || !titleish(rest) || rest.split_whitespace().count() > 8 || !para.is_empty() {
                return None;
            }
            if !first.is_uppercase() {
                return None;
            }
        }
        if levels == 1 && lettered {
            return None; // "A Something" is not a clause number
        }
        // a lone lettered prefix like "D.4" needs an upper-case start too
        if !(first.is_uppercase() || first == '(' || !first.is_ascii()) {
            return None;
        }
        return Some(Heading { clause: format!("{num}{para}"), title: title_of(rest) });
    }
    None
}

/// Kind and bare number of a clause label, for matching. Section/Chapter/
/// Clause/ข้อ/§ prefixes are dropped; Table/Figure/Annex keep their kind.
pub fn clause_key(s: &str) -> (String, String) {
    let t = s.trim();
    let low = t.to_lowercase();
    let strip = |p: &str| low[p.len()..].trim().to_string();
    for (p, kind) in [
        ("table", "table"),
        ("figure", "figure"),
        ("fig.", "figure"),
        ("annex", "annex"),
        ("appendix", "annex"),
        ("section", ""),
        ("chapter", ""),
        ("clause", ""),
        ("article", ""),
        ("cl.", ""),
        ("part", ""),
        ("ข้อ", ""),
        ("§", ""),
    ] {
        if low.starts_with(p) {
            let rest = strip(p);
            if !rest.is_empty() {
                return (kind.to_string(), rest.replace(' ', ""));
            }
        }
    }
    (String::new(), low.replace(' ', ""))
}

/// True when `child` (bare) lies under `parent` (bare): `22.5.5.1` under `22.5`,
/// `6.5.2.2(1)p` under `6.5.2.2`, `d.4` under `d`.
pub fn is_descendant(child: &str, parent: &str) -> bool {
    child.len() > parent.len()
        && child.starts_with(parent)
        && matches!(child.as_bytes()[parent.len()], b'.' | b'(' | b'-')
}

/// Natural sort key: numeric components compare as numbers.
pub fn natural_key(bare: &str) -> Vec<(u32, String)> {
    let mut out = Vec::new();
    for part in bare.split(|c: char| c == '.' || c == '(' || c == ')' || c == '-').filter(|p| !p.is_empty()) {
        match part.parse::<u32>() {
            Ok(n) => out.push((n, String::new())),
            Err(_) => out.push((u32::MAX, part.to_string())),
        }
    }
    out
}

struct Section {
    heading: Option<Heading>,
    lines: Vec<(String, Option<u32>)>,
}

impl Section {
    fn body_len(&self) -> usize {
        self.lines.iter().skip(if self.heading.is_some() { 1 } else { 0 }).map(|(l, _)| l.trim().len()).sum()
    }
    fn text(&self) -> String {
        let mut s = String::new();
        for (l, _) in &self.lines {
            s.push_str(l);
            s.push('\n');
        }
        s.trim().to_string()
    }
}

pub fn chunk_lines(lines: &[Line]) -> Vec<RawChunk> {
    // 1. split into sections at headings
    let mut sections: Vec<Section> = vec![Section { heading: None, lines: Vec::new() }];
    let n = lines.len();
    for i in 0..n {
        let t = lines[i].text.trim_end();
        if t.trim().is_empty() {
            if let Some(s) = sections.last_mut() {
                s.lines.push((String::new(), lines[i].page));
            }
            continue;
        }
        if crate::chunk::rx().toc.is_match(t.trim()) {
            continue; // table-of-contents line
        }
        let next = lines[i + 1..].iter().map(|l| l.text.as_str()).find(|l| !l.trim().is_empty());
        if let Some(h) = detect_heading(t, next) {
            sections.push(Section { heading: Some(h), lines: vec![(t.trim().to_string(), lines[i].page)] });
        } else {
            sections.last_mut().unwrap().lines.push((t.to_string(), lines[i].page));
        }
    }
    sections.retain(|s| s.heading.is_some() || s.lines.iter().any(|(l, _)| !l.trim().is_empty()));

    // 2. breadcrumbs and folding of heading-only parents into their first child
    let mut titles: HashMap<String, String> = HashMap::new();
    let mut out = Vec::new();
    let mut carry: Vec<(String, Option<u32>)> = Vec::new(); // folded parent lines
    for idx in 0..sections.len() {
        let sec = &sections[idx];
        let (clause, title) = match &sec.heading {
            Some(h) => (h.clause.clone(), h.title.clone()),
            None => (String::new(), String::new()),
        };
        let (kind, bare) = clause_key(&clause);
        // context: titles of every proper prefix that we have seen
        let mut ctx = Vec::new();
        if !bare.is_empty() {
            let comps: Vec<&str> = bare.split('.').collect();
            for k in 1..=comps.len() {
                let pre = comps[..k].join(".");
                if pre == bare && kind.is_empty() {
                    break;
                }
                if let Some(t) = titles.get(&pre) {
                    if !t.is_empty() {
                        ctx.push(t.clone());
                    }
                }
            }
            if kind.is_empty() || kind == "annex" {
                titles.insert(bare.clone(), title.clone());
            }
        }
        // fold tiny parent into following child
        let next_bare = sections.get(idx + 1).and_then(|s| s.heading.as_ref()).map(|h| clause_key(&h.clause).1);
        let is_parent_of_next = match &next_bare {
            Some(nb) => !bare.is_empty() && (is_descendant(nb, &bare) || kind == "annex" && nb.starts_with(&format!("{bare}."))),
            None => false,
        };
        let chapterish = clause.starts_with("Section") || clause.starts_with("Chapter") || clause.starts_with("Part") || kind == "annex";
        if sec.heading.is_some() && sec.body_len() < TINY && (is_parent_of_next || chapterish && next_bare.is_some()) {
            carry.extend(sec.lines.iter().cloned().filter(|(l, _)| !l.trim().is_empty()));
            continue;
        }
        let mut sec_lines: Vec<(String, Option<u32>)> = Vec::new();
        sec_lines.append(&mut carry);
        sec_lines.extend(sec.lines.iter().cloned());
        let page = sec_lines.iter().find(|(l, _)| !l.trim().is_empty()).and_then(|(_, p)| *p);
        let merged = Section { heading: sec.heading.clone(), lines: sec_lines };
        let text = merged.text();
        if text.is_empty() {
            continue;
        }
        let context = ctx.join(" > ");
        let title = if title.is_empty() { ctx.last().cloned().unwrap_or_default() } else { title };
        if text.len() <= MAX_SINGLE {
            out.push(RawChunk { clause, title, page, text, context, part: 0 });
        } else {
            for (k, (piece, pg)) in split_long(&merged.lines).into_iter().enumerate() {
                out.push(RawChunk {
                    clause: clause.clone(),
                    title: title.clone(),
                    page: pg.or(page),
                    text: piece,
                    context: context.clone(),
                    part: k as u32,
                });
            }
        }
    }
    if !carry.is_empty() {
        let text = carry.iter().map(|(l, _)| l.as_str()).collect::<Vec<_>>().join("\n");
        let h = sections.iter().rev().find_map(|s| s.heading.clone());
        out.push(RawChunk {
            clause: h.as_ref().map(|h| h.clause.clone()).unwrap_or_default(),
            title: h.map(|h| h.title).unwrap_or_default(),
            page: carry[0].1,
            text,
            context: String::new(),
            part: 0,
        });
    }
    out
}

/// Split a long clause into ~TARGET-char pieces with ~OVERLAP chars of
/// overlap, cutting at line or sentence boundaries; a short tail joins the
/// previous piece.
fn split_long(lines: &[(String, Option<u32>)]) -> Vec<(String, Option<u32>)> {
    // units: lines, with very long lines broken at sentence ends
    let mut units: Vec<(String, Option<u32>)> = Vec::new();
    for (l, p) in lines {
        if l.trim().is_empty() {
            continue;
        }
        if l.len() <= 400 {
            units.push((l.clone(), *p));
            continue;
        }
        let mut cur = String::new();
        for w in l.split_inclusive(' ') {
            cur.push_str(w);
            let t = cur.trim_end();
            if cur.len() >= 250 && (t.ends_with('.') || t.ends_with(';') || cur.len() > 600) {
                units.push((cur.trim().to_string(), *p));
                cur.clear();
            }
        }
        if !cur.trim().is_empty() {
            units.push((cur.trim().to_string(), *p));
        }
    }
    let mut pieces: Vec<(String, Option<u32>)> = Vec::new();
    let mut i = 0;
    while i < units.len() {
        let start = i;
        let mut len = 0;
        while i < units.len() && (len == 0 || len + units[i].0.len() + 1 <= TARGET) {
            len += units[i].0.len() + 1;
            i += 1;
        }
        let text = units[start..i].iter().map(|u| u.0.as_str()).collect::<Vec<_>>().join("\n");
        pieces.push((text, units[start].1));
        if i >= units.len() {
            break;
        }
        // step back for overlap, but always advance at least one unit
        let mut back = 0;
        let mut j = i;
        while j > start + 1 && back + units[j - 1].0.len() <= OVERLAP {
            back += units[j - 1].0.len() + 1;
            j -= 1;
        }
        i = j;
    }
    if pieces.len() >= 2 && pieces.last().unwrap().0.len() < 300 {
        let tail = pieces.pop().unwrap();
        let last = pieces.last_mut().unwrap();
        // drop the overlapping lines the tail repeats
        let mut extra = Vec::new();
        for l in tail.0.lines() {
            if !last.0.contains(l) {
                extra.push(l);
            }
        }
        if !extra.is_empty() {
            last.0.push('\n');
            last.0.push_str(&extra.join("\n"));
        }
    }
    pieces
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(l: &str) -> Option<String> {
        detect_heading(l, Some("The next line")).map(|h| h.clause)
    }

    #[test]
    fn headings() {
        assert_eq!(h("22.5.5.1 For nonprestressed members, Vc shall be calculated"), Some("22.5.5.1".into()));
        assert_eq!(h("9.6.1.2 As,min shall be the greater of (a) and (b)"), Some("9.6.1.2".into()));
        assert_eq!(h("6.5.2.2(1)P The design value shall be checked"), Some("6.5.2.2(1)P".into()));
        assert_eq!(h("D.4 Drained conditions"), Some("D.4".into()));
        assert_eq!(h("Table 22.5.5.1 Vc for nonprestressed members"), Some("Table 22.5.5.1".into()));
        assert_eq!(h("Section 4.3 Loads on piles"), Some("Section 4.3".into()));
        assert_eq!(h("ข้อ 3.2 การออกแบบฐานราก"), Some("ข้อ 3.2".into()));
        assert_eq!(h("Annex D (informative) Bearing resistance"), Some("Annex D".into()));
        assert_eq!(h("4 General"), Some("4".into()));
        assert_eq!(h("22.5"), Some("22.5".into()));
        // body lines that only look numeric
        assert_eq!(h("0.75 for shear and torsion"), None);
        assert_eq!(h("1.5 times the effective depth"), None);
        assert_eq!(h("2.5 MPa minimum"), None);
        assert_eq!(h("150 mm in all cases where the member is thin."), None);
        assert_eq!(h("Table 5 shows the values"), None);
        assert_eq!(h("Section 4 applies to all members"), None);
        assert_eq!(h("22.5 One-way shear strength ........ 345"), None);
        assert_eq!(h("1.05 Shear capacity"), None);
    }

    #[test]
    fn clause_keys() {
        assert_eq!(clause_key("Section 4.3"), ("".into(), "4.3".into()));
        assert_eq!(clause_key("Table 22.5.5.1"), ("table".into(), "22.5.5.1".into()));
        assert_eq!(clause_key("ข้อ 3.2"), ("".into(), "3.2".into()));
        assert_eq!(clause_key("Annex D"), ("annex".into(), "d".into()));
        assert!(is_descendant("22.5.5.1", "22.5"));
        assert!(!is_descendant("22.50", "22.5"));
        assert!(is_descendant("6.5.2.2(1)p", "6.5.2.2"));
    }

    #[test]
    fn folding_and_splitting() {
        let mut txt = String::from("9.6 Reinforcement limits\n9.6.1 Minimum flexural reinforcement\n9.6.1.1 A minimum area shall be provided.\n9.6.1.2 Long clause.\n");
        for i in 0..60 {
            txt.push_str(&format!("Sentence number {i} of a long clause about anchorage of bars in tension.\n"));
        }
        txt.push_str("9.6.2 Next sibling heading with real body text that is long enough to stand alone as a clause in the standard library test case here.\n");
        let lines = crate::extract::text_to_lines(&txt, None);
        let c = chunk_lines(&lines);
        assert_eq!(c[0].clause, "9.6.1.1");
        assert!(c[0].text.starts_with("9.6 Reinforcement limits"));
        assert_eq!(c[0].context, "Reinforcement limits > Minimum flexural reinforcement");
        let parts: Vec<_> = c.iter().filter(|x| x.clause == "9.6.1.2").collect();
        assert!(parts.len() >= 3, "{}", parts.len());
        assert!(parts.iter().all(|p| p.text.len() <= TARGET + 400));
        assert_eq!(c.last().unwrap().clause, "9.6.2");
    }
}
