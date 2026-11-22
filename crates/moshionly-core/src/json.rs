//! Helpers for working with JSON values.

use serde_json::{Number, Value};

/// Largest magnitude at which every whole number is exactly representable as
/// an `f64`.
const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_992.0;

/// Rewrites whole floating-point numbers as integers, in place.
///
/// Properties are `f64` in Rust, which serde prints as `240.0`. Files and
/// messages read better, and stay smaller, with `240`.
pub(crate) fn tidy_numbers(value: &mut Value) {
    match value {
        Value::Number(number) => {
            if let Some(float) = number.as_f64().filter(|_| number.is_f64())
                && float.fract() == 0.0
                && float.abs() < MAX_SAFE_INTEGER
            {
                *number = Number::from(float as i64);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(tidy_numbers),
        Value::Object(fields) => fields.values_mut().for_each(tidy_numbers),
        Value::Null | Value::Bool(_) | Value::String(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn whole_floats_become_integers_everywhere() {
        let mut value =
            json!({ "x": 240.0, "y": -0.0, "points": [1.5, 2.0], "nested": { "w": 1e3 } });
        tidy_numbers(&mut value);
        assert_eq!(
            value.to_string(),
            r#"{"x":240,"y":0,"points":[1.5,2],"nested":{"w":1000}}"#
        );
    }

    #[test]
    fn huge_and_fractional_numbers_are_left_alone() {
        let mut value = json!([1e300, 0.1, 9_007_199_254_740_993_u64]);
        tidy_numbers(&mut value);
        assert_eq!(value, json!([1e300, 0.1, 9_007_199_254_740_993_u64]));
    }
}
