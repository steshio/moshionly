//! Writing Lottie JSON: numbers rounded to what can be seen, and values left
//! out when they are the default.
//!
//! Lottie files are read by players on phones and in browsers, and their
//! size matters. Numbers are rounded to a precision that cannot be seen
//! (a thousandth of a pixel, a ten-thousandth of a colour channel), whole
//! numbers are written without a decimal point, and properties at their
//! default are omitted.

use serde_json::{Map, Value};

/// How many decimals each kind of number keeps.
pub(crate) mod places {
    /// Times in frames.
    pub const TIME: i32 = 3;
}

/// `value` rounded to `places` decimals.
pub(crate) fn round(value: f64, places: i32) -> f64 {
    let scale = 10f64.powi(places);
    let rounded = (value * scale).round() / scale;
    if !rounded.is_finite() {
        return 0.0;
    }
    // No negative zero.
    if rounded == 0.0 { 0.0 } else { rounded }
}

/// A number rounded to `places` decimals, written as an integer when it is
/// one.
pub(crate) fn num(value: f64, places: i32) -> Value {
    let rounded = round(value, places);
    if rounded == rounded.trunc() && rounded.abs() < 1e15 {
        Value::from(rounded as i64)
    } else {
        Value::from(rounded)
    }
}

/// A JSON object with the given entries, in order.
pub(crate) fn object<const N: usize>(entries: [(&str, Value); N]) -> Value {
    let mut map = Map::with_capacity(N);
    for (key, value) in entries {
        map.insert(key.to_owned(), value);
    }
    Value::Object(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_short() {
        assert_eq!(num(100.0, 3).to_string(), "100");
        assert_eq!(num(0.1 + 0.2, 3).to_string(), "0.3");
        assert_eq!(num(1.23456, 2).to_string(), "1.23");
        assert_eq!(num(-0.0001, 3).to_string(), "0");
        assert_eq!(num(f64::NAN, 3).to_string(), "0");
    }

    #[test]
    fn objects_keep_their_order() {
        let value = object([("ty", Value::from(4)), ("nm", Value::from("Card"))]);
        assert_eq!(value.to_string(), r#"{"ty":4,"nm":"Card"}"#);
    }
}
