//! Cantilever RC retaining wall, external stability, following Das, Principles of Foundation
//! Engineering, chapter "Retaining walls" (Rankine active pressure on the vertical plane
//! through the heel, Example 13.1 style). Per metre run of wall. Origin at the underside of
//! the toe; the stem's back face is vertical and its batter is on the front face.

use serde_json::{json, Value};

use super::{check_summary, flag, non_negative, num, positive, prop, schema, Design, ModuleInfo, SheetBuilder};
use crate::drawing::{Drawing, Entity};
use crate::expr::fmt4;

pub(super) fn info() -> ModuleInfo {
    ModuleInfo {
        name: "retaining_wall_cantilever".into(),
        title: "Cantilever retaining wall: external stability".into(),
        standard: "Das, Principles of Foundation Engineering (Rankine, Meyerhof bearing)".into(),
        description: "Level backfill, Rankine active pressure on the vertical plane through the heel (H' = H + base \
            thickness), uniform surcharge q, optional water table in the backfill (with base uplift) and optional passive \
            resistance in front. Checks overturning FS >= 2, sliding FS >= 1.5 (delta = k1 phi2, c_a = k2 c2), \
            eccentricity e <= B/6, q_max/q_min, and bearing FS >= 3 with Meyerhof's general equation (depth and \
            inclination factors). Per metre run."
            .into(),
        input_schema: schema(
            vec![
                ("H", prop("number", "stem height above the top of the base (m)", None)),
                ("t_top", prop("number", "stem thickness at the top (m)", Some(json!(0.3)))),
                ("t_bot", prop("number", "stem thickness at the base (m)", Some(json!(0.5)))),
                ("t_base", prop("number", "base slab thickness (m)", Some(json!(0.6)))),
                ("toe", prop("number", "toe length, front of base to front of stem (m)", None)),
                ("heel", prop("number", "heel length, back of stem to end of base (m)", None)),
                ("gamma_c", prop("number", "unit weight of concrete (kN/m3)", Some(json!(24)))),
                ("gamma1", prop("number", "backfill unit weight (kN/m3)", Some(json!(18)))),
                ("phi1", prop("number", "backfill friction angle (deg)", Some(json!(30)))),
                ("q", prop("number", "uniform surcharge on the backfill (kPa)", Some(json!(0)))),
                ("gamma2", prop("number", "foundation soil unit weight (kN/m3)", Some(json!(18)))),
                ("phi2", prop("number", "foundation soil friction angle (deg)", Some(json!(20)))),
                ("c2", prop("number", "foundation soil cohesion (kPa)", Some(json!(0)))),
                ("D", prop("number", "depth of the base underside below the front ground (m)", None)),
                ("k1", prop("number", "delta = k1*phi2 for base friction", Some(json!(0.6667)))),
                ("k2", prop("number", "c_a = k2*c2 for base adhesion", Some(json!(0.6667)))),
                ("passive", prop("boolean", "include Rankine passive resistance over D in sliding", Some(json!(false)))),
                ("toe_soil", prop("boolean", "count the soil above the toe as resisting weight", Some(json!(false)))),
                ("water_table", prop("boolean", "water table in the backfill", Some(json!(false)))),
                ("z_w", prop("number", "water table depth below the backfill surface (m)", None)),
                ("gamma_sat1", prop("number", "saturated backfill unit weight (kN/m3)", Some(json!(20)))),
                ("FS_ot_min", prop("number", "required FS against overturning", Some(json!(2.0)))),
                ("FS_sl_min", prop("number", "required FS against sliding", Some(json!(1.5)))),
                ("FS_bc_min", prop("number", "required FS against bearing failure", Some(json!(3.0)))),
            ],
            &["H", "toe", "heel", "D"],
        ),
    }
}

pub(super) fn run(a: &Value) -> Result<Design, String> {
    let h = positive("H", num(a, "H", None)?)?;
    let t_top = positive("t_top", num(a, "t_top", Some(0.3))?)?;
    let t_bot = positive("t_bot", num(a, "t_bot", Some(0.5))?)?;
    if t_bot < t_top {
        return Err("t_bot must not be less than t_top".into());
    }
    let t_b = positive("t_base", num(a, "t_base", Some(0.6))?)?;
    let toe = non_negative("toe", num(a, "toe", None)?)?;
    let heel = non_negative("heel", num(a, "heel", None)?)?;
    let gc = positive("gamma_c", num(a, "gamma_c", Some(24.0))?)?;
    let g1 = positive("gamma1", num(a, "gamma1", Some(18.0))?)?;
    let phi1 = num(a, "phi1", Some(30.0))?;
    if !(0.0..60.0).contains(&phi1) || phi1 <= 0.0 {
        return Err("phi1 must be between 0 and 60 degrees (c = 0 backfill)".into());
    }
    let q = non_negative("q", num(a, "q", Some(0.0))?)?;
    let g2 = positive("gamma2", num(a, "gamma2", Some(18.0))?)?;
    let phi2 = num(a, "phi2", Some(20.0))?;
    if !(0.0..60.0).contains(&phi2) {
        return Err("phi2 must be between 0 and 60 degrees".into());
    }
    let c2 = non_negative("c2", num(a, "c2", Some(0.0))?)?;
    let d = non_negative("D", num(a, "D", None)?)?;
    let k1 = non_negative("k1", num(a, "k1", Some(2.0 / 3.0))?)?;
    let k2 = non_negative("k2", num(a, "k2", Some(2.0 / 3.0))?)?;
    let passive = flag(a, "passive", false)?;
    let toe_soil = flag(a, "toe_soil", false)?;
    let water = flag(a, "water_table", false)?;
    let fs_ot = positive("FS_ot_min", num(a, "FS_ot_min", Some(2.0))?)?;
    let fs_sl = positive("FS_sl_min", num(a, "FS_sl_min", Some(1.5))?)?;
    let fs_bc = positive("FS_bc_min", num(a, "FS_bc_min", Some(3.0))?)?;

    let mut sb = SheetBuilder::new("Cantilever retaining wall: external stability", "Das, Principles of Foundation Engineering, Retaining walls");
    sb.input("H", h, "m", "stem height above the base");
    sb.input("t_top", t_top, "m", "stem thickness at top");
    sb.input("t_bot", t_bot, "m", "stem thickness at base");
    sb.input("t_b", t_b, "m", "base thickness");
    sb.input("L_toe", toe, "m", "toe length");
    sb.input("L_heel", heel, "m", "heel length");
    sb.input("gamma_c", gc, "kN/m³", "concrete unit weight");
    sb.input("gamma_1", g1, "kN/m³", "backfill unit weight");
    sb.input("phi_1", phi1, "deg", "backfill friction angle");
    sb.input("q", q, "kPa", "surcharge on backfill");
    sb.input("gamma_2", g2, "kN/m³", "foundation soil unit weight");
    sb.input("phi_2", phi2, "deg", "foundation soil friction angle");
    sb.input("c_2", c2, "kPa", "foundation soil cohesion");
    sb.input("D", d, "m", "depth of base underside below front ground");
    sb.input("k_1", k1, "-", "base friction factor, delta = k1 phi2");
    sb.input("k_2", k2, "-", "base adhesion factor, c_a = k2 c2");

    sb.step("Hp", "H + t_b", "m", "height of the virtual back H'", "Das, Rankine analysis on the vertical through the heel")?;
    sb.step("B", "L_toe + t_bot + L_heel", "m", "base width", "geometry")?;
    sb.step("K_a", "(1 - sind(phi_1))/(1 + sind(phi_1))", "-", "Rankine active coefficient (level backfill)", "Rankine; Das")?;

    // ---- lateral thrust and overturning moment about the toe
    let mut z_wt = None;
    if water {
        let zw = non_negative("z_w", num(a, "z_w", None)?)?;
        let hp = h + t_b;
        if zw >= hp {
            sb.note("The water table is at or below the base: ignored.");
        } else {
            z_wt = Some(zw);
            let gs = positive("gamma_sat1", num(a, "gamma_sat1", Some(20.0))?)?;
            sb.input("z_w", zw, "m", "water table depth below backfill surface");
            sb.input("gamma_sat", gs, "kN/m³", "saturated backfill unit weight");
            sb.input("gamma_w", 9.81, "kN/m³", "unit weight of water");
            sb.step("h_w", "Hp - z_w", "m", "height of water above the base underside", "geometry")?;
        }
    }
    let mut thrust = vec![];
    let mut moment = vec![];
    if z_wt.is_some() {
        sb.step("P_1", "0.5*K_a*gamma_1*z_w^2", "kN/m", "active thrust above the water table", "Rankine")?;
        sb.step("P_2", "K_a*gamma_1*z_w*h_w", "kN/m", "active thrust below the water table from the soil above it", "Rankine")?;
        sb.step("P_3", "0.5*K_a*(gamma_sat - gamma_w)*h_w^2", "kN/m", "active thrust from submerged soil", "Rankine")?;
        sb.step("P_w", "0.5*gamma_w*h_w^2", "kN/m", "hydrostatic water thrust", "hydrostatics")?;
        thrust.extend(["P_1", "P_2", "P_3", "P_w"]);
        moment.extend(["P_1*(h_w + z_w/3)", "P_2*h_w/2", "P_3*h_w/3", "P_w*h_w/3"]);
    } else {
        sb.step("P_1", "0.5*gamma_1*Hp^2*K_a", "kN/m", "Rankine active thrust from the backfill", "Das, Rankine")?;
        thrust.push("P_1");
        moment.push("P_1*Hp/3");
    }
    if q > 0.0 {
        sb.step("P_q", "q*K_a*Hp", "kN/m", "active thrust from the surcharge", "Das, Rankine")?;
        thrust.push("P_q");
        moment.push("P_q*Hp/2");
    }
    sb.step("P_a", &thrust.join(" + "), "kN/m", "total horizontal thrust", "")?;
    if z_wt.is_some() {
        sb.step("U", "0.5*gamma_w*h_w*B", "kN/m", "uplift under the base (triangular, zero at the toe)", "assumed seepage under the base")?;
        moment.push("U*2*B/3");
    }
    sb.step("M_o", &moment.join(" + "), "kN·m/m", "overturning moment about the toe", "Das")?;

    // ---- weights and resisting moments about the toe
    sb.step("W_1", "t_top*H*gamma_c", "kN/m", "stem, rectangular part", "geometry")?;
    sb.step("x_1", "L_toe + (t_bot - t_top) + t_top/2", "m", "lever arm of W_1", "geometry")?;
    let mut ws = vec!["W_1"];
    let mut ms = vec!["W_1*x_1"];
    if t_bot > t_top {
        sb.step("W_2", "0.5*(t_bot - t_top)*H*gamma_c", "kN/m", "stem, triangular batter", "geometry")?;
        sb.step("x_2", "L_toe + 2/3*(t_bot - t_top)", "m", "lever arm of W_2", "geometry")?;
        ws.push("W_2");
        ms.push("W_2*x_2");
    }
    sb.step("W_3", "B*t_b*gamma_c", "kN/m", "base slab", "geometry")?;
    sb.step("x_3", "B/2", "m", "lever arm of W_3", "geometry")?;
    ws.push("W_3");
    ms.push("W_3*x_3");
    if heel > 0.0 {
        if z_wt.is_some() {
            sb.step("W_4", "L_heel*(gamma_1*min(z_w, H) + gamma_sat*max(H - z_w, 0))", "kN/m", "backfill on the heel", "geometry")?;
        } else {
            sb.step("W_4", "L_heel*H*gamma_1", "kN/m", "backfill on the heel", "geometry")?;
        }
        sb.step("x_4", "L_toe + t_bot + L_heel/2", "m", "lever arm of W_4", "geometry")?;
        ws.push("W_4");
        ms.push("W_4*x_4");
    }
    if toe_soil && d > t_b && toe > 0.0 {
        sb.step("W_5", "L_toe*(D - t_b)*gamma_2", "kN/m", "soil above the toe", "geometry")?;
        sb.step("x_5", "L_toe/2", "m", "lever arm of W_5", "geometry")?;
        ws.push("W_5");
        ms.push("W_5*x_5");
    }
    let mut sv = ws.join(" + ");
    if z_wt.is_some() {
        sv.push_str(" - U");
    }
    sb.step("SV", &sv, "kN/m", "sum of vertical forces", "Das")?;
    sb.step("M_R", &ms.join(" + "), "kN·m/m", "resisting moment about the toe", "Das")?;

    // ---- overturning
    sb.check("FS_ot", "M_R/M_o", "-", "factor of safety against overturning", "Das, FS(overturning) = sum M_R / sum M_o", &format!("{fs_ot}"), "FS_ot", &format!("FS overturning >= {fs_ot}"))?;

    // ---- sliding
    sb.step("delta", "k_1*phi_2", "deg", "base friction angle", "Das, delta = k1 phi2")?;
    sb.step("c_a", "k_2*c_2", "kPa", "base adhesion", "Das, c_a = k2 c2")?;
    if passive {
        sb.step("K_p", "(1 + sind(phi_2))/(1 - sind(phi_2))", "-", "Rankine passive coefficient", "Rankine")?;
        sb.step("P_p", "0.5*K_p*gamma_2*D^2 + 2*c_2*sqrt(K_p)*D", "kN/m", "passive resistance in front", "Das")?;
    } else {
        sb.step("P_p", "0", "kN/m", "passive resistance in front (ignored)", "Das, conservative")?;
    }
    sb.check(
        "FS_sl",
        "(SV*tand(delta) + B*c_a + P_p)/P_a",
        "-",
        "factor of safety against sliding",
        "Das, FS(sliding)",
        &format!("{fs_sl}"),
        "FS_sl",
        &format!("FS sliding >= {fs_sl}"),
    )?;

    // ---- eccentricity and base pressure
    sb.step("x_R", "(M_R - M_o)/SV", "m", "position of the resultant from the toe", "Das")?;
    let e = sb.check("e", "B/2 - x_R", "m", "eccentricity of the resultant", "Das, e <= B/6", "abs(e)", "B/6", "|e| <= B/6 (no tension under the base)")?;
    let b = sb.get("B");
    if e.abs() <= b / 6.0 {
        sb.step("q_max", "SV/B*(1 + 6*abs(e)/B)", "kPa", "maximum base pressure", "Das")?;
        sb.step("q_min", "SV/B*(1 - 6*abs(e)/B)", "kPa", "minimum base pressure", "Das")?;
    } else {
        sb.step("q_max", "4*SV/(3*(B - 2*abs(e)))", "kPa", "maximum base pressure (triangular, partial contact)", "statics")?;
        sb.step("q_min", "0", "kPa", "minimum base pressure (partial contact)", "statics")?;
    }

    // ---- bearing capacity, Meyerhof general equation
    sb.step("B_p", "B - 2*abs(e)", "m", "effective base width B'", "Meyerhof effective width")?;
    if sb.get("B_p") <= 0.0 {
        return Err("the resultant falls outside the base (B' <= 0): the wall overturns".into());
    }
    sb.step("q_2", "gamma_2*D", "kPa", "overburden at the base", "Das")?;
    sb.step("N_q", "exp(pi*tand(phi_2))*tand(45 + phi_2/2)^2", "-", "Nq", "Reissner (1924); Das")?;
    let phi0 = phi2 < 1e-9;
    if phi0 {
        sb.step("N_c", "pi + 2", "-", "Nc at phi = 0", "Prandtl (1921); Das")?;
    } else {
        sb.step("N_c", "(N_q - 1)/tand(phi_2)", "-", "Nc", "Prandtl (1921); Das")?;
    }
    sb.step("N_g", "2*(N_q + 1)*tand(phi_2)", "-", "N_gamma", "Vesic (1973); Das")?;
    let dr = if d / sb.get("B_p") <= 1.0 { "D/B_p" } else { "atan(D/B_p)" };
    if phi0 {
        sb.step("F_qd", "1", "-", "depth factor F_qd", "Hansen (1970); Das")?;
        sb.step("F_cd", &format!("1 + 0.4*{dr}"), "-", "depth factor F_cd (phi = 0)", "Hansen (1970); Das")?;
    } else {
        sb.step("F_qd", &format!("1 + 2*tand(phi_2)*(1 - sind(phi_2))^2*{dr}"), "-", "depth factor F_qd", "Hansen (1970); Das")?;
        sb.step("F_cd", "F_qd - (1 - F_qd)/(N_c*tand(phi_2))", "-", "depth factor F_cd", "Hansen (1970); Das")?;
    }
    sb.step("psi", "atan(P_a/SV)*180/pi", "deg", "inclination of the resultant to the vertical", "Das")?;
    sb.step("F_ci", "(1 - psi/90)^2", "-", "inclination factor F_ci = F_qi", "Meyerhof (1963); Das")?;
    if phi0 {
        sb.step("F_gi", "1", "-", "inclination factor F_gi (N_gamma = 0 at phi = 0)", "Meyerhof (1963); Das")?;
    } else {
        sb.step("F_gi", "max(0, 1 - psi/phi_2)^2", "-", "inclination factor F_gi", "Meyerhof (1963); Das")?;
    }
    sb.step("q_u", "c_2*N_c*F_cd*F_ci + q_2*N_q*F_qd*F_ci + 0.5*gamma_2*B_p*N_g*F_gi", "kPa", "ultimate bearing capacity (F_gd = 1, shape factors 1 for a strip)", "Meyerhof (1963) general equation; Das")?;
    sb.check("FS_bc", "q_u/q_max", "-", "factor of safety against bearing failure", "Das, FS(bearing) = q_u/q_max", &format!("{fs_bc}"), "FS_bc", &format!("FS bearing >= {fs_bc}"))?;

    sb.note("Rankine active pressure on the vertical plane through the heel, level backfill, wall friction neglected; per metre run.");
    if q > 0.0 {
        sb.note("The surcharge adds thrust but its weight over the heel is not counted as resisting (conservative).");
    }
    if z_wt.is_some() {
        sb.note("Water table in the backfill only, with no water in front; uplift varies linearly from the heel to zero at the toe.");
    }
    if !passive {
        sb.note("Passive resistance in front of the toe is ignored (conservative).");
    }
    sb.note("Structural design of the stem, toe and heel (bending and shear) and global (deep-seated) stability are not checked.");

    let sheet = sb.finish()?;
    let v = |k: &str| sheet.value(k).unwrap_or(f64::NAN);
    let summary = format!(
        "Cantilever wall H = {} m (H' = {} m), base B = {} m (toe {} m, heel {} m). Pa = {} kN/m; FS overturning = {}, sliding = {}, \
         bearing = {}; e = {} m (B/6 = {} m), q_max = {} kPa, q_min = {} kPa. {}. {}",
        fmt4(h),
        fmt4(v("Hp")),
        fmt4(v("B")),
        fmt4(toe),
        fmt4(heel),
        fmt4(v("P_a")),
        fmt4(v("FS_ot")),
        fmt4(v("FS_sl")),
        fmt4(v("FS_bc")),
        fmt4(v("e")),
        fmt4(v("B") / 6.0),
        fmt4(v("q_max")),
        fmt4(v("q_min")),
        if sheet.all_pass { "All checks pass" } else { "One or more checks FAIL" },
        check_summary(&sheet)
    );
    let drawing = wall_drawing(h, t_top, t_bot, t_b, toe, heel, d, q, z_wt, v("K_a"), g1, sheet.value("gamma_sat").unwrap_or(g1), v("P_a"));
    Ok(Design { module: "retaining_wall_cantilever".into(), pass: sheet.all_pass, sheet, drawing: Some(drawing), summary })
}

#[allow(clippy::too_many_arguments)]
fn wall_drawing(h: f64, t_top: f64, t_bot: f64, t_b: f64, toe: f64, heel: f64, d: f64, q: f64, z_w: Option<f64>, ka: f64, g1: f64, gsat: f64, pa: f64) -> Drawing {
    let mut dr = Drawing::new("Cantilever retaining wall section", "m");
    dr.scale_note = "Scale: NTS".into();
    dr.layer("concrete", "#4a4a4a").layer("soil", "#8a6d3b").layer("ground", "#5b4a2a").layer("water", "#1f78c8").layer("dims", "#2b2b2b").layer("pressure", "#c0392b");
    let hp = h + t_b;
    let b = toe + t_bot + heel;
    let back = toe + t_bot;
    let ext = 0.45 * hp;
    let below = 0.15 * hp;
    // wall concrete
    dr.add(Entity::Polyline {
        layer: "concrete".into(),
        points: vec![[0.0, 0.0], [b, 0.0], [b, t_b], [back, t_b], [back, hp], [back - t_top, hp], [toe, t_b], [0.0, t_b]],
        closed: true,
        hatch: Some("concrete".into()),
    });
    // backfill
    dr.add(Entity::Polyline { layer: "soil".into(), points: vec![[back, t_b], [b, t_b], [b, 0.0], [b + ext, 0.0], [b + ext, hp], [back, hp]], closed: true, hatch: Some("soil".into()) });
    dr.add(Entity::Line { layer: "ground".into(), a: [back, hp], b: [b + ext, hp] });
    // foundation soil
    dr.add(Entity::Polyline { layer: "soil".into(), points: vec![[-ext, 0.0], [b + ext, 0.0], [b + ext, -below], [-ext, -below]], closed: true, hatch: Some("soil".into()) });
    // front fill and ground
    if d > 0.0 {
        let pts = if d > t_b {
            let xf = toe + (t_bot - t_top) * ((d - t_b) / h).min(1.0);
            vec![[-ext, d], [xf, d], [toe, t_b], [0.0, t_b], [0.0, 0.0], [-ext, 0.0]]
        } else {
            vec![[-ext, d], [0.0, d], [0.0, 0.0], [-ext, 0.0]]
        };
        let top = pts[1];
        dr.add(Entity::Polyline { layer: "soil".into(), points: pts, closed: true, hatch: Some("soil".into()) });
        dr.add(Entity::Line { layer: "ground".into(), a: [-ext, d], b: top });
    }
    // surcharge
    let th = 0.045 * hp.max(b);
    if q > 0.0 {
        for k in 0..5 {
            let x = back + 0.1 * ext + (b + 0.9 * ext - back) * k as f64 / 4.0;
            dr.add(Entity::Line { layer: "pressure".into(), a: [x, hp + 0.12 * hp], b: [x, hp + 0.01 * hp] });
            dr.add(Entity::Polyline { layer: "pressure".into(), points: vec![[x, hp], [x - 0.012 * hp, hp + 0.035 * hp], [x + 0.012 * hp, hp + 0.035 * hp]], closed: true, hatch: Some("solid".into()) });
        }
        dr.add(Entity::Text { layer: "pressure".into(), at: [back + 0.1 * ext, hp + 0.15 * hp], text: format!("q = {} kPa", fmt4(q)), height: th, align: "left".into() });
    }
    // water table
    if let Some(zw) = z_w {
        let y = hp - zw;
        dr.add(Entity::Line { layer: "water".into(), a: [back, y], b: [b + ext, y] });
        let s = 0.025 * hp;
        let x = b + 0.6 * ext;
        dr.add(Entity::Polyline { layer: "water".into(), points: vec![[x - s, y + 1.2 * s], [x + s, y + 1.2 * s], [x, y]], closed: true, hatch: None });
    }
    // pressure diagram on its own baseline right of the backfill
    let x0 = b + ext + 0.25 * hp;
    let gw = 9.81;
    let (top_p, base_p, mid) = match z_w {
        Some(zw) => {
            let hw = hp - zw;
            let at_wt = ka * (q + g1 * zw);
            (ka * q, ka * (q + g1 * zw + (gsat - gw) * hw) + gw * hw, Some((hw, at_wt)))
        }
        None => (ka * q, ka * (q + g1 * hp), None),
    };
    let sc = if base_p > 0.0 { 0.3 * hp / base_p } else { 1.0 };
    let mut pts = vec![[x0, hp], [x0 + sc * top_p, hp]];
    if let Some((hw, p)) = mid {
        pts.push([x0 + sc * p, hw]);
    }
    pts.push([x0 + sc * base_p, 0.0]);
    pts.push([x0, 0.0]);
    dr.add(Entity::Polyline { layer: "pressure".into(), points: pts, closed: true, hatch: Some("solid".into()) });
    dr.add(Entity::Text { layer: "pressure".into(), at: [x0, -0.08 * hp], text: format!("Pa = {} kN/m", fmt4(pa)), height: th, align: "left".into() });
    dr.add(Entity::Text { layer: "pressure".into(), at: [x0 + sc * base_p + 0.02 * hp, 0.02 * hp], text: format!("{} kPa", fmt4(base_p)), height: 0.8 * th, align: "left".into() });
    // dimensions
    dr.add(Entity::Dim { layer: "dims".into(), a: [0.0, -below], b: [toe, -below], offset: -0.08 * hp, text: None });
    dr.add(Entity::Dim { layer: "dims".into(), a: [toe, -below], b: [back, -below], offset: -0.08 * hp, text: None });
    dr.add(Entity::Dim { layer: "dims".into(), a: [back, -below], b: [b, -below], offset: -0.08 * hp, text: None });
    dr.add(Entity::Dim { layer: "dims".into(), a: [0.0, -below], b: [b, -below], offset: -0.18 * hp, text: Some(format!("B = {}", fmt4(b))) });
    dr.add(Entity::Dim { layer: "dims".into(), a: [back - t_top, hp], b: [back, hp], offset: 0.05 * hp, text: None });
    dr.add(Entity::Dim { layer: "dims".into(), a: [b + ext, 0.0], b: [b + ext, t_b], offset: -0.05 * hp, text: None });
    dr.add(Entity::Dim { layer: "dims".into(), a: [b + ext, t_b], b: [b + ext, hp], offset: -0.05 * hp, text: Some(format!("H = {}", fmt4(h))) });
    if d > 0.0 {
        dr.add(Entity::Dim { layer: "dims".into(), a: [-ext, 0.0], b: [-ext, d], offset: 0.06 * hp, text: Some(format!("D = {}", fmt4(d))) });
    }
    dr
}
