//! Rectangular RC beam, flexure and shear, ACI 318-19 (SI units: mm, MPa, kN, kN·m).
//! One layer of tension bars; compression steel is ignored (conservative for flexure).

use serde_json::{json, Value};

use super::{check_summary, num, opt_num, positive, prop, schema, Design, ModuleInfo, SheetBuilder};
use crate::drawing::{Drawing, Entity};
use crate::expr::fmt4;

pub(super) fn info() -> ModuleInfo {
    ModuleInfo {
        name: "rc_beam_aci318".into(),
        title: "Rectangular RC beam: flexure and shear".into(),
        standard: "ACI 318-19 (SI)".into(),
        description: "Checks a singly reinforced rectangular beam section (one layer of tension bars) for Mu and Vu: \
            beta1, stress block, strain-controlled phi, phiMn, As,min, epsilon_t >= 0.004, bar spacing, Vc by Table 22.5.5.1, \
            Vs, section limit, Av,min and s_max. If the stirrup spacing s is omitted it is designed (rounded down to 25 mm)."
            .into(),
        input_schema: schema(
            vec![
                ("b", prop("number", "web width b (mm)", Some(json!(300)))),
                ("h", prop("number", "overall depth h (mm)", None)),
                ("cover", prop("number", "clear cover to the stirrup (mm)", Some(json!(40)))),
                ("stirrup_dia", prop("number", "stirrup bar diameter (mm)", Some(json!(9)))),
                ("bar_dia", prop("number", "tension bar diameter (mm)", Some(json!(20)))),
                ("n_bars", prop("integer", "number of tension bars in one layer", None)),
                ("fc", prop("number", "concrete strength f'c (MPa)", Some(json!(28)))),
                ("fy", prop("number", "longitudinal steel yield strength fy (MPa)", Some(json!(420)))),
                ("fyt", prop("number", "stirrup yield strength fyt (MPa); capped at 420 for shear (20.2.2.4)", Some(json!(240)))),
                ("Mu", prop("number", "factored moment Mu (kN·m)", None)),
                ("Vu", prop("number", "factored shear Vu at the critical section (kN)", Some(json!(0)))),
                ("legs", prop("integer", "number of stirrup legs", Some(json!(2)))),
                ("s", prop("number", "stirrup spacing (mm); omit to design it", None)),
                ("lambda", prop("number", "lightweight concrete factor lambda", Some(json!(1.0)))),
                ("agg_size", prop("number", "nominal maximum aggregate size (mm), for clear bar spacing 25.2.1", Some(json!(20)))),
                ("Es", prop("number", "steel modulus Es (MPa)", Some(json!(200000)))),
            ],
            &["h", "n_bars", "Mu"],
        ),
    }
}

/// "RB" for plain round bars up to 9 mm (Thai practice), "DB" for deformed bars.
fn bar_mark(d: f64) -> &'static str {
    if d <= 9.0 {
        "RB"
    } else {
        "DB"
    }
}

pub(super) fn run(a: &Value) -> Result<Design, String> {
    let b = positive("b", num(a, "b", Some(300.0))?)?;
    let h = positive("h", num(a, "h", None)?)?;
    let cover = positive("cover", num(a, "cover", Some(40.0))?)?;
    let ds = positive("stirrup_dia", num(a, "stirrup_dia", Some(9.0))?)?;
    let db = positive("bar_dia", num(a, "bar_dia", Some(20.0))?)?;
    let n = num(a, "n_bars", None)?;
    if n < 1.0 || n.fract() != 0.0 {
        return Err("input 'n_bars' must be a whole number >= 1".into());
    }
    let fc = positive("fc", num(a, "fc", Some(28.0))?)?;
    let fy = positive("fy", num(a, "fy", Some(420.0))?)?;
    let fyt = positive("fyt", num(a, "fyt", Some(240.0))?)?;
    let mu = num(a, "Mu", None)?.abs();
    let vu = num(a, "Vu", Some(0.0))?.abs();
    let legs = num(a, "legs", Some(2.0))?;
    if legs < 1.0 || legs.fract() != 0.0 {
        return Err("input 'legs' must be a whole number >= 1".into());
    }
    let s_given = opt_num(a, "s")?;
    if let Some(s) = s_given {
        positive("s", s)?;
    }
    let lambda = positive("lambda", num(a, "lambda", Some(1.0))?)?;
    let dagg = positive("agg_size", num(a, "agg_size", Some(20.0))?)?;
    let es = positive("Es", num(a, "Es", Some(200000.0))?)?;
    if h <= cover + ds + db {
        return Err("h is too small for the cover, stirrup and bar".into());
    }

    let mut sb = SheetBuilder::new("Rectangular RC beam: flexure and shear", "ACI 318-19 (SI)");
    sb.input("b", b, "mm", "web width");
    sb.input("h", h, "mm", "overall depth");
    sb.input("c_c", cover, "mm", "clear cover to stirrup");
    sb.input("d_s", ds, "mm", "stirrup diameter");
    sb.input("d_b", db, "mm", "tension bar diameter");
    sb.input("n", n, "-", "number of tension bars (one layer)");
    sb.input("f_c", fc, "MPa", "concrete compressive strength f'c");
    sb.input("f_y", fy, "MPa", "longitudinal yield strength");
    sb.input("f_yt", fyt, "MPa", "stirrup yield strength");
    sb.input("E_s", es, "MPa", "steel modulus");
    sb.input("lambda", lambda, "-", "lightweight concrete factor");
    sb.input("M_u", mu, "kN·m", "factored moment");
    sb.input("V_u", vu, "kN", "factored shear");
    sb.input("n_leg", legs, "-", "stirrup legs");
    sb.input("d_agg", dagg, "mm", "maximum aggregate size");
    if let Some(s) = s_given {
        sb.input("s", s, "mm", "stirrup spacing (given)");
    }

    // ---- flexure
    sb.step("d", "h - c_c - d_s - d_b/2", "mm", "effective depth", "geometry")?;
    sb.step("A_s", "n*pi*d_b^2/4", "mm²", "tension steel area", "geometry")?;
    if n >= 2.0 {
        sb.check(
            "s_clr",
            "(b - 2*c_c - 2*d_s - n*d_b)/(n - 1)",
            "mm",
            "clear spacing between bars",
            "ACI 318-19 25.2.1",
            "max(25, d_b, 4/3*d_agg)",
            "s_clr",
            "clear bar spacing >= max(25, db, 4/3 dagg)",
        )?;
    }
    sb.step("beta_1", "max(0.65, min(0.85, 0.85 - 0.05*(f_c - 28)/7))", "-", "stress block factor", "ACI 318-19 Table 22.2.2.4.3")?;
    sb.step("a", "A_s*f_y/(0.85*f_c*b)", "mm", "depth of the equivalent stress block", "ACI 318-19 22.2.2.4.1")?;
    sb.step("c", "a/beta_1", "mm", "neutral axis depth", "ACI 318-19 22.2.2.4.2")?;
    sb.check("eps_t", "0.003*(d - c)/c", "-", "net tensile strain in the extreme tension steel", "ACI 318-19 22.2.1.2, 9.3.3.1", "0.004", "eps_t", "eps_t >= 0.004 (9.3.3.1)")?;
    sb.step("eps_ty", "f_y/E_s", "-", "yield strain", "ACI 318-19 21.2.2.1")?;
    sb.step("phi_f", "min(0.9, max(0.65, 0.65 + 0.25*(eps_t - eps_ty)/0.003))", "-", "strength reduction factor (strain-controlled)", "ACI 318-19 Table 21.2.2")?;
    sb.step("M_n", "A_s*f_y*(d - a/2)/1e6", "kN·m", "nominal moment strength", "ACI 318-19 22.2")?;
    sb.check("phi_Mn", "phi_f*M_n", "kN·m", "design moment strength", "ACI 318-19 9.5.1.1", "M_u", "phi_Mn", "Mu <= phi Mn")?;
    sb.check(
        "A_smin",
        "max(0.25*sqrt(f_c)/f_y, 1.4/f_y)*b*d",
        "mm²",
        "minimum flexural reinforcement",
        "ACI 318-19 9.6.1.2",
        "A_smin",
        "A_s",
        "As >= As,min",
    )?;

    // ---- shear
    sb.step("rf_c", "min(sqrt(f_c), 8.3)", "MPa", "sqrt(f'c), limited to 8.3 MPa", "ACI 318-19 22.5.3.1")?;
    sb.step("f_ytv", "min(f_yt, 420)", "MPa", "stirrup yield strength used for shear", "ACI 318-19 20.2.2.4")?;
    sb.step("A_v", "n_leg*pi*d_s^2/4", "mm²", "stirrup area per spacing", "geometry")?;
    sb.step("rho_w", "A_s/(b*d)", "-", "longitudinal tension reinforcement ratio", "ACI 318-19 22.5.5.1")?;
    sb.step("Av_s_min", "max(0.062*rf_c, 0.35)*b/f_ytv", "mm²/mm", "minimum shear reinforcement per unit length", "ACI 318-19 9.6.3.4")?;
    sb.step("V_cmax", "0.42*lambda*rf_c*b*d/1000", "kN", "upper limit on Vc", "ACI 318-19 22.5.5.1.1")?;

    // Vc for the case Av >= Av,min (used to design s, and to check a given s when it complies)
    let vc_with_min = |sb: &mut SheetBuilder| -> Result<f64, String> {
        sb.step("V_ca", "0.17*lambda*rf_c*b*d/1000", "kN", "Vc, Table 22.5.5.1 (a)", "ACI 318-19 Table 22.5.5.1(a)")?;
        sb.step("V_cb", "0.66*lambda*rho_w^(1/3)*rf_c*b*d/1000", "kN", "Vc, Table 22.5.5.1 (b) with Nu = 0", "ACI 318-19 Table 22.5.5.1(b)")?;
        sb.step("V_c", "min(max(V_ca, V_cb), V_cmax)", "kN", "concrete shear strength, Av >= Av,min", "ACI 318-19 22.5.5.1, 22.5.5.1.1")
    };

    let designed = s_given.is_none();
    if designed {
        vc_with_min(&mut sb)?;
        let vs_req = sb.step("V_sreq", "max(V_u/0.75 - V_c, 0)", "kN", "required stirrup shear strength", "ACI 318-19 22.5.1.1, 21.2.1")?;
        let mut terms = vec!["s_Avmin"];
        sb.step("s_Avmin", "A_v/Av_s_min", "mm", "largest spacing that gives Av >= Av,min", "ACI 318-19 9.6.3.4")?;
        if vs_req > 0.0 {
            sb.step("s_req", "A_v*f_ytv*d/(V_sreq*1000)", "mm", "spacing for the required Vs", "ACI 318-19 22.5.8.5.3")?;
            terms.push("s_req");
        }
        // s_max depends on Vs, which depends on s: try the d/2 limit, halve it if the
        // resulting Vs exceeds 0.33 sqrt(f'c) b d.
        let d = sb.get("d");
        let rfc = sb.get("rf_c");
        let av = sb.get("A_v");
        let fytv = sb.get("f_ytv");
        let lim = |v: f64| (v / 25.0).floor() * 25.0;
        let cand = lim(terms.iter().map(|t| sb.get(t)).fold((d / 2.0).min(600.0), f64::min));
        let vs_cand = if cand > 0.0 { av * fytv * d / cand / 1000.0 } else { f64::INFINITY };
        let halved = vs_cand > 0.33 * rfc * b * d / 1000.0;
        if halved {
            sb.step("s_max0", "min(d/4, 300)", "mm", "maximum spacing (Vs > 0.33 sqrt(f'c) b d)", "ACI 318-19 Table 9.7.6.2.2")?;
        } else {
            sb.step("s_max0", "min(d/2, 600)", "mm", "maximum spacing", "ACI 318-19 Table 9.7.6.2.2")?;
        }
        terms.push("s_max0");
        let e = format!("floor(min({})/25)*25", terms.join(", "));
        sb.check("s", &e, "mm", "designed stirrup spacing, rounded down to 25 mm", "design choice", "50", "s", "practical spacing s >= 50 mm")?;
    } else {
        let s = sb.get("s");
        if sb.get("A_v") / s >= sb.get("Av_s_min") {
            vc_with_min(&mut sb)?;
        } else {
            sb.step("lambda_s", "min(1, sqrt(2/(1 + 0.004*d)))", "-", "size effect factor", "ACI 318-19 22.5.5.1.3")?;
            sb.step(
                "V_c",
                "min(0.66*lambda_s*lambda*rho_w^(1/3)*rf_c*b*d/1000, V_cmax)",
                "kN",
                "concrete shear strength, Av < Av,min",
                "ACI 318-19 Table 22.5.5.1(c), 22.5.5.1.1",
            )?;
        }
    }
    if sb.get("s") <= 0.0 {
        return Err("the designed stirrup spacing rounds to zero: increase the stirrup size or legs, or the section".into());
    }
    sb.check("V_s", "A_v*f_ytv*d/s/1000", "kN", "stirrup shear strength", "ACI 318-19 22.5.8.5.3", "V_s", "0.66*rf_c*b*d/1000", "Vs <= 0.66 sqrt(f'c) b d (22.5.1.2)")?;
    let vs = sb.get("V_s");
    if vs > 0.33 * sb.get("rf_c") * b * sb.get("d") / 1000.0 {
        sb.check("s_max", "min(d/4, 300)", "mm", "maximum stirrup spacing (Vs > 0.33 sqrt(f'c) b d)", "ACI 318-19 Table 9.7.6.2.2", "s", "s_max", "s <= s_max")?;
    } else {
        sb.check("s_max", "min(d/2, 600)", "mm", "maximum stirrup spacing", "ACI 318-19 Table 9.7.6.2.2", "s", "s_max", "s <= s_max")?;
    }
    sb.check("A_vmin", "Av_s_min*s", "mm²", "minimum shear reinforcement at spacing s", "ACI 318-19 9.6.3.4", "A_vmin", "A_v", "Av >= Av,min")?;
    sb.check("phi_Vn", "0.75*(V_c + V_s)", "kN", "design shear strength", "ACI 318-19 9.5.1.1, 21.2.1", "V_u", "phi_Vn", "Vu <= phi Vn")?;

    sb.note("Singly reinforced section with one layer of tension bars; compression steel and skin reinforcement are not considered.");
    sb.note("Shear: Nu = 0; minimum shear reinforcement is checked everywhere (conservative compared with the exemptions of 9.6.3.1).");
    sb.note("Development length, anchorage, deflection, crack control (24.3) and torsion are not checked.");
    if designed {
        sb.note("Stirrup spacing designed for Av >= Av,min, so Vc uses Table 22.5.5.1 (a)/(b).");
    }

    let sheet = sb.finish()?;
    let v = |k: &str| sheet.value(k).unwrap_or(f64::NAN);
    let s = v("s");
    let bars = format!("{}-{}{}", n as i64, bar_mark(db), fmt4(db));
    let stirrup = format!("{}{}@{}", bar_mark(ds), fmt4(ds), fmt4(s));
    let summary = format!(
        "RC beam {}x{} mm, {} bottom, stirrups {} ({} legs). phiMn = {} kN·m vs Mu = {} kN·m; phiVn = {} kN vs Vu = {} kN; eps_t = {}. {}. {}",
        fmt4(b),
        fmt4(h),
        bars,
        stirrup,
        legs as i64,
        fmt4(v("phi_Mn")),
        fmt4(mu),
        fmt4(v("phi_Vn")),
        fmt4(vu),
        fmt4(v("eps_t")),
        if sheet.all_pass { "All checks pass" } else { "One or more checks FAIL" },
        check_summary(&sheet)
    );
    let drawing = section_drawing(b, h, cover, ds, db, n as usize, &bars, &format!("{stirrup} ({} legs)", legs as i64), v("d"));
    Ok(Design { module: "rc_beam_aci318".into(), pass: sheet.all_pass, sheet, drawing: Some(drawing), summary })
}

#[allow(clippy::too_many_arguments)]
fn section_drawing(b: f64, h: f64, cover: f64, ds: f64, db: f64, n: usize, bars: &str, stirrup: &str, d: f64) -> Drawing {
    let mut dr = Drawing::new("Beam cross-section", "mm");
    dr.scale_note = "Scale: NTS".into();
    dr.layer("concrete", "#4a4a4a").layer("rebar", "#c0392b").layer("stirrup", "#1f6fb2").layer("dims", "#2b2b2b").layer("text", "#111111");
    dr.add(Entity::Polyline { layer: "concrete".into(), points: vec![[0.0, 0.0], [b, 0.0], [b, h], [0.0, h]], closed: true, hatch: Some("concrete".into()) });
    // stirrup centreline with a 135-degree hook at the top-left corner
    let o = cover + ds / 2.0;
    dr.add(Entity::Polyline { layer: "stirrup".into(), points: vec![[o, o], [b - o, o], [b - o, h - o], [o, h - o]], closed: true, hatch: None });
    let hook = (6.0 * ds).max(75.0).min(0.3 * (b - 2.0 * o));
    let k = hook / std::f64::consts::SQRT_2;
    dr.add(Entity::Line { layer: "stirrup".into(), a: [o, h - o], b: [o + k, h - o - k] });
    dr.add(Entity::Line { layer: "stirrup".into(), a: [o + ds, h - o], b: [o + ds + k, h - o - k] });
    // tension bars, evenly spread in one layer
    let y = cover + ds + db / 2.0;
    let x0 = cover + ds + db / 2.0;
    let x1 = b - x0;
    for i in 0..n {
        let x = if n == 1 { b / 2.0 } else { x0 + (x1 - x0) * i as f64 / (n - 1) as f64 };
        dr.add(Entity::Circle { layer: "rebar".into(), c: [x, y], r: db / 2.0, filled: true });
    }
    // dimensions
    let off = 0.12 * h.max(b);
    dr.add(Entity::Dim { layer: "dims".into(), a: [0.0, 0.0], b: [b, 0.0], offset: -off, text: None });
    dr.add(Entity::Dim { layer: "dims".into(), a: [0.0, 0.0], b: [0.0, h], offset: off, text: None });
    dr.add(Entity::Dim { layer: "dims".into(), a: [b, h], b: [b, h - d], offset: 1.4 * off, text: Some(format!("d = {}", fmt4(d))) });
    dr.add(Entity::Dim { layer: "dims".into(), a: [b, 0.0], b: [b, cover], offset: -0.6 * off, text: Some(format!("cover {}", fmt4(cover))) });
    // labels
    let th = 0.05 * h.max(b);
    dr.add(Entity::Text { layer: "text".into(), at: [b / 2.0, -2.2 * off], text: format!("{bars} (bottom)"), height: th, align: "center".into() });
    dr.add(Entity::Text { layer: "text".into(), at: [b / 2.0, -2.2 * off - 1.6 * th], text: format!("Stirrup {stirrup}"), height: th, align: "center".into() });
    dr
}
