//! Terzaghi (1943) ultimate and allowable bearing capacity of a shallow footing, as taught
//! from Das, Principles of Foundation Engineering. Nc and Nq from the closed forms; Nγ from
//! the Kumbhojkar (1993) solution, interpolated linearly between whole degrees. Mirrors the
//! verified `terzaghi-bearing-capacity` skill script.

use serde_json::{json, Value};

use super::{check_summary, flag, non_negative, num, opt_num, positive, prop, schema, text, Design, ModuleInfo, SheetBuilder};
use crate::drawing::{Drawing, Entity};
use crate::expr::fmt4;

/// Terzaghi Nγ (Kumbhojkar 1993), φ = 0..50° in 1° steps.
pub(crate) const NGAMMA: [f64; 51] = [
    0.00, 0.01, 0.04, 0.06, 0.10, 0.14, 0.20, 0.27, 0.35, 0.44, // 0-9
    0.56, 0.69, 0.85, 1.04, 1.26, 1.52, 1.82, 2.18, 2.59, 3.07, // 10-19
    3.64, 4.31, 5.09, 6.00, 7.08, 8.34, 9.84, 11.60, 13.70, 16.18, // 20-29
    19.13, 22.65, 26.87, 31.94, 38.04, 45.41, 54.36, 65.27, 78.61, 95.03, // 30-39
    115.31, 140.51, 171.99, 211.56, 261.60, 325.34, 407.11, 512.84, 650.67, 831.99, // 40-49
    1072.80, // 50
];

pub(super) fn info() -> ModuleInfo {
    ModuleInfo {
        name: "bearing_terzaghi".into(),
        title: "Bearing capacity of a shallow footing (Terzaghi 1943)".into(),
        standard: "Terzaghi (1943); Das, Principles of Foundation Engineering".into(),
        description: "Ultimate, net and allowable bearing pressure and allowable gross load for strip, square, circular or \
            rectangular footings; general or local shear (c' = 2/3 c, tan phi' = 2/3 tan phi); water table cases I-III (Das); \
            factor of safety on the gross ultimate pressure. Optional applied load Q is checked against Q_all."
            .into(),
        input_schema: schema(
            vec![
                ("shape", prop("string", "strip | square | circle | rect", Some(json!("strip")))),
                ("B", prop("number", "width, diameter or shorter side (m)", None)),
                ("L", prop("number", "length (m), rect only, L >= B", None)),
                ("D", prop("number", "depth of the base below ground Df (m)", Some(json!(0)))),
                ("phi", prop("number", "friction angle phi (deg), 0-50", None)),
                ("c", prop("number", "cohesion c or undrained strength cu (kPa)", Some(json!(0)))),
                ("gamma", prop("number", "moist unit weight above the water table (kN/m3)", None)),
                ("gamma_sat", prop("number", "saturated unit weight (kN/m3), needed when the water table is within D + B", None)),
                ("z_w", prop("number", "water table depth below ground surface (m); omit when deep", None)),
                ("FS", prop("number", "factor of safety on q_ult", Some(json!(3.0)))),
                ("local_shear", prop("boolean", "use Terzaghi's local shear reduction", Some(json!(false)))),
                ("Q", prop("number", "applied gross load to check (kN, or kN/m for a strip)", None)),
            ],
            &["B", "phi", "gamma"],
        ),
    }
}

pub(super) fn run(a: &Value) -> Result<Design, String> {
    let shape = text(a, "shape", "strip")?;
    let shape = match shape.as_str() {
        "strip" | "square" | "circle" | "rect" => shape,
        "circular" => "circle".into(),
        "rectangle" | "rectangular" => "rect".into(),
        other => return Err(format!("shape must be strip, square, circle or rect (got '{other}')")),
    };
    let b = positive("B", num(a, "B", None)?)?;
    let l = opt_num(a, "L")?;
    let d = non_negative("D", num(a, "D", Some(0.0))?)?;
    let phi = num(a, "phi", None)?;
    if !(0.0..=50.0).contains(&phi) {
        return Err("phi must be between 0 and 50 degrees (range of the N_gamma table)".into());
    }
    let c = non_negative("c", num(a, "c", Some(0.0))?)?;
    let gamma = positive("gamma", num(a, "gamma", None)?)?;
    let gamma_sat = opt_num(a, "gamma_sat")?;
    let z_w = opt_num(a, "z_w")?;
    let fs = positive("FS", num(a, "FS", Some(3.0))?)?;
    let local = flag(a, "local_shear", false)?;
    let q_app = opt_num(a, "Q")?;

    let mut sb = SheetBuilder::new("Bearing capacity, Terzaghi (1943)", "Terzaghi (1943); Das, Principles of Foundation Engineering");
    sb.input("B", b, "m", if shape == "circle" { "footing diameter" } else { "footing width" });
    let l = if shape == "rect" {
        let l = l.ok_or("a rectangular footing needs L")?;
        if l < b {
            return Err("B must be the shorter side (B <= L)".into());
        }
        sb.input("L", l, "m", "footing length");
        Some(l)
    } else {
        None
    };
    sb.input("D", d, "m", "depth of the base");
    sb.input("phi", phi, "deg", "friction angle");
    sb.input("c", c, "kPa", "cohesion");
    sb.input("gamma", gamma, "kN/m³", "unit weight above the water table");
    sb.input("FS", fs, "-", "factor of safety");

    // strength used: local shear reduces c and tan(phi) by 2/3
    if local {
        sb.step("c_u", "2/3*c", "kPa", "cohesion used (local shear)", "Terzaghi (1943) local shear")?;
        sb.step("phi_u", "atan(2/3*tand(phi))*180/pi", "deg", "friction angle used (local shear)", "Terzaghi (1943) local shear")?;
    } else {
        sb.step("c_u", "c", "kPa", "cohesion used (general shear)", "Terzaghi (1943)")?;
        sb.step("phi_u", "phi", "deg", "friction angle used (general shear)", "Terzaghi (1943)")?;
    }
    let phi_u = sb.get("phi_u");
    sb.step("phi_r", "phi_u*pi/180", "rad", "friction angle in radians", "")?;
    sb.step("N_q", "exp(2*(0.75*pi - phi_r/2)*tan(phi_r))/(2*cos(pi/4 + phi_r/2)^2)", "-", "bearing capacity factor Nq", "Terzaghi (1943) closed form")?;
    if phi_u < 1e-9 {
        sb.step("N_c", "1.5*pi + 1", "-", "bearing capacity factor Nc at phi = 0", "Terzaghi (1943), limit of (Nq-1)cot(phi)")?;
    } else {
        sb.step("N_c", "(N_q - 1)/tan(phi_r)", "-", "bearing capacity factor Nc", "Terzaghi (1943) closed form")?;
    }
    let i = (phi_u.floor() as usize).min(49);
    let ng_expr = if phi_u >= 50.0 { format!("{:.2}", NGAMMA[50]) } else { format!("{:.2} + (phi_u - {i})*({:.2} - {:.2})", NGAMMA[i], NGAMMA[i + 1], NGAMMA[i]) };
    sb.step("N_g", &ng_expr, "-", "bearing capacity factor N_gamma, linear between whole degrees", "Kumbhojkar (1993), Das Table 3.1/4.1")?;

    // shape multipliers on the c-term and on 0.5 gamma B N_gamma
    let (sc, sg, sref) = match shape.as_str() {
        "strip" => ("1", "1", "Terzaghi (1943) strip"),
        "square" => ("1.3", "0.8", "Terzaghi (1943) square: 1.3 c Nc and 0.4 gamma B N_gamma"),
        "circle" => ("1.3", "0.6", "Terzaghi (1943) circle: 1.3 c Nc and 0.3 gamma B N_gamma"),
        _ => ("1 + 0.3*B/L", "1 - 0.2*B/L", "rectangle interpolation (Bowles, Das); not in Terzaghi (1943)"),
    };
    sb.step("s_c", sc, "-", "shape multiplier on the c-term", sref)?;
    sb.step("s_g", sg, "-", "shape multiplier on 0.5 gamma B N_gamma", sref)?;

    // water table (Das cases I-III)
    let case;
    match z_w {
        Some(zw) if zw < d + b => {
            let gs = gamma_sat.ok_or("the water table is within D + B: gamma_sat is required")?;
            sb.input("gamma_sat", gs, "kN/m³", "saturated unit weight");
            sb.input("gamma_w", 9.81, "kN/m³", "unit weight of water");
            sb.input("z_w", non_negative("z_w", zw)?, "m", "water table depth below ground");
            sb.step("g_sub", "gamma_sat - gamma_w", "kN/m³", "submerged unit weight", "")?;
            if zw <= d {
                case = "I";
                sb.step("q", "gamma*z_w + g_sub*(D - z_w)", "kPa", "effective overburden at the base (case I)", "Das, water table case I")?;
                sb.step("g_3", "g_sub", "kN/m³", "unit weight in the gamma-term (case I)", "Das, water table case I")?;
            } else {
                case = "II";
                sb.step("q", "gamma*D", "kPa", "effective overburden at the base (case II)", "Das, water table case II")?;
                sb.step("g_3", "g_sub + (z_w - D)/B*(gamma - g_sub)", "kN/m³", "unit weight in the gamma-term (case II)", "Das, water table case II")?;
            }
        }
        _ => {
            case = "III";
            sb.step("q", "gamma*D", "kPa", "overburden at the base (water table deep, case III)", "Das, water table case III")?;
            sb.step("g_3", "gamma", "kN/m³", "unit weight in the gamma-term", "Das, water table case III")?;
        }
    }

    sb.step("q_c", "s_c*c_u*N_c", "kPa", "cohesion term", "Terzaghi (1943)")?;
    sb.step("q_q", "q*N_q", "kPa", "overburden term", "Terzaghi (1943)")?;
    sb.step("q_g", "s_g*0.5*g_3*B*N_g", "kPa", "self-weight term", "Terzaghi (1943)")?;
    sb.step("q_ult", "q_c + q_q + q_g", "kPa", "ultimate bearing pressure (gross)", "Terzaghi (1943)")?;
    sb.step("q_net", "q_ult - q", "kPa", "net ultimate bearing pressure", "Das")?;
    sb.step("q_all", "q_ult/FS", "kPa", "allowable gross bearing pressure", "Das")?;
    sb.step("q_alln", "q_net/FS", "kPa", "allowable net bearing pressure", "Das")?;
    let (area, unit) = match shape.as_str() {
        "strip" => ("B", "m²/m"),
        "circle" => ("pi*B^2/4", "m²"),
        "square" => ("B^2", "m²"),
        _ => ("B*L", "m²"),
    };
    sb.step("A", area, unit, "bearing area", "geometry")?;
    let qunit = if shape == "strip" { "kN/m" } else { "kN" };
    if let Some(qa) = q_app {
        sb.input("Q", non_negative("Q", qa)?, qunit, "applied gross load");
        sb.check("Q_all", "q_all*A", qunit, "allowable gross load", "Das", "Q", "Q_all", "Q <= Q_all")?;
    } else {
        sb.step("Q_all", "q_all*A", qunit, "allowable gross load", "Das")?;
    }

    sb.note(&format!("Water table case {case} (Das). Terzaghi's theory ignores the shear strength of the soil above the base and is meant for D <= B."));
    if d > b {
        sb.note("D/B > 1: Terzaghi's equation is conservative here; consider a general equation with depth factors.");
    }
    if local {
        sb.note("Local shear: c and tan(phi) reduced by 2/3; use it only for loose sand or soft clay.");
    }
    if shape == "rect" {
        sb.note("The rectangular shape multipliers are an interpolation convention (Bowles, Das), not part of Terzaghi (1943).");
    }
    sb.note("Vertical, centric load; no inclination, eccentricity or settlement check.");

    let sheet = sb.finish()?;
    let v = |k: &str| sheet.value(k).unwrap_or(f64::NAN);
    let summary = format!(
        "Terzaghi {} footing B = {} m{} at D = {} m, phi = {} deg ({} shear), c = {} kPa: Nc = {}, Nq = {}, Ngamma = {}; \
         q_ult = {} kPa, q_all = {} kPa (FS {}), Q_all = {} {}.{}",
        shape,
        fmt4(b),
        l.map(|l| format!(" x L = {} m", fmt4(l))).unwrap_or_default(),
        fmt4(d),
        fmt4(phi),
        if local { "local" } else { "general" },
        fmt4(c),
        fmt4(v("N_c")),
        fmt4(v("N_q")),
        fmt4(v("N_g")),
        fmt4(v("q_ult")),
        fmt4(v("q_all")),
        fmt4(fs),
        fmt4(v("Q_all")),
        qunit,
        if q_app.is_some() { format!(" {}", check_summary(&sheet)) } else { String::new() }
    );
    let drawing = footing_drawing(
        &format!("{} footing, Terzaghi", shape),
        b,
        d,
        z_w,
        &format!("Q_all = {} {}", fmt4(v("Q_all")), qunit),
    );
    Ok(Design { module: "bearing_terzaghi".into(), pass: sheet.all_pass, sheet, drawing: Some(drawing), summary })
}

/// Section through a pad/strip footing with column, ground, soil hatch, optional water table,
/// dimensions B and D, and a load arrow with a label. Units: m.
pub(crate) fn footing_drawing(title: &str, b: f64, d: f64, z_w: Option<f64>, load_label: &str) -> Drawing {
    let mut dr = Drawing::new(title, "m");
    dr.scale_note = "Scale: NTS".into();
    dr.layer("soil", "#8a6d3b").layer("concrete", "#4a4a4a").layer("water", "#1f78c8").layer("dims", "#2b2b2b").layer("load", "#c0392b");
    let t = (0.25 * b).clamp(0.3, 1.0).min(if d > 0.0 { d } else { 0.3 });
    let t = if t <= 0.0 { 0.3 } else { t };
    let cw = 0.25 * b;
    let half = 1.4 * b;
    let deep = d + 1.3 * b;
    let top = 0.25 * b.max(d);
    // soil block with the footing notch
    dr.add(Entity::Polyline {
        layer: "soil".into(),
        points: vec![
            [-half, 0.0],
            [-cw / 2.0, 0.0],
            [-cw / 2.0, -d + t],
            [-b / 2.0, -d + t],
            [-b / 2.0, -d],
            [b / 2.0, -d],
            [b / 2.0, -d + t],
            [cw / 2.0, -d + t],
            [cw / 2.0, 0.0],
            [half, 0.0],
            [half, -deep],
            [-half, -deep],
        ],
        closed: true,
        hatch: Some("soil".into()),
    });
    // footing and column stub
    dr.add(Entity::Polyline {
        layer: "concrete".into(),
        points: vec![[-b / 2.0, -d], [b / 2.0, -d], [b / 2.0, -d + t], [cw / 2.0, -d + t], [cw / 2.0, top], [-cw / 2.0, top], [-cw / 2.0, -d + t], [-b / 2.0, -d + t]],
        closed: true,
        hatch: Some("concrete".into()),
    });
    if let Some(zw) = z_w {
        if zw < deep {
            dr.add(Entity::Line { layer: "water".into(), a: [-half, -zw], b: [half, -zw] });
            let s = 0.08 * b;
            let x = 0.75 * half;
            dr.add(Entity::Polyline { layer: "water".into(), points: vec![[x - s, -zw + 1.2 * s], [x + s, -zw + 1.2 * s], [x, -zw]], closed: true, hatch: None });
            dr.add(Entity::Text { layer: "water".into(), at: [x + 1.5 * s, -zw + 0.3 * s], text: "GWT".into(), height: 0.07 * b.max(d), align: "left".into() });
        }
    }
    // load arrow
    let ah = 0.35 * b.max(d);
    dr.add(Entity::Line { layer: "load".into(), a: [0.0, top + ah], b: [0.0, top] });
    dr.add(Entity::Polyline { layer: "load".into(), points: vec![[0.0, top], [-0.05 * ah * 2.0, top + 0.25 * ah], [0.05 * ah * 2.0, top + 0.25 * ah]], closed: true, hatch: Some("solid".into()) });
    dr.add(Entity::Text { layer: "load".into(), at: [0.08 * b, top + 0.75 * ah], text: load_label.into(), height: 0.07 * b.max(d), align: "left".into() });
    // dimensions
    dr.add(Entity::Dim { layer: "dims".into(), a: [-b / 2.0, -d], b: [b / 2.0, -d], offset: -0.25 * b, text: Some(format!("B = {}", fmt4(b))) });
    if d > 0.0 {
        dr.add(Entity::Dim { layer: "dims".into(), a: [b / 2.0, 0.0], b: [b / 2.0, -d], offset: 0.3 * b, text: Some(format!("D = {}", fmt4(d))) });
    }
    dr
}
