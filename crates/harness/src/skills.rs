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

/// A YAML scalar: "double quoted" with its \" and \\ escapes, 'single quoted' with '' for a
/// quote, or plain.
fn unquote(v: &str) -> String {
    let v = v.trim();
    if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') {
        let mut out = String::new();
        let mut it = v[1..v.len() - 1].chars();
        while let Some(c) = it.next() {
            if c == '\\' {
                match it.next() {
                    Some('n') => out.push(' '),
                    Some(d) => out.push(d),
                    None => {}
                }
            } else {
                out.push(c);
            }
        }
        return out;
    }
    if v.len() >= 2 && v.starts_with('\'') && v.ends_with('\'') {
        return v[1..v.len() - 1].replace("''", "'");
    }
    v.to_string()
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

/// The folder skills are added to and created in (TellusHive's own; the others listed in the
/// settings, such as `~/.claude/skills`, are only read).
pub fn own_dir() -> PathBuf {
    crate::config::data_dir().join("skills")
}

/// A folder name for a skill: its name in lower case, letters, digits and dashes.
fn folder_name(name: &str) -> String {
    let s: String = name.trim().chars().map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    let s = s.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-");
    if s.is_empty() { "skill".into() } else { s }
}

/// The folder holding SKILL.md under `root`: `root` itself or one level down (an archive
/// often wraps the skill in a folder of its own).
fn skill_root(root: &Path) -> Option<PathBuf> {
    if root.join("SKILL.md").is_file() {
        return Some(root.to_path_buf());
    }
    let mut found = std::fs::read_dir(root).ok()?.flatten().map(|e| e.path()).filter(|p| p.join("SKILL.md").is_file());
    let first = found.next();
    if found.next().is_some() {
        return None; // several skills: add them one by one
    }
    first
}

fn copy_dir(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| format!("cannot create {}: {e}", to.display()))?;
    for e in std::fs::read_dir(from).map_err(|e| e.to_string())?.flatten() {
        let (p, q) = (e.path(), to.join(e.file_name()));
        if e.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        if p.is_dir() {
            copy_dir(&p, &q)?;
        } else {
            std::fs::copy(&p, &q).map_err(|e| format!("cannot copy {}: {e}", p.display()))?;
        }
    }
    Ok(())
}

/// Adds a skill to `dest` from a skill folder, its SKILL.md, or a `.skill` / `.zip` archive
/// (unpacked with the system's `tar`). The skill needs a name and a description in its front
/// matter; one of the same name already there is replaced only when `replace` is set.
pub fn install(src: &Path, dest: &Path, replace: bool) -> Result<Skill, String> {
    let tmp = std::env::temp_dir().join(format!("th_skill_{}_{}", std::process::id(), crate::engine::stamp()));
    let _cleanup = Cleanup(tmp.clone());
    let root = if src.is_dir() {
        src.to_path_buf()
    } else if src.file_name().is_some_and(|n| n.eq_ignore_ascii_case("SKILL.md")) {
        src.parent().ok_or("SKILL.md has no folder")?.to_path_buf()
    } else {
        let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        if ext != "skill" && ext != "zip" {
            return Err(format!("{} is not a skill: choose a skill folder, its SKILL.md, or a .skill or .zip file", src.display()));
        }
        std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
        let out = std::process::Command::new("tar").arg("-xf").arg(src).arg("-C").arg(&tmp).output().map_err(|e| format!("cannot unpack {}: {e}", src.display()))?;
        if !out.status.success() {
            return Err(format!("cannot unpack {}: {}", src.display(), String::from_utf8_lossy(&out.stderr).trim()));
        }
        tmp.clone()
    };
    let root = skill_root(&root).ok_or_else(|| format!("no single SKILL.md found in {}", src.display()))?;
    let text = std::fs::read_to_string(root.join("SKILL.md")).map_err(|e| e.to_string())?;
    let (name, desc) = parse_front_matter(&text);
    let name = name.filter(|n| !n.trim().is_empty()).ok_or("SKILL.md has no `name:` in its front matter")?;
    let desc = desc.filter(|d| !d.trim().is_empty()).ok_or("SKILL.md has no `description:` in its front matter (agents choose skills by it)")?;
    let target = dest.join(folder_name(&name));
    if target.exists() {
        if !replace {
            return Err(format!("a skill named '{name}' is already installed"));
        }
        std::fs::remove_dir_all(&target).map_err(|e| format!("cannot replace {}: {e}", target.display()))?;
    }
    copy_dir(&root, &target)?;
    Ok(Skill { name, description: desc, dir: target })
}

struct Cleanup(PathBuf);
impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Writes a new skill to `dest`: SKILL.md with its front matter and the instructions.
pub fn create(dest: &Path, name: &str, description: &str, instructions: &str) -> Result<Skill, String> {
    let (name, description) = (name.trim(), description.trim().replace('\n', " "));
    if name.is_empty() || description.is_empty() {
        return Err("give the skill a name and a description (agents choose skills by the description)".into());
    }
    let target = dest.join(folder_name(name));
    if target.exists() {
        return Err(format!("a skill named '{name}' already exists"));
    }
    std::fs::create_dir_all(&target).map_err(|e| format!("cannot create {}: {e}", target.display()))?;
    let q = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""));
    let body = format!("---\nname: {}\ndescription: {}\n---\n\n{}\n", q(name), q(&description), instructions.trim());
    std::fs::write(target.join("SKILL.md"), body).map_err(|e| e.to_string())?;
    Ok(Skill { name: name.into(), description, dir: target })
}

/// Removes a skill, only from TellusHive's own folder `own`.
pub fn remove(skill: &Skill, own: &Path) -> Result<(), String> {
    let (d, o) = (skill.dir.canonicalize().map_err(|e| e.to_string())?, own.canonicalize().map_err(|e| e.to_string())?);
    if d.parent() != Some(o.as_path()) {
        return Err(format!("'{}' is not in TellusHive's skills folder, so it is left alone", skill.name));
    }
    std::fs::remove_dir_all(&d).map_err(|e| format!("cannot remove {}: {e}", d.display()))
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
    fn add_create_and_remove_skills() {
        let base = std::env::temp_dir().join(format!("th_skills_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (src, own) = (base.join("src/My Skill"), base.join("own"));
        std::fs::create_dir_all(src.join("references")).unwrap();
        std::fs::write(src.join("SKILL.md"), "---\nname: Rankine wall\ndescription: Rankine earth pressure on a cantilever wall\n---\nSteps").unwrap();
        std::fs::write(src.join("references/table.md"), "Ka table").unwrap();
        // a folder, with its reference files
        let k = install(&src, &own, false).unwrap();
        assert_eq!(k.name, "Rankine wall");
        assert!(own.join("rankine-wall/references/table.md").is_file());
        assert!(install(&src, &own, false).unwrap_err().contains("already installed"));
        assert!(install(&src, &own, true).is_ok());
        // a .skill archive with the skill in a folder of its own
        // (a .skill file is a zip: made as .zip, then renamed)
        let (zip, arc) = (base.join("pack.zip"), base.join("pack.skill"));
        let st = std::process::Command::new("tar").arg("-a").arg("-cf").arg(&zip).arg("-C").arg(base.join("src")).arg("My Skill").status();
        if st.is_ok_and(|s| s.success()) && std::fs::rename(&zip, &arc).is_ok() {
            assert_eq!(&std::fs::read(&arc).unwrap()[..2], b"PK", "a zip archive");
            std::fs::remove_dir_all(own.join("rankine-wall")).unwrap();
            let k = install(&arc, &own, false).unwrap();
            assert!(k.dir.join("SKILL.md").is_file());
        }
        // a SKILL.md without a description is refused
        let bad = base.join("bad");
        std::fs::create_dir_all(&bad).unwrap();
        std::fs::write(bad.join("SKILL.md"), "---\nname: x\n---\n").unwrap();
        assert!(install(&bad, &own, false).unwrap_err().contains("description"));
        // a new skill is read back by scan
        let n = create(&own, "Office: \"cover\" rule", "Concrete cover the office uses", "Use 40 mm.").unwrap();
        let all = scan(&[own.display().to_string()]);
        assert!(all.iter().any(|s| s.name == "Office: \"cover\" rule" && s.description == "Concrete cover the office uses"));
        // removal only inside the own folder
        remove(&n, &own).unwrap();
        assert!(!n.dir.exists());
        let outside = Skill { name: "src".into(), description: String::new(), dir: src.clone() };
        assert!(remove(&outside, &own).is_err());
        assert!(src.is_dir());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn front_matter_forms() {
        let (n, d) = parse_front_matter("---\nname: terzaghi\ndescription: \"Bearing capacity\"\n---\nbody");
        assert_eq!(n.as_deref(), Some("terzaghi"));
        assert_eq!(d.as_deref(), Some("Bearing capacity"));
        let (_, d) = parse_front_matter("---\nname: x\ndescription: >\n  line one\n  line two\nother: 1\n---\n");
        assert_eq!(d.as_deref(), Some("line one line two"));
    }
}
