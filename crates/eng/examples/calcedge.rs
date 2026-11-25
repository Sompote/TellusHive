use th_eng::calc::{run_sheet, CalcInput, CheckDef, StepDef, VarDef};

fn sheet(label: &str, inputs: &[(&str, f64)], steps: &[(&str, &str, Option<(&str, &str)>)]) {
    let input = CalcInput {
        title: label.into(),
        standard: String::new(),
        inputs: inputs.iter().map(|(s, v)| VarDef { symbol: s.to_string(), value: *v, unit: String::new(), description: String::new() }).collect(),
        steps: steps.iter().map(|(s, e, c)| StepDef { symbol: s.to_string(), expr: e.to_string(), unit: String::new(), description: String::new(), reference: String::new(), check: c.map(|(d, cap)| CheckDef { demand: d.into(), capacity: cap.into(), label: "chk".into() }) }).collect(),
        notes: vec![],
    };
    match run_sheet(&input) {
        Ok(s) => println!("{label}: OK all_pass={} values={:?}", s.all_pass, s.steps.iter().map(|x| (x.symbol.clone(), x.value)).collect::<Vec<_>>()),
        Err(e) => println!("{label}: ERROR {e}"),
    }
}

fn main() {
    sheet("C1 division by zero", &[("a", 1.0), ("b", 0.0)], &[("x", "a/b", None)]);
    sheet("C2 sqrt of negative", &[("a", -4.0)], &[("x", "sqrt(a)", None)]);
    sheet("C3 undefined symbol", &[("a", 1.0)], &[("x", "a + bb", None)]);
    sheet("C4 overflow", &[("a", 10.0)], &[("x", "a^400", None)]);
    sheet("C5 failing check", &[("Mu", 150.0), ("phiMn", 147.2)], &[("u", "Mu/phiMn", Some(("Mu", "phiMn")))]);
    sheet("C6 degrees trig", &[("phi", 30.0)], &[("Ka", "(1 - sind(phi))/(1 + sind(phi))", None), ("t", "tand(45 + phi/2)^2", None)]);
    sheet("C7 radians trap", &[("phi", 30.0)], &[("Ka", "(1 - sin(phi))/(1 + sin(phi))", None)]);
    sheet("C8 redefine an input", &[("a", 1.0)], &[("a", "2*a", None)]);
    sheet("C9 0/0 in check", &[("a", 0.0)], &[("x", "a", Some(("a", "a")))]);
    sheet("C10 log of zero", &[("a", 0.0)], &[("x", "ln(a)", None)]);
}
