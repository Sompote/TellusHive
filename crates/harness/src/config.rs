//! Settings (providers, keys, external agents) and the agent team the user designs.

use std::path::{Path, PathBuf};

use th_llm::Provider;
use th_mcp::ServerConfig;
use serde::{Deserialize, Serialize};

/// Where everything global lives: `~/Library/Application Support/TellusHive` on macOS.
pub fn data_dir() -> PathBuf {
    if let Ok(d) = std::env::var("TELLUSHIVE_HOME") {
        return PathBuf::from(d);
    }
    dirs::data_dir().unwrap_or_else(std::env::temp_dir).join("TellusHive")
}

/// Moves the folders of the program's earlier name (AgenticDesign) to the TellusHive names
/// once, and rewrites the paths saved in settings.json and app_state.json to match. A folder is
/// moved only when the new one does not exist yet, so nothing is ever overwritten. Returns
/// what was done, for the log.
pub fn migrate_legacy() -> Vec<String> {
    let mut done = vec![];
    if std::env::var("TELLUSHIVE_HOME").is_ok() {
        return done;
    }
    let home = dirs::home_dir().unwrap_or_default();
    let base = dirs::data_dir().unwrap_or_else(std::env::temp_dir);
    let moves = [(base.join("AgenticDesign"), base.join("TellusHive")), (home.join("Documents/AgenticDesign"), home.join("Documents/TellusHive"))];
    let mut renamed: Vec<(String, String)> = vec![];
    for (old, new) in &moves {
        if old.is_dir() && !new.exists() {
            match std::fs::rename(old, new) {
                Ok(()) => {
                    done.push(format!("moved {} to {}", old.display(), new.display()));
                    renamed.push((old.display().to_string(), new.display().to_string()));
                }
                Err(e) => done.push(format!("could not move {}: {e}", old.display())),
            }
        }
    }
    if renamed.is_empty() {
        return done;
    }
    // the files that hold absolute paths: the global settings and each project's chat history
    let mut files = vec![data_dir().join("settings.json"), data_dir().join("app_state.json")];
    if let Ok(rd) = std::fs::read_dir(&moves[1].1) {
        for e in rd.flatten() {
            files.extend([e.path().join("chat.json"), e.path().join("project.json")]);
        }
    }
    for p in files {
        let Ok(mut s) = std::fs::read_to_string(&p) else { continue };
        let before = s.clone();
        for (old, new) in &renamed {
            // a path is followed by '/' or the closing quote, so "AgenticDesignX" is left alone
            s = s.replace(&format!("{old}/"), &format!("{new}/")).replace(&format!("{old}\""), &format!("{new}\"")).replace(&format!("{old}\\"), &format!("{new}\\"));
        }
        if s != before {
            let _ = std::fs::write(&p, s);
            done.push(format!("updated the paths in {}", p.display()));
        }
    }
    done
}

pub fn read_json<T: for<'de> Deserialize<'de>>(p: &Path) -> Result<T, String> {
    let s = std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?;
    serde_json::from_str(&s).map_err(|e| format!("{}: {e}", p.display()))
}

/// Writes JSON atomically; `private` restricts the file to the owner (it may hold API keys).
pub fn write_json<T: Serialize>(p: &Path, v: &T, private: bool) -> Result<(), String> {
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let tmp = p.with_extension("tmp");
    let s = serde_json::to_string_pretty(v).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, s).map_err(|e| format!("{}: {e}", tmp.display()))?;
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    let _ = private;
    std::fs::rename(&tmp, p).map_err(|e| e.to_string())
}

/// An external coding agent the coordinator can hand work to.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ExternalAgent {
    pub enabled: bool,
    /// Program path; empty = find it on PATH.
    pub path: String,
    /// Extra arguments, e.g. the permission mode.
    pub args: Vec<String>,
    pub timeout_min: u64,
    /// Run on the program's own login (Claude or ChatGPT subscription): ANTHROPIC_API_KEY /
    /// OPENAI_API_KEY are removed from its environment so a key in the shell does not
    /// override the login.
    pub subscription: bool,
}

impl Default for ExternalAgent {
    fn default() -> Self {
        ExternalAgent { enabled: true, path: String::new(), args: vec![], timeout_min: 30, subscription: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub providers: Vec<Provider>,
    /// Provider and model of agents that do not choose one.
    pub default_provider: String,
    pub default_model: String,
    /// A vision-capable provider/model that describes images for text-only models (DeepSeek).
    pub vision_provider: String,
    pub vision_model: String,
    pub embedder: th_kb::EmbedderConfig,
    pub claude_code: ExternalAgent,
    pub codex: ExternalAgent,
    /// MCP servers ("plugins").
    pub plugins: Vec<ServerConfig>,
    /// Folders scanned for skills (each skill is a folder with SKILL.md).
    pub skill_dirs: Vec<String>,
    /// Python interpreter for the run_python tool; empty = find python3 on PATH and in the usual folders.
    pub python: String,
    /// Opens an analysis program's window on the project's model when the agents work in it.
    pub open_models: bool,
    pub projects_dir: String,
    pub dark: bool,
    /// The agent harness (th-hive) set-up.
    pub harness: HarnessSettings,
}

/// Settings of the agent harness: the base bundle's knobs and the plugins left out
/// (a DeepSeek Harness profile patch in spirit).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct HarnessSettings {
    /// Estimated prompt tokens above which old context is pruned and then summarised.
    pub compaction_budget_tokens: usize,
    /// Tool calls of one reply that may run at the same time (delegations, reads).
    pub max_parallel: usize,
    /// Tool outputs longer than this many characters are saved to sessions/spill.
    pub spill_chars: usize,
    /// Identical tool calls in a row that trigger a reminder.
    pub repeat_thresholds: Vec<usize>,
    /// Retries of a failed model request.
    pub retries: u32,
    /// Plugin names to leave out, e.g. "hive-team-board".
    pub disabled_plugins: Vec<String>,
}

impl Default for HarnessSettings {
    fn default() -> Self {
        HarnessSettings { compaction_budget_tokens: 90_000, max_parallel: 8, spill_chars: 12_000, repeat_thresholds: vec![3, 5, 8], retries: 3, disabled_plugins: vec![] }
    }
}

impl Default for Settings {
    fn default() -> Self {
        let home = dirs::home_dir().unwrap_or_default();
        let mut claude = ExternalAgent::default();
        claude.args = vec!["--permission-mode".into(), "acceptEdits".into()];
        let mut codex = ExternalAgent::default();
        codex.args = vec!["--sandbox".into(), "workspace-write".into()];
        Settings {
            providers: Provider::presets(),
            default_provider: "openrouter".into(),
            default_model: "deepseek/deepseek-v4.1-flash-1017".into(),
            vision_provider: "openrouter".into(),
            vision_model: "deepseek/deepseek-v4.1-flash-1017".into(),
            embedder: th_kb::EmbedderConfig::default(),
            claude_code: claude,
            codex,
            plugins: default_plugins(),
            python: String::new(),
            open_models: true,
            skill_dirs: vec![data_dir().join("skills").display().to_string(), home.join(".claude/skills").display().to_string()],
            projects_dir: home.join("Documents/TellusHive").display().to_string(),
            dark: false,
            harness: HarnessSettings::default(),
        }
    }
}

/// Tellus (2D FE) and Tellus3D, when their programs are found on this computer.
/// The Tellus family and the other engineering programs on this computer, each found at its
/// installed app or its build folder (the newest copy that exists), plus CADCraft.
pub fn default_plugins() -> Vec<ServerConfig> {
    let home = dirs::home_dir().unwrap_or_default();
    let h = |rel: &str| home.join(rel).display().to_string();
    let arch_granum = if cfg!(target_arch = "aarch64") { "ENV3/DEM/dist/education/mac_arm/granum" } else { "ENV3/DEM/dist/education/mac_x86/granum" };
    let programs: Vec<(&str, &str, Vec<String>, u64)> = vec![
        ("tellus", "Tellus 2D geotechnical finite element analysis: plane strain, borehole or drawn soil, plates, anchors, geogrids, embedded piles, staged construction, consolidation, phi-c reduction safety factor, dynamics. Excavations, embankments, slopes, foundations, tunnels.",
            vec!["/Applications/Tellus.app/Contents/MacOS/tellus".into(), h("ENV3/Tellus/soil_FEM/target/release/tellus")], 1800),
        ("tellus3d", "Tellus3D 3D geotechnical finite element analysis: boreholes or drawn clusters, plates, piles, TBM tunnels, staged construction, consolidation, drains and vacuum, safety factor, dynamics.",
            vec!["/Applications/Tellus3D.app/Contents/MacOS/tellus3d".into(), h("ENV3/Tellus/Tellus3D/target/release/tellus3d")], 3600),
        ("pondus", "Pondus structural analysis and design: 3D frames, walls and slabs, load combinations, ACI 318 / AISC 360 / EC3 member design, pushover, staged construction, PDF report, export to Aedifex BIM, Excel, DXF, IFC.",
            vec!["/Applications/Pondus.app/Contents/MacOS/pondus".into(), h("ENV3/structure_pondus/pondus/target/release/pondus"), h("ENV3/structure/pondus/target/release/pondus")], 1800),
        ("aedifex", "Aedifex BIM modeller (Revit-like): levels, grids, walls, slabs, beams, columns, footings, rebar, rooms, sheets; builds the building model from a structural design (Pondus), models terrain (survey points, contours, graded cut and fill slopes with berms), imports Revit .rvt, exports IFC4 and drawings.",
            vec!["/Applications/Aedifex.app/Contents/MacOS/aedifex".into(), h("ENV3/BIM/aedifex/target/release/aedifex"), h("ENV3/bim_tunnel/aedifex/target/release/aedifex")], 1800),
        ("granum", "Granum discrete element method (DEM) for soil: particle packings, triaxial and direct shear tests, bonded (cemented) clay, embankments, staged models; GPU (CUDA) or CPU engine.",
            vec!["/Applications/Granum.app/Contents/MacOS/granum".into(), h("ENV3/DEM/target/release/granum"), h(arch_granum)], 3600),
        ("granumfl", "GranumFL coupled DEM-SPH (grains and water): debris flows, landslide run-out, scour, flow through granular media, Patong-type flash-flood scenarios.",
            vec!["/Applications/GranumFL.app/Contents/MacOS/granum".into(), h("ENV3/DEM_SPH/target/release/granum"), h("ENV3/DEM_SPH/dist/education/mac_app/GranumFL-Education/GranumFL.app/Contents/MacOS/granum")], 3600),
        ("continuax", "ContinuaX structural finite element analysis (ABAQUS-like): solids, shells and beams, static, modal and dynamic steps, nonlinear materials, earthquake time histories.",
            vec!["/Applications/ContinuaX.app/Contents/MacOS/continuax".into(), h("ENV3/GenFEM/target/release/continuax")], 3600),
        ("motus", "Motus 1-D seismic site response: equivalent-linear and nonlinear analysis of a soil column, ground motions, design spectra, liquefaction triggering, Newmark displacement.",
            vec!["/Applications/Motus.app/Contents/MacOS/motus".into(), h("ENV3/site_reponse/target/release/motus")], 1800),
    ];
    let mut out = vec![];
    for (name, desc, paths, timeout_s) in programs {
        // the newest copy: a fresh build beats an older installed app, and the other way round
        let found = paths
            .iter()
            .filter_map(|p| std::fs::metadata(p).ok().filter(|m| m.is_file()).and_then(|m| m.modified().ok()).map(|t| (t, p.clone())))
            .max_by_key(|(t, _)| *t)
            .map(|(_, p)| p);
        out.push(ServerConfig {
            name: name.into(),
            description: desc.into(),
            enabled: found.is_some(),
            command: found.unwrap_or_else(|| paths.last().cloned().unwrap_or_default()),
            args: vec!["--mcp".into()],
            timeout_s,
            ..Default::default()
        });
    }
    // CADCraft (AutoCAD-like 2D CAD): shows the drawings and lets the drafter draw live
    let cad = home.join("ENV2/cad/cadcraft/target/release/cadcraft-cli");
    out.push(ServerConfig {
        name: "cadcraft".into(),
        description: "CADCraft 2D/3D CAD (AutoCAD-like). `open` {path} opens a DXF in the CADCraft window; command_line takes AutoCAD syntax (line 0,0 10,0; circle 5,2 1; zoom e); render returns a picture; save writes DXF/PDF.".into(),
        command: cad.display().to_string(),
        args: vec!["mcp".into(), "--connect".into(), "127.0.0.1:7979".into(), "--lazy".into(), "--launch".into()],
        enabled: cad.exists(),
        timeout_s: 300,
        ..Default::default()
    });
    out
}

impl Settings {
    pub fn path() -> PathBuf {
        data_dir().join("settings.json")
    }
    pub fn load() -> Settings {
        let mut s: Settings = read_json(&Self::path()).unwrap_or_default();
        // presets added in later versions appear for old settings files
        for p in Provider::presets() {
            if !s.providers.iter().any(|q| q.name == p.name) {
                s.providers.push(p);
            }
        }
        for p in default_plugins() {
            match s.plugins.iter_mut().find(|q| q.name == p.name) {
                // a program installed or moved since: follow it
                Some(q) => {
                    if !std::path::Path::new(&q.command).is_file() && std::path::Path::new(&p.command).is_file() {
                        q.command = p.command;
                        q.enabled = true;
                    }
                }
                None => s.plugins.push(p),
            }
        }
        s
    }
    pub fn save(&self) -> Result<(), String> {
        write_json(&Self::path(), self, true)
    }
    pub fn provider(&self, name: &str) -> Option<&Provider> {
        let name = if name.is_empty() { &self.default_provider } else { name };
        self.providers.iter().find(|p| p.name == name)
    }
}

/// One agent of the team.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AgentSpec {
    pub id: String,
    pub name: String,
    /// One line shown to the coordinator so it knows whom to delegate to.
    pub role: String,
    pub instructions: String,
    /// Empty = the settings default.
    pub provider: String,
    pub model: String,
    /// Built-in tool names, "*" = all of them (delegation is governed by `can_delegate`).
    pub tools: Vec<String>,
    /// Skill names, "*" = every skill found.
    pub skills: Vec<String>,
    /// MCP server names.
    pub plugins: Vec<String>,
    pub can_delegate: bool,
    /// May hand work to Claude Code / Codex.
    pub can_use_external: bool,
    pub max_turns: usize,
    pub temperature: f32,
    /// Thinking before each reply: "" (the provider's default), "off", "low", "medium",
    /// "high". Off makes routine agents (drafting, modelling) several times faster.
    pub thinking: String,
    /// Hex colour of its node and chat bubbles.
    pub color: String,
    /// Position on the team canvas.
    pub pos: [f32; 2],
}

impl Default for AgentSpec {
    fn default() -> Self {
        AgentSpec {
            id: "agent".into(),
            name: "Agent".into(),
            role: String::new(),
            instructions: String::new(),
            provider: String::new(),
            model: String::new(),
            tools: vec![],
            skills: vec![],
            plugins: vec![],
            can_delegate: false,
            can_use_external: false,
            max_turns: 30,
            temperature: 0.2,
            thinking: String::new(),
            color: "#2f74d0".into(),
            pos: [0.0, 0.0],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct TeamSpec {
    pub name: String,
    pub description: String,
    /// The agent the user talks to.
    pub coordinator: String,
    pub agents: Vec<AgentSpec>,
    /// Delegation links drawn on the canvas (from, to). Empty and not `custom_links` = the
    /// coordinator may call everyone.
    pub links: Vec<(String, String)>,
    /// True once the engineer has drawn or removed links by hand, so an empty `links` then means
    /// "nobody delegates" instead of the default star from the coordinator.
    pub custom_links: bool,
}

impl Default for TeamSpec {
    fn default() -> Self {
        default_team()
    }
}

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

/// Coordinator + designer + checker + drafter + reporter.
pub fn default_team() -> TeamSpec {
    let coordinator = AgentSpec {
        id: "coordinator".into(),
        name: "Coordinator".into(),
        role: "Plans the design, delegates to the specialists, reviews their work and answers the engineer.".into(),
        instructions: "You lead a civil-engineering design office. Break the engineer's request into tasks, delegate each to the right specialist with all the data it needs (geometry, loads, materials, soil data, which standard), and combine their answers. Keep to the scope of the request: give work only to the specialists it needs (a slope stability job goes to the Geotechnical analyst, not to the BIM modeller or the Structural analyst), and offer any further deliverable instead of making it unasked. Always have the Checker verify a design before you report it as final. Ask the Drafter for drawings and the Reporter for the calculation report when the engineer wants deliverables. If data is missing, state the assumption you made and flag it clearly. Use Claude Code or Codex only for software work (scripts, data processing, file conversion) that the built-in tools cannot do. Pass a given factored shear Vu to the designer and the checker as the engineer wrote it; unless the engineer says otherwise it is the support reaction, not the shear at distance d. Revisions: when the Checker returns FAIL, send the Designer all of its findings in one revision task, then have the Checker check that revision. Allow at most two revision rounds; if the design still fails, stop and report the open findings to the engineer and ask how to proceed instead of starting a third round.".into(),
        tools: s(&["list_standards", "search_standards", "get_clause", "list_files", "read_file", "write_file", "read_skill", "describe_image", "list_design_modules"]),
        skills: s(&["*"]),
        can_delegate: true,
        can_use_external: true,
        max_turns: 40,
        color: "#2f74d0".into(),
        pos: [60.0, 330.0],
        ..Default::default()
    };
    let designer = AgentSpec {
        id: "designer".into(),
        name: "Designer".into(),
        role: "Designs members and foundations: looks up the clauses of the chosen standard and writes traceable calculations.".into(),
        instructions: "You are a senior design engineer. Before each calculation, search the standards library for the governing clauses of the standard the project uses and cite them by number. Prefer a built-in design module when one fits; otherwise write the calculation with calc_sheet, one step per equation, each with its clause reference. Never invent clause text or factors: if the library does not contain the clause, say so and state the value you assumed and why. Report the governing checks with their utilisation ratios. Loads as given: unless the engineer says otherwise, a given factored shear Vu is the support reaction (the shear at the support face), not the shear at distance d. Reduce it to the critical section yourself where the standard allows, and do not report an Mu/Vu inconsistency that comes only from reading Vu at d (for a uniform load on a simply supported span, Vu = wL/2 and Mu = wL^2/8, so Mu = Vu L/4).".into(),
        tools: s(&["list_standards", "search_standards", "get_clause", "list_design_modules", "run_design_module", "calc_sheet", "list_files", "read_file", "read_skill"]),
        skills: s(&["*"]),
        plugins: vec![],
        color: "#2e9e4b".into(),
        pos: [420.0, 0.0],
        ..Default::default()
    };
    let checker = AgentSpec {
        id: "checker".into(),
        name: "Checker".into(),
        role: "Independently checks a design: recomputes the key results, verifies clause use, and returns PASS or a list of errors.".into(),
        instructions: "You are the independent checking engineer. Do not trust the designer's numbers: recompute the governing quantities yourself (calc_sheet or a design module), confirm each factor and equation against the standards library, and check units, load combinations and detailing limits. Reply with a verdict line 'CHECK: PASS' or 'CHECK: FAIL', then a numbered list of findings, each with the clause and the corrected value. Loads as given: unless the engineer says otherwise, a given factored shear Vu is the support reaction (the shear at the support face), not the shear at distance d. Before you report a load inconsistency, check it with Vu read as the support reaction (for a uniform load on a simply supported span, Mu = Vu L/4); flag an inconsistency only if it remains. Report every finding in one reply so the design can be revised in a single round: do not stop at the first error. Recompute only the governing quantities and keep the reply short.".into(),
        tools: s(&["list_standards", "search_standards", "get_clause", "list_design_modules", "run_design_module", "calc_sheet", "list_files", "read_file", "read_skill"]),
        skills: s(&["*"]),
        color: "#c23b2e".into(),
        pos: [420.0, 95.0],
        thinking: "high".into(),
        ..Default::default()
    };
    let drafter = AgentSpec {
        id: "drafter".into(),
        name: "Drafter".into(),
        role: "Produces 2D engineering drawings (SVG + DXF) of sections, plans and details.".into(),
        instructions: "You are a CAD drafter. Draw to real dimensions in the units given, use layers (outline, concrete, rebar, soil, dims, text), dimension every governing size, and label bars and materials. Design modules already produce a section drawing; draw extra views and details with make_drawing. Each drawing is saved as <project folder>/drawings/<name>.dxf; when the engineer wants to see or edit a drawing in CAD, open that DXF (absolute path) with the CADCraft plugin's `open` tool, and use its command_line for edits the engineer asks for in CAD. Look at each finished drawing once, at the end (one render or describe_image per drawing), not after every change.".into(),
        tools: s(&["make_drawing", "list_files", "read_file"]),
        plugins: s(&["cadcraft"]),
        color: "#9a6a0a".into(),
        pos: [420.0, 190.0],
        thinking: "off".into(),
        max_turns: 60,
        ..Default::default()
    };
    let reporter = AgentSpec {
        id: "reporter".into(),
        name: "Reporter".into(),
        role: "Assembles the calculation report (PDF + Markdown) from the saved calculations and drawings.".into(),
        instructions: "You write calculation reports for submission. Collect the saved calculations and drawings (list_files), then call write_report with sections: design basis (standards, materials, loads, assumptions), each calculation (calc_ref blocks), drawings (drawing_ref blocks), the checker's verdict, and conclusions. Write in clear engineering prose; do not change any number.".into(),
        tools: s(&["write_report", "list_files", "read_file", "list_standards"]),
        color: "#7b4fd6".into(),
        pos: [420.0, 285.0],
        thinking: "off".into(),
        ..Default::default()
    };
    let analyst_tools = s(&["list_standards", "search_standards", "get_clause", "calc_sheet", "list_files", "read_file", "write_file", "read_skill", "describe_image"]);
    let geo = AgentSpec {
        id: "geo_analyst".into(),
        name: "Geotechnical analyst".into(),
        role: "Runs geotechnical finite element and site response analyses in Tellus (2D), Tellus3D and Motus: excavations, embankments, slopes, foundations, tunnels, consolidation, seismic site response.".into(),
        instructions: "You are a geotechnical numerical analyst. Build the model in the program that fits: Tellus for plane-strain problems, Tellus3D when the geometry is truly 3D, Motus for 1-D site response. Keep the model boundaries far enough (at least 3 times the slope height, excavation depth or embankment width), choose the soil model and parameters from the data you are given and state each parameter's source, use staged construction as built, and check convergence. Report the governing results (displacements, forces in structures, safety factor, peak accelerations) with the pictures the program makes, and say what the model cannot capture.".into(),
        tools: analyst_tools.clone(),
        skills: s(&["*"]),
        plugins: s(&["tellus", "tellus3d", "motus"]),
        color: "#a87a1e".into(),
        pos: [420.0, 380.0],
        max_turns: 60,
        ..Default::default()
    };
    let structural = AgentSpec {
        id: "struct_analyst".into(),
        name: "Structural analyst".into(),
        role: "Analyses and designs structures in Pondus (frames, walls, slabs; ACI/AISC/EC3 member design, pushover) and ContinuaX (continuum FE: solids, shells, dynamics).".into(),
        instructions: "You are a structural analyst. Use Pondus for building frames, walls and slabs with member design to the code the project follows, and ContinuaX for continuum or shell models, modal and time-history analyses. Build the geometry, supports, loads and combinations exactly as given, check that the loads balance the reactions, and report the governing members with their utilisation ratios. Use Pondus' export to Aedifex when the BIM modeller needs the structure.".into(),
        tools: analyst_tools.clone(),
        skills: s(&["*"]),
        plugins: s(&["pondus", "continuax"]),
        color: "#1d8fb5".into(),
        pos: [420.0, 475.0],
        max_turns: 60,
        ..Default::default()
    };
    let particles = AgentSpec {
        id: "particle_modeller".into(),
        name: "Particle modeller".into(),
        role: "Runs discrete element (Granum) and coupled grain-water DEM-SPH (GranumFL) simulations: element tests, granular flows, debris flows, landslide run-out, scour.".into(),
        instructions: "You are a particle-method modeller. Use Granum for dry or bonded granular soil (packings, triaxial or direct shear tests, granular structures) and GranumFL when water and grains interact (debris flows, run-out, scour). Calibrate micro-parameters against the macro test data you are given before any boundary-value run, keep the particle count within what the machine can run, and report what was calibrated, the results and their sensitivity to particle size.".into(),
        tools: analyst_tools,
        skills: s(&["*"]),
        plugins: s(&["granum", "granumfl"]),
        color: "#5b7aa6".into(),
        pos: [420.0, 570.0],
        max_turns: 60,
        ..Default::default()
    };
    let bim = AgentSpec {
        id: "bim_modeller".into(),
        name: "BIM modeller".into(),
        role: "Builds the BIM model in Aedifex (levels, grids, walls, slabs, beams, columns, footings, rebar, and terrain: site ground and graded slopes) from the design, and exports IFC and drawings; can show drawings in CADCraft.".into(),
        instructions: "You are a BIM modeller. Model only what the task describes, and never invent a building or structure that is not in it. A slope, excavation, embankment or site is terrain: model it with add_graded_slope (the cross-section with its berms along the alignment) or add_topography (survey points or contours), with z as the absolute elevation, and add no levels of walls, slabs or frames for it. Build the building model in Aedifex from the agreed design: levels and grids first, then structure (footings, columns, beams, slabs, walls) with the sizes and reinforcement from the calculations, then export IFC4 and the drawings the engineer asks for. Use the Pondus export when the structure was designed in Pondus. Check the model against the design (sizes, levels, bar marks) before you report, and list anything you had to assume.".into(),
        tools: s(&["list_files", "read_file", "write_file", "make_drawing", "describe_image"]),
        plugins: s(&["aedifex", "cadcraft"]),
        color: "#c25a35".into(),
        pos: [420.0, 665.0],
        max_turns: 80,
        thinking: "off".into(),
        ..Default::default()
    };
    TeamSpec {
        name: "Design office".into(),
        description: "Coordinator with designer, checker, drafter and reporter.".into(),
        coordinator: "coordinator".into(),
        agents: vec![coordinator, designer, checker, drafter, reporter, geo, structural, particles, bim],
        links: vec![],
        custom_links: false,
    }
}

impl TeamSpec {
    pub fn dir() -> PathBuf {
        data_dir().join("teams")
    }
    pub fn file_name(name: &str) -> String {
        let slug: String = name.chars().map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
        format!("{}.json", slug.trim_matches('-'))
    }
    pub fn list() -> Vec<TeamSpec> {
        let mut out: Vec<TeamSpec> = std::fs::read_dir(Self::dir())
            .map(|rd| rd.flatten().filter(|e| e.path().extension().is_some_and(|x| x == "json")).filter_map(|e| read_json(&e.path()).ok()).collect())
            .unwrap_or_default();
        if !out.iter().any(|t| t.name == default_team().name) {
            out.insert(0, default_team());
        }
        out
    }
    pub fn load(name: &str) -> TeamSpec {
        let mut t: TeamSpec = read_json(&Self::dir().join(Self::file_name(name))).unwrap_or_else(|_| default_team());
        // the built-in team gains the specialists added in later versions
        let d = default_team();
        if t.name == d.name {
            let mut added = false;
            for a in &d.agents {
                if !t.agents.iter().any(|b| b.id == a.id) {
                    if t.uses_links() {
                        t.links.push((t.coordinator.clone(), a.id.clone()));
                    }
                    t.agents.push(a.clone());
                    added = true;
                }
            }
            // lay the grown team out again, as the default team is
            if added {
                for a in t.agents.iter_mut() {
                    if let Some(da) = d.agents.iter().find(|x| x.id == a.id) {
                        a.pos = da.pos;
                    }
                }
            }
        }
        t
    }
    pub fn save(&self) -> Result<(), String> {
        write_json(&Self::dir().join(Self::file_name(&self.name)), self, false)
    }
    pub fn agent(&self, id: &str) -> Option<&AgentSpec> {
        self.agents.iter().find(|a| a.id == id || a.name.eq_ignore_ascii_case(id))
    }
    /// True when delegation follows the drawn links rather than the default star.
    pub fn uses_links(&self) -> bool {
        self.custom_links || !self.links.is_empty()
    }
    /// The delegation links in force: the drawn ones, or the coordinator to everyone by default.
    pub fn effective_links(&self) -> Vec<(String, String)> {
        if self.uses_links() {
            self.links.clone()
        } else {
            self.agents.iter().filter(|a| a.id != self.coordinator).map(|a| (self.coordinator.clone(), a.id.clone())).collect()
        }
    }
    /// The agents `from` may delegate to.
    pub fn reachable(&self, from: &str) -> Vec<&AgentSpec> {
        let links = self.effective_links();
        self.agents.iter().filter(|a| a.id != from && links.iter().any(|(f, t)| f == from && *t == a.id)).collect()
    }
}
