//! The reference tables of `docs/file-format.md`, generated from the schema
//! so that the documentation cannot drift from the code.

use std::fmt::Write as _;

use serde_json::Value;

use super::{
    defs, node_schema, ref_name, snake_case, string_choices, without_default_sentence, without_null,
};
use crate::model::{AnimationKind, Family, NodeType};
use crate::validate::animation_applies_to;

/// The fields every node has, which the reference explains once in prose.
const BASE_FIELDS: [&str; 4] = ["id", "parent", "index", "type"];

/// `a`, `a or b`, `a, b or c`.
fn list_or(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [only] => only.clone(),
        [rest @ .., last] => format!("{} or {last}", rest.join(", ")),
    }
}

/// What a schema accepts, as Markdown: short enums spelt out, other shared
/// types as links to their section.
fn value(schema: &Value) -> String {
    let (schema, _) = without_null(schema);
    if let Some(name) = ref_name(schema) {
        if name == "NodeId" {
            return "id".into();
        }
        return match defs().get(name).and_then(string_choices) {
            Some(choices) if choices.len() <= 5 => list_or(
                &choices
                    .iter()
                    .map(|choice| format!("`\"{choice}\"`"))
                    .collect::<Vec<_>>(),
            ),
            _ => {
                let name = snake_case(name);
                format!("[{name}](#{name})")
            }
        };
    }
    if let Some(constant) = schema.get("const") {
        return format!("`{constant}`");
    }
    if schema["pattern"]
        .as_str()
        .is_some_and(|pattern| pattern.starts_with("^#("))
    {
        return "colour".into();
    }
    let ty = match &schema["type"] {
        Value::String(ty) => ty.as_str(),
        Value::Array(types) => types
            .iter()
            .filter_map(Value::as_str)
            .find(|ty| *ty != "null")
            .unwrap_or("value"),
        _ => "value",
    };
    match ty {
        "array" => {
            let item = value(&schema["items"]);
            match (schema["minItems"].as_u64(), schema["maxItems"].as_u64()) {
                (Some(min), Some(max)) if min == max && max <= 4 => {
                    format!("`[{}]`", vec![item; max as usize].join(", "))
                }
                _ => format!("list of {item}"),
            }
        }
        "integer" => "whole number".into(),
        other => other.to_owned(),
    }
}

/// Text made safe for a table cell.
fn cell(text: &str) -> String {
    text.replace('|', "\\|")
}

/// A table of the properties of an object schema. `keep` picks the fields.
/// Returns nothing when no field is picked.
fn table(schema: &Value, keep: impl Fn(&str) -> bool) -> String {
    let required: Vec<&str> = schema["required"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let mut rows = String::new();
    for (name, property) in schema["properties"].as_object().into_iter().flatten() {
        if !keep(name) {
            continue;
        }
        let description = property["description"].as_str().unwrap_or_default();
        let default = property.get("default").filter(|value| !value.is_null());
        let (default, meaning) = match default {
            Some(default) => (
                format!("`{default}`"),
                without_default_sentence(description),
            ),
            None if required.contains(&name.as_str()) => ("required".to_owned(), description),
            // The column already says so.
            None => (
                "none".to_owned(),
                description
                    .strip_suffix("Default: none.")
                    .map_or(description, str::trim_end),
            ),
        };
        let _ = writeln!(
            rows,
            "| `{name}` | {} | {} | {} |",
            cell(&value(property)),
            cell(&default),
            cell(meaning)
        );
    }
    if rows.is_empty() {
        return rows;
    }
    format!("| Field | Value | Default | Meaning |\n|---|---|---|---|\n{rows}")
}

/// The field names that all of `types` have, apart from the base fields.
fn shared_fields(types: &[NodeType]) -> Vec<String> {
    let mut shared: Option<Vec<String>> = None;
    for ty in types {
        let names = super::field_names(*ty);
        shared = Some(match shared {
            None => names.to_vec(),
            Some(shared) => shared
                .into_iter()
                .filter(|name| names.contains(name))
                .collect(),
        });
    }
    shared
        .unwrap_or_default()
        .into_iter()
        .filter(|name| !BASE_FIELDS.contains(&name.as_str()))
        .collect()
}

/// One section per node type, with the fields that are its own.
fn type_sections(text: &mut String, types: &[NodeType], shared: &[String]) {
    for ty in types {
        let _ = writeln!(text, "#### `{ty}`\n\n{}\n", ty.description());
        let own = table(&node_schema(*ty), |name| {
            !BASE_FIELDS.contains(&name) && !shared.iter().any(|shared| shared == name)
        });
        if own.is_empty() {
            let _ = writeln!(text, "It has no fields of its own.\n");
        } else {
            let _ = writeln!(text, "{own}");
        }
    }
}

/// A section for one shared value type.
fn value_type_section(text: &mut String, name: &str, definition: &Value) {
    let _ = writeln!(text, "#### `{}`\n", snake_case(name));
    let description = definition["description"].as_str().unwrap_or_default();
    let _ = writeln!(text, "{description}\n");
    if let Some(choices) = definition["oneOf"]
        .as_array()
        .filter(|_| string_choices(definition).is_some())
    {
        let _ = writeln!(text, "| Value | Meaning |\n|---|---|");
        for choice in choices {
            let _ = writeln!(
                text,
                "| `{}` | {} |",
                choice["const"],
                cell(choice["description"].as_str().unwrap_or_default())
            );
        }
        text.push('\n');
        return;
    }
    if definition.get("properties").is_some() {
        let _ = writeln!(text, "{}", table(definition, |_| true));
    }
    let options = definition
        .get("oneOf")
        .or_else(|| definition.get("anyOf"))
        .and_then(Value::as_array);
    for option in options.into_iter().flatten() {
        let description = option["description"].as_str().unwrap_or_default();
        match option["properties"]["type"]["const"].as_str() {
            // A tagged alternative: its own little table.
            Some(tag) => {
                let _ = writeln!(text, "With `\"type\": \"{tag}\"`: {description}\n");
                let fields = table(option, |name| name != "type");
                if !fields.is_empty() {
                    let _ = writeln!(text, "{fields}");
                }
            }
            None => {
                let _ = writeln!(text, "- {}: {description}", value(option));
            }
        }
    }
    if options.is_some() {
        text.push('\n');
    }
}

/// The reference part of `docs/file-format.md` as Markdown: every layer
/// type, every animation type and every shared value type, with every field,
/// what it holds, its default and its meaning.
///
/// The text is generated from the node definitions. A test compares it with
/// the document, so the two cannot drift apart.
pub fn reference() -> String {
    let of_kind = |wanted: Option<AnimationKind>| -> Vec<NodeType> {
        NodeType::ALL
            .iter()
            .copied()
            .filter(|ty| ty.animation_kind() == wanted)
            .collect()
    };
    let layers = of_kind(None);
    let timed: Vec<NodeType> = NodeType::ALL
        .iter()
        .copied()
        .filter(|ty| {
            matches!(
                ty.animation_kind(),
                Some(AnimationKind::Property | AnimationKind::Preset | AnimationKind::Text)
            )
        })
        .collect();
    let layer_fields = shared_fields(&layers);
    let timed_fields = shared_fields(&timed);
    let mut text = String::new();

    let _ = writeln!(text, "### Fields of every layer\n");
    let _ = writeln!(
        text,
        "{}",
        table(&node_schema(NodeType::Rect), |name| layer_fields
            .iter()
            .any(|field| field == name))
    );
    let _ = writeln!(text, "### Layer types\n");
    type_sections(&mut text, &layers, &layer_fields);

    let _ = writeln!(text, "### Fields of every animation\n");
    let _ = writeln!(
        text,
        "Every animation that runs over time has these. `show`, `hide` and `animation_group` \
         list their own fields below.\n"
    );
    let _ = writeln!(
        text,
        "{}",
        table(&node_schema(NodeType::Move), |name| timed_fields
            .iter()
            .any(|field| field == name))
    );
    for (heading, kind, intro) in [
        (
            "Property animations",
            AnimationKind::Property,
            "Each changes one property of its target between two values.",
        ),
        (
            "Instant animations",
            AnimationKind::Instant,
            "These happen at one moment and have no easing.",
        ),
        (
            "Preset animations",
            AnimationKind::Preset,
            "Ready-made entrances and exits, each stored as one node with a few settings.",
        ),
        (
            "Text animations",
            AnimationKind::Text,
            "These work on the content of a text layer.",
        ),
        (
            "Timeline structure",
            AnimationKind::Structure,
            "Folders for the timeline. They do not change playback.",
        ),
    ] {
        let _ = writeln!(text, "### {heading}\n\n{intro}\n");
        let shared: &[String] = match kind {
            AnimationKind::Instant | AnimationKind::Structure => &[],
            _ => &timed_fields,
        };
        type_sections(&mut text, &of_kind(Some(kind)), shared);
    }

    let _ = writeln!(text, "### Which layers an animation works on\n");
    let _ = writeln!(text, "| Animation | Works on |\n|---|---|");
    for animation in NodeType::ALL {
        if animation.family() != Family::Animation || *animation == NodeType::AnimationGroup {
            continue;
        }
        let targets: Vec<String> = layers
            .iter()
            .filter(|layer| animation_applies_to(*animation, **layer))
            .map(|layer| format!("`{layer}`"))
            .collect();
        let all_but_artboards = targets.len() == layers.len() - 1
            && !animation_applies_to(*animation, NodeType::Artboard);
        let targets = if all_but_artboards {
            "every layer except `artboard`".to_owned()
        } else {
            targets.join(", ")
        };
        let _ = writeln!(text, "| `{animation}` | {targets} |");
    }
    text.push('\n');

    let _ = writeln!(text, "### Value types\n");
    for (name, definition) in defs() {
        if name != "NodeId" {
            value_type_section(&mut text, name, definition);
        }
    }
    // Sections end with a blank line; never more than one.
    while text.contains("\n\n\n") {
        text = text.replace("\n\n\n", "\n\n");
    }
    text.truncate(text.trim_end().len());
    text.push('\n');
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reference_covers_every_type_and_field() {
        let reference = reference();
        for ty in NodeType::ALL {
            assert!(
                reference.contains(&format!("#### `{ty}`\n")),
                "no section for {ty}"
            );
            for field in super::super::field_names(*ty) {
                if !BASE_FIELDS.contains(&field.as_str()) {
                    assert!(
                        reference.contains(&format!("| `{field}` |")),
                        "{ty}: no row for {field}"
                    );
                }
            }
        }
        for name in defs().keys().filter(|name| *name != "NodeId") {
            assert!(
                reference.contains(&format!("#### `{}`\n", snake_case(name))),
                "no section for the value type {name}"
            );
        }
    }

    #[test]
    fn shared_fields_are_listed_once() {
        let reference = reference();
        // `opacity` is a field of every layer, and the name of an animation
        // type, but only one table explains the field.
        assert_eq!(
            reference.matches("| `opacity` | number | `100` |").count(),
            1
        );
        assert_eq!(
            reference
                .matches("| `easing` | [easing](#easing) |")
                .count(),
            1
        );
        assert!(reference.contains("| `sweep` | number | `360` | How far the arc runs"));
        assert!(reference.contains("| `source` | string | required |"));
        assert!(reference.contains("| `fill` | [paint](#paint) | none | Paint of the inside. |"));
        assert!(
            reference.contains("| none | Size at the start. Default: the size at that moment. |")
        );
        assert!(reference.contains("| `fade_in` | every layer except `artboard` |"));
        assert!(!reference.contains("\n\n\n"));
        assert!(
            reference
                .contains("| `align` | `\"left\"`, `\"center\"` or `\"right\"` | `\"left\"` |")
        );
        assert!(reference.contains("| `blend_mode` | [blend_mode](#blend_mode) | `\"normal\"` |"));
        assert!(reference.contains("| `corner_radius` | `group`, `rect`, `image` |"));
        assert!(
            reference
                .contains("With `\"type\": \"linear_gradient\"`: Colours change along a line.")
        );
        assert!(reference.contains("| `\"multiply\"` | Multiply colours; always darker. |"));
        assert!(reference.contains("#### `mask_group`\n\nA group whose first child (lowest index) is an alpha mask for the other children.\n\nIt has no fields of its own."));
    }
}
