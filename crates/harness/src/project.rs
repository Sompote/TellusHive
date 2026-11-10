//! A design project: a folder with the chosen standards, the team, attachments and every
//! artifact the agents produce (calculations, drawings, reports, pictures).
//!
//! ```text
//! <project>/project.json        name, description, standards (library ids), team
//! <project>/attachments/        files the engineer dropped into the chat
//! <project>/calcs/<name>.json   calculation sheets (th_eng::calc::Sheet) + <name>.md
//! <project>/drawings/<name>.svg/.dxf/.json
//! <project>/reports/<name>.pdf/.md
//! <project>/images/             pictures returned by plugins
//! <project>/work/               scratch folder for Claude Code / Codex
//! <project>/chat.json           conversation of the coordinator + the transcript shown in the UI
//! ```

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::{read_json, write_json};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ProjectInfo {
    pub name: String,
    pub description: String,
    /// Library ids of the standards the engineer chose for this project.
    pub standards: Vec<String>,
    pub team: String,
    /// Free text the agents always see: client, site, units, office rules.
    pub notes: String,
}

impl Default for ProjectInfo {
    fn default() -> Self {
        ProjectInfo { name: "Untitled".into(), description: String::new(), standards: vec![], team: crate::config::default_team().name, notes: String::new() }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub enum ArtifactKind {
    Calculation,
    Drawing,
    Report,
    Image,
    Attachment,
    Work,
}

impl ArtifactKind {
    pub fn folder(self) -> &'static str {
        match self {
            ArtifactKind::Calculation => "calcs",
            ArtifactKind::Drawing => "drawings",
            ArtifactKind::Report => "reports",
            ArtifactKind::Image => "images",
            ArtifactKind::Attachment => "attachments",
            ArtifactKind::Work => "work",
        }
    }
    pub fn all() -> [ArtifactKind; 6] {
        [ArtifactKind::Calculation, ArtifactKind::Drawing, ArtifactKind::Report, ArtifactKind::Image, ArtifactKind::Attachment, ArtifactKind::Work]
    }
    pub fn label(self) -> &'static str {
        match self {
            ArtifactKind::Calculation => "Calculations",
            ArtifactKind::Drawing => "Drawings",
            ArtifactKind::Report => "Reports",
            ArtifactKind::Image => "Pictures",
            ArtifactKind::Attachment => "Attachments",
            ArtifactKind::Work => "Work files",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Artifact {
    pub kind: ArtifactKind,
    /// Base name without extension (one artifact may have several files: .svg + .dxf).
    pub name: String,
    pub files: Vec<PathBuf>,
    pub modified: u64,
}

#[derive(Debug, Clone)]
pub struct Project {
    pub dir: PathBuf,
    pub info: ProjectInfo,
}

pub fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.trim().chars() {
        if c.is_alphanumeric() || c == '.' {
            out.push(c);
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    let out = out.trim_matches('_').to_string();
    if out.is_empty() {
        "untitled".into()
    } else {
        out.chars().take(80).collect()
    }
}

impl Project {
    pub fn create(dir: &Path, info: ProjectInfo) -> Result<Project, String> {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for k in ArtifactKind::all() {
            std::fs::create_dir_all(dir.join(k.folder())).map_err(|e| e.to_string())?;
        }
        let p = Project { dir: dir.to_path_buf(), info };
        p.save()?;
        Ok(p)
    }

    pub fn open(dir: &Path) -> Result<Project, String> {
        let info: ProjectInfo = read_json(&dir.join("project.json"))?;
        for k in ArtifactKind::all() {
            let _ = std::fs::create_dir_all(dir.join(k.folder()));
        }
        Ok(Project { dir: dir.to_path_buf(), info })
    }

    /// Opens the folder as a project, creating it if needed.
    pub fn open_or_create(dir: &Path) -> Result<Project, String> {
        if dir.join("project.json").is_file() {
            Project::open(dir)
        } else {
            let name = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "Untitled".into());
            Project::create(dir, ProjectInfo { name, ..Default::default() })
        }
    }

    pub fn save(&self) -> Result<(), String> {
        write_json(&self.dir.join("project.json"), &self.info, false)
    }

    pub fn folder(&self, k: ArtifactKind) -> PathBuf {
        self.dir.join(k.folder())
    }

    /// A path inside the project; rejects absolute paths and `..` that leave it.
    pub fn resolve(&self, rel: &str) -> Result<PathBuf, String> {
        let p = Path::new(rel.trim());
        let joined = if p.is_absolute() { p.to_path_buf() } else { self.dir.join(p) };
        let mut norm = PathBuf::new();
        for c in joined.components() {
            match c {
                std::path::Component::ParentDir => {
                    norm.pop();
                }
                std::path::Component::CurDir => {}
                other => norm.push(other),
            }
        }
        if !norm.starts_with(&self.dir) {
            return Err(format!("'{rel}' is outside the project folder"));
        }
        Ok(norm)
    }

    pub fn artifacts(&self) -> Vec<Artifact> {
        let mut out: Vec<Artifact> = vec![];
        for k in ArtifactKind::all() {
            let Ok(rd) = std::fs::read_dir(self.folder(k)) else { continue };
            let mut files: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect();
            files.sort();
            for f in files {
                let name = f.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                if name.starts_with('.') || f.extension().is_some_and(|e| e == "tmp") {
                    continue;
                }
                // group by stem for calculations / drawings / reports; one per file otherwise
                let group = matches!(k, ArtifactKind::Calculation | ArtifactKind::Drawing | ArtifactKind::Report);
                let modified = f.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
                if group {
                    if let Some(a) = out.iter_mut().find(|a| a.kind == k && a.name == name) {
                        a.files.push(f);
                        a.modified = a.modified.max(modified);
                        continue;
                    }
                    out.push(Artifact { kind: k, name, files: vec![f], modified });
                } else {
                    let name = f.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                    out.push(Artifact { kind: k, name, files: vec![f], modified });
                }
            }
        }
        out
    }

    /// Copies a file into attachments/ and returns the new path.
    pub fn attach(&self, src: &Path) -> Result<PathBuf, String> {
        let name = src.file_name().ok_or("no file name")?.to_string_lossy().to_string();
        let mut dst = self.folder(ArtifactKind::Attachment).join(&name);
        if dst == src {
            return Ok(dst);
        }
        let mut n = 1;
        while dst.exists() {
            let stem = src.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            let ext = src.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
            dst = self.folder(ArtifactKind::Attachment).join(format!("{stem}_{n}{ext}"));
            n += 1;
        }
        std::fs::copy(src, &dst).map_err(|e| format!("{}: {e}", src.display()))?;
        Ok(dst)
    }

    pub fn rel(&self, p: &Path) -> String {
        p.strip_prefix(&self.dir).map(|r| r.display().to_string()).unwrap_or_else(|_| p.display().to_string())
    }
}

/// Reads a file as text for an agent: PDF text, spreadsheet-like CSV, plain text.
pub fn read_text(p: &Path, max_chars: usize) -> Result<String, String> {
    let ext = p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    let text = match ext.as_str() {
        "pdf" => {
            let bytes = std::fs::read(p).map_err(|e| e.to_string())?;
            let t = pdf_extract::extract_text_from_mem(&bytes).map_err(|e| format!("PDF text extraction failed: {e}"))?;
            if t.trim().is_empty() {
                return Err("the PDF has no text layer (scanned?); use describe_image on page pictures instead".into());
            }
            t
        }
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" => return Err("this is an image; use describe_image to look at it".into()),
        _ => {
            let bytes = std::fs::read(p).map_err(|e| format!("{}: {e}", p.display()))?;
            String::from_utf8_lossy(&bytes).to_string()
        }
    };
    Ok(clip(&text, max_chars))
}

pub fn clip(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let head: String = s.chars().take(max_chars).collect();
        format!("{head}\n…[truncated: {} more characters]", s.chars().count() - max_chars)
    }
}

pub fn is_image(p: &Path) -> bool {
    matches!(p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).as_deref(), Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_stays_inside() {
        let d = std::env::temp_dir().join(format!("adproj_{}", std::process::id()));
        let p = Project::create(&d, ProjectInfo::default()).unwrap();
        assert!(p.resolve("calcs/a.json").is_ok());
        assert!(p.resolve("../x").is_err());
        assert!(p.resolve("/etc/passwd").is_err());
        std::fs::write(p.folder(ArtifactKind::Drawing).join("beam.svg"), "<svg/>").unwrap();
        std::fs::write(p.folder(ArtifactKind::Drawing).join("beam.dxf"), "0\nEOF").unwrap();
        let a = p.artifacts();
        assert_eq!(a.iter().filter(|a| a.kind == ArtifactKind::Drawing).count(), 1);
        assert_eq!(a[0].files.len(), 2);
        let _ = std::fs::remove_dir_all(&d);
    }
}
