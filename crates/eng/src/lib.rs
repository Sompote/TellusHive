//! th-eng: the deterministic engineering engine that the agents call.
//!
//! - [`expr`]: a small expression evaluator, used to show workings.
//! - [`calc`]: traceable calculation sheets (inputs, steps with clause references, checks).
//! - [`drawing`]: 2D drawings exported to SVG, DXF R12 and PNG.
//! - [`design`]: verified design modules (RC beam ACI 318-19, Terzaghi and EC7 bearing,
//!   cantilever retaining wall).
//! - [`report`]: calculation reports as A4 PDF and Markdown.
//!
//! Errors are `Result<T, String>`. Units: kN, kPa, m in geotechnics; MPa, mm in RC sections.

pub mod calc;
pub mod design;
pub mod drawing;
pub mod expr;
pub mod report;
