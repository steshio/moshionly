//! Numeric fields: type a number or a calculation, drag to scrub, step with
//! the arrow keys.

mod expr;
mod field;

pub use expr::{ExprContext, ExprError, evaluate, format_value};
pub use field::{NumericField, NumericResponse, scrub_value, step_value};
