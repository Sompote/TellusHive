//! Live models: each project keeps one model file per analysis program (Aedifex, Pondus,
//! Tellus, Tellus3D) in its `models/` folder. The program's MCP server works on that file
//! and it is saved after every change, so the engineer can open the same model in the
//! program's own window, rotate it and read the results while the agents work.
//!
//! Two ways keep the file current. Aedifex and Pondus take `--open <file> --autosave <file>`
//! and save it themselves. The installed Tellus and Tellus3D builds lack those flags, so the
//! engine opens the file with their `open_project` tool when the program starts and calls
//! `save_project` after every tool that may change the model.

use std::path::{Path, PathBuf};

use th_mcp::ServerConfig;

/// How one program keeps its model file.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LiveSpec {
    pub plugin: &'static str,
    /// What the engineer reads: "BIM model".
    pub label: &'static str,
    pub ext: &'static str,
    /// The program saves the file itself (`--open` / `--autosave`).
    pub autosave_flags: bool,
    /// The program's window reloads the file when it changes, so it follows the agents live.
    pub follows_file: bool,
}

pub const LIVE: [LiveSpec; 4] = [
    LiveSpec { plugin: "aedifex", label: "BIM model", ext: "aedifex", autosave_flags: true, follows_file: true },
    LiveSpec { plugin: "pondus", label: "Structural model", ext: "pondus", autosave_flags: true, follows_file: false },
    LiveSpec { plugin: "tellus", label: "2D geotechnical model", ext: "tellus", autosave_flags: false, follows_file: false },
    LiveSpec { plugin: "tellus3d", label: "3D geotechnical model", ext: "tellus3d", autosave_flags: false, follows_file: false },
];

pub fn spec(plugin: &str) -> Option<&'static LiveSpec> {
    LIVE.iter().find(|s| s.plugin == plugin)
}

/// The program's name as the engineer knows it: "Tellus3D".
pub fn program_name(plugin: &str) -> String {
    match plugin {
        "tellus3d" => "Tellus3D".into(),
        "continuax" => "ContinuaX".into(),
        "granumfl" => "GranumFL".into(),
        "cadcraft" => "CADCraft".into(),
        p => {
            let mut c = p.chars();
            c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
        }
    }
}

/// `<project>/models/<plugin>.<ext>`
pub fn model_path(project_dir: &Path, s: &LiveSpec) -> PathBuf {
    project_dir.join("models").join(format!("{}.{}", s.plugin, s.ext))
}

/// The server configuration for this project: the model flags added for the programs that
/// take them. The models folder is made so the first save can land.
pub fn bind(cfg: &ServerConfig, project_dir: &Path) -> ServerConfig {
    let mut c = cfg.clone();
    if let Some(s) = spec(&cfg.name).filter(|s| s.autosave_flags) {
        let m = model_path(project_dir, s);
        let _ = std::fs::create_dir_all(m.parent().unwrap_or(project_dir));
        let m = m.display().to_string();
        if Path::new(&m).is_file() {
            c.args.extend(["--open".to_string(), m.clone()]);
        }
        c.args.extend(["--autosave".to_string(), m]);
    }
    c
}

/// True when a tool only reads, draws or exports, so the model need not be saved after it.
pub fn reads_only(tool: &str) -> bool {
    const READ: [&str; 12] = ["get_", "list_", "show_", "check_", "validate", "export", "save_", "open_", "write_report", "describe", "screenshot", "render"];
    READ.iter().any(|p| tool.starts_with(p))
}

/// The line for the agent's system prompt about a plugin with a live model.
pub fn prompt_note(plugin: &str) -> Option<String> {
    let s = spec(plugin)?;
    Some(format!(
        "  The project's {} is models/{}.{}: it is open in {} when the program starts and is saved after every change, and the engineer watches it in {}'s own window. Continue that model; start a new one only when the engineer asks for it.",
        s.label.to_lowercase(),
        s.plugin,
        s.ext,
        program_name(plugin),
        program_name(plugin)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binds_the_model_flags() {
        let d = std::env::temp_dir().join(format!("thlive_{}", std::process::id()));
        let cfg = ServerConfig { name: "aedifex".into(), command: "x".into(), args: vec!["--mcp".into()], ..Default::default() };
        let b = bind(&cfg, &d);
        assert_eq!(b.args[0], "--mcp");
        assert!(b.args.contains(&"--autosave".into()) && !b.args.contains(&"--open".into()));
        std::fs::write(d.join("models/aedifex.aedifex"), "{}").unwrap();
        assert!(bind(&cfg, &d).args.contains(&"--open".into()));
        // Tellus is saved through its tools, so its arguments stay as they are
        let t = ServerConfig { name: "tellus".into(), args: vec!["--mcp".into()], ..Default::default() };
        assert_eq!(bind(&t, &d).args, vec!["--mcp".to_string()]);
        assert!(reads_only("get_results") && !reads_only("calculate") && !reads_only("add_soil_material"));
        let _ = std::fs::remove_dir_all(&d);
    }
}
