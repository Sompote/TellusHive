//! Bearing resistance of a pad or strip footing to EN 1997-1:2004 Annex D, vertical load,
//! with the GEO partial factors of Design Approach 1 (Combinations 1 and 2) and Design
//! Approach 2. Depth factors are not used (Annex D has none) and inclination factors are 1.

use serde_json::{json, Value};

use super::bearing_terzaghi::footing_drawing;
use super::{flag, non_negative, num, opt_num, positive, prop, schema, Design, ModuleInfo, SheetBuilder};
use crate::expr::fmt4;

pub(super) fn info() -> ModuleInfo {
    ModuleInfo {
        name: "bearing_ec7".into(),
        title: "Bearing resistance of a shallow footing (EN 1997-1 Annex D)".into(),
        standard: "EN 1997-1:2004 Annex D, Annex A (DA1-C1, DA1-C2, DA2)".into(),
        description: "Drained (c', phi') and/or undrained (cu) bearing resistance with Annex D factors, shape factors on the \
            effective footing B' x L' (eccentricities e_B, e_L of the resultant), vertical centric load (inclination factors 1, \
            no depth factors). Checks Vd <= Rd for DA1 Combination 1 (A1+M1+R1), DA1 Combination 2 (A2+M2+R1) and DA2 \
            (A1+M1+R2) and reports the governing utilisation. Gk must include the footing and backfill weight."
            .into(),
        input_schema: schema(
            vec![
                ("B", prop("number", "footing width (m)", None)),
                ("L", prop("number", "footing length (m); defaults to B (square); ignored for a strip", None)),
                ("strip", prop("boolean", "strip footing (per metre run, B'/L' = 0)", Some(json!(false)))),
                ("D", prop("number", "depth of the base below ground (m)", Some(json!(0)))),
                ("phi_k", prop("number", "characteristic effective friction angle phi'k (deg); 0 skips the drained check unless c_k > 0", Some(json!(0)))),
                ("c_k", prop("number", "characteristic effective cohesion c'k (kPa)", Some(json!(0)))),
                ("cu_k", prop("number", "characteristic undrained strength cu,k (kPa); give it to run the undrained check", None)),
                ("gamma", prop("number", "unit weight above the water table (kN/m3)", None)),
                ("gamma_sat", prop("number", "saturated unit weight (kN/m3), needed when the water table is within D + B'", None)),
                ("z_w", prop("number", "water table depth below ground (m); omit when deep", None)),
                ("Gk", prop("number", "characteristic permanent vertical load incl. footing and backfill (kN, or kN/m for a strip)", None)),
                ("Qk", prop("number", "characteristic variable vertical load (kN, or kN/m)", Some(json!(0)))),
                ("e_B", prop("number", "eccentricity of the resultant across B (m)", Some(json!(0)))),
                ("e_L", prop("number", "eccentricity of the resultant along L (m)", Some(json!(0)))),
            ],
            &["B", "gamma", "Gk"],
        ),
    }
}

/// One combination: name, gamma_G, gamma_Q, material set ("1" = M1, "2" = M2), gamma_R;v.
const COMBOS: [(&str, &str, f64, f64, &str, f64); 3] = [
    ("1", "DA1-C1 (A1+M1+R1)", 1.35, 1.5, "1", 1.0),
    ("2", "DA1-C2 (A2+M2+R1)", 1.0, 1.3, "2", 1.0),
    ("3", "DA2 (A1+M1+R2)", 1.35, 1.5, "1", 1.4),
];

pub(super) fn run(a: &Value) -> Result<Design, String> {
    let b = positive("B", num(a, "B", None)?)?;
    let strip = flag(a, "strip", false)?;
    let l = if strip { None } else { Some(positive("L", num(a, "L", Some(b))?)?) };
    let d = non_negative("D", num(a, "D", Some(0.0))?)?;
    let phi_k = num(a, "phi_k", Some(0.0))?;
    if !(0.0..60.0).contains(&phi_k) {
        return Err("phi_k must be between 0 and 60 degrees".into());
    }
    let c_k = non_negative("c_k", num(a, "c_k", Some(0.0))?)?;
    let cu_k = opt_num(a, "cu_k")?;
    let gamma = positive("gamma", num(a, "gamma", None)?)?;
    let gamma_sat = opt_num(a, "gamma_sat")?;
    let z_w = opt_num(a, "z_w")?;
    let gk = non_negative("Gk", num(a, "Gk", None)?)?;
    let qk = non_negative("Qk", num(a, "Qk", Some(0.0))?)?;
    let e_b = num(a, "e_B", Some(0.0))?.abs();
    let e_l = if strip { 0.0 } else { num(a, "e_L", Some(0.0))?.abs() };
    let drained = phi_k > 0.0 || c_k > 0.0;
    let undrained = cu_k.is_some();
    if !drained && !undrained {
        return Err("give phi_k and/or c_k for a drained check, or cu_k for an undrained check".into());
    }
    if let Some(cu) = cu_k {
        positive("cu_k", cu)?;
    }

    let unit_v = if strip { "kN/m" } else { "kN" };
    let mut sb = SheetBuilder::new("Bearing resistance, EN 1997-1 Annex D", "EN 1997-1:2004 Annex D; Annex A Tables A.3, A.4, A.5");
    sb.input("B", b, "m", "footing width");
    if let Some(l) = l {
        sb.input("L", l, "m", "footing length");
    }
    sb.input("D", d, "m", "depth of the base");
    sb.input("gamma", gamma, "kN/m³", "unit weight above the water table");
    sb.input("G_k", gk, unit_v, "characteristic permanent load");
    sb.input("Q_k", qk, unit_v, "characteristic variable load");
    sb.input("e_B", e_b, "m", "eccentricity across B");
    if drained {
        sb.input("phi_k", phi_k, "deg", "characteristic phi'");
        sb.input("c_k", c_k, "kPa", "characteristic c'");
    }
    if let Some(cu) = cu_k {
        sb.input("cu_k", cu, "kPa", "characteristic cu");
    }

    // effective dimensions
    sb.step("B_e", "B - 2*e_B", "m", "effective width B'", "EN 1997-1 D.1(2)")?;
    if strip {
        sb.step("r", "0", "-", "B'/L' for a strip", "EN 1997-1 D.4")?;
        sb.step("A_e", "B_e", "m²/m", "effective area per metre run", "EN 1997-1 D.1(2)")?;
    } else {
        sb.input("e_L", e_l, "m", "eccentricity along L");
        sb.step("L_e", "L - 2*e_L", "m", "effective length L'", "EN 1997-1 D.1(2)")?;
        sb.step("r", "min(B_e, L_e)/max(B_e, L_e)", "-", "B'/L' (shorter over longer)", "EN 1997-1 D.4")?;
        sb.step("A_e", "B_e*L_e", "m²", "effective area A'", "EN 1997-1 D.1(2)")?;
    }
    let b_e = sb.get("B_e");
    if b_e <= 0.0 || (!strip && sb.get("L_e") <= 0.0) {
        return Err("the eccentricity leaves no effective footing area (B' or L' <= 0)".into());
    }
    if e_b > b / 6.0 || l.is_some_and(|l| e_l > l / 6.0) {
        sb.note("The resultant lies outside the middle third (e > B/6): part of the base is in tension; EN 1997-1 6.5.4 asks for special care when e > B/3.");
    }

    // water table: effective overburden q', total overburden q (undrained), gamma' under the base
    let b_dim = b_e; // depth of influence below the base taken as B'
    match z_w {
        Some(zw) if zw < d + b_dim => {
            let gs = gamma_sat.ok_or("the water table is within D + B': gamma_sat is required")?;
            sb.input("gamma_sat", gs, "kN/m³", "saturated unit weight");
            sb.input("gamma_w", 9.81, "kN/m³", "unit weight of water");
            sb.input("z_w", non_negative("z_w", zw)?, "m", "water table depth");
            sb.step("g_sub", "gamma_sat - gamma_w", "kN/m³", "submerged unit weight", "")?;
            if zw <= d {
                sb.step("q_e", "gamma*z_w + g_sub*(D - z_w)", "kPa", "effective overburden at base level q'", "EN 1997-1 D.4")?;
                sb.step("q_t", "gamma*z_w + gamma_sat*(D - z_w)", "kPa", "total overburden at base level q", "EN 1997-1 D.3")?;
                sb.step("g_e", "g_sub", "kN/m³", "effective unit weight below the base gamma'", "EN 1997-1 D.4")?;
            } else {
                sb.step("q_e", "gamma*D", "kPa", "effective overburden at base level q'", "EN 1997-1 D.4")?;
                sb.step("q_t", "gamma*D", "kPa", "total overburden at base level q", "EN 1997-1 D.3")?;
                sb.step("g_e", "g_sub + (z_w - D)/B_e*(gamma - g_sub)", "kN/m³", "gamma' below the base, interpolated over B' (Das case II)", "EN 1997-1 D.4; interpolation")?;
            }
        }
        _ => {
            sb.step("q_e", "gamma*D", "kPa", "effective overburden at base level q'", "EN 1997-1 D.4")?;
            sb.step("q_t", "gamma*D", "kPa", "total overburden at base level q", "EN 1997-1 D.3")?;
            sb.step("g_e", "gamma", "kN/m³", "unit weight below the base (water table deep)", "EN 1997-1 D.4")?;
        }
    }

    // bearing pressure per material set
    for (m, g_phi, g_c, g_cu) in [("1", 1.0, 1.0, 1.0), ("2", 1.25, 1.25, 1.4)] {
        let mref = if m == "1" { "EN 1997-1 Table A.4 (M1)" } else { "EN 1997-1 Table A.4 (M2)" };
        if drained {
            sb.step(&format!("phi_d{m}"), &format!("atan(tand(phi_k)/{g_phi})*180/pi"), "deg", &format!("design phi' (M{m})"), mref)?;
            sb.step(&format!("c_d{m}"), &format!("c_k/{g_c}"), "kPa", &format!("design c' (M{m})"), mref)?;
            sb.step(&format!("Nq_{m}"), &format!("exp(pi*tand(phi_d{m}))*tand(45 + phi_d{m}/2)^2"), "-", "Nq", "EN 1997-1 D.4")?;
            if sb.get(&format!("phi_d{m}")) < 1e-9 {
                sb.step(&format!("Nc_{m}"), "pi + 2", "-", "Nc at phi' = 0", "EN 1997-1 D.4 (limit)")?;
            } else {
                sb.step(&format!("Nc_{m}"), &format!("(Nq_{m} - 1)/tand(phi_d{m})"), "-", "Nc", "EN 1997-1 D.4")?;
            }
            sb.step(&format!("Ng_{m}"), &format!("2*(Nq_{m} - 1)*tand(phi_d{m})"), "-", "N_gamma (rough base, delta >= phi'/2)", "EN 1997-1 D.4")?;
            sb.step(&format!("sq_{m}"), &format!("1 + r*sind(phi_d{m})"), "-", "shape factor s_q", "EN 1997-1 D.4")?;
            sb.step(&format!("sg_{m}"), "1 - 0.3*r", "-", "shape factor s_gamma", "EN 1997-1 D.4")?;
            sb.step(&format!("sc_{m}"), &format!("(sq_{m}*Nq_{m} - 1)/(Nq_{m} - 1)"), "-", "shape factor s_c", "EN 1997-1 D.4")?;
            sb.step(
                &format!("qd_{m}"),
                &format!("c_d{m}*Nc_{m}*sc_{m} + q_e*Nq_{m}*sq_{m} + 0.5*g_e*B_e*Ng_{m}*sg_{m}"),
                "kPa",
                &format!("drained bearing resistance R/A' (M{m})"),
                "EN 1997-1 D.4 (D.2)",
            )?;
        }
        if undrained {
            sb.step(&format!("cu_d{m}"), &format!("cu_k/{g_cu}"), "kPa", &format!("design cu (M{m})"), mref)?;
            sb.step(&format!("scu_{m}"), "1 + 0.2*r", "-", "shape factor s_c (undrained)", "EN 1997-1 D.3")?;
            sb.step(&format!("qu_{m}"), &format!("(pi + 2)*cu_d{m}*scu_{m} + q_t"), "kPa", &format!("undrained bearing resistance R/A' (M{m})"), "EN 1997-1 D.3 (D.1)")?;
        }
    }

    // combinations
    let mut rows: Vec<(String, f64, f64, f64, &str)> = vec![]; // label, Vd, Rd, util, condition
    for (k, name, g_g, g_q, m, g_r) in COMBOS {
        let aref = if g_g > 1.0 { "EN 1997-1 Table A.3 (A1)" } else { "EN 1997-1 Table A.3 (A2)" };
        let vd = sb.step(&format!("V_d{k}"), &format!("{g_g}*G_k + {g_q}*Q_k"), unit_v, &format!("design vertical load, {name}"), aref)?;
        let rref = if g_r > 1.0 { "EN 1997-1 Table A.5 (R2), 2.4.7.3.4" } else { "EN 1997-1 Table A.5 (R1), 2.4.7.3.4" };
        for (cond, q) in [("drained", "qd"), ("undrained", "qu")] {
            if (cond == "drained" && !drained) || (cond == "undrained" && !undrained) {
                continue;
            }
            let sym = format!("R_{}{k}", &cond[..1]);
            let rd = sb.check(
                &sym,
                &format!("{q}_{m}*A_e/{g_r}"),
                unit_v,
                &format!("design bearing resistance, {cond}, {name}"),
                rref,
                &format!("V_d{k}"),
                &sym,
                &format!("Vd <= Rd, {cond}, {name}"),
            )?;
            rows.push((format!("{name}, {cond}"), vd, rd, if rd > 0.0 { vd / rd } else { f64::INFINITY }, cond));
        }
    }

    sb.note("Annex D method: vertical centric load on the effective area (inclination factors = 1), no depth factors, no base inclination.");
    sb.note("Partial factors: DA1-C1 gamma_G 1.35, gamma_Q 1.5, M1, R1; DA1-C2 gamma_G 1.0, gamma_Q 1.3, gamma_tan(phi') 1.25, gamma_c' 1.25, gamma_cu 1.4, R1; DA2 A1+M1 with gamma_R;v 1.4. The permanent load is taken as unfavourable.");
    sb.note("Check national annex values before use; sliding, overall stability and settlement (SLS) are not checked here.");
    if strip {
        sb.note("Strip footing: loads and resistances per metre run, B'/L' = 0.");
    }

    let sheet = sb.finish()?;
    let gov = rows.iter().cloned().fold(None::<(String, f64, f64, f64, &str)>, |acc, r| match acc {
        Some(a) if a.3 >= r.3 => Some(a),
        _ => Some(r),
    });
    let detail = rows.iter().map(|r| format!("{}: Vd = {} vs Rd = {} {} (util {:.2})", r.0, fmt4(r.1), fmt4(r.2), unit_v, r.3)).collect::<Vec<_>>().join("; ");
    let summary = match &gov {
        Some(g) => format!(
            "EC7 Annex D bearing, {} B = {} m{} at D = {} m. Governing: {} with utilisation {:.2} ({}). {}.",
            if strip { "strip" } else { "pad" },
            fmt4(b),
            l.map(|l| format!(" x L = {} m", fmt4(l))).unwrap_or_default(),
            fmt4(d),
            g.0,
            g.3,
            if sheet.all_pass { "all combinations pass" } else { "FAILS" },
            detail
        ),
        None => "no combination evaluated".into(),
    };
    let label = match &gov {
        Some(g) => format!("Vd = {} {} (gov.)", fmt4(g.1), unit_v),
        None => String::new(),
    };
    let drawing = footing_drawing("Footing section, EN 1997-1", b, d, z_w, &label);
    Ok(Design { module: "bearing_ec7".into(), pass: sheet.all_pass, sheet, drawing: Some(drawing), summary })
}
