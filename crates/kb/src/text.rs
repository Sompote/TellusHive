//! Text normalisation, tokenisation and hashing shared by BM25 and the
//! hashed embedder.

/// FNV-1a 64-bit hash with a one-byte namespace seed. Stable across Rust
/// versions and platforms, which matters because hashed vectors and IDF
/// tables are stored on disk.
pub fn fnv1a(seed: u8, bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    h ^= seed as u64;
    h = h.wrapping_mul(0x0000_0100_0000_01b3);
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    // final avalanche so that low bits (bucket) and the top bit (sign) are well mixed
    h ^= h >> 33;
    h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
    h ^= h >> 33;
    h
}

/// Replace ligatures, soft hyphens and exotic spaces that PDF extraction
/// produces. Keeps line structure.
pub fn clean_chars(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\u{FB00}' => out.push_str("ff"),
            '\u{FB01}' => out.push_str("fi"),
            '\u{FB02}' => out.push_str("fl"),
            '\u{FB03}' => out.push_str("ffi"),
            '\u{FB04}' => out.push_str("ffl"),
            '\u{00AD}' | '\u{200B}' | '\u{FEFF}' => {}
            '\u{00A0}' | '\u{2002}' | '\u{2003}' | '\u{2009}' | '\u{202F}' => out.push(' '),
            '\u{2010}' | '\u{2011}' | '\u{2012}' | '\u{2212}' => out.push('-'),
            '\r' => {}
            _ => match legacy_char(c) {
                Some(r) => out.push(r),
                None => out.push(c),
            },
        }
    }
    out
}

/// A character that a PDF's text layer gives in a legacy private-use form, as the character it
/// stands for. Thai fonts made for Windows keep the positioned forms of the marks (a tone mark
/// lowered or moved left, ฐ and ญ without their tails) at U+F700..U+F71A, and the Symbol font
/// sits at U+F020..U+F0FF; extracted as they are, they show as boxes and are never found by a
/// search.
pub fn legacy_char(c: char) -> Option<char> {
    const THAI: [char; 27] = [
        '\u{0E10}', '\u{0E34}', '\u{0E35}', '\u{0E36}', '\u{0E37}', '\u{0E48}', '\u{0E49}', '\u{0E4A}', '\u{0E4B}', '\u{0E4C}',
        '\u{0E48}', '\u{0E49}', '\u{0E4A}', '\u{0E4B}', '\u{0E4C}', '\u{0E0D}', '\u{0E31}', '\u{0E4D}', '\u{0E47}', '\u{0E48}',
        '\u{0E49}', '\u{0E4A}', '\u{0E4B}', '\u{0E4C}', '\u{0E38}', '\u{0E39}', '\u{0E3A}',
    ];
    let o = c as u32;
    match o {
        0xF700..=0xF71A => Some(THAI[(o - 0xF700) as usize]),
        // Symbol font: the signs engineering text uses
        0xF0B0 => Some('°'),
        0xF0B1 => Some('±'),
        0xF0A3 => Some('≤'),
        0xF0B3 => Some('≥'),
        0xF0B4 | 0xF0D7 => Some('×'),
        0xF0B8 => Some('÷'),
        0xF0A5 => Some('∞'),
        0xF0D6 => Some('√'),
        0xF0B9 => Some('≠'),
        0xF0BB => Some('≈'),
        0xF0E5 => Some('Σ'),
        0xF061 => Some('α'),
        0xF062 => Some('β'),
        0xF067 => Some('γ'),
        0xF066 => Some('φ'),
        0xF073 => Some('σ'),
        0xF074 => Some('τ'),
        0xF06D => Some('μ'),
        0xF044 => Some('Δ'),
        0xF071 => Some('θ'),
        0xF072 => Some('ρ'),
        0xF065 => Some('ε'),
        0xF06C => Some('λ'),
        0xF070 => Some('π'),
        0xF020 => Some(' '),
        _ => None,
    }
}

/// Whether `s` has characters that [`legacy_char`] repairs.
pub fn has_legacy(s: &str) -> bool {
    s.chars().any(|c| legacy_char(c).is_some())
}

fn is_thai(c: char) -> bool {
    ('\u{0E00}'..='\u{0E7F}').contains(&c)
}

const STOPWORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "been", "by", "for", "from", "has", "have", "if",
    "in", "into", "is", "it", "its", "of", "on", "or", "shall", "should", "such", "than", "that",
    "the", "their", "then", "there", "these", "this", "to", "was", "were", "which", "with", "where",
    "when", "what", "how", "may", "can", "not", "all", "any", "each", "other", "per", "used",
    "use", "using", "do", "does",
];

pub fn is_stopword(w: &str) -> bool {
    STOPWORDS.contains(&w)
}

/// Very light English stemmer: plural folding only. Applied identically to
/// documents and queries, so its crudeness does not create mismatches.
pub fn stem(w: &str) -> String {
    if w.chars().any(|c| c.is_ascii_digit()) || !w.is_ascii() {
        return w.to_string();
    }
    let n = w.len();
    if n > 4 && w.ends_with("ies") {
        return format!("{}y", &w[..n - 3]);
    }
    if n > 3 && w.ends_with('s') && !w.ends_with("ss") && !w.ends_with("us") && !w.ends_with("is") {
        return w[..n - 1].to_string();
    }
    w.to_string()
}

/// Raw lowercase tokens. Letters/digits form tokens; a dot between two
/// digits stays inside the token so clause numbers like `22.5.5.1` survive.
/// Runs of Thai script (no word spaces) become overlapping character bigrams.
pub fn raw_tokens(s: &str) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if is_thai(c) {
            let start = i;
            while i < chars.len() && is_thai(chars[i]) {
                i += 1;
            }
            let run: Vec<char> = chars[start..i].iter().copied().filter(|c| c.is_alphanumeric()).collect();
            if run.len() == 1 {
                out.push(run[0].to_string());
            }
            for w in run.windows(2) {
                out.push(w.iter().collect());
            }
            continue;
        }
        if c.is_alphanumeric() {
            let mut t = String::new();
            while i < chars.len() {
                let d = chars[i];
                if is_thai(d) {
                    break;
                }
                if d.is_alphanumeric() {
                    t.extend(d.to_lowercase());
                    i += 1;
                } else if d == '.'
                    && i + 1 < chars.len()
                    && chars[i + 1].is_ascii_digit()
                    && i > 0
                    && (chars[i - 1].is_ascii_digit() || chars[i - 1].is_ascii_uppercase())
                    && t.len() <= 12
                {
                    t.push('.');
                    i += 1;
                } else {
                    break;
                }
            }
            out.push(t);
            continue;
        }
        i += 1;
    }
    out
}

/// Content terms: stopwords removed, plural-folded. Used by BM25 and the
/// word features of the hashed embedder.
pub fn terms(s: &str) -> Vec<String> {
    raw_tokens(s)
        .into_iter()
        .filter(|t| !is_stopword(t))
        .map(|t| stem(&t))
        .collect()
}

/// Lowercased text with all non-alphanumeric runs collapsed to one space,
/// used for character n-grams.
pub fn squash(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut space = true;
    for c in s.chars() {
        if c.is_alphanumeric() || c == '.' && !space {
            for l in c.to_lowercase() {
                out.push(l);
            }
            space = false;
        } else if !space {
            out.push(' ');
            space = true;
        }
    }
    out.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tokens_keep_clause_numbers() {
        let t = raw_tokens("See 22.5.5.1 and Table 9.3.1.1; Av,min = 0.062 MPa.");
        assert!(t.contains(&"22.5.5.1".to_string()));
        assert!(t.contains(&"9.3.1.1".to_string()));
        assert!(t.contains(&"av".to_string()));
        assert!(t.contains(&"0.062".to_string()));
        assert!(t.contains(&"mpa".to_string()));
    }
    /// Positioned Thai marks from Windows fonts become the real marks: "ค่าอัตราส่วน ...
    /// ไม่น้อยกว่า" as a 2566 foundation regulation's PDF gives it.
    #[test]
    fn legacy_thai_marks() {
        let pdf = "ค\u{F70A}าอัตราส\u{F70A}วน ไม\u{F70A}น\u{F70B}อยกว\u{F70A}า 3.00 \u{F0B0}C";
        assert_eq!(clean_chars(pdf), "ค่าอัตราส่วน ไม่น้อยกว่า 3.00 °C");
        assert!(has_legacy(pdf) && !has_legacy(&clean_chars(pdf)));
        assert_eq!(legacy_char('\u{F700}'), Some('ฐ'));
        assert_eq!(legacy_char('\u{F71A}'), Some('\u{0E3A}'));
        assert_eq!(legacy_char('ก'), None);
    }
    #[test]
    fn thai_bigrams() {
        let t = raw_tokens("ข้อ 3.2 ฐานราก");
        assert!(t.contains(&"3.2".to_string()));
        assert!(t.len() >= 3);
    }
    #[test]
    fn stemming() {
        assert_eq!(stem("factors"), "factor");
        assert_eq!(stem("properties"), "property");
        assert_eq!(stem("stress"), "stress");
    }
}
