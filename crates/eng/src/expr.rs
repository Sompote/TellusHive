//! A small, hand-written expression evaluator (recursive descent).
//!
//! Grammar, lowest precedence first:
//!
//! ```text
//! expr    := term (('+' | '-') term)*
//! term    := unary (('*' | '/') unary)*
//! unary   := ('-' | '+') unary | power
//! power   := primary (('^' | '**') unary)?        right-associative, so 2^3^2 = 2^9
//! primary := number | name | name '(' args ')' | '(' expr ')'
//! ```
//!
//! Unary minus binds looser than `^`, so `-2^2 = -4` as in ordinary mathematics.
//! Variables come from the caller's map and take priority over the constants `pi` and `e`,
//! because engineers routinely call an eccentricity `e`.

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Name(String),
    Op(char), // + - * / ^ ( ) ,
}

/// Splits the source into tokens. `**` becomes `^`.
fn lex(src: &str) -> Result<Vec<Tok>, String> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit() || (c == '.' && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            let end = scan_number(&chars, i);
            let text: String = chars[i..end].iter().collect();
            let v: f64 = text.parse().map_err(|_| format!("bad number '{text}'"))?;
            out.push(Tok::Num(v));
            i = end;
        } else if c.is_alphabetic() || c == '_' {
            let end = scan_name(&chars, i);
            out.push(Tok::Name(chars[i..end].iter().collect()));
            i = end;
        } else if c == '*' && chars.get(i + 1) == Some(&'*') {
            out.push(Tok::Op('^'));
            i += 2;
        } else if "+-*/^(),".contains(c) {
            out.push(Tok::Op(c));
            i += 1;
        } else {
            return Err(format!("unexpected character '{c}' at position {i}"));
        }
    }
    Ok(out)
}

/// End index of a number starting at `i`: digits, one decimal point, optional exponent.
fn scan_number(chars: &[char], mut i: usize) -> usize {
    while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
        i += 1;
    }
    // exponent only if followed by digits (optionally signed), so "2e" is 2 then name e
    if i < chars.len() && (chars[i] == 'e' || chars[i] == 'E') {
        let mut j = i + 1;
        if j < chars.len() && (chars[j] == '+' || chars[j] == '-') {
            j += 1;
        }
        if j < chars.len() && chars[j].is_ascii_digit() {
            while j < chars.len() && chars[j].is_ascii_digit() {
                j += 1;
            }
            i = j;
        }
    }
    i
}

fn scan_name(chars: &[char], mut i: usize) -> usize {
    while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
        i += 1;
    }
    i
}

struct Parser<'a> {
    toks: Vec<Tok>,
    pos: usize,
    vars: &'a BTreeMap<String, f64>,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }
    fn eat_op(&mut self, c: char) -> bool {
        if self.peek() == Some(&Tok::Op(c)) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn expr(&mut self) -> Result<f64, String> {
        let mut v = self.term()?;
        loop {
            if self.eat_op('+') {
                v += self.term()?;
            } else if self.eat_op('-') {
                v -= self.term()?;
            } else {
                return Ok(v);
            }
        }
    }
    fn term(&mut self) -> Result<f64, String> {
        let mut v = self.unary()?;
        loop {
            if self.eat_op('*') {
                v *= self.unary()?;
            } else if self.eat_op('/') {
                let d = self.unary()?;
                if d == 0.0 {
                    return Err("division by zero".into());
                }
                v /= d;
            } else {
                return Ok(v);
            }
        }
    }
    fn unary(&mut self) -> Result<f64, String> {
        if self.eat_op('-') {
            return Ok(-self.unary()?);
        }
        if self.eat_op('+') {
            return self.unary();
        }
        self.power()
    }
    fn power(&mut self) -> Result<f64, String> {
        let base = self.primary()?;
        if self.eat_op('^') {
            let exp = self.unary()?; // right-associative through unary -> power
            let v = base.powf(exp);
            if !v.is_finite() {
                return Err(format!("{base}^{exp} is not a real number"));
            }
            return Ok(v);
        }
        Ok(base)
    }
    fn primary(&mut self) -> Result<f64, String> {
        match self.peek().cloned() {
            Some(Tok::Num(v)) => {
                self.pos += 1;
                Ok(v)
            }
            Some(Tok::Op('(')) => {
                self.pos += 1;
                let v = self.expr()?;
                if !self.eat_op(')') {
                    return Err("missing ')'".into());
                }
                Ok(v)
            }
            Some(Tok::Name(name)) => {
                self.pos += 1;
                if self.eat_op('(') {
                    let mut args = Vec::new();
                    if !self.eat_op(')') {
                        loop {
                            args.push(self.expr()?);
                            if self.eat_op(',') {
                                continue;
                            }
                            if self.eat_op(')') {
                                break;
                            }
                            return Err(format!("expected ',' or ')' in call to {name}"));
                        }
                    }
                    call(&name, &args)
                } else if let Some(v) = self.vars.get(&name) {
                    Ok(*v)
                } else if name == "pi" {
                    Ok(std::f64::consts::PI)
                } else if name == "e" {
                    Ok(std::f64::consts::E)
                } else {
                    Err(format!("unknown variable '{name}'"))
                }
            }
            Some(Tok::Op(c)) => Err(format!("unexpected '{c}'")),
            None => Err("unexpected end of expression".into()),
        }
    }
}

/// Built-in functions. Trigonometry is in radians except the `*d` variants (degrees).
fn call(name: &str, a: &[f64]) -> Result<f64, String> {
    let one = |f: fn(f64) -> f64| -> Result<f64, String> {
        if a.len() != 1 {
            return Err(format!("{name}() takes 1 argument, got {}", a.len()));
        }
        Ok(f(a[0]))
    };
    let v = match name {
        "sqrt" => {
            if a.len() == 1 && a[0] < 0.0 {
                return Err(format!("sqrt of negative number {}", a[0]));
            }
            one(f64::sqrt)?
        }
        "abs" => one(f64::abs)?,
        "exp" => one(f64::exp)?,
        "ln" | "log10" => {
            if a.len() == 1 && a[0] <= 0.0 {
                return Err(format!("{name} of non-positive number {}", a[0]));
            }
            if name == "ln" {
                one(f64::ln)?
            } else {
                one(f64::log10)?
            }
        }
        "sin" => one(f64::sin)?,
        "cos" => one(f64::cos)?,
        "tan" => one(f64::tan)?,
        "asin" => one(f64::asin)?,
        "acos" => one(f64::acos)?,
        "atan" => one(f64::atan)?,
        "sind" => one(|x| x.to_radians().sin())?,
        "cosd" => one(|x| x.to_radians().cos())?,
        "tand" => one(|x| x.to_radians().tan())?,
        "floor" => one(f64::floor)?,
        "ceil" => one(f64::ceil)?,
        "round" => one(f64::round)?,
        "min" | "max" => {
            if a.is_empty() {
                return Err(format!("{name}() needs at least 1 argument"));
            }
            let mut v = a[0];
            for x in &a[1..] {
                v = if name == "min" { v.min(*x) } else { v.max(*x) };
            }
            v
        }
        _ => return Err(format!("unknown function '{name}'")),
    };
    if v.is_finite() {
        Ok(v)
    } else {
        Err(format!("{name}({}) is not a real number", a.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ")))
    }
}

/// Evaluates `src` with the given variables.
pub fn eval(src: &str, vars: &BTreeMap<String, f64>) -> Result<f64, String> {
    let toks = lex(src)?;
    if toks.is_empty() {
        return Err("empty expression".into());
    }
    let mut p = Parser { toks, pos: 0, vars };
    let v = p.expr()?;
    if p.pos != p.toks.len() {
        return Err(format!("unexpected {:?} after the end of the expression", p.toks[p.pos]));
    }
    if !v.is_finite() {
        return Err("result is not a finite number".into());
    }
    Ok(v)
}

/// Formats a value to `sig` significant figures, without trailing zeros, switching to
/// scientific notation for very large or very small magnitudes.
pub fn fmt_sig(v: f64, sig: usize) -> String {
    if !v.is_finite() {
        return format!("{v}");
    }
    if v == 0.0 {
        return "0".into();
    }
    let sig = sig.max(1);
    let mag = v.abs().log10().floor() as i32;
    if !(-4..9).contains(&mag) {
        let s = format!("{:.*e}", sig - 1, v);
        // tidy "1.200e5" -> "1.2e5"
        if let Some((m, e)) = s.split_once('e') {
            let m = if m.contains('.') { m.trim_end_matches('0').trim_end_matches('.') } else { m };
            return format!("{m}e{e}");
        }
        return s;
    }
    let decimals = (sig as i32 - 1 - mag).max(0) as usize;
    // round to the significant figures first, also for large numbers (12345 -> 12350)
    let factor = 10f64.powi(sig as i32 - 1 - mag);
    let r = (v * factor).round() / factor;
    let s = format!("{r:.decimals$}");
    let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s };
    if s == "-0" {
        "0".into()
    } else {
        s
    }
}

/// Four significant figures, the precision used to show workings.
pub fn fmt4(v: f64) -> String {
    fmt_sig(v, 4)
}

/// Returns `src` with every known variable replaced by its value to 4 significant figures,
/// e.g. `0.85*f_c*b*(h-50)` -> `0.85*28*300*(450-50)`. Function names, numbers and unknown
/// names are left as written. Negative values are bracketed so the workings stay correct.
pub fn substitute(src: &str, vars: &BTreeMap<String, f64>) -> String {
    let chars: Vec<char> = src.chars().collect();
    let mut out = String::with_capacity(src.len() + 16);
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_ascii_digit() || (c == '.' && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            let end = scan_number(&chars, i);
            out.extend(&chars[i..end]);
            i = end;
        } else if c.is_alphabetic() || c == '_' {
            let end = scan_name(&chars, i);
            let name: String = chars[i..end].iter().collect();
            // a name followed by '(' is a function call
            let mut j = end;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            let is_call = chars.get(j) == Some(&'(');
            match vars.get(&name) {
                Some(v) if !is_call => {
                    let s = fmt4(*v);
                    if *v < 0.0 {
                        out.push('(');
                        out.push_str(&s);
                        out.push(')');
                    } else {
                        out.push_str(&s);
                    }
                }
                _ => out.push_str(&name),
            }
            i = end;
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

/// True if `s` is a valid variable name for the evaluator.
pub fn is_identifier(s: &str) -> bool {
    let mut it = s.chars();
    match it.next() {
        Some(c) if c.is_alphabetic() || c == '_' => it.all(|c| c.is_alphanumeric() || c == '_'),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(s: &str) -> f64 {
        eval(s, &BTreeMap::new()).unwrap()
    }

    #[test]
    fn precedence_and_associativity() {
        assert_eq!(ev("1+2*3"), 7.0);
        assert_eq!(ev("(1+2)*3"), 9.0);
        assert_eq!(ev("2^3^2"), 512.0);
        assert_eq!(ev("-2^2"), -4.0);
        assert_eq!(ev("2^-1"), 0.5);
        assert_eq!(ev("2**3"), 8.0);
        assert_eq!(ev("10-4-3"), 3.0);
        assert_eq!(ev("8/4/2"), 1.0);
        assert_eq!(ev("--3"), 3.0);
        assert_eq!(ev("1e-3*1000"), 1.0);
        assert_eq!(ev("2.5E2"), 250.0);
        assert_eq!(ev(".5+.5"), 1.0);
    }

    #[test]
    fn functions_and_constants() {
        assert!((ev("sqrt(28)") - 5.2915).abs() < 1e-4);
        assert!((ev("sind(30)") - 0.5).abs() < 1e-12);
        assert!((ev("tand(45)") - 1.0).abs() < 1e-12);
        assert!((ev("cos(pi)") + 1.0).abs() < 1e-12);
        assert!((ev("ln(e)") - 1.0).abs() < 1e-12);
        assert_eq!(ev("log10(1000)"), 3.0);
        assert_eq!(ev("max(1, 5, 3)"), 5.0);
        assert_eq!(ev("min(4)"), 4.0);
        assert_eq!(ev("floor(2.7)+ceil(2.1)+round(2.5)"), 8.0);
        assert!((ev("atan(1)*4") - std::f64::consts::PI).abs() < 1e-12);
    }

    #[test]
    fn variables_shadow_constants() {
        let mut v = BTreeMap::new();
        v.insert("e".to_string(), 0.25);
        v.insert("f_c".to_string(), 28.0);
        v.insert("phi_Mn".to_string(), 100.0);
        assert_eq!(eval("e*4", &v).unwrap(), 1.0);
        assert_eq!(eval("phi_Mn/f_c*28", &v).unwrap(), 100.0);
    }

    #[test]
    fn errors() {
        let v = BTreeMap::new();
        assert!(eval("1/0", &v).unwrap_err().contains("division"));
        assert!(eval("sqrt(-1)", &v).is_err());
        assert!(eval("x+1", &v).unwrap_err().contains("'x'"));
        assert!(eval("(1+2", &v).is_err());
        assert!(eval("1+", &v).is_err());
        assert!(eval("1 2", &v).is_err());
        assert!(eval("foo(1)", &v).is_err());
        assert!(eval("", &v).is_err());
        assert!(eval("3 # 4", &v).is_err());
        assert!(eval("(-8)^0.5", &v).is_err());
    }

    #[test]
    fn substitution() {
        let mut v = BTreeMap::new();
        v.insert("f_c".to_string(), 28.0);
        v.insert("b".to_string(), 300.0);
        v.insert("h".to_string(), 450.0);
        v.insert("x".to_string(), -2.5);
        v.insert("e".to_string(), 0.123456);
        assert_eq!(substitute("0.85*f_c*b*(h-50)", &v), "0.85*28*300*(450-50)");
        assert_eq!(substitute("sqrt(f_c) + x", &v), "sqrt(28) + (-2.5)");
        assert_eq!(substitute("1e-3*e", &v), "1e-3*0.1235");
        assert_eq!(substitute("unknown*2", &v), "unknown*2");
        assert_eq!(fmt4(12345.0), "12350");
        assert_eq!(fmt4(0.0021), "0.0021");
        assert_eq!(fmt4(146.876), "146.9");
        assert_eq!(fmt4(2.0e-7), "2e-7");
    }
}
