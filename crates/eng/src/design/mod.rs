//! Built-in, verified design modules. Each one takes JSON arguments, builds a calculation
//! sheet with the calc engine (so every step shows its expression, the numbers put in and
//! the clause), and returns the sheet, a drawing and a one-paragraph summary.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::calc::{self, CalcInput, CheckDef, Sheet, StepDef, VarDef};
use crate::drawing::Drawing;
use crate::expr;

mod bearing_ec7;
mod bearing_terzaghi;
mod rc_beam;
mod retaining_wall;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Design {
    pub module: String,
    pub sheet: calc::Sheet,
    pub drawing: Option<Drawing>,
    pub summary: String,
    pub pass: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModuleInfo {
    pub name: String,
    pub title: String,
    pub standard: String,
    pub description: String,
    /// JSON Schema object: properties (units in the descriptions), required, defaults.
    pub input_schema: Value,
}

pub fn modules() -> Vec<ModuleInfo> {
    vec![rc_beam::info(), bearing_terzaghi::info(), bearing_ec7::info(), retaining_wall::info()]
}

pub fn run_module(name: &str, args: &Value) -> Result<Design, String> {
    let args = if args.is_null() { &Value::Object(Default::default()) } else { args };
    if !args.is_object() {
        return Err("module arguments must be a JSON object".into());
    }
    match name {
        "rc_beam_aci318" => rc_beam::run(args),
        "bearing_terzaghi" => bearing_terzaghi::run(args),
        "bearing_ec7" => bearing_ec7::run(args),
        "retaining_wall_cantilever" => retaining_wall::run(args),
        _ => Err(format!(
            "unknown module '{name}'; available: {}",
            modules().iter().map(|m| m.name.as_str()).collect::<Vec<_>>().join(", ")
        )),
    }
}

// ------------------------------------------------------------------ argument helpers

/// Reads a number; accepts JSON numbers and numeric strings. `default` None = required.
pub(crate) fn num(args: &Value, key: &str, default: Option<f64>) -> Result<f64, String> {
    match args.get(key) {
        None | Some(Value::Null) => default.ok_or_else(|| format!("missing required input '{key}'")),
        Some(Value::Number(n)) => n.as_f64().filter(|v| v.is_finite()).ok_or_else(|| format!("input '{key}' is not a finite number")),
        Some(Value::String(s)) => s.trim().parse::<f64>().map_err(|_| format!("input '{key}' must be a number, got \"{s}\"")),
        Some(v) => Err(format!("input '{key}' must be a number, got {v}")),
    }
}

pub(crate) fn opt_num(args: &Value, key: &str) -> Result<Option<f64>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        _ => num(args, key, None).map(Some),
    }
}

pub(crate) fn text(args: &Value, key: &str, default: &str) -> Result<String, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(default.into()),
        Some(Value::String(s)) => Ok(s.trim().to_lowercase()),
        Some(v) => Err(format!("input '{key}' must be a string, got {v}")),
    }
}

pub(crate) fn flag(args: &Value, key: &str, default: bool) -> Result<bool, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(Value::Bool(b)) => Ok(*b),
        Some(Value::String(s)) => match s.trim().to_lowercase().as_str() {
            "true" | "yes" | "1" => Ok(true),
            "false" | "no" | "0" => Ok(false),
            _ => Err(format!("input '{key}' must be true or false")),
        },
        Some(v) => Err(format!("input '{key}' must be true or false, got {v}")),
    }
}

pub(crate) fn positive(key: &str, v: f64) -> Result<f64, String> {
    if v > 0.0 {
        Ok(v)
    } else {
        Err(format!("input '{key}' must be greater than zero (got {v})"))
    }
}

pub(crate) fn non_negative(key: &str, v: f64) -> Result<f64, String> {
    if v >= 0.0 {
        Ok(v)
    } else {
        Err(format!("input '{key}' must not be negative (got {v})"))
    }
}

/// One JSON-schema property.
pub(crate) fn prop(kind: &str, description: &str, default: Option<Value>) -> Value {
    let mut p = json!({ "type": kind, "description": description });
    if let Some(d) = default {
        p["default"] = d;
    }
    p
}

pub(crate) fn schema(props: Vec<(&str, Value)>, required: &[&str]) -> Value {
    let mut m = serde_json::Map::new();
    for (k, v) in props {
        m.insert(k.to_string(), v);
    }
    json!({ "type": "object", "properties": Value::Object(m), "required": required })
}

// ------------------------------------------------------------------ sheet builder

/// Builds a `CalcInput` while evaluating as it goes, so a module can branch on values
/// (e.g. pick the shear formula). `finish` re-runs the whole sheet through `calc::run_sheet`,
/// so the returned sheet is exactly what the generic engine produces from the same input.
pub(crate) struct SheetBuilder {
    input: CalcInput,
    vars: BTreeMap<String, f64>,
}

impl SheetBuilder {
    pub fn new(title: &str, standard: &str) -> Self {
        SheetBuilder { input: CalcInput { title: title.into(), standard: standard.into(), ..Default::default() }, vars: BTreeMap::new() }
    }
    pub fn input(&mut self, symbol: &str, value: f64, unit: &str, description: &str) -> f64 {
        self.input.inputs.push(VarDef { symbol: symbol.into(), value, unit: unit.into(), description: description.into() });
        self.vars.insert(symbol.into(), value);
        value
    }
    pub fn step(&mut self, symbol: &str, expr_src: &str, unit: &str, description: &str, reference: &str) -> Result<f64, String> {
        self.push(symbol, expr_src, unit, description, reference, None)
    }
    /// A step with a check `demand <= capacity` (both expressions).
    #[allow(clippy::too_many_arguments)]
    pub fn check(&mut self, symbol: &str, expr_src: &str, unit: &str, description: &str, reference: &str, demand: &str, capacity: &str, label: &str) -> Result<f64, String> {
        let c = CheckDef { demand: demand.into(), capacity: capacity.into(), label: label.into() };
        self.push(symbol, expr_src, unit, description, reference, Some(c))
    }
    fn push(&mut self, symbol: &str, expr_src: &str, unit: &str, description: &str, reference: &str, check: Option<CheckDef>) -> Result<f64, String> {
        let v = expr::eval(expr_src, &self.vars).map_err(|e| format!("{symbol} = {expr_src}: {e}"))?;
        self.vars.insert(symbol.into(), v);
        self.input.steps.push(StepDef { symbol: symbol.into(), expr: expr_src.into(), unit: unit.into(), description: description.into(), reference: reference.into(), check });
        Ok(v)
    }
    pub fn get(&self, symbol: &str) -> f64 {
        self.vars.get(symbol).copied().unwrap_or(f64::NAN)
    }
    pub fn note(&mut self, s: &str) {
        self.input.notes.push(s.into());
    }
    pub fn finish(self) -> Result<Sheet, String> {
        calc::run_sheet(&self.input)
    }
}

/// A short PASS/FAIL list of every check, for summaries.
pub(crate) fn check_summary(sheet: &Sheet) -> String {
    sheet
        .checks()
        .iter()
        .map(|c| format!("{} ({:.2}, {})", c.label, c.ratio, if c.pass { "ok" } else { "FAIL" }))
        .collect::<Vec<_>>()
        .join("; ")
}
