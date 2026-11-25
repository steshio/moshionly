//! A few readable words for each tool call, so the chat shows "Edit · Fade
//! in the title" rather than a blob of JSON.
//!
//! The words are built from the call's arguments. They follow the tools of
//! `moshionly-agent`; a tool this module does not know still gets a title
//! made from its name and a summary of its first plain arguments.

use serde_json::Value;

/// The longest summary, in characters.
const MAX_CHARS: usize = 80;

/// The tool's name for people: `apply_edits` becomes "Edit".
pub fn title(name: &str) -> String {
    let known = match name {
        "apply_edits" => "Edit",
        "screenshot" => "Screenshot",
        "evaluate" => "Measure",
        "get_overview" => "Overview",
        "query_nodes" => "Find",
        "get_node" => "Inspect",
        "get_schema" => "Read the format",
        "list_fonts" => "Fonts",
        "import_image" => "Import image",
        "export" => "Export",
        "undo_last_agent_change" => "Undo",
        _ => return generic_title(name),
    };
    known.to_owned()
}

/// Finds the name a node is shown with from its id, when it has one.
pub type NameOf<'a> = &'a dyn Fn(&str) -> Option<String>;

/// A few words saying what a call to `name` with `input` was asked to do.
/// Empty when the arguments say nothing worth showing. Node ids are shown
/// as they are; see [`describe_with`] to show names instead.
pub fn describe(name: &str, input: &Value) -> String {
    describe_with(name, input, &|_| None)
}

/// Like [`describe`], with the nodes the call names shown by the names
/// `name_of` finds for their ids.
pub fn describe_with(name: &str, input: &Value, name_of: NameOf<'_>) -> String {
    let node = |key: &str| string(input, key).map(|id| node_name(id, name_of));
    let text = match name {
        "apply_edits" => edits(input, name_of),
        "screenshot" => screenshot(input, name_of),
        "evaluate" => evaluate(input),
        "query_nodes" => query(input, name_of),
        "get_node" => node("id").unwrap_or_default(),
        "get_schema" => string(input, "type").map(str::to_owned).unwrap_or_default(),
        "list_fonts" => string(input, "query").map(quoted).unwrap_or_default(),
        "import_image" => string(input, "source").map(file_name).unwrap_or_default(),
        "export" => export(input, name_of),
        "get_overview" | "undo_last_agent_change" => String::new(),
        _ => generic(input),
    };
    shorten(&text)
}

/// `some_tool_name` as "Some tool name", without an MCP server prefix.
fn generic_title(name: &str) -> String {
    let bare = name.rsplit("__").next().unwrap_or(name);
    let spaced = bare.replace(['_', '-'], " ");
    let mut characters = spaced.trim().chars();
    match characters.next() {
        Some(first) => first.to_uppercase().chain(characters).collect(),
        None => "Tool".to_owned(),
    }
}

fn edits(input: &Value, name_of: NameOf<'_>) -> String {
    if let Some(label) = string(input, "label") {
        return label.to_owned();
    }
    let Some(operations) = input.get("operations").and_then(Value::as_array) else {
        return String::new();
    };
    let described: Vec<String> = operations
        .iter()
        .take(3)
        .map(|op| operation(op, name_of))
        .collect();
    let mut text = described.join(", ");
    let rest = operations.len().saturating_sub(described.len());
    if rest > 0 {
        text.push_str(&format!(" and {rest} more"));
    }
    if input.get("dry_run").and_then(Value::as_bool) == Some(true) {
        text.push_str(" (check only)");
    }
    text
}

/// One operation of `apply_edits` in two or three words.
fn operation(operation: &Value, name_of: NameOf<'_>) -> String {
    // `make_component` reads as "make component".
    let op = string(operation, "op")
        .unwrap_or("change")
        .replace('_', " ");
    let what = operation
        .get("node")
        .and_then(|node| {
            string(node, "name")
                .map(quoted)
                .or_else(|| string(node, "type").map(str::to_owned))
        })
        .or_else(|| string(operation, "id").map(|id| node_name(id, name_of)))
        .or_else(|| {
            let count = operation.get("ids").and_then(Value::as_array)?.len();
            Some(format!("{count} nodes"))
        });
    match what {
        Some(what) => format!("{op} {what}"),
        None => op.to_owned(),
    }
}

fn screenshot(input: &Value, name_of: NameOf<'_>) -> String {
    let subject = string(input, "layer")
        .map(|layer| format!("layer {}", node_name(layer, name_of)))
        .or_else(|| string(input, "artboard").map(|id| node_name(id, name_of)));
    let times = times(input);
    match (subject, times) {
        (Some(subject), Some(times)) => format!("{subject} at {times}"),
        (Some(subject), None) => subject,
        (None, Some(times)) => format!("at {times}"),
        (None, None) => String::new(),
    }
}

fn evaluate(input: &Value) -> String {
    let properties = strings(input, "properties");
    let layers = count(input, "layers");
    let mut text = if properties.is_empty() {
        "values".to_owned()
    } else {
        list(&properties)
    };
    if let Some(layers) = layers {
        text.push_str(&format!(" of {}", plural(layers, "layer", "layers")));
    }
    if let Some(times) = times(input) {
        text.push_str(&format!(" at {times}"));
    }
    text
}

fn query(input: &Value, name_of: NameOf<'_>) -> String {
    let mut words = Vec::new();
    let types = strings(input, "type");
    if !types.is_empty() {
        words.push(list(&types));
    } else if let Some(family) = string(input, "family") {
        words.push(format!("{family}s"));
    }
    if let Some(name) = string(input, "name_exact").or_else(|| string(input, "name")) {
        words.push(format!("named {}", quoted(name)));
    }
    if let Some(target) = string(input, "target") {
        words.push(format!("on {}", node_name(target, name_of)));
    }
    if let Some(artboard) = string(input, "artboard") {
        words.push(format!("in {}", node_name(artboard, name_of)));
    }
    words.join(" ")
}

fn export(input: &Value, name_of: NameOf<'_>) -> String {
    let format = string(input, "format").map(str::to_uppercase);
    let target = string(input, "file_name")
        .or_else(|| string(input, "path").map(file_name_str))
        .map(str::to_owned)
        .or_else(|| string(input, "artboard").map(|id| node_name(id, name_of)));
    match (format, target) {
        (Some(format), Some(target)) => format!("{format} of {target}"),
        (Some(format), None) => format,
        (None, Some(target)) => target,
        (None, None) => String::new(),
    }
}

/// The first few plain arguments of a tool this module does not know, as
/// `key: value`.
fn generic(input: &Value) -> String {
    let Some(object) = input.as_object() else {
        return String::new();
    };
    let shown: Vec<String> = object
        .iter()
        .filter_map(|(key, value)| {
            let value = match value {
                Value::String(text) => quoted(text),
                Value::Number(number) => number.to_string(),
                Value::Bool(flag) => flag.to_string(),
                _ => return None,
            };
            Some(format!("{key}: {value}"))
        })
        .take(2)
        .collect();
    shown.join(", ")
}

/// `time` or `times` as "500 ms" or "0, 500 and 1000 ms".
fn times(input: &Value) -> Option<String> {
    let mut values: Vec<String> = input
        .get("times")
        .and_then(Value::as_array)
        .map(|times| times.iter().filter_map(number).collect())
        .unwrap_or_default();
    if values.is_empty()
        && let Some(time) = input.get("time").and_then(number)
    {
        values.push(time);
    }
    if values.is_empty() {
        return None;
    }
    if values.len() > 4 {
        return Some(format!("{} times", values.len()));
    }
    Some(format!("{} ms", list(&values)))
}

fn number(value: &Value) -> Option<String> {
    let number = value.as_f64()?;
    Some(if number.fract() == 0.0 && number.abs() < 1e15 {
        format!("{}", number as i64)
    } else {
        format!("{number}")
    })
}

fn string<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

fn strings(value: &Value, key: &str) -> Vec<String> {
    match value.get(key) {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        Some(Value::String(text)) => vec![text.clone()],
        _ => Vec::new(),
    }
}

fn count(value: &Value, key: &str) -> Option<usize> {
    value.get(key).and_then(Value::as_array).map(Vec::len)
}

fn quoted(text: &str) -> String {
    format!("“{text}”")
}

/// A node in quotes: its name when `name_of` knows it, else its id.
fn node_name(id: &str, name_of: NameOf<'_>) -> String {
    match name_of(id) {
        Some(name) if !name.trim().is_empty() => quoted(name.trim()),
        _ => quoted(id),
    }
}

fn file_name(path: &str) -> String {
    file_name_str(path).to_owned()
}

fn file_name_str(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// "a", "a and b", "a, b and c".
fn list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// Cuts `text` to [`MAX_CHARS`] on a character boundary, on one line.
fn shorten(text: &str) -> String {
    let line = text.lines().next().unwrap_or("").trim();
    if line.chars().count() <= MAX_CHARS {
        return line.to_owned();
    }
    let mut short: String = line.chars().take(MAX_CHARS - 1).collect();
    short.push('…');
    short
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn known_tools_have_short_titles() {
        assert_eq!(title("apply_edits"), "Edit");
        assert_eq!(title("undo_last_agent_change"), "Undo");
        assert_eq!(title("mcp__other__make_coffee"), "Make coffee");
        assert_eq!(title(""), "Tool");
    }

    #[test]
    fn edits_use_their_label_or_list_their_operations() {
        assert_eq!(
            describe(
                "apply_edits",
                &json!({ "label": "Fade in the title", "operations": [] })
            ),
            "Fade in the title"
        );
        let input = json!({ "operations": [
            { "op": "add", "node": { "type": "rect", "name": "Card" } },
            { "op": "add", "node": { "type": "fade_in", "target": "card" } },
            { "op": "update", "id": "title", "set": { "x": 4 } },
            { "op": "remove", "ids": ["a", "b"] },
        ]});
        assert_eq!(
            describe("apply_edits", &input),
            "add “Card”, add fade_in, update “title” and 1 more"
        );
        assert_eq!(
            describe(
                "apply_edits",
                &json!({ "operations": [{ "op": "remove", "ids": ["a", "b"] }], "dry_run": true })
            ),
            "remove 2 nodes (check only)"
        );
        let components = json!({ "operations": [
            { "op": "make_component", "ids": ["a", "b"] },
            { "op": "detach", "id": "copy" },
        ]});
        assert_eq!(
            describe("apply_edits", &components),
            "make component 2 nodes, detach “copy”"
        );
    }

    #[test]
    fn screenshots_say_what_and_when() {
        assert_eq!(
            describe("screenshot", &json!({ "artboard": "Launch", "time": 500 })),
            "“Launch” at 500 ms"
        );
        assert_eq!(
            describe(
                "screenshot",
                &json!({ "layer": "t1", "times": [0, 250.5, 1000] })
            ),
            "layer “t1” at 0, 250.5 and 1000 ms"
        );
        assert_eq!(
            describe("screenshot", &json!({ "times": [0, 1, 2, 3, 4, 5] })),
            "at 6 times"
        );
        assert_eq!(describe("screenshot", &json!({})), "");
    }

    #[test]
    fn measuring_names_properties_layers_and_times() {
        assert_eq!(
            describe(
                "evaluate",
                &json!({ "layers": ["a", "b"], "properties": ["opacity", "x"], "times": [0, 500] })
            ),
            "opacity and x of 2 layers at 0 and 500 ms"
        );
    }

    #[test]
    fn finding_describes_the_filters() {
        assert_eq!(
            describe(
                "query_nodes",
                &json!({ "type": ["text"], "name": "Title", "artboard": "Launch" })
            ),
            "text named “Title” in “Launch”"
        );
        assert_eq!(
            describe("query_nodes", &json!({ "family": "animation" })),
            "animations"
        );
    }

    #[test]
    fn simple_tools_show_their_one_argument() {
        assert_eq!(describe("get_node", &json!({ "id": "t1" })), "“t1”");
        assert_eq!(describe("get_schema", &json!({ "type": "text" })), "text");
        assert_eq!(
            describe(
                "import_image",
                &json!({ "source": "/Users/me/Pictures/logo.png" })
            ),
            "logo.png"
        );
        assert_eq!(
            describe("export", &json!({ "format": "mp4", "artboard": "Launch" })),
            "MP4 of “Launch”"
        );
        assert_eq!(describe("get_overview", &json!({})), "");
    }

    #[test]
    fn nodes_are_shown_by_name_when_known() {
        let name_of = |id: &str| (id == "a1").then(|| "Launch".to_owned());
        assert_eq!(
            describe_with(
                "screenshot",
                &json!({ "artboard": "a1", "time": 0 }),
                &name_of
            ),
            "“Launch” at 0 ms"
        );
        assert_eq!(
            describe_with("get_node", &json!({ "id": "zz" }), &name_of),
            "“zz”"
        );
        assert_eq!(
            describe_with(
                "apply_edits",
                &json!({ "operations": [{ "op": "update", "id": "a1" }] }),
                &name_of
            ),
            "update “Launch”"
        );
    }

    #[test]
    fn unknown_tools_show_their_first_plain_arguments() {
        assert_eq!(
            describe(
                "echo",
                &json!({ "text": "hi", "nested": { "a": 1 }, "n": 3, "x": true })
            ),
            "text: “hi”, n: 3"
        );
        assert_eq!(describe("echo", &json!("not an object")), "");
    }

    #[test]
    fn summaries_stay_on_one_short_line() {
        let long = "é".repeat(200);
        let summary = describe("apply_edits", &json!({ "label": long }));
        assert_eq!(summary.chars().count(), MAX_CHARS);
        assert!(summary.ends_with('…'));
        assert_eq!(
            describe("apply_edits", &json!({ "label": "first\nsecond" })),
            "first"
        );
    }
}
