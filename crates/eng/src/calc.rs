//! A traceable calculation sheet. The agent writes the inputs and the steps (each an
//! expression that may use the inputs and every earlier step); the engine evaluates them in
//! order and records, for each step, the expression, the expression with the numbers put in,
//! the value, and the clause it came from. Optional checks compare a demand with a capacity.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::expr::{self, fmt4};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct VarDef {
    pub symbol: String,
    pub value: f64,
    pub unit: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct StepDef {
    pub symbol: String,
    pub expr: String,
    pub unit: String,
    pub description: String,
    /// The clause the step comes from, e.g. "ACI 318-19 22.2.2.4.1".
    pub reference: String,
    pub check: Option<CheckDef>,
}

/// Passes when demand <= capacity. Both are expressions over the variables known at that
/// step, including the step's own symbol.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct CheckDef {
    pub demand: String,
    pub capacity: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct CalcInput {
    pub title: String,
    pub standard: String,
    pub inputs: Vec<VarDef>,
    pub steps: Vec<StepDef>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Step {
    pub symbol: String,
    pub description: String,
    pub expr: String,
    pub substituted: String,
    pub value: f64,
    pub unit: String,
    pub reference: String,
    pub check: Option<Check>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Check {
    pub label: String,
    pub demand: f64,
    pub capacity: f64,
    /// demand / capacity (the utilisation).
    pub ratio: f64,
    pub pass: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Sheet {
    pub title: String,
    pub standard: String,
    pub inputs: Vec<VarDef>,
    pub steps: Vec<Step>,
    pub notes: Vec<String>,
    pub all_pass: bool,
}

/// Utilisation demand/capacity, robust to a zero capacity.
pub fn ratio(demand: f64, capacity: f64) -> f64 {
    if capacity.abs() > 1e-300 {
        demand / capacity
    } else if demand <= 0.0 {
        0.0
    } else {
        f64::INFINITY
    }
}

/// Builds a `Check` from numbers. A relative tolerance of 1e-9 absorbs rounding noise.
pub fn make_check(label: &str, demand: f64, capacity: f64) -> Check {
    let pass = demand <= capacity + 1e-9 * capacity.abs().max(demand.abs());
    Check { label: label.into(), demand, capacity, ratio: ratio(demand, capacity), pass }
}

/// Evaluates the sheet. Inputs first, then the steps in order; each step's symbol becomes a
/// variable for later steps. Errors name the failing input or step.
pub fn run_sheet(input: &CalcInput) -> Result<Sheet, String> {
    let mut vars: BTreeMap<String, f64> = BTreeMap::new();
    for v in &input.inputs {
        if !expr::is_identifier(&v.symbol) {
            return Err(format!("input '{}': symbol must be letters, digits and underscores, starting with a letter", v.symbol));
        }
        if !v.value.is_finite() {
            return Err(format!("input '{}': value is not a finite number", v.symbol));
        }
        vars.insert(v.symbol.clone(), v.value);
    }
    let mut steps = Vec::with_capacity(input.steps.len());
    for (k, s) in input.steps.iter().enumerate() {
        let name = if s.symbol.is_empty() { format!("#{}", k + 1) } else { s.symbol.clone() };
        if !expr::is_identifier(&s.symbol) {
            return Err(format!("step {} '{}': symbol must be letters, digits and underscores, starting with a letter", k + 1, s.symbol));
        }
        let value = expr::eval(&s.expr, &vars).map_err(|e| format!("step {} '{}' ({}): {e}", k + 1, name, s.expr))?;
        let substituted = expr::substitute(&s.expr, &vars);
        vars.insert(s.symbol.clone(), value);
        let check = match &s.check {
            None => None,
            Some(c) => {
                let d = expr::eval(&c.demand, &vars).map_err(|e| format!("step {} '{}': check demand ({}): {e}", k + 1, name, c.demand))?;
                let cap = expr::eval(&c.capacity, &vars).map_err(|e| format!("step {} '{}': check capacity ({}): {e}", k + 1, name, c.capacity))?;
                let label = if c.label.is_empty() { format!("{} <= {}", c.demand, c.capacity) } else { c.label.clone() };
                Some(make_check(&label, d, cap))
            }
        };
        steps.push(Step {
            symbol: s.symbol.clone(),
            description: s.description.clone(),
            expr: s.expr.clone(),
            substituted,
            value,
            unit: s.unit.clone(),
            reference: s.reference.clone(),
            check,
        });
    }
    let all_pass = steps.iter().filter_map(|s| s.check.as_ref()).all(|c| c.pass);
    Ok(Sheet {
        title: input.title.clone(),
        standard: input.standard.clone(),
        inputs: input.inputs.clone(),
        steps,
        notes: input.notes.clone(),
        all_pass,
    })
}

/// Escapes the pipe so a cell does not break a Markdown table.
fn cell(s: &str) -> String {
    s.replace('|', "\\|").replace('\n', " ")
}

impl Sheet {
    /// Every check in step order.
    pub fn checks(&self) -> Vec<&Check> {
        self.steps.iter().filter_map(|s| s.check.as_ref()).collect()
    }

    /// The value of a step or input by symbol.
    pub fn value(&self, symbol: &str) -> Option<f64> {
        self.steps
            .iter()
            .rev()
            .find(|s| s.symbol == symbol)
            .map(|s| s.value)
            .or_else(|| self.inputs.iter().find(|v| v.symbol == symbol).map(|v| v.value))
    }

    pub fn to_markdown(&self) -> String {
        let mut o = String::new();
        if !self.title.is_empty() {
            o.push_str(&format!("### {}\n\n", self.title));
        }
        if !self.standard.is_empty() {
            o.push_str(&format!("Standard: {}\n\n", self.standard));
        }
        if !self.inputs.is_empty() {
            o.push_str("**Inputs**\n\n| Symbol | Value | Unit | Description |\n|---|---:|---|---|\n");
            for v in &self.inputs {
                o.push_str(&format!("| {} | {} | {} | {} |\n", cell(&v.symbol), fmt4(v.value), cell(&v.unit), cell(&v.description)));
            }
            o.push('\n');
        }
        if !self.steps.is_empty() {
            o.push_str("**Calculation**\n\n");
            for s in &self.steps {
                let desc = if s.description.is_empty() { String::new() } else { format!("{}: ", s.description) };
                let num = if s.substituted != s.expr && s.substituted != fmt4(s.value) { format!(" = {}", s.substituted) } else { String::new() };
                let r = if s.reference.is_empty() { String::new() } else { format!(" _({})_", s.reference) };
                o.push_str(&format!("- {desc}`{} = {}{num}` = **{} {}**{r}\n", s.symbol, s.expr, fmt4(s.value), s.unit));
            }
            o.push('\n');
        }
        let checks = self.checks();
        if !checks.is_empty() {
            o.push_str("**Checks**\n\n| Check | Demand | Capacity | Ratio | Result |\n|---|---:|---:|---:|---|\n");
            for c in checks {
                o.push_str(&format!(
                    "| {} | {} | {} | {:.3} | {} |\n",
                    cell(&c.label),
                    fmt4(c.demand),
                    fmt4(c.capacity),
                    c.ratio,
                    if c.pass { "PASS" } else { "**FAIL**" }
                ));
            }
            o.push('\n');
        }
        if !self.notes.is_empty() {
            o.push_str("**Notes**\n\n");
            for n in &self.notes {
                o.push_str(&format!("- {n}\n"));
            }
            o.push('\n');
        }
        o.push_str(if self.all_pass { "Result: all checks PASS.\n" } else { "Result: one or more checks FAIL.\n" });
        o
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sheet_runs_in_order_and_names_failing_step() {
        let inp: CalcInput = serde_json::from_value(serde_json::json!({
            "title": "Simple beam",
            "inputs": [{"symbol": "w", "value": 10, "unit": "kN/m"}, {"symbol": "L", "value": 6, "unit": "m"}],
            "steps": [
                {"symbol": "M", "expr": "w*L^2/8", "unit": "kN m", "reference": "statics",
                 "check": {"demand": "M", "capacity": "50", "label": "M <= 50"}},
                {"symbol": "V", "expr": "w*L/2", "unit": "kN"}
            ]
        }))
        .unwrap();
        let s = run_sheet(&inp).unwrap();
        assert_eq!(s.steps[0].value, 45.0);
        assert_eq!(s.steps[0].substituted, "10*6^2/8");
        assert!(s.all_pass);
        assert!((s.steps[0].check.as_ref().unwrap().ratio - 0.9).abs() < 1e-12);
        assert!(s.to_markdown().contains("PASS"));

        let mut bad = inp.clone();
        bad.steps[1].expr = "w*Lx/2".into();
        let e = run_sheet(&bad).unwrap_err();
        assert!(e.contains("'V'") && e.contains("Lx"), "{e}");
    }
}
