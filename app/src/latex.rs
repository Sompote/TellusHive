//! LaTeX math in the chat replies, shown as text: the models write `$\phi M_n \ge M_u$` and
//! `$$\frac{w L^2}{8}$$`, which the chat turns into "ϕMₙ ≥ Mᵤ" (with real raised and lowered
//! text) and "wL²/8". Symbols become their Unicode characters, fractions a/b (with brackets
//! where needed), roots √, accents combining marks; commands it does not know show their name.

/// Where a piece of the formula sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Script {
    Normal,
    Sup,
    Sub,
}

/// A run of text at one position.
pub type Piece = (String, Script);

/// The formula as pieces of text.
pub fn render(src: &str) -> Vec<Piece> {
    let mut p = Parser { c: src.chars().collect(), i: 0 };
    let mut out = vec![];
    p.seq(&mut out, Script::Normal, false);
    tidy(out)
}

/// The formula as plain text (super- and subscripts written inline), for measuring and tests.
#[cfg(test)]
pub fn plain(src: &str) -> String {
    render(src).into_iter().map(|(s, k)| if k == Script::Normal { s } else { format!("{}{s}", if k == Script::Sup { "^" } else { "_" }) }).collect()
}

/// A `$…$` math span in `chars` starting at `k`: (formula, index after it, display). The
/// closing dollar must follow a non-space and not be followed by a digit, and the formula must
/// not start with a space, so prices ("$5 and $10") stay text.
pub fn span_at(chars: &[char], k: usize) -> Option<(String, usize, bool)> {
    let at = |i: usize| chars.get(i).copied();
    match (at(k), at(k + 1)) {
        (Some('$'), Some('$')) => {
            let end = (k + 2..chars.len().saturating_sub(1)).find(|&i| chars[i] == '$' && chars[i + 1] == '$' && chars[i - 1] != '\\')?;
            let body: String = chars[k + 2..end].iter().collect();
            (!body.trim().is_empty()).then(|| (body.trim().to_string(), end + 2, true))
        }
        (Some('$'), Some(n)) if !n.is_whitespace() => {
            let end = (k + 1..chars.len()).find(|&i| chars[i] == '$' && chars[i - 1] != '\\')?;
            if end == k + 1 || chars[end - 1].is_whitespace() || at(end + 1).is_some_and(|c| c.is_ascii_digit()) {
                return None;
            }
            Some((chars[k + 1..end].iter().collect(), end + 1, false))
        }
        (Some('\\'), Some(open @ ('(' | '['))) => {
            let close = if open == '(' { ')' } else { ']' };
            let end = (k + 2..chars.len().saturating_sub(1)).find(|&i| chars[i] == '\\' && chars[i + 1] == close)?;
            let body: String = chars[k + 2..end].iter().collect();
            (!body.trim().is_empty()).then(|| (body.trim().to_string(), end + 2, open == '['))
        }
        _ => None,
    }
}

/// A display formula that opens on this line (`$$` or `\[` alone or with the start of the
/// formula): the closing mark it waits for.
pub fn display_open(line: &str) -> Option<&'static str> {
    let t = line.trim();
    if t.starts_with("$$") && !(t.len() > 4 && t.ends_with("$$")) {
        return Some("$$");
    }
    if t.starts_with("\\[") && !t.ends_with("\\]") {
        return Some("\\]");
    }
    None
}

/// A display formula on one line (`$$…$$` or `\[…\]`, nothing else on it).
pub fn display_line(line: &str) -> Option<&str> {
    let t = line.trim();
    let body = t.strip_prefix("$$").and_then(|r| r.strip_suffix("$$")).or_else(|| t.strip_prefix("\\[").and_then(|r| r.strip_suffix("\\]")))?;
    (!body.trim().is_empty() && !body.contains("$$")).then(|| body.trim())
}

struct Parser {
    c: Vec<char>,
    i: usize,
}

fn push(out: &mut Vec<Piece>, s: &str, k: Script) {
    if s.is_empty() {
        return;
    }
    match out.last_mut() {
        Some((t, last)) if *last == k => t.push_str(s),
        _ => out.push((s.to_string(), k)),
    }
}

/// A relation or binary operator, spaced on the line and tight in a script.
fn op(out: &mut Vec<Piece>, s: &str, k: Script) {
    if k == Script::Normal {
        push(out, &format!(" {s} "), k);
    } else {
        push(out, s, k);
    }
}

/// Whether a + or − here is binary (after a term) rather than a sign.
fn after_term(out: &[Piece]) -> bool {
    out.last().and_then(|(s, _)| s.trim_end().chars().last()).is_some_and(|c| c.is_alphanumeric() || matches!(c, ')' | ']' | '}' | '′' | '|' | '°' | '∞' | '%' | '⟩'))
}

/// The symbols set as operators (spaced on both sides).
fn spaced(s: &str) -> bool {
    matches!(s, "×" | "·" | "÷" | "±" | "∓" | "≤" | "≥" | "≪" | "≫" | "≠" | "≈" | "∼" | "≃" | "≅" | "≡" | "∝" | "→" | "←" | "⇒" | "⇐" | "↔" | "⇔" | "↦" | "∈" | "∉" | "⊂" | "⊆" | "∪" | "∩" | "∧" | "∨" | "∴" | "∵" | "∗" | "⋆")
}

fn text_of(p: &[Piece]) -> String {
    p.iter().map(|(s, _)| s.as_str()).collect()
}

/// Joins runs at the same position and collapses repeated spaces.
fn tidy(p: Vec<Piece>) -> Vec<Piece> {
    let mut out: Vec<Piece> = vec![];
    for (s, k) in p {
        let mut t = String::new();
        for ch in s.chars() {
            if ch == ' ' && (t.ends_with(' ') || (t.is_empty() && out.last().is_some_and(|(x, _)| x.ends_with(' ')))) {
                continue;
            }
            t.push(ch);
        }
        push(&mut out, &t, k);
    }
    if let Some((s, _)) = out.first_mut() {
        *s = s.trim_start().to_string();
    }
    if let Some((s, _)) = out.last_mut() {
        *s = s.trim_end().to_string();
    }
    out.retain(|(s, _)| !s.is_empty());
    out
}

/// A fraction's part in brackets when needed: a numerator that is a sum or holds an operator,
/// a denominator of more than one symbol (a/(bd²), not a/bd²); plain numbers never.
fn wrap(p: Vec<Piece>, denominator: bool) -> Vec<Piece> {
    let t = text_of(&p);
    let t = t.trim();
    let visible = t.chars().filter(|c| !c.is_whitespace()).count();
    let number = t.chars().all(|c| c.is_ascii_digit() || c == '.' || c == ',');
    let has_op = t.chars().any(|c| matches!(c, ' ' | '+' | '−' | '-' | '/' | '±' | '·' | '×'));
    let simple = visible <= 1 || number || (t.starts_with('(') && t.ends_with(')')) || if denominator { false } else { !has_op };
    if simple {
        return p;
    }
    let mut out = vec![("(".to_string(), Script::Normal)];
    out.extend(p);
    out.push((")".to_string(), Script::Normal));
    out
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.c.get(self.i).copied()
    }

    /// Runs until the end, or a `}` when `in_group`.
    fn seq(&mut self, out: &mut Vec<Piece>, k: Script, in_group: bool) {
        while let Some(ch) = self.peek() {
            if ch == '}' {
                if in_group {
                    self.i += 1;
                    return;
                }
                self.i += 1;
                continue;
            }
            self.atom(out, k);
        }
    }

    /// `{…}` or one token, as pieces at position `k`.
    fn arg(&mut self, k: Script) -> Vec<Piece> {
        while self.peek() == Some(' ') {
            self.i += 1;
        }
        let mut out = vec![];
        match self.peek() {
            Some('{') => {
                self.i += 1;
                self.seq(&mut out, k, true);
            }
            Some(_) => self.atom(&mut out, k),
            None => {}
        }
        out
    }

    /// `{…}` as raw text (for \text and the like).
    fn raw_arg(&mut self) -> String {
        while self.peek() == Some(' ') {
            self.i += 1;
        }
        if self.peek() != Some('{') {
            return self.peek().map(|c| {
                self.i += 1;
                c.to_string()
            }).unwrap_or_default();
        }
        self.i += 1;
        let (mut depth, mut s) = (1, String::new());
        while let Some(c) = self.peek() {
            self.i += 1;
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            if c == '\\' {
                // \{ \} \% \& \_ \$ \# keep their character; \, and other spaces a space
                if let Some(n) = self.peek() {
                    self.i += 1;
                    s.push(if matches!(n, ',' | ';' | ':' | '!' | ' ') { ' ' } else { n });
                }
                continue;
            }
            s.push(c);
        }
        s
    }

    /// `[…]` if there is one (an optional argument).
    fn opt_arg(&mut self) -> Option<Vec<Piece>> {
        if self.peek() != Some('[') {
            return None;
        }
        self.i += 1;
        let start = self.i;
        let mut depth = 1;
        while let Some(c) = self.peek() {
            match c {
                '[' => depth += 1,
                ']' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            self.i += 1;
        }
        let inner: String = self.c[start..self.i].iter().collect();
        self.i += 1;
        Some(render(&inner))
    }

    fn script(&mut self, out: &mut Vec<Piece>, k: Script, sup: bool) {
        let at = if k != Script::Normal { k } else if sup { Script::Sup } else { Script::Sub };
        let p = self.arg(at);
        let t = text_of(&p);
        // a degree sign and primes sit on the line in their own glyphs
        if sup && matches!(t.trim(), "∘" | "°") {
            push(out, "°", k);
            return;
        }
        if sup && !t.is_empty() && t.chars().all(|c| matches!(c, '′' | '\'')) {
            push(out, &"′".repeat(t.chars().count()), k);
            return;
        }
        for (s, _) in p {
            push(out, &s, at);
        }
    }

    fn atom(&mut self, out: &mut Vec<Piece>, k: Script) {
        let Some(ch) = self.peek() else { return };
        self.i += 1;
        match ch {
            '\\' => self.command(out, k),
            '{' => {
                let mut inner = vec![];
                self.seq(&mut inner, k, true);
                for (s, kk) in inner {
                    push(out, &s, kk);
                }
            }
            '^' => self.script(out, k, true),
            '_' => self.script(out, k, false),
            // spaces in a formula are LaTeX's to set, as here: around operators and relations
            ' ' | '\n' | '\t' | '\r' => {}
            '=' | '<' | '>' => op(out, &ch.to_string(), k),
            '+' | '-' => {
                let s = if ch == '-' { "−" } else { "+" };
                if after_term(out) {
                    op(out, s, k)
                } else {
                    push(out, s, k)
                }
            }
            ',' => push(out, if k == Script::Normal && !self.peek().is_some_and(|c| c.is_ascii_digit()) { ", " } else { "," }, k),
            '\'' => push(out, "′", k),
            '~' | '&' => push(out, " ", k),
            c => push(out, &c.to_string(), k),
        }
    }

    fn command(&mut self, out: &mut Vec<Piece>, k: Script) {
        let Some(first) = self.peek() else { return };
        // control symbols: \, \; \{ \% \\ …
        if !first.is_ascii_alphabetic() {
            self.i += 1;
            match first {
                ',' | ';' | ':' | ' ' | '>' => push(out, " ", k),
                '!' => {}
                '\\' => push(out, "\n", k),
                '|' => push(out, "‖", k),
                c => push(out, &c.to_string(), k),
            }
            return;
        }
        let start = self.i;
        while self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
            self.i += 1;
        }
        let name: String = self.c[start..self.i].iter().collect();
        let skip_space = |p: &mut Parser| {
            while p.peek() == Some(' ') {
                p.i += 1;
            }
        };
        match name.as_str() {
            // text and fonts
            "text" | "textrm" | "textnormal" | "mbox" | "textit" | "textbf" | "texttt" | "textsf" | "emph" => {
                let s = self.raw_arg();
                push(out, &s, k);
            }
            "mathrm" | "mathbf" | "mathit" | "mathsf" | "mathtt" | "boldsymbol" | "bm" | "mathcal" | "mathscr" | "operatorname" | "mathfrak" => {
                for (s, kk) in self.arg(k) {
                    push(out, &s, kk);
                }
            }
            "mathbb" => {
                let s = self.raw_arg();
                let m: String = s.chars().map(|c| match c {
                    'R' => 'ℝ',
                    'N' => 'ℕ',
                    'Z' => 'ℤ',
                    'Q' => 'ℚ',
                    'C' => 'ℂ',
                    o => o,
                }).collect();
                push(out, &m, k);
            }
            // fractions, roots, binomials
            "frac" | "dfrac" | "tfrac" | "cfrac" => {
                let a = wrap(self.arg(k), false);
                let b = wrap(self.arg(k), true);
                for (s, kk) in a {
                    push(out, &s, kk);
                }
                push(out, "/", k);
                for (s, kk) in b {
                    push(out, &s, kk);
                }
            }
            "binom" | "dbinom" => {
                let (a, b) = (text_of(&self.arg(k)), text_of(&self.arg(k)));
                push(out, &format!("C({a}, {b})"), k);
            }
            "sqrt" => {
                let n = self.opt_arg();
                if let Some(n) = n {
                    for (s, _) in n {
                        push(out, &s, Script::Sup);
                    }
                }
                push(out, "√", k);
                let x = self.arg(k);
                let long = text_of(&x).chars().filter(|c| !c.is_whitespace()).count() > 1;
                if long {
                    push(out, "(", k);
                }
                for (s, kk) in x {
                    push(out, &s, kk);
                }
                if long {
                    push(out, ")", k);
                }
            }
            // accents: a combining mark after each character
            "bar" | "overline" | "hat" | "widehat" | "vec" | "overrightarrow" | "dot" | "ddot" | "tilde" | "widetilde" | "underline" => {
                let mark = match name.as_str() {
                    "bar" | "overline" => '\u{0305}',
                    "hat" | "widehat" => '\u{0302}',
                    "vec" | "overrightarrow" => '\u{20D7}',
                    "dot" => '\u{0307}',
                    "ddot" => '\u{0308}',
                    "tilde" | "widetilde" => '\u{0303}',
                    _ => '\u{0332}',
                };
                for (s, kk) in self.arg(k) {
                    let m: String = s.chars().flat_map(|c| if c.is_whitespace() { vec![c] } else { vec![c, mark] }).collect();
                    push(out, &m, kk);
                }
            }
            // units (siunitx)
            "SI" | "qty" => {
                let n = self.raw_arg();
                push(out, &n, k);
                push(out, " ", k);
                for (s, kk) in render(&self.raw_arg()) {
                    push(out, &s, if k == Script::Normal { kk } else { k });
                }
            }
            "si" | "unit" | "num" => {
                for (s, kk) in render(&self.raw_arg()) {
                    push(out, &s, if k == Script::Normal { kk } else { k });
                }
            }
            "tag" => {
                let s = self.raw_arg();
                push(out, &format!("   ({s})"), k);
            }
            // layout commands with nothing to show
            "left" | "right" | "big" | "Big" | "bigg" | "Bigg" | "bigl" | "bigr" | "Bigl" | "Bigr" | "biggl" | "biggr" | "middle" => {
                skip_space(self);
                // \left. and \right. are invisible
                if self.peek() == Some('.') {
                    self.i += 1;
                }
            }
            "displaystyle" | "textstyle" | "scriptstyle" | "limits" | "nolimits" | "nonumber" | "notag" | "centering" | "noindent" => skip_space(self),
            "begin" | "end" => {
                let env = self.raw_arg();
                if name == "begin" && env.starts_with("array") {
                    // the column spec {lcr}
                    let _ = self.raw_arg();
                }
                if name == "end" {
                    push(out, " ", k);
                }
            }
            "label" | "hspace" | "vspace" | "phantom" | "hphantom" | "vphantom" => {
                let _ = self.raw_arg();
            }
            "quad" => push(out, "  ", k),
            "qquad" => push(out, "    ", k),
            _ => match symbol(&name) {
                Some(s) if spaced(s) => op(out, s, k),
                Some(s) => push(out, s, k),
                // a function or unknown command shows its name (\max, \sin, \foo)
                None => push(out, &name, k),
            },
        }
    }
}

/// A symbol command's character.
fn symbol(name: &str) -> Option<&'static str> {
    let greek = match name {
        "alpha" => "α",
        "beta" => "β",
        "gamma" => "γ",
        "delta" => "δ",
        "epsilon" => "ϵ",
        "varepsilon" => "ε",
        "zeta" => "ζ",
        "eta" => "η",
        "theta" => "θ",
        "vartheta" => "ϑ",
        "iota" => "ι",
        "kappa" => "κ",
        "lambda" => "λ",
        "mu" => "μ",
        "nu" => "ν",
        "xi" => "ξ",
        "pi" => "π",
        "varpi" => "ϖ",
        "rho" => "ρ",
        "varrho" => "ϱ",
        "sigma" => "σ",
        "varsigma" => "ς",
        "tau" => "τ",
        "upsilon" => "υ",
        "phi" => "ϕ",
        "varphi" => "φ",
        "chi" => "χ",
        "psi" => "ψ",
        "omega" => "ω",
        "Gamma" => "Γ",
        "Delta" => "Δ",
        "Theta" => "Θ",
        "Lambda" => "Λ",
        "Xi" => "Ξ",
        "Pi" => "Π",
        "Sigma" => "Σ",
        "Upsilon" => "Υ",
        "Phi" => "Φ",
        "Psi" => "Ψ",
        "Omega" => "Ω",
        "ell" => "ℓ",
        "hbar" => "ℏ",
        "partial" => "∂",
        "nabla" => "∇",
        "infty" => "∞",
        "prime" => "′",
        "degree" => "°",
        "circ" => "∘",
        "AA" => "Å",
        _ => "",
    };
    if !greek.is_empty() {
        return Some(greek);
    }
    let op = match name {
        "times" => "×",
        "cdot" => "·",
        "div" => "÷",
        "pm" => "±",
        "mp" => "∓",
        "ast" => "∗",
        "star" => "⋆",
        "bullet" => "•",
        "le" | "leq" | "leqslant" => "≤",
        "ge" | "geq" | "geqslant" => "≥",
        "ll" => "≪",
        "gg" => "≫",
        "ne" | "neq" => "≠",
        "approx" => "≈",
        "sim" => "∼",
        "simeq" => "≃",
        "cong" => "≅",
        "equiv" => "≡",
        "propto" => "∝",
        "to" | "rightarrow" => "→",
        "leftarrow" | "gets" => "←",
        "Rightarrow" | "implies" => "⇒",
        "Leftarrow" => "⇐",
        "leftrightarrow" => "↔",
        "Leftrightarrow" | "iff" => "⇔",
        "uparrow" => "↑",
        "downarrow" => "↓",
        "mapsto" => "↦",
        "sum" => "Σ",
        "prod" => "∏",
        "int" => "∫",
        "iint" => "∬",
        "oint" => "∮",
        "in" => "∈",
        "notin" => "∉",
        "subset" => "⊂",
        "subseteq" => "⊆",
        "cup" => "∪",
        "cap" => "∩",
        "emptyset" | "varnothing" => "∅",
        "forall" => "∀",
        "exists" => "∃",
        "neg" | "lnot" => "¬",
        "land" | "wedge" => "∧",
        "lor" | "vee" => "∨",
        "angle" => "∠",
        "perp" => "⊥",
        "parallel" => "∥",
        "mid" => "|",
        "therefore" => "∴",
        "because" => "∵",
        "cdots" | "dots" | "ldots" | "dotsc" | "dotsb" => "…",
        "vdots" => "⋮",
        "ddots" => "⋱",
        "langle" => "⟨",
        "rangle" => "⟩",
        "lceil" => "⌈",
        "rceil" => "⌉",
        "lfloor" => "⌊",
        "rfloor" => "⌋",
        "lbrace" => "{",
        "rbrace" => "}",
        "vert" | "lvert" | "rvert" => "|",
        "Vert" | "lVert" | "rVert" => "‖",
        "backslash" => "\\",
        "percent" => "%",
        "colon" => ":",
        "checkmark" => "✓",
        _ => "",
    };
    (!op.is_empty()).then_some(op)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engineering_formulas() {
        assert_eq!(plain(r"H = 3\text{ m}"), "H = 3 m");
        assert_eq!(plain(r"\phi M_n \ge M_u"), "ϕM_n ≥ M_u");
        assert_eq!(plain(r"\frac{w L^2}{8}"), "wL^2/8");
        assert_eq!(plain(r"M_u = \frac{1.2 D + 1.6 L}{\phi b d^2}"), "M_u = (1.2D + 1.6L)/(ϕbd^2)");
        assert_eq!(plain(r"\frac{M}{12}"), "M/12");
        assert_eq!(plain(r"x = -b \pm 2"), "x = −b ± 2");
        assert_eq!(plain(r"1,000 + a,b"), "1,000 + a, b");
        assert_eq!(plain(r"\sqrt{f'_c}"), "√(f′_c)");
        assert_eq!(plain(r"\sqrt{2}"), "√2");
        assert_eq!(plain(r"\theta = 30^\circ"), "θ = 30°");
        assert_eq!(plain(r"\theta = 30^{\circ}"), "θ = 30°");
        assert_eq!(plain(r"FS = \dfrac{c_u N_c}{\gamma H} \approx 1.5"), "FS = c_uN_c/(γH) ≈ 1.5");
        assert_eq!(plain(r"s_u = 20\,\text{kPa}"), "s_u = 20 kPa");
        assert_eq!(plain(r"\gamma = 16~\mathrm{kN/m^3}"), "γ = 16 kN/m^3");
        assert_eq!(plain(r"\varepsilon_t \geq 0.004"), "ε_t ≥ 0.004");
        assert_eq!(plain(r"\left( \frac{a}{b} \right)"), "(a/b)");
        assert_eq!(plain(r"\SI{20}{kPa}"), "20 kPa");
        assert_eq!(plain(r"\bar{x}"), "x\u{0305}");
        assert_eq!(plain(r"\sigma'_v = \gamma' z"), "σ′_v = γ′z");
        assert_eq!(plain(r"\max(a, b)"), "max(a, b)");
        assert_eq!(plain(r"x^{-1}"), "x^−1");
        assert_eq!(plain(r"E_{s,\text{eff}}"), "E_s,eff");
    }

    #[test]
    fn spans() {
        let ch = |s: &str| s.chars().collect::<Vec<_>>();
        assert_eq!(span_at(&ch("$x^2$ m"), 0), Some(("x^2".into(), 5, false)));
        assert_eq!(span_at(&ch("$$a=b$$"), 0), Some(("a=b".into(), 7, true)));
        assert_eq!(span_at(&ch(r"\(a\) b"), 0), Some(("a".into(), 5, false)));
        // prices are not formulas
        assert_eq!(span_at(&ch("$5 and $10"), 0), None);
        assert_eq!(span_at(&ch("$ 5 $"), 0), None);
        assert_eq!(display_line("$$ \\frac{a}{b} $$"), Some("\\frac{a}{b}"));
        assert_eq!(display_line("\\[x\\]"), Some("x"));
        assert_eq!(display_open("$$"), Some("$$"));
        assert_eq!(display_open("\\["), Some("\\]"));
        assert_eq!(display_open("$$x$$"), None);
    }
}
