//! The small arithmetic language of numeric fields.
//!
//! A numeric field accepts a plain number or a short calculation:
//!
//! | Typed | Field | Result |
//! |---|---|---|
//! | `100+20` | any | `120` |
//! | `(40 + 8) * 2` | any | `96` |
//! | `1,5` | any | `1.5` (comma as decimal mark) |
//! | `50%*2` | opacity, unit `%` | `100` |
//! | `50%` | width, currently `300` | `150` (percent of the current value) |
//! | `1.5s` | time, unit `ms` | `1500` |
//!
//! Grammar, with the usual precedence and left-to-right evaluation:
//!
//! ```text
//! expr    := term (("+" | "-") term)*
//! term    := factor (("*" | "×" | "/" | "÷") factor)*
//! factor  := ("-" | "+") factor | primary
//! primary := number unit? | "(" expr ")"
//! ```

use thiserror::Error;

/// Why an expression could not be evaluated.
#[derive(Clone, Debug, PartialEq, Error)]
pub enum ExprError {
    /// The input was empty or only whitespace.
    #[error("enter a number")]
    Empty,
    /// A character that does not belong in an expression.
    #[error("unexpected \"{found}\" at position {position}")]
    UnexpectedChar {
        /// The offending character.
        found: char,
        /// Zero-based character index in the input.
        position: usize,
    },
    /// The input ended in the middle of an expression, for example `2+`.
    #[error("the expression is incomplete")]
    UnexpectedEnd,
    /// A `(` without its `)`.
    #[error("a parenthesis is not closed")]
    UnclosedParenthesis,
    /// A unit the field does not understand.
    #[error("unknown unit \"{0}\"")]
    UnknownUnit(String),
    /// Division by zero.
    #[error("cannot divide by zero")]
    DivisionByZero,
    /// The result is not a finite number.
    #[error("the result is not a number")]
    NotFinite,
}

/// What an expression is evaluated against.
#[derive(Clone, Copy, Debug, Default)]
pub struct ExprContext<'a> {
    /// The field's current value. `50%` means half of this, unless the field's
    /// own unit is `%`.
    pub current: f64,
    /// The field's unit, for example `"px"`, `"%"`, `"ms"` or `"°"`. A number
    /// followed by this unit is taken as is.
    pub unit: &'a str,
    /// Other units the field accepts, each with the factor that converts a
    /// value in that unit into the field's unit. A time field in milliseconds
    /// passes `[("s", 1000.0)]`.
    pub conversions: &'a [(&'a str, f64)],
}

/// Evaluates what the user typed into a numeric field.
pub fn evaluate(input: &str, context: &ExprContext<'_>) -> Result<f64, ExprError> {
    let chars: Vec<char> = input.chars().collect();
    let mut parser = Parser {
        chars: &chars,
        position: 0,
        context,
    };
    parser.skip_whitespace();
    if parser.peek().is_none() {
        return Err(ExprError::Empty);
    }
    let value = parser.expr()?;
    parser.skip_whitespace();
    if let Some(found) = parser.peek() {
        return Err(ExprError::UnexpectedChar {
            found,
            position: parser.position,
        });
    }
    if value.is_finite() {
        Ok(value)
    } else {
        Err(ExprError::NotFinite)
    }
}

struct Parser<'a> {
    chars: &'a [char],
    position: usize,
    context: &'a ExprContext<'a>,
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.position).copied()
    }

    fn skip_whitespace(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.position += 1;
        }
    }

    /// Consumes `expected` if it is next, ignoring leading whitespace.
    fn eat(&mut self, expected: &[char]) -> Option<char> {
        self.skip_whitespace();
        let next = self.peek()?;
        if expected.contains(&next) {
            self.position += 1;
            Some(next)
        } else {
            None
        }
    }

    fn expr(&mut self) -> Result<f64, ExprError> {
        let mut value = self.term()?;
        // A minus can be typed as a hyphen or as a real minus sign.
        while let Some(op) = self.eat(&['+', '-', '−']) {
            let right = self.term()?;
            if op == '+' {
                value += right;
            } else {
                value -= right;
            }
        }
        Ok(value)
    }

    fn term(&mut self) -> Result<f64, ExprError> {
        let mut value = self.factor()?;
        while let Some(op) = self.eat(&['*', '×', '/', '÷']) {
            let right = self.factor()?;
            if matches!(op, '*' | '×') {
                value *= right;
            } else if right == 0.0 {
                return Err(ExprError::DivisionByZero);
            } else {
                value /= right;
            }
        }
        Ok(value)
    }

    fn factor(&mut self) -> Result<f64, ExprError> {
        match self.eat(&['-', '−', '+']) {
            Some('+') => self.factor(),
            Some(_) => Ok(-self.factor()?),
            None => self.primary(),
        }
    }

    fn primary(&mut self) -> Result<f64, ExprError> {
        self.skip_whitespace();
        match self.peek() {
            None => Err(ExprError::UnexpectedEnd),
            Some('(') => {
                self.position += 1;
                let value = self.expr()?;
                if self.eat(&[')']).is_none() {
                    return Err(match self.peek() {
                        None => ExprError::UnclosedParenthesis,
                        Some(found) => ExprError::UnexpectedChar {
                            found,
                            position: self.position,
                        },
                    });
                }
                Ok(value)
            }
            Some(c) if c.is_ascii_digit() || c == '.' || c == ',' => {
                let number = self.number()?;
                self.unit(number)
            }
            Some(found) => Err(ExprError::UnexpectedChar {
                found,
                position: self.position,
            }),
        }
    }

    fn number(&mut self) -> Result<f64, ExprError> {
        let start = self.position;
        let mut text = String::new();
        let mut seen_mark = false;
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                text.push(c);
            } else if (c == '.' || c == ',') && !seen_mark {
                seen_mark = true;
                text.push('.');
            } else {
                break;
            }
            self.position += 1;
        }
        text.parse::<f64>().map_err(|_| ExprError::UnexpectedChar {
            found: self.chars[start],
            position: start,
        })
    }

    /// Applies the unit that follows a number, if any.
    fn unit(&mut self, number: f64) -> Result<f64, ExprError> {
        // No whitespace is skipped here: `50 %` is unusual, and skipping would
        // make `2 s` ambiguous with a typo.
        let start = self.position;
        let mut unit = String::new();
        while let Some(c) = self.peek() {
            if c.is_alphabetic() || c == '%' || c == '°' {
                unit.push(c);
                self.position += 1;
            } else {
                break;
            }
        }
        if unit.is_empty() || unit.eq_ignore_ascii_case(self.context.unit) {
            return Ok(number);
        }
        if unit == "%" {
            return Ok(self.context.current * number / 100.0);
        }
        if let Some((_, factor)) = self
            .context
            .conversions
            .iter()
            .find(|(name, _)| unit.eq_ignore_ascii_case(name))
        {
            return Ok(number * factor);
        }
        self.position = start;
        Err(ExprError::UnknownUnit(unit))
    }
}

/// Formats a value for display in a field: at most `decimals` places, without
/// trailing zeros and without a negative zero.
pub fn format_value(value: f64, decimals: usize) -> String {
    let mut text = format!("{value:.decimals$}");
    if text.contains('.') {
        while text.ends_with('0') {
            text.pop();
        }
        if text.ends_with('.') {
            text.pop();
        }
    }
    if text == "-0" {
        text = "0".to_owned();
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(input: &str) -> Result<f64, ExprError> {
        evaluate(input, &ExprContext::default())
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn plain_numbers() {
        assert_eq!(plain("42"), Ok(42.0));
        assert_eq!(plain("  3.5 "), Ok(3.5));
        assert_eq!(plain(".5"), Ok(0.5));
        assert_eq!(plain("1,5"), Ok(1.5));
        assert_eq!(plain("-12"), Ok(-12.0));
        assert_eq!(plain("+7"), Ok(7.0));
    }

    #[test]
    fn arithmetic_with_precedence() {
        assert_eq!(plain("100+20"), Ok(120.0));
        assert_eq!(plain("100 - 20 - 30"), Ok(50.0));
        assert_eq!(plain("2+3*4"), Ok(14.0));
        assert_eq!(plain("(2+3)*4"), Ok(20.0));
        assert_eq!(plain("10/4"), Ok(2.5));
        assert_eq!(plain("2×3"), Ok(6.0));
        assert_eq!(plain("9÷3"), Ok(3.0));
        assert_eq!(plain("-(2+3)"), Ok(-5.0));
        assert_eq!(plain("2*-3"), Ok(-6.0));
        assert_eq!(plain("8/2/2"), Ok(2.0));
        assert_eq!(plain("5−2"), Ok(3.0));
    }

    #[test]
    fn percent_in_a_percent_field_is_just_the_unit() {
        let context = ExprContext {
            current: 80.0,
            unit: "%",
            conversions: &[],
        };
        assert_eq!(evaluate("50%*2", &context), Ok(100.0));
        assert_eq!(evaluate("25%", &context), Ok(25.0));
    }

    #[test]
    fn percent_elsewhere_is_relative_to_the_current_value() {
        let context = ExprContext {
            current: 300.0,
            unit: "px",
            conversions: &[],
        };
        assert_eq!(evaluate("50%", &context), Ok(150.0));
        assert_eq!(evaluate("50%*2", &context), Ok(300.0));
        assert_eq!(evaluate("100% + 20", &context), Ok(320.0));
    }

    #[test]
    fn units_and_conversions() {
        let context = ExprContext {
            current: 0.0,
            unit: "ms",
            conversions: &[("s", 1000.0)],
        };
        assert_eq!(evaluate("250ms", &context), Ok(250.0));
        assert_eq!(evaluate("1.5s", &context), Ok(1500.0));
        assert_eq!(evaluate("1s + 250", &context), Ok(1250.0));
        assert_eq!(evaluate("2S", &context), Ok(2000.0));
        assert_eq!(
            evaluate("3px", &context),
            Err(ExprError::UnknownUnit("px".to_owned()))
        );
        let degrees = ExprContext {
            current: 0.0,
            unit: "°",
            conversions: &[],
        };
        assert_eq!(evaluate("90°", &degrees), Ok(90.0));
    }

    #[test]
    fn errors_say_what_is_wrong() {
        assert_eq!(plain(""), Err(ExprError::Empty));
        assert_eq!(plain("   "), Err(ExprError::Empty));
        assert_eq!(plain("2+"), Err(ExprError::UnexpectedEnd));
        assert_eq!(plain("(2+3"), Err(ExprError::UnclosedParenthesis));
        assert_eq!(plain("4/0"), Err(ExprError::DivisionByZero));
        assert_eq!(
            plain("2 $ 3"),
            Err(ExprError::UnexpectedChar {
                found: '$',
                position: 2
            })
        );
        assert_eq!(
            plain("2 3"),
            Err(ExprError::UnexpectedChar {
                found: '3',
                position: 2
            })
        );
        assert_eq!(
            plain("abc"),
            Err(ExprError::UnexpectedChar {
                found: 'a',
                position: 0
            })
        );
        assert!(plain("1.2.3").is_err());
        assert!(!plain("2+").unwrap_err().to_string().is_empty());
    }

    #[test]
    fn never_panics_on_odd_input() {
        for input in [
            "((((",
            "))))",
            "%%",
            "1e309",
            "..",
            "-",
            "*",
            "１２",
            "9".repeat(400).as_str(),
        ] {
            let _ = plain(input);
        }
        assert_eq!(plain(&"9".repeat(400)), Err(ExprError::NotFinite));
    }

    #[test]
    fn formatting_trims_zeros() {
        assert_eq!(format_value(12.0, 2), "12");
        assert_eq!(format_value(12.5, 2), "12.5");
        assert_eq!(format_value(12.345, 2), "12.35");
        assert_eq!(format_value(-0.001, 2), "0");
        assert_eq!(format_value(100.0, 0), "100");
        assert_eq!(format_value(0.1 + 0.2, 2), "0.3");
        assert!(close(plain("0.1+0.2").unwrap(), 0.3));
    }
}
