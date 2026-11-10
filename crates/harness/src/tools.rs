//! The built-in tools: standards library, calculations, design modules, drawings, reports,
//! project files, skills and image reading. The same functions back the chat agents and this
//! program's own MCP server (`--mcp`), so Claude Code and Codex get identical tools.

use std::path::PathBuf;

use th_eng::{calc, design, drawing, report};
use th_kb::Library;
use th_llm::ToolSpec;
use serde_json::{json, Value};

use crate::config::Settings;
use crate::project::{clip, is_image, read_text, slug, ArtifactKind, Project};
use crate::skills::Skill;

/// What a tool needs to run.
pub struct Ctx<'a> {
    pub project: &'a Project,
    pub library: &'a Library,
    pub skills: &'a [Skill],
    pub settings: &'a Settings,
}

#[derive(Debug, Default, Clone)]
pub struct ToolOut {
    pub text: String,
    pub artifacts: Vec<(ArtifactKind, PathBuf)>,
}

impl ToolOut {
    fn text(s: impl Into<String>) -> Self {
        ToolOut { text: s.into(), artifacts: vec![] }
    }
}

fn spec(name: &str, description: &str, parameters: Value) -> ToolSpec {
    ToolSpec { name: name.into(), description: description.into(), parameters }
}

/// Every built-in tool (delegation and external agents are added by the engine).
pub fn builtin_specs() -> Vec<ToolSpec> {
    vec![
        spec("list_standards", "List the design standards in the library and which ones the engineer chose for this project.", json!({"type":"object","properties":{}})),
        spec(
            "search_standards",
            "Search the standards library (vector + keyword search) for clauses relevant to a question. Searches the project's chosen standards unless `codes` names others. Cite the clause numbers you use.",
            json!({"type":"object","properties":{
                "query":{"type":"string","description":"What to look for, in words or a clause number, e.g. 'minimum shear reinforcement' or '22.5.5.1'"},
                "codes":{"type":"array","items":{"type":"string"},"description":"Standard codes or ids to restrict to, e.g. ['ACI 318-19']"},
                "k":{"type":"integer","description":"Number of results (default 6)"}},"required":["query"]}),
        ),
        spec(
            "get_clause",
            "Read one clause (and its sub-clauses) of a standard by number.",
            json!({"type":"object","properties":{"code":{"type":"string","description":"Standard code or id, e.g. 'ACI 318-19'"},"clause":{"type":"string","description":"e.g. '22.5.5.1'"}},"required":["code","clause"]}),
        ),
        spec("list_design_modules", "List the built-in, verified design modules with their input schemas.", json!({"type":"object","properties":{}})),
        spec(
            "run_design_module",
            "Run a built-in design module. Saves the calculation sheet (calcs/<name>) and its drawing (drawings/<name>.svg/.dxf). Returns the checks and the full sheet.",
            json!({"type":"object","properties":{
                "module":{"type":"string"},
                "args":{"type":"object","description":"Inputs following the module's input_schema"},
                "name":{"type":"string","description":"Artifact name, e.g. 'B1_beam'"}},"required":["module","args"]}),
        ),
        spec(
            "calc_sheet",
            "Write a traceable calculation: inputs, then steps evaluated in order (each step's symbol becomes a variable for later steps), each with its clause reference, and optional demand<=capacity checks. Expressions: + - * / ^ ( ), sqrt abs exp ln log10 sin cos tan (radians) sind cosd tand (degrees) min max floor ceil round, pi. Saved to calcs/<name>.",
            json!({"type":"object","properties":{
                "name":{"type":"string"},
                "title":{"type":"string"},
                "standard":{"type":"string"},
                "inputs":{"type":"array","items":{"type":"object","properties":{"symbol":{"type":"string"},"value":{"type":"number"},"unit":{"type":"string"},"description":{"type":"string"}},"required":["symbol","value"]}},
                "steps":{"type":"array","items":{"type":"object","properties":{
                    "symbol":{"type":"string"},"expr":{"type":"string"},"unit":{"type":"string"},"description":{"type":"string"},"reference":{"type":"string"},
                    "check":{"type":"object","properties":{"demand":{"type":"string"},"capacity":{"type":"string"},"label":{"type":"string"}}}},"required":["symbol","expr"]}},
                "notes":{"type":"array","items":{"type":"string"}}},"required":["name","title","inputs","steps"]}),
        ),
        spec(
            "make_drawing",
            "Make a 2D drawing; saved as drawings/<name>.svg (screen/report) and .dxf (CAD). Coordinates y-up in `units`. Entities: {type:'line',layer,a:[x,y],b:[x,y]}, {type:'polyline',layer,points:[[x,y],..],closed,hatch:'concrete'|'soil'|'solid'|'water'|null}, {type:'circle',layer,c,r,filled}, {type:'arc',layer,c,r,start_deg,end_deg}, {type:'text',layer,at,text,height,align:'left'|'center'|'right'}, {type:'dim',layer,a,b,offset,text?}.",
            json!({"type":"object","properties":{
                "name":{"type":"string"},
                "drawing":{"type":"object","properties":{
                    "title":{"type":"string"},"units":{"type":"string"},"scale_note":{"type":"string"},
                    "layers":{"type":"array","items":{"type":"object","properties":{"name":{"type":"string"},"color":{"type":"string"}}}},
                    "entities":{"type":"array","items":{"type":"object"}}},"required":["title","entities"]}},"required":["name","drawing"]}),
        ),
        spec(
            "write_report",
            "Write the calculation report as PDF and Markdown (reports/<name>). For a long report send a few sections per call: the first call normally, the next ones with append=true and the same name (the PDF is rebuilt each time). Blocks: {type:'paragraph',text}, {type:'note',text}, {type:'table',headers,rows}, {type:'calc_ref',name} (a saved calculation), {type:'drawing_ref',name,caption} (a saved drawing), {type:'image',path,caption} (project-relative png/jpg).",
            json!({"type":"object","properties":{
                "name":{"type":"string"},
                "append":{"type":"boolean","description":"Add these sections to the report of the same name"},
                "title":{"type":"string"},"engineer":{"type":"string"},"checker":{"type":"string"},
                "standards":{"type":"array","items":{"type":"string"}},
                "sections":{"type":"array","items":{"type":"object","properties":{"heading":{"type":"string"},"blocks":{"type":"array","items":{"type":"object"}}},"required":["heading","blocks"]}}},
                "required":["name","sections"]}),
        ),
        spec("list_files", "List the project's files: calculations, drawings, reports, pictures, attachments, work files.", json!({"type":"object","properties":{}})),
        spec(
            "read_file",
            "Read a project file as text (PDF text layer, CSV, Markdown, JSON, code). Paths are project-relative, e.g. 'attachments/soil.pdf' or 'calcs/B1.md'.",
            json!({"type":"object","properties":{"path":{"type":"string"},"max_chars":{"type":"integer"}},"required":["path"]}),
        ),
        spec(
            "write_file",
            "Write a text file in the project's work/ folder (notes, CSV, scripts for an external agent).",
            json!({"type":"object","properties":{"path":{"type":"string","description":"Relative to work/"},"content":{"type":"string"}},"required":["path","content"]}),
        ),
        spec(
            "read_skill",
            "Load the full instructions of one of your skills, or one of its files.",
            json!({"type":"object","properties":{"name":{"type":"string"},"file":{"type":"string","description":"Optional file inside the skill folder"}},"required":["name"]}),
        ),
        spec(
            "run_python",
            "Run a Python 3 script on this computer and return its printed output. Give either `code` (saved as work/py/<name>.py so the calculation stays traceable) or `skill` + `file` to run a script that ships with one of your skills. The working directory is the project's work/ folder; files the script writes there (CSV, PNG plots, JSON) are reported back. numpy, scipy, pandas and matplotlib (Agg backend) are usually installed. Print every result you will report, with units.",
            json!({"type":"object","properties":{
                "code":{"type":"string","description":"Python source to run"},
                "name":{"type":"string","description":"Script name for work/py/<name>.py (default 'script')"},
                "skill":{"type":"string","description":"Run a script from this skill's folder instead of `code`"},
                "file":{"type":"string","description":"Script path inside the skill folder, e.g. 'scripts/bearing.py'"},
                "args":{"type":"array","items":{"type":"string"},"description":"Command-line arguments"},
                "stdin":{"type":"string","description":"Text sent to the script's standard input"},
                "timeout_s":{"type":"integer","description":"Time limit in seconds (default 120, max 900)"}}}),
        ),
        spec(
            "describe_image",
            "Look at an image in the project (a sketch, a photo, a drawing, a plugin picture) and answer a question about it.",
            json!({"type":"object","properties":{"path":{"type":"string"},"question":{"type":"string"}},"required":["path"]}),
        ),
    ]
}

fn arg_str<'a>(a: &'a Value, k: &str) -> Option<&'a str> {
    a.get(k).and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty())
}

fn need_str<'a>(a: &'a Value, k: &str) -> Result<&'a str, String> {
    arg_str(a, k).ok_or_else(|| format!("missing argument '{k}'"))
}

/// Maps "ACI 318-19" or "aci-318-19" to a library id.
fn resolve_standard(lib: &Library, code: &str) -> Option<String> {
    let norm = |s: &str| s.to_ascii_lowercase().chars().filter(|c| c.is_alphanumeric()).collect::<String>();
    let n = norm(code);
    lib.list().into_iter().find(|m| norm(&m.id) == n || norm(&m.code) == n || norm(&m.name) == n).map(|m| m.id)
}

fn hits_text(hits: &[th_kb::Hit]) -> String {
    if hits.is_empty() {
        return "No matching clauses found in the selected standards. Do not invent clause text: tell the engineer the clause was not found and state any value you assume.".into();
    }
    let mut s = String::new();
    for (i, h) in hits.iter().enumerate() {
        let page = h.page.map(|p| format!(", p. {p}")).unwrap_or_default();
        s.push_str(&format!("[{}] {} cl. {} {}{} (score {:.3})\n{}\n\n", i + 1, h.standard_code, h.clause, h.title, page, h.score, clip(h.text.trim(), 1800)));
    }
    s
}

fn save_text(p: &PathBuf, s: &str) -> Result<(), String> {
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    std::fs::write(p, s).map_err(|e| format!("{}: {e}", p.display()))
}

fn save_sheet(ctx: &Ctx, name: &str, sheet: &calc::Sheet) -> Result<Vec<(ArtifactKind, PathBuf)>, String> {
    let dir = ctx.project.folder(ArtifactKind::Calculation);
    let json_p = dir.join(format!("{name}.json"));
    let md_p = dir.join(format!("{name}.md"));
    save_text(&json_p, &serde_json::to_string_pretty(sheet).map_err(|e| e.to_string())?)?;
    save_text(&md_p, &sheet.to_markdown())?;
    Ok(vec![(ArtifactKind::Calculation, json_p)])
}

fn save_drawing(ctx: &Ctx, name: &str, d: &drawing::Drawing) -> Result<Vec<(ArtifactKind, PathBuf)>, String> {
    let dir = ctx.project.folder(ArtifactKind::Drawing);
    let svg = dir.join(format!("{name}.svg"));
    save_text(&svg, &drawing::to_svg(d, 1400))?;
    save_text(&dir.join(format!("{name}.dxf")), &drawing::to_dxf(d))?;
    save_text(&dir.join(format!("{name}.json")), &serde_json::to_string_pretty(d).map_err(|e| e.to_string())?)?;
    Ok(vec![(ArtifactKind::Drawing, svg)])
}

/// Replaces calc_ref / drawing_ref blocks with the saved artifacts and makes image paths absolute.
fn resolve_report(ctx: &Ctx, v: &Value) -> Result<report::Report, String> {
    let mut v = v.clone();
    let mut warnings = vec![];
    if let Some(secs) = v.get_mut("sections").and_then(|s| s.as_array_mut()) {
        for sec in secs {
            let Some(blocks) = sec.get_mut("blocks").and_then(|b| b.as_array_mut()) else { continue };
            for b in blocks.iter_mut() {
                let ty = b.get("type").and_then(|t| t.as_str()).unwrap_or("").to_string();
                match ty.as_str() {
                    "calc_ref" => {
                        let name = arg_str(b, "name").unwrap_or("").trim_end_matches(".json").to_string();
                        let p = ctx.project.folder(ArtifactKind::Calculation).join(format!("{name}.json"));
                        match std::fs::read_to_string(&p).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok()) {
                            Some(sheet) => *b = json!({"type":"sheet","sheet":sheet}),
                            None => {
                                warnings.push(format!("calculation '{name}' not found"));
                                *b = json!({"type":"note","text":format!("[missing calculation {name}]")});
                            }
                        }
                    }
                    "drawing_ref" => {
                        let name = arg_str(b, "name").unwrap_or("").trim_end_matches(".svg").to_string();
                        let caption = arg_str(b, "caption").unwrap_or(&name).to_string();
                        let p = ctx.project.folder(ArtifactKind::Drawing).join(format!("{name}.json"));
                        match std::fs::read_to_string(&p).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok()) {
                            Some(d) => *b = json!({"type":"drawing","drawing":d,"caption":caption}),
                            None => {
                                warnings.push(format!("drawing '{name}' not found"));
                                *b = json!({"type":"note","text":format!("[missing drawing {name}]")});
                            }
                        }
                    }
                    "image" => {
                        if let Some(p) = arg_str(b, "path") {
                            let abs = ctx.project.resolve(p)?;
                            b["path"] = json!(abs.display().to_string());
                        }
                        if b.get("caption").is_none() {
                            b["caption"] = json!("");
                        }
                    }
                    "note" | "paragraph" => {
                        if b.get("text").is_none() {
                            b["text"] = json!("");
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    let mut r: report::Report = serde_json::from_value(json!({
        "title": v.get("title").cloned().unwrap_or(json!("Calculation report")),
        "project": v.get("project").cloned().unwrap_or(json!(ctx.project.info.name)),
        "engineer": v.get("engineer").cloned().unwrap_or(json!("")),
        "checker": v.get("checker").cloned().unwrap_or(json!("")),
        "date": v.get("date").cloned().unwrap_or(json!(today())),
        "standards": v.get("standards").cloned().unwrap_or(json!([])),
        "sections": v.get("sections").cloned().unwrap_or(json!([])),
    }))
    .map_err(|e| format!("report structure: {e}"))?;
    if r.standards.is_empty() {
        r.standards = ctx.library.list().into_iter().filter(|m| ctx.project.info.standards.contains(&m.id)).map(|m| format!("{} {}", m.code, m.name)).collect();
    }
    if !warnings.is_empty() {
        r.sections.push(report::Section { heading: "Report warnings".into(), blocks: warnings.into_iter().map(|w| report::Block::Note { text: w }).collect() });
    }
    Ok(r)
}

pub fn today() -> String {
    // civil date from the Unix clock (UTC), no extra dependency
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let z = secs / 86_400 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}")
}

/// Asks a vision model (or Claude Code when none is set up) about an image.
/// An image as a model input; SVG drawings are rasterised first (models read PNG/JPEG only).
fn image_for_model(path: &std::path::Path) -> Result<th_llm::Part, String> {
    if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("svg")) {
        let svg = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let (png, _, _) = drawing::svg_to_png(&svg, 1600)?;
        let tmp = std::env::temp_dir().join(format!("ad_svg_{}.png", std::process::id()));
        std::fs::write(&tmp, png).map_err(|e| e.to_string())?;
        let part = th_llm::image_part(&tmp);
        let _ = std::fs::remove_file(&tmp);
        return part;
    }
    th_llm::image_part(path)
}

pub fn describe_image(settings: &Settings, path: &std::path::Path, question: &str) -> Result<String, String> {
    let q = if question.trim().is_empty() {
        "Describe this image for a civil/geotechnical engineer: what it shows, every dimension, level, load, soil layer, label and note you can read, exactly as written."
    } else {
        question
    };
    if let Some(p) = settings.provider(&settings.vision_provider).filter(|p| p.key().is_some() || p.base_url.contains("localhost")) {
        let msg = th_llm::Message::user_parts(vec![th_llm::Part::Text { text: q.into() }, image_for_model(path)?]);
        let r = th_llm::chat(p, &settings.vision_model, &[msg], &[], &th_llm::ChatOptions::default())?;
        return Ok(r.message.text());
    }
    // fallback: Claude Code reads images with its Read tool
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let dir = path.parent().unwrap_or(std::path::Path::new("."));
    let mut cfg = settings.claude_code.clone();
    cfg.args = vec!["--allowedTools".into(), "Read".into()];
    cfg.timeout_min = 5;
    let prompt = format!("Read the image file {} and answer: {q}", path.display());
    let out = crate::external::run(crate::external::Which::ClaudeCode, &cfg, &prompt, dir, dir, None, &cancel)
        .map_err(|e| format!("no vision model is set up (Settings → Vision) and Claude Code failed: {e}"))?;
    Ok(out.text)
}

pub fn exec_builtin(ctx: &Ctx, name: &str, a: &Value) -> Result<ToolOut, String> {
    match name {
        "list_standards" => {
            let all = ctx.library.list();
            if all.is_empty() {
                return Ok(ToolOut::text("The standards library is empty. Ask the engineer to import the standards (Standards tab) before citing any clause."));
            }
            let mut s = String::from("Standards in the library (* = chosen for this project):\n");
            for m in all {
                let on = if ctx.project.info.standards.contains(&m.id) { "*" } else { " " };
                s.push_str(&format!("{on} {} | {} | {} | id {} | {} chunks\n", m.code, m.name, m.discipline, m.id, m.chunks));
            }
            if ctx.project.info.standards.is_empty() {
                s.push_str("\nNo standard is chosen for this project yet; searches cover the whole library.");
            }
            Ok(ToolOut::text(s))
        }
        "search_standards" => {
            let q = need_str(a, "query")?;
            let k = a.get("k").and_then(|k| k.as_u64()).unwrap_or(6).clamp(1, 20) as usize;
            let ids: Vec<String> = match a.get("codes").and_then(|c| c.as_array()) {
                Some(codes) if !codes.is_empty() => {
                    let mut ids = vec![];
                    for c in codes.iter().filter_map(|c| c.as_str()) {
                        match resolve_standard(ctx.library, c) {
                            Some(id) => ids.push(id),
                            None => return Ok(ToolOut::text(format!("Standard '{c}' is not in the library. Available: {}", ctx.library.list().iter().map(|m| m.code.clone()).collect::<Vec<_>>().join(", ")))),
                        }
                    }
                    ids
                }
                _ => ctx.project.info.standards.clone(),
            };
            let hits = ctx.library.search(q, &ids, k)?;
            Ok(ToolOut::text(hits_text(&hits)))
        }
        "get_clause" => {
            let code = need_str(a, "code")?;
            let clause = need_str(a, "clause")?;
            let id = resolve_standard(ctx.library, code).ok_or_else(|| format!("standard '{code}' is not in the library"))?;
            let hits = ctx.library.clause(&id, clause);
            if hits.is_empty() {
                return Ok(ToolOut::text(format!("Clause {clause} was not found in {code}. Try search_standards with words from the clause.")));
            }
            Ok(ToolOut::text(clip(&hits_text(&hits), 16_000)))
        }
        "list_design_modules" => {
            let mods: Vec<Value> = design::modules().into_iter().map(|m| json!({"name":m.name,"title":m.title,"standard":m.standard,"description":m.description,"input_schema":m.input_schema})).collect();
            Ok(ToolOut::text(serde_json::to_string_pretty(&mods).unwrap_or_default()))
        }
        "run_design_module" => {
            let module = need_str(a, "module")?;
            let args = a.get("args").cloned().unwrap_or(json!({}));
            let d = design::run_module(module, &args)?;
            let name = slug(arg_str(a, "name").unwrap_or(module));
            let mut arts = save_sheet(ctx, &name, &d.sheet)?;
            if let Some(dr) = &d.drawing {
                arts.extend(save_drawing(ctx, &name, dr)?);
            }
            let verdict = if d.pass { "ALL CHECKS PASS" } else { "SOME CHECKS FAIL" };
            Ok(ToolOut { text: format!("{verdict}\n{}\nSaved calcs/{name}.json{}\n\n{}", d.summary, if d.drawing.is_some() { format!(", drawings/{name}.svg/.dxf") } else { String::new() }, clip(&d.sheet.to_markdown(), 12_000)), artifacts: arts })
        }
        "calc_sheet" => {
            let input: calc::CalcInput = serde_json::from_value(a.clone()).map_err(|e| format!("calc_sheet arguments: {e}"))?;
            let sheet = calc::run_sheet(&input)?;
            let name = slug(arg_str(a, "name").unwrap_or(&input.title));
            let arts = save_sheet(ctx, &name, &sheet)?;
            let verdict = if sheet.all_pass { "all checks pass" } else { "SOME CHECKS FAIL" };
            Ok(ToolOut { text: format!("Saved calcs/{name} ({verdict}).\n\n{}", clip(&sheet.to_markdown(), 12_000)), artifacts: arts })
        }
        "make_drawing" => {
            let d: drawing::Drawing = serde_json::from_value(a.get("drawing").cloned().unwrap_or(Value::Null)).map_err(|e| format!("drawing: {e}"))?;
            let name = slug(arg_str(a, "name").unwrap_or(&d.title));
            let arts = save_drawing(ctx, &name, &d)?;
            Ok(ToolOut { text: format!("Saved drawings/{name}.svg, .dxf and .json ({} entities).", d.entities.len()), artifacts: arts })
        }
        "write_report" => {
            // append=true adds these sections to the report of the same name (long reports in parts)
            let name0 = slug(arg_str(a, "name").unwrap_or("report"));
            let draft = ctx.project.folder(ArtifactKind::Report).join(format!(".{name0}.draft.json"));
            let mut a = a.clone();
            if a.get("append").and_then(|v| v.as_bool()).unwrap_or(false) {
                if let Some(prev) = std::fs::read_to_string(&draft).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok()) {
                    let mut secs = prev.get("sections").and_then(|s| s.as_array()).cloned().unwrap_or_default();
                    secs.extend(a.get("sections").and_then(|s| s.as_array()).cloned().unwrap_or_default());
                    for k in ["title", "engineer", "checker", "standards", "project", "date"] {
                        if a.get(k).is_none() {
                            if let Some(v) = prev.get(k) {
                                a[k] = v.clone();
                            }
                        }
                    }
                    a["sections"] = Value::Array(secs);
                }
            }
            if a.get("title").is_none() {
                a["title"] = json!("Calculation report");
            }
            save_text(&draft, &a.to_string())?;
            let a = &a;
            let r = resolve_report(ctx, a)?;
            let name = slug(arg_str(a, "name").unwrap_or(&r.title));
            let dir = ctx.project.folder(ArtifactKind::Report);
            let pdf = dir.join(format!("{name}.pdf"));
            let sum = report::write_pdf(&r, &pdf)?;
            save_text(&dir.join(format!("{name}.md")), &report::to_markdown(&r))?;
            Ok(ToolOut { text: format!("Wrote reports/{name}.pdf ({} pages; sections: {}) and reports/{name}.md.", sum.pages, sum.sections.join(", ")), artifacts: vec![(ArtifactKind::Report, pdf)] })
        }
        "list_files" => {
            let mut s = String::new();
            for art in ctx.project.artifacts() {
                let files: Vec<String> = art.files.iter().map(|f| ctx.project.rel(f)).collect();
                s.push_str(&format!("{:?}: {}\n", art.kind, files.join(", ")));
            }
            if s.is_empty() {
                s = "The project has no files yet.".into();
            }
            Ok(ToolOut::text(s))
        }
        "read_file" => {
            let p = ctx.project.resolve(need_str(a, "path")?)?;
            let max = a.get("max_chars").and_then(|m| m.as_u64()).unwrap_or(40_000).clamp(1000, 200_000) as usize;
            if is_image(&p) {
                return Ok(ToolOut::text(describe_image(ctx.settings, &p, "")?));
            }
            Ok(ToolOut::text(read_text(&p, max)?))
        }
        "write_file" => {
            let rel = need_str(a, "path")?;
            let p = ctx.project.resolve(&format!("work/{}", rel.trim_start_matches("work/")))?;
            let content = a.get("content").and_then(|c| c.as_str()).unwrap_or("");
            save_text(&p, content)?;
            Ok(ToolOut { text: format!("Wrote {} ({} bytes).", ctx.project.rel(&p), content.len()), artifacts: vec![(ArtifactKind::Work, p)] })
        }
        "read_skill" => {
            let n = need_str(a, "name")?;
            let skill = ctx.skills.iter().find(|s| s.name == n).ok_or_else(|| format!("no skill '{n}'; available: {}", ctx.skills.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join(", ")))?;
            if let Some(f) = arg_str(a, "file") {
                let p = skill.dir.join(f);
                if !p.starts_with(&skill.dir) || f.contains("..") {
                    return Err("file must be inside the skill folder".into());
                }
                return Ok(ToolOut::text(read_text(&p, 60_000)?));
            }
            let files = skill.files();
            Ok(ToolOut::text(format!("{}\n\n--- files in this skill: {}", clip(&skill.body()?, 60_000), files.join(", "))))
        }
        "describe_image" => {
            let p = ctx.project.resolve(need_str(a, "path")?)?;
            Ok(ToolOut::text(describe_image(ctx.settings, &p, arg_str(a, "question").unwrap_or(""))?))
        }
        "run_python" => run_python(ctx, a),
        other => Err(format!("unknown tool '{other}'")),
    }
}

/// Finds the Python interpreter: the one set in Settings, else python3 on PATH or in the usual
/// install folders (a window started from the Dock does not get the shell's PATH).
pub fn find_python(settings: &Settings) -> Option<PathBuf> {
    if !settings.python.trim().is_empty() {
        let p = PathBuf::from(settings.python.trim());
        return p.is_file().then_some(p);
    }
    let home = dirs::home_dir().unwrap_or_default();
    let mut dirs: Vec<PathBuf> = std::env::var("PATH").unwrap_or_default().split(':').filter(|s| !s.is_empty()).map(PathBuf::from).collect();
    for d in ["miniconda3/bin", "miniforge3/bin", "anaconda3/bin", "mambaforge/bin", ".pyenv/shims", ".local/bin"] {
        dirs.push(home.join(d));
    }
    dirs.extend(["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"].map(PathBuf::from));
    for name in ["python3", "python"] {
        if let Some(p) = dirs.iter().map(|d| d.join(name)).find(|p| p.is_file()) {
            return Some(p);
        }
    }
    None
}

/// Modification times of the project's files (hidden files skipped), to report what a script wrote.
fn snapshot(dir: &std::path::Path) -> std::collections::HashMap<PathBuf, std::time::SystemTime> {
    let mut m = std::collections::HashMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.') || n == "__pycache__") {
                continue;
            }
            if p.is_dir() {
                stack.push(p);
            } else if let Ok(t) = e.metadata().and_then(|m| m.modified()) {
                m.insert(p, t);
            }
            if m.len() > 20_000 {
                return m;
            }
        }
    }
    m
}

fn run_python(ctx: &Ctx, a: &Value) -> Result<ToolOut, String> {
    use std::io::{Read, Write};
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    let python = find_python(ctx.settings).ok_or("Python 3 was not found; install it or set its path in Settings (Python interpreter)")?;
    let work = ctx.project.folder(ArtifactKind::Work);
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let script = if let Some(code) = a.get("code").and_then(|c| c.as_str()).filter(|c| !c.trim().is_empty()) {
        let name = slug(arg_str(a, "name").unwrap_or("script").trim_end_matches(".py"));
        let p = work.join("py").join(format!("{name}.py"));
        save_text(&p, code)?;
        p
    } else if let Some(sk) = arg_str(a, "skill") {
        let f = need_str(a, "file")?;
        let skill = ctx.skills.iter().find(|s| s.name == sk).ok_or_else(|| format!("no skill '{sk}'"))?;
        if f.contains("..") || std::path::Path::new(f).is_absolute() {
            return Err("file must be inside the skill folder".into());
        }
        let p = skill.dir.join(f);
        if !p.is_file() {
            return Err(format!("skill '{sk}' has no file '{f}'; files: {}", skill.files().join(", ")));
        }
        p
    } else {
        return Err("give `code`, or `skill` and `file`".into());
    };
    let args: Vec<String> = a.get("args").and_then(|v| v.as_array()).map(|v| v.iter().map(|x| x.as_str().map(str::to_string).unwrap_or_else(|| x.to_string())).collect()).unwrap_or_default();
    let limit = a.get("timeout_s").and_then(|t| t.as_u64()).unwrap_or(120).clamp(5, 900);
    let stdin_text = a.get("stdin").and_then(|s| s.as_str()).unwrap_or("").to_string();

    let before = snapshot(&ctx.project.dir);
    let t0 = Instant::now();
    let mut child = Command::new(&python)
        .arg(&script)
        .args(&args)
        .current_dir(&work)
        .env("MPLBACKEND", "Agg")
        .env("PYTHONIOENCODING", "utf-8")
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .env("PROJECT_DIR", &ctx.project.dir)
        .stdin(if stdin_text.is_empty() { Stdio::null() } else { Stdio::piped() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot start {}: {e}", python.display()))?;
    if let Some(mut si) = child.stdin.take() {
        std::thread::spawn(move || {
            let _ = si.write_all(stdin_text.as_bytes());
        });
    }
    let mut out_pipe = child.stdout.take().ok_or("no stdout")?;
    let mut err_pipe = child.stderr.take().ok_or("no stderr")?;
    let out_t = std::thread::spawn(move || {
        let mut b = vec![];
        let _ = out_pipe.read_to_end(&mut b);
        String::from_utf8_lossy(&b).into_owned()
    });
    let err_t = std::thread::spawn(move || {
        let mut b = vec![];
        let _ = err_pipe.read_to_end(&mut b);
        String::from_utf8_lossy(&b).into_owned()
    });
    let status = loop {
        if let Some(st) = child.try_wait().map_err(|e| e.to_string())? {
            break Some(st);
        }
        if t0.elapsed() > Duration::from_secs(limit) {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let stdout = out_t.join().unwrap_or_default();
    let stderr = err_t.join().unwrap_or_default();
    let secs = t0.elapsed().as_secs_f64();

    let mut changed: Vec<PathBuf> = snapshot(&ctx.project.dir).into_iter().filter(|(p, t)| before.get(p) != Some(t)).map(|(p, _)| p).collect();
    changed.sort();
    let artifacts: Vec<(ArtifactKind, PathBuf)> = changed.iter().map(|p| (if is_image(p) { ArtifactKind::Image } else { ArtifactKind::Work }, p.clone())).collect();

    let tail = |s: &str, n: usize| -> String {
        let c: Vec<char> = s.chars().collect();
        if c.len() <= n { s.to_string() } else { format!("…[{} earlier characters cut]\n{}", c.len() - n, c[c.len() - n..].iter().collect::<String>()) }
    };
    let shown = if script.starts_with(&ctx.project.dir) { ctx.project.rel(&script) } else { script.display().to_string() };
    let mut text = match status {
        Some(st) if st.success() => format!("{shown} finished in {secs:.1} s."),
        Some(st) => format!("{shown} FAILED (exit code {}) after {secs:.1} s.", st.code().map(|c| c.to_string()).unwrap_or_else(|| "signal".into())),
        None => format!("{shown} was stopped after the {limit} s time limit."),
    };
    if !stdout.trim().is_empty() {
        text.push_str(&format!("\n\n--- output ---\n{}", tail(stdout.trim_end(), 20_000)));
    }
    if !stderr.trim().is_empty() {
        text.push_str(&format!("\n\n--- errors and warnings ---\n{}", tail(stderr.trim_end(), 6_000)));
    }
    if !changed.is_empty() {
        let list: Vec<String> = changed.iter().map(|p| ctx.project.rel(p)).collect();
        text.push_str(&format!("\n\nFiles written: {}", list.join(", ")));
    }
    if status.is_some_and(|s| !s.success()) || status.is_none() {
        return Err(text);
    }
    Ok(ToolOut { text, artifacts })
}

#[cfg(test)]
mod tests {
    #[test]
    fn date_is_iso() {
        let d = super::today();
        assert_eq!(d.len(), 10);
        assert!(d.starts_with("20"));
    }

    #[test]
    fn run_python_runs_code_and_reports_files() {
        use super::*;
        let settings = Settings::default();
        if find_python(&settings).is_none() {
            eprintln!("no python3 here; skipped");
            return;
        }
        let d = std::env::temp_dir().join(format!("adpy_{}", std::process::id()));
        let project = Project::create(&d, crate::project::ProjectInfo::default()).unwrap();
        let library = Library::open(&d.join("kb")).unwrap();
        let ctx = Ctx { project: &project, library: &library, skills: &[], settings: &settings };

        let ok = exec_builtin(&ctx, "run_python", &json!({"code": "import sys\nq = 2.0*9.81\nprint(f'q = {q:.2f} kPa', sys.argv[1])\nopen('out.csv','w').write('a,b\\n1,2\\n')", "name": "t1", "args": ["x"]})).unwrap();
        assert!(ok.text.contains("q = 19.62 kPa x"), "{}", ok.text);
        assert!(ok.text.contains("work/out.csv"), "{}", ok.text);
        assert!(d.join("work/py/t1.py").is_file());

        let bad = exec_builtin(&ctx, "run_python", &json!({"code": "raise ValueError('boom')"})).unwrap_err();
        assert!(bad.contains("FAILED") && bad.contains("boom"), "{bad}");

        let slow = exec_builtin(&ctx, "run_python", &json!({"code": "import time\ntime.sleep(30)", "timeout_s": 5})).unwrap_err();
        assert!(slow.contains("time limit"), "{slow}");
        let _ = std::fs::remove_dir_all(&d);
    }
}
