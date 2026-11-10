//! Skills: folders holding a `SKILL.md` (YAML-style front matter with `name` and
//! `description`, then instructions) plus any reference files or scripts. The same format as
//! Claude skills, so the user's `~/.claude/skills` work here unchanged. An agent sees each
//! equipped skill's one-line description and loads the full text with `read_skill`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub dir: PathBuf,
}

impl Skill {
    pub fn body(&self) -> Result<String, String> {
        std::fs::read_to_string(self.dir.join("SKILL.md")).map_err(|e| e.to_string())
    }
    /// Files inside the skill folder (relative paths), for `read_skill` to list.
    pub fn files(&self) -> Vec<String> {
        let mut out = vec![];
        walk(&self.dir, &self.dir, &mut out, 0);
        out.sort();
        out
    }
}

fn walk(root: &Path, d: &Path, out: &mut Vec<String>, depth: usize) {
    if depth > 3 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(d) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || name == "__pycache__" || name == "node_modules" {
            continue;
        }
        if p.is_dir() {
            walk(root, &p, out, depth + 1);
        } else if let Ok(r) = p.strip_prefix(root) {
            out.push(r.display().to_string());
        }
    }
}

/// Parses the front matter of a SKILL.md.
pub fn parse_front_matter(text: &str) -> (Option<String>, Option<String>) {
    let mut name = None;
    let mut desc = None;
    let t = text.trim_start();
    if let Some(rest) = t.strip_prefix("---") {
        if let Some(end) = rest.find("\n---") {
            let fm = &rest[..end];
            let mut lines = fm.lines().peekable();
            while let Some(l) = lines.next() {
                if let Some(v) = l.strip_prefix("name:") {
                    name = Some(unquote(v));
                } else if let Some(v) = l.strip_prefix("description:") {
                    let v = v.trim();
                    if v == "|" || v == ">" || v.is_empty() {
                        // block scalar: the indented lines that follow
                        let mut acc = vec![];
                        while let Some(n) = lines.peek() {
                            if n.starts_with(' ') || n.starts_with('\t') {
                                acc.push(n.trim().to_string());
                                lines.next();
                            } else {
                                break;
                            }
                        }
                        desc = Some(acc.join(" "));
                    } else {
                        desc = Some(unquote(v));
                    }
                }
            }
        }
    }
    (name, desc)
}

fn unquote(v: &str) -> String {
    v.trim().trim_matches('"').trim_matches('\'').to_string()
}

/// Every skill found in the given folders (first one wins on a name clash).
pub fn scan(dirs: &[String]) -> Vec<Skill> {
    let mut out: Vec<Skill> = vec![];
    for d in dirs {
        let Ok(rd) = std::fs::read_dir(d) else { continue };
        let mut entries: Vec<_> = rd.flatten().map(|e| e.path()).filter(|p| p.join("SKILL.md").is_file()).collect();
        entries.sort();
        for p in entries {
            let text = std::fs::read_to_string(p.join("SKILL.md")).unwrap_or_default();
            let (name, desc) = parse_front_matter(&text);
            let name = name.unwrap_or_else(|| p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
            if out.iter().any(|s| s.name == name) {
                continue;
            }
            let description = desc.unwrap_or_else(|| text.lines().find(|l| !l.trim().is_empty() && !l.starts_with('#')).unwrap_or("").chars().take(200).collect());
            out.push(Skill { name, description, dir: p });
        }
    }
    out
}

/// The skills an agent is equipped with.
pub fn equipped<'a>(all: &'a [Skill], names: &[String]) -> Vec<&'a Skill> {
    if names.iter().any(|n| n == "*") {
        return all.iter().collect();
    }
    all.iter().filter(|s| names.contains(&s.name)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn front_matter_forms() {
        let (n, d) = parse_front_matter("---\nname: terzaghi\ndescription: \"Bearing capacity\"\n---\nbody");
        assert_eq!(n.as_deref(), Some("terzaghi"));
        assert_eq!(d.as_deref(), Some("Bearing capacity"));
        let (_, d) = parse_front_matter("---\nname: x\ndescription: >\n  line one\n  line two\nother: 1\n---\n");
        assert_eq!(d.as_deref(), Some("line one line two"));
    }
}
