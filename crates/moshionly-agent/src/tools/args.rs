//! Reading a tool's arguments, with errors a model can act on.
//!
//! Models get arguments slightly wrong in a handful of predictable ways: a
//! misspelt name, a number in quotes, a list sent as a JSON string, one id
//! where a list is expected. [`Args`] accepts the harmless ones and answers
//! the others with a message that names the argument, says what it holds and
//! what would be accepted.

use std::ops::RangeInclusive;

use serde_json::{Map, Value};

use crate::tool::ToolError;

/// The arguments of a tool call, or of one object nested in them.
#[derive(Debug, Clone)]
pub(crate) struct Args {
    /// Where these arguments are, for messages: empty for the tool's own
    /// input, `operations[2]` for an object inside it.
    path: String,
    values: Map<String, Value>,
}

// Every reader is in use once all the tools are in place.
#[allow(dead_code)]
impl Args {
    /// Reads the input of a tool, which may only have the `allowed` names.
    /// A missing input counts as an empty one.
    pub fn new(arguments: &Value, allowed: &[&str]) -> Result<Self, ToolError> {
        Self::nested("", arguments, allowed)
    }

    /// Reads an object found at `path` inside a tool's input.
    pub fn nested(
        path: impl Into<String>,
        value: &Value,
        allowed: &[&str],
    ) -> Result<Self, ToolError> {
        let path = path.into();
        let values = match from_json_text(value) {
            Value::Object(values) => values,
            Value::Null => Map::new(),
            other => {
                let what = if path.is_empty() {
                    "The arguments".to_owned()
                } else {
                    format!("\"{path}\"")
                };
                return Err(ToolError::argument(
                    path,
                    format!(
                        "{what} should be a JSON object with named fields, but this is {}.",
                        shown(&other)
                    ),
                ));
            }
        };
        let args = Self { path, values };
        if let Some(unknown) = args
            .values
            .keys()
            .find(|name| !allowed.contains(&name.as_str()))
        {
            let hint = closest(unknown, allowed)
                .map(|name| format!(" Did you mean \"{name}\"?"))
                .unwrap_or_default();
            let owner = if args.path.is_empty() {
                "an argument of this tool".to_owned()
            } else {
                format!("a field of \"{}\"", args.path)
            };
            return Err(ToolError::argument(
                args.field(unknown),
                format!(
                    "\"{unknown}\" is not {owner}.{hint} It takes: {}.",
                    allowed.join(", ")
                ),
            ));
        }
        Ok(args)
    }

    /// The full path of one of these arguments, for messages.
    pub fn field(&self, name: &str) -> String {
        if self.path.is_empty() {
            name.to_owned()
        } else {
            format!("{}.{name}", self.path)
        }
    }

    /// The value of an argument. A missing argument and an explicit `null`
    /// are both `None`.
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.values.get(name).filter(|value| !value.is_null())
    }

    /// Whether the argument was given.
    pub fn has(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// The error for an argument that has to be there and is not.
    pub fn missing(&self, name: &str, what: &str) -> ToolError {
        let field = self.field(name);
        ToolError::argument(field.clone(), format!("\"{field}\" is required: {what}."))
    }

    /// The error for an argument that holds the wrong thing.
    pub fn wrong(&self, name: &str, expected: &str) -> ToolError {
        let field = self.field(name);
        let found = self.get(name).map_or("missing".to_owned(), shown);
        ToolError::argument(
            field.clone(),
            format!("\"{field}\" should be {expected}, but it is {found}."),
        )
    }

    /// A text argument.
    pub fn string(&self, name: &str) -> Result<Option<String>, ToolError> {
        match self.get(name) {
            None => Ok(None),
            Some(Value::String(text)) => Ok(Some(text.clone())),
            Some(_) => Err(self.wrong(name, "text in quotes")),
        }
    }

    /// A text argument that has to be there. `what` says what it is for.
    pub fn required_string(&self, name: &str, what: &str) -> Result<String, ToolError> {
        self.string(name)?.ok_or_else(|| self.missing(name, what))
    }

    /// A number. A number written in quotes is accepted.
    pub fn number(&self, name: &str) -> Result<Option<f64>, ToolError> {
        let Some(value) = self.get(name) else {
            return Ok(None);
        };
        as_number(value)
            .map(Some)
            .ok_or_else(|| self.wrong(name, "a number"))
    }

    /// A whole number inside `range`.
    pub fn integer(
        &self,
        name: &str,
        range: RangeInclusive<i64>,
    ) -> Result<Option<i64>, ToolError> {
        let Some(value) = self.get(name) else {
            return Ok(None);
        };
        let expected = || format!("a whole number from {} to {}", range.start(), range.end());
        let number = as_number(value).ok_or_else(|| self.wrong(name, &expected()))?;
        if number.fract() != 0.0 || number < *range.start() as f64 || number > *range.end() as f64 {
            return Err(self.wrong(name, &expected()));
        }
        Ok(Some(number as i64))
    }

    /// A yes or no. `"true"` and `"false"` in quotes are accepted.
    pub fn boolean(&self, name: &str) -> Result<Option<bool>, ToolError> {
        match self.get(name) {
            None => Ok(None),
            Some(Value::Bool(flag)) => Ok(Some(*flag)),
            Some(Value::String(text)) if text == "true" => Ok(Some(true)),
            Some(Value::String(text)) if text == "false" => Ok(Some(false)),
            Some(_) => Err(self.wrong(name, "true or false")),
        }
    }

    /// One of a fixed set of words.
    pub fn choice<'c>(
        &self,
        name: &str,
        choices: &[&'c str],
    ) -> Result<Option<&'c str>, ToolError> {
        let Some(value) = self.get(name) else {
            return Ok(None);
        };
        let expected = || {
            let quoted: Vec<String> = choices
                .iter()
                .map(|choice| format!("\"{choice}\""))
                .collect();
            format!("one of {}", quoted.join(", "))
        };
        let text = value
            .as_str()
            .ok_or_else(|| self.wrong(name, &expected()))?;
        choices
            .iter()
            .copied()
            .find(|choice| *choice == text)
            .map(Some)
            .ok_or_else(|| self.wrong(name, &expected()))
    }

    /// A list of anything. A list sent as a JSON string is accepted.
    pub fn array(&self, name: &str) -> Result<Option<Vec<Value>>, ToolError> {
        let Some(value) = self.get(name) else {
            return Ok(None);
        };
        match from_json_text(value) {
            Value::Array(items) => Ok(Some(items)),
            _ => Err(self.wrong(name, "a list")),
        }
    }

    /// A list of texts. One text on its own counts as a list of one.
    pub fn strings(&self, name: &str) -> Result<Option<Vec<String>>, ToolError> {
        let Some(value) = self.get(name) else {
            return Ok(None);
        };
        let expected = "a list of texts in quotes";
        match from_json_text(value) {
            Value::String(text) => Ok(Some(vec![text])),
            Value::Array(items) => items
                .into_iter()
                .map(|item| match item {
                    Value::String(text) => Ok(text),
                    _ => Err(self.wrong(name, expected)),
                })
                .collect::<Result<Vec<_>, _>>()
                .map(Some),
            _ => Err(self.wrong(name, expected)),
        }
    }

    /// A list of numbers. One number on its own counts as a list of one.
    pub fn numbers(&self, name: &str) -> Result<Option<Vec<f64>>, ToolError> {
        let Some(value) = self.get(name) else {
            return Ok(None);
        };
        let expected = "a list of numbers";
        match from_json_text(value) {
            Value::Array(items) => items
                .iter()
                .map(|item| as_number(item).ok_or_else(|| self.wrong(name, expected)))
                .collect::<Result<Vec<_>, _>>()
                .map(Some),
            single => as_number(&single)
                .map(|number| Some(vec![number]))
                .ok_or_else(|| self.wrong(name, expected)),
        }
    }

    /// An object. An object sent as a JSON string is accepted.
    pub fn object(&self, name: &str) -> Result<Option<Map<String, Value>>, ToolError> {
        let Some(value) = self.get(name) else {
            return Ok(None);
        };
        match from_json_text(value) {
            Value::Object(fields) => Ok(Some(fields)),
            _ => Err(self.wrong(name, "a JSON object with named fields")),
        }
    }
}

/// A finite number from a JSON number or from a number in quotes.
fn as_number(value: &Value) -> Option<f64> {
    let number = match value {
        Value::Number(number) => number.as_f64()?,
        Value::String(text) => text.trim().parse().ok()?,
        _ => return None,
    };
    number.is_finite().then_some(number)
}

/// A list or an object that arrived as a string of JSON, unpacked. Anything
/// else is returned as it is.
fn from_json_text(value: &Value) -> Value {
    if let Value::String(text) = value {
        let text = text.trim();
        if (text.starts_with('[') || text.starts_with('{'))
            && let Ok(parsed @ (Value::Array(_) | Value::Object(_))) = serde_json::from_str(text)
        {
            return parsed;
        }
    }
    value.clone()
}

/// A value as it is quoted in a message: compact JSON, cut short.
pub(crate) fn shown(value: &Value) -> String {
    const LIMIT: usize = 80;
    let text = value.to_string();
    if text.chars().count() <= LIMIT {
        return text;
    }
    let mut short: String = text.chars().take(LIMIT).collect();
    short.push('…');
    short
}

/// Number of slips between two short words, where a slip is a letter
/// added, dropped or replaced, or two neighbouring letters swapped.
fn edit_distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut rows = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for (i, row) in rows.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in rows[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let substitution = rows[i - 1][j - 1] + usize::from(a[i - 1] != b[j - 1]);
            let mut best = substitution.min(rows[i - 1][j] + 1).min(rows[i][j - 1] + 1);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                best = best.min(rows[i - 2][j - 2] + 1);
            }
            rows[i][j] = best;
        }
    }
    rows[a.len()][b.len()]
}

/// The candidate a misspelt `word` most likely meant, when one is close.
pub(crate) fn closest<'c>(word: &str, candidates: &[&'c str]) -> Option<&'c str> {
    let word = word.to_lowercase();
    candidates
        .iter()
        .copied()
        .filter_map(|candidate| {
            let shorter = word.chars().count().min(candidate.chars().count());
            let contained = shorter >= 3 && (candidate.contains(&word) || word.contains(candidate));
            let distance = if contained {
                1
            } else {
                edit_distance(&word, candidate)
            };
            // One slip for every three letters, and never a whole rewrite.
            (distance <= (shorter / 3).max(1) && distance < shorter)
                .then_some((distance, candidate))
        })
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, candidate)| candidate)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const ALLOWED: &[&str] = &[
        "id", "time", "times", "ids", "depth", "exact", "set", "kind",
    ];

    fn args(value: Value) -> Args {
        Args::new(&value, ALLOWED).unwrap()
    }

    fn message(error: ToolError) -> String {
        error.to_string()
    }

    #[test]
    fn missing_arguments_are_an_empty_object() {
        let args = Args::new(&Value::Null, ALLOWED).unwrap();
        assert!(!args.has("id"));
        assert_eq!(args.string("id").unwrap(), None);
        assert_eq!(
            message(args.required_string("id", "the id of a node").unwrap_err()),
            "\"id\" is required: the id of a node."
        );
        // An explicit null is the same as leaving the argument out.
        assert!(!self::args(json!({ "id": null })).has("id"));
    }

    #[test]
    fn an_unknown_name_gets_a_suggestion_and_the_list() {
        let error = Args::new(&json!({ "tiem": 3 }), ALLOWED).unwrap_err();
        assert!(matches!(&error, ToolError::Argument { field, .. } if field == "tiem"));
        assert_eq!(
            message(error),
            "\"tiem\" is not an argument of this tool. Did you mean \"time\"? It takes: id, \
             time, times, ids, depth, exact, set, kind."
        );
        let error =
            Args::nested("operations[2]", &json!({ "colour": 1 }), &["op", "id"]).unwrap_err();
        assert_eq!(
            message(error),
            "\"colour\" is not a field of \"operations[2]\". It takes: op, id."
        );
    }

    #[test]
    fn arguments_that_are_not_an_object_are_refused() {
        let error = Args::new(&json!([1, 2]), ALLOWED).unwrap_err();
        assert_eq!(
            message(error),
            "The arguments should be a JSON object with named fields, but this is [1,2]."
        );
        let error = Args::nested("crop", &json!(5), &["x"]).unwrap_err();
        assert_eq!(
            message(error),
            "\"crop\" should be a JSON object with named fields, but this is 5."
        );
    }

    #[test]
    fn numbers_may_come_in_quotes() {
        let args = args(json!({ "time": "500", "depth": 2.0, "id": 7 }));
        assert_eq!(args.number("time").unwrap(), Some(500.0));
        assert_eq!(args.integer("depth", 0..=5).unwrap(), Some(2));
        assert_eq!(
            message(args.string("id").unwrap_err()),
            "\"id\" should be text in quotes, but it is 7."
        );
        let args = self::args(json!({ "time": "soon", "depth": 9 }));
        assert_eq!(
            message(args.number("time").unwrap_err()),
            "\"time\" should be a number, but it is \"soon\"."
        );
        assert_eq!(
            message(args.integer("depth", 0..=5).unwrap_err()),
            "\"depth\" should be a whole number from 0 to 5, but it is 9."
        );
        assert!(
            self::args(json!({ "depth": 1.5 }))
                .integer("depth", 0..=5)
                .is_err()
        );
    }

    #[test]
    fn lists_may_come_as_one_item_or_as_json_text() {
        let args = args(json!({ "ids": "ab12", "times": 250 }));
        assert_eq!(args.strings("ids").unwrap(), Some(vec!["ab12".to_owned()]));
        assert_eq!(args.numbers("times").unwrap(), Some(vec![250.0]));

        let args = self::args(json!({ "ids": "[\"a\", \"b\"]", "times": "[0, 500]" }));
        assert_eq!(
            args.strings("ids").unwrap(),
            Some(vec!["a".to_owned(), "b".to_owned()])
        );
        assert_eq!(args.numbers("times").unwrap(), Some(vec![0.0, 500.0]));
        assert_eq!(args.array("times").unwrap().unwrap().len(), 2);

        let args = self::args(json!({ "ids": [1], "times": ["x"] }));
        assert_eq!(
            message(args.strings("ids").unwrap_err()),
            "\"ids\" should be a list of texts in quotes, but it is [1]."
        );
        assert!(args.numbers("times").is_err());
    }

    #[test]
    fn objects_flags_and_choices() {
        let args = args(json!({ "set": "{\"x\": 1}", "exact": "true", "kind": "layer" }));
        assert_eq!(args.object("set").unwrap().unwrap()["x"], 1);
        assert_eq!(args.boolean("exact").unwrap(), Some(true));
        assert_eq!(
            args.choice("kind", &["layer", "animation"]).unwrap(),
            Some("layer")
        );
        let args = self::args(json!({ "set": 3, "exact": "yes", "kind": "shape" }));
        assert!(args.object("set").is_err());
        assert_eq!(
            message(args.boolean("exact").unwrap_err()),
            "\"exact\" should be true or false, but it is \"yes\"."
        );
        assert_eq!(
            message(args.choice("kind", &["layer", "animation"]).unwrap_err()),
            "\"kind\" should be one of \"layer\", \"animation\", but it is \"shape\"."
        );
    }

    #[test]
    fn nested_fields_are_named_with_their_path() {
        let args = Args::nested("operations[1]", &json!({ "id": 4 }), &["id"]).unwrap();
        let error = args.string("id").unwrap_err();
        assert!(matches!(&error, ToolError::Argument { field, .. } if field == "operations[1].id"));
        assert_eq!(
            message(error),
            "\"operations[1].id\" should be text in quotes, but it is 4."
        );
    }

    #[test]
    fn long_values_are_cut_short_in_messages() {
        let long = json!("x".repeat(500));
        let text = shown(&long);
        assert_eq!(text.chars().count(), 81);
        assert!(text.ends_with('…'));
    }

    #[test]
    fn suggestions_need_to_be_close() {
        assert_eq!(closest("nmae", &["name", "parent"]), Some("name"));
        assert_eq!(closest("target_id", &["target", "type"]), Some("target"));
        assert_eq!(closest("zzz", &["name", "parent"]), None);
        assert_eq!(closest("x", &["y"]), None);
    }
}
