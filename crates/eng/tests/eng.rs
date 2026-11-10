//! Integration tests: each design module against an independent reference, every drawing
//! through SVG, PNG and DXF, and a full PDF report.

use th_eng::{calc, design, drawing, report};
use serde_json::json;

fn close(got: f64, want: f64, rel: f64, what: &str) {
    assert!(((got - want) / want).abs() <= rel, "{what}: got {got}, expected {want} (rel tol {rel})");
}

fn val(d: &design::Design, k: &str) -> f64 {
    d.sheet.value(k).unwrap_or_else(|| panic!("no value {k} in sheet"))
}

/// Hand calculation, b = 300, h = 500, cover 40, stirrup 10, 3-DB20, f'c 28, fy 420:
///   d   = 500 - 40 - 10 - 20/2                  = 440 mm
///   As  = 3 * pi * 20^2 / 4                     = 942.48 mm2
///   a   = 942.48 * 420 / (0.85 * 28 * 300)      = 55.44 mm,  beta1 = 0.85, c = 65.22 mm
///   eps_t = 0.003 (440 - 65.22) / 65.22         = 0.01724 > 0.005 -> phi = 0.90
///   Mn  = 942.48 * 420 * (440 - 27.72) / 1e6    = 163.20 kN m,  phi Mn = 146.88 kN m
/// Shear, 2-leg 10 mm stirrups at 150, fyt 420:
///   rho_w = 942.48 / (300 * 440) = 0.007140, rho_w^(1/3) = 0.19258
///   Vc(a) = 0.17 sqrt(28) 300 440 / 1000        = 118.74 kN
///   Vc(b) = 0.66 * 0.19258 * sqrt(28) * 132000  = 88.78 kN  -> Vc = 118.74 kN (< 0.42 limit 293.4)
///   Vs  = 157.08 * 420 * 440 / 150 / 1000       = 193.52 kN, phi Vn = 0.75 (312.26) = 234.20 kN
///   As,min = max(0.25 sqrt(28)/420, 1.4/420) * 300 * 440 = 440 mm2
#[test]
fn rc_beam_matches_hand_calculation() {
    let d = design::run_module(
        "rc_beam_aci318",
        &json!({"b": 300, "h": 500, "cover": 40, "stirrup_dia": 10, "bar_dia": 20, "n_bars": 3,
                "fc": 28, "fy": 420, "fyt": 420, "Mu": 120, "Vu": 150, "legs": 2, "s": 150}),
    )
    .unwrap();
    close(val(&d, "d"), 440.0, 1e-9, "d");
    close(val(&d, "a"), 55.438, 1e-4, "a");
    close(val(&d, "eps_t"), 0.017239, 1e-3, "eps_t");
    close(val(&d, "phi_f"), 0.90, 1e-9, "phi");
    close(val(&d, "M_n"), 163.20, 1e-3, "Mn");
    close(val(&d, "phi_Mn"), 146.88, 1e-3, "phi Mn");
    close(val(&d, "A_smin"), 440.0, 1e-6, "As,min");
    close(val(&d, "V_c"), 118.74, 1e-3, "Vc");
    close(val(&d, "V_s"), 193.52, 1e-3, "Vs");
    close(val(&d, "phi_Vn"), 234.20, 1e-3, "phi Vn");
    assert!(d.pass, "{}", d.summary);
    // the workings carry the numbers
    let step = d.sheet.steps.iter().find(|s| s.symbol == "a").unwrap();
    assert_eq!(step.substituted, "942.5*420/(0.85*28*300)");

    // overload -> fails; designed spacing -> multiple of 25 within s_max
    let f = design::run_module("rc_beam_aci318", &json!({"h": 500, "n_bars": 3, "Mu": 200, "Vu": 150})).unwrap();
    assert!(!f.pass);
    let s = val(&f, "s");
    assert!(s > 0.0 && (s / 25.0).fract() == 0.0 && s <= val(&f, "s_max"));
    // missing input is an error, not a panic
    assert!(design::run_module("rc_beam_aci318", &json!({"h": 500})).unwrap_err().contains("n_bars"));
}

/// Das, Principles of Foundation Engineering, Example 3.1 (as reproduced by the
/// terzaghi-bearing-capacity skill): 1.5 m square footing, D = 1 m, phi = 20, c = 15.2 kPa,
/// gamma = 17.8, FS = 4 -> q_ult = 520.84 kPa, Q_all = 293.0 kN (Das ~293 kN).
#[test]
fn terzaghi_das_example_3_1() {
    let d = design::run_module(
        "bearing_terzaghi",
        &json!({"shape": "square", "B": 1.5, "D": 1.0, "phi": 20, "c": 15.2, "gamma": 17.8, "FS": 4}),
    )
    .unwrap();
    close(val(&d, "N_c"), 17.69, 1e-3, "Nc");
    close(val(&d, "N_q"), 7.44, 1e-3, "Nq");
    close(val(&d, "N_g"), 3.64, 1e-9, "Ngamma");
    close(val(&d, "q_ult"), 520.84, 1e-4, "q_ult");
    close(val(&d, "Q_all"), 293.0, 5e-4, "Q_all");

    // skill Example 2 (water table case I) and Example 3 (local shear, rect)
    let e2 = design::run_module("bearing_terzaghi", &json!({"B": 2, "D": 1.5, "phi": 30, "gamma": 17, "gamma_sat": 19.5, "z_w": 1.0})).unwrap();
    close(val(&e2, "q_ult"), 675.92, 1e-4, "example 2 q_ult");
    let e3 = design::run_module("bearing_terzaghi", &json!({"shape": "rect", "B": 2, "L": 4, "D": 1, "phi": 28, "c": 5, "gamma": 17, "local_shear": true})).unwrap();
    close(val(&e3, "q_ult"), 237.40, 1e-3, "example 3 q_ult");
    let e4 = design::run_module("bearing_terzaghi", &json!({"B": 2, "D": 1, "phi": 0, "c": 50, "gamma": 18})).unwrap();
    close(val(&e4, "q_ult"), 303.62, 1e-4, "example 4 q_ult");
}

/// Cross-check with the ec7-shallow-foundation skill script (bearing_capacity.py) with its
/// depth factors switched off, because Annex D has none:
/// B 2 x L 3 m, D 1 m, phi'k 30, c'k 5 kPa, gamma 19, Gk 800, Qk 300 kN.
///   DA1-C1: Vd 1530, q_ult 975.4, Rd 5852.4    DA1-C2: Vd 1190, q_ult 492.9, Rd 2957.6
///   DA2:    Vd 1530, Rd 4180.3
/// Undrained cu 60 kPa, DA1-C1: Rd 2211.1 (script uses Nc = 5.14, here pi + 2).
#[test]
fn ec7_matches_skill_script() {
    let d = design::run_module(
        "bearing_ec7",
        &json!({"B": 2, "L": 3, "D": 1, "phi_k": 30, "c_k": 5, "gamma": 19, "Gk": 800, "Qk": 300}),
    )
    .unwrap();
    close(val(&d, "V_d1"), 1530.0, 1e-9, "Vd1");
    close(val(&d, "V_d2"), 1190.0, 1e-9, "Vd2");
    close(val(&d, "qd_1"), 975.4, 1e-3, "q_ult C1");
    close(val(&d, "qd_2"), 492.9, 1e-3, "q_ult C2");
    close(val(&d, "R_d1"), 5852.4, 1e-3, "Rd C1");
    close(val(&d, "R_d2"), 2957.6, 1e-3, "Rd C2");
    close(val(&d, "R_d3"), 4180.3, 1e-3, "Rd DA2");
    assert!(d.pass);
    let u = design::run_module("bearing_ec7", &json!({"B": 2, "L": 3, "D": 1, "cu_k": 60, "gamma": 19, "Gk": 800, "Qk": 300})).unwrap();
    close(val(&u, "R_u1"), 2211.1, 1e-3, "undrained Rd C1");
    // M2 uses gamma_cu = 1.4 for cu (the script uses 1.25 here, a known slip in the skill)
    close(val(&u, "cu_d2"), 60.0 / 1.4, 1e-9, "cu,d M2");
}

/// Hand check: H 5, t_top 0.3, t_bot 0.5, t_base 0.6, toe 0.8, heel 2.5, gamma_c 24,
/// backfill 18 / 30 deg, base soil 19 / 20 deg / c 40, D 1.2.
///   H' = 5.6, B = 3.8, Ka = 1/3, Pa = 0.5 * 18 * 5.6^2 / 3 = 94.08 kN/m, Mo = 94.08 * 5.6/3 = 175.62
///   W1 = 0.3*5*24 = 36 @ 1.15 -> 41.40;  W2 = 0.5*0.2*5*24 = 12 @ 0.9333 -> 11.20
///   W3 = 3.8*0.6*24 = 54.72 @ 1.9 -> 103.97;  W4 = 2.5*5*18 = 225 @ 2.55 -> 573.75
///   SV = 327.72, MR = 730.32 -> FS_ot = 4.159
///   Sliding: (327.72 tan 13.33 + 3.8 * 26.67) / 94.08 = (77.67 + 101.33) / 94.08 = 1.903
#[test]
fn retaining_wall_hand_check() {
    let d = design::run_module(
        "retaining_wall_cantilever",
        &json!({"H": 5, "t_top": 0.3, "t_bot": 0.5, "t_base": 0.6, "toe": 0.8, "heel": 2.5,
                "gamma1": 18, "phi1": 30, "gamma2": 19, "phi2": 20, "c2": 40, "D": 1.2}),
    )
    .unwrap();
    close(val(&d, "P_a"), 94.08, 1e-6, "Pa");
    close(val(&d, "SV"), 327.72, 1e-6, "SV");
    close(val(&d, "M_R"), 730.318, 1e-5, "MR");
    close(val(&d, "FS_ot"), 4.1586, 1e-4, "FS overturning");
    close(val(&d, "FS_sl"), 1.9027, 1e-3, "FS sliding");
    let e = val(&d, "e");
    close(val(&d, "q_max"), 327.72 / 3.8 * (1.0 + 6.0 * e.abs() / 3.8), 1e-9, "q_max");
    assert!(val(&d, "FS_bc") > 0.0);
    // water table and surcharge paths run and lower the safety factors
    let w = design::run_module(
        "retaining_wall_cantilever",
        &json!({"H": 5, "toe": 0.8, "heel": 2.5, "phi2": 20, "c2": 40, "D": 1.2, "q": 10,
                "water_table": true, "z_w": 3.0, "passive": true}),
    )
    .unwrap();
    assert!(val(&w, "FS_ot") < val(&d, "FS_ot"));
}

#[test]
fn every_module_draws_svg_png_dxf() {
    let cases = [
        ("rc_beam_aci318", json!({"h": 500, "n_bars": 4, "Mu": 150, "Vu": 120})),
        ("bearing_terzaghi", json!({"shape": "square", "B": 1.5, "D": 1, "phi": 20, "c": 15.2, "gamma": 17.8, "gamma_sat": 19, "z_w": 1.5})),
        ("bearing_ec7", json!({"B": 2, "strip": true, "D": 1, "phi_k": 32, "gamma": 19, "Gk": 300, "Qk": 100, "e_B": 0.1})),
        ("retaining_wall_cantilever", json!({"H": 5, "toe": 0.8, "heel": 2.5, "D": 1.2, "q": 10})),
    ];
    assert_eq!(design::modules().len(), cases.len());
    for (name, args) in cases {
        let d = design::run_module(name, &args).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(!d.sheet.steps.is_empty() && !d.summary.is_empty());
        let dr = d.drawing.as_ref().expect("drawing");
        let svg = drawing::to_svg(dr, 900);
        assert!(svg.starts_with("<svg") && svg.ends_with("</svg>"), "{name}");
        let (png, w, h) = drawing::svg_to_png(&svg, 900).unwrap();
        assert!(png.starts_with(b"\x89PNG") && w == 900 && h > 0, "{name}");
        let dxf = drawing::to_dxf(dr);
        let lines: Vec<&str> = dxf.lines().map(|l| l.trim()).collect();
        let count = |s: &str| lines.iter().filter(|l| **l == s).count();
        assert_eq!(count("SECTION"), 4, "{name}");
        assert_eq!(count("SECTION"), count("ENDSEC"), "{name}");
        assert_eq!(count("TABLE"), count("ENDTAB"), "{name}");
        assert_eq!(count("POLYLINE"), count("SEQEND"), "{name}");
        assert_eq!(lines.last(), Some(&"EOF"), "{name}");
        assert_eq!(lines.len() % 2, 0, "{name}: group code / value pairs");
        // the schema lists required inputs
        let info = design::modules().into_iter().find(|m| m.name == name).unwrap();
        assert!(info.input_schema["properties"].is_object() && info.input_schema["required"].is_array());
        // round trip through serde
        let back: design::Design = serde_json::from_str(&serde_json::to_string(&d).unwrap()).unwrap();
        assert_eq!(back, d);
    }
}

#[test]
fn calc_input_tolerates_omitted_and_extra_fields() {
    let inp: calc::CalcInput = serde_json::from_value(json!({
        "name": "extra field from the tool call",
        "title": "Punching",
        "inputs": [{"symbol": "V", "value": 100}],
        "steps": [{"symbol": "V2", "expr": "2*V"}]
    }))
    .unwrap();
    let s = calc::run_sheet(&inp).unwrap();
    assert_eq!(s.steps[0].value, 200.0);
    assert!(s.all_pass && s.notes.is_empty());
}

#[test]
fn full_report_pdf_and_markdown() {
    let dir = std::env::temp_dir().join(format!("th_eng_report_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let beam = design::run_module("rc_beam_aci318", &json!({"h": 500, "n_bars": 3, "Mu": 200, "Vu": 150})).unwrap();
    let wall = design::run_module("retaining_wall_cantilever", &json!({"H": 5, "toe": 0.8, "heel": 2.5, "D": 1.2})).unwrap();
    let dr = beam.drawing.clone().unwrap();
    let svg = drawing::to_svg(&dr, 600);
    let (png, _, _) = drawing::svg_to_png(&svg, 600).unwrap();
    let img = dir.join("beam.png");
    std::fs::write(&img, png).unwrap();
    let r = report::Report {
        title: "Design calculations".into(),
        project: "Test building / อาคารทดสอบ".into(),
        engineer: "A. Engineer".into(),
        checker: "B. Checker".into(),
        date: "2026-11-10".into(),
        standards: vec!["ACI 318-19".into(), "EN 1997-1:2004".into()],
        sections: vec![
            report::Section {
                heading: "Beam B1".into(),
                blocks: vec![
                    report::Block::Paragraph { text: "Beam at grid A, ข้อความภาษาไทย. ".repeat(20) },
                    report::Block::Note { text: "Loads from the frame analysis.".into() },
                    report::Block::Table { headers: vec!["Case".into(), "Mu (kN·m)".into()], rows: vec![vec!["1.2D+1.6L".into(), "200".into()]] },
                    report::Block::Sheet { sheet: beam.sheet.clone() },
                    report::Block::Drawing { drawing: dr, caption: "Section B1".into() },
                ],
            },
            report::Section {
                heading: "Retaining wall".into(),
                blocks: vec![
                    report::Block::Sheet { sheet: wall.sheet.clone() },
                    report::Block::Image { path: img.display().to_string(), caption: "Image block".into() },
                    report::Block::Svg { svg, caption: "SVG block".into() },
                ],
            },
        ],
    };
    let path = dir.join("report.pdf");
    let sum = report::write_pdf(&r, &path).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    assert!(bytes.starts_with(b"%PDF"));
    assert!(sum.pages > 1, "pages {}", sum.pages);
    assert_eq!(sum.sections, vec!["Beam B1".to_string(), "Retaining wall".to_string()]);
    let md = report::to_markdown(&r);
    assert!(md.contains("## Beam B1") && md.contains("FAIL") && md.contains("ACI 318-19"));
    // JSON form used by the agents
    let back: report::Report = serde_json::from_value(serde_json::to_value(&r).unwrap()).unwrap();
    assert_eq!(back, r);
    eprintln!("report written to {} ({} pages)", sum.path, sum.pages);
}
