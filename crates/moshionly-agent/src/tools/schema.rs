//! `get_schema`: what node types there are and what fields each has.
//!
//! Everything here is read from `moshionly_core::schema`, which derives it
//! from the node definitions, so the answers cannot drift from what the
//! validator accepts.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use moshionly_core::validate::animation_applies_to;
use moshionly_core::{Family, Node, NodeId, NodeType, build, schema};
use serde_json::{Map, Value, json};

use super::args::{Args, closest};
use super::{Context, ToolDef, input_schema};
use crate::tool::{ToolError, ToolOutput};

pub(crate) static GET_SCHEMA: ToolDef = ToolDef {
    name: "get_schema",
    title: "Node types and fields",
    description: "Without arguments: the list of node types, layers first, then animations, \
        one line each. With a type: every field of that type with what it holds, its default \
        and its unit, the value types it uses (paint, easing, stroke and so on), which layers \
        an animation works on, and a ready-made apply_edits operation to copy. Look a type up \
        before the first time you add or change a node of it; unknown fields are refused, not \
        ignored.",
    read_only: true,
    schema: || {
        input_schema(
            json!({
                "type": {
                    "type": "string",
                    "description": "A node type such as \"rect\", \"text\", \"move\" or \"slide_in\", or a value type such as \"easing\" or \"paint\". Leave out for the list of node types."
                },
                "format": {
                    "type": "string",
                    "enum": ["text", "json_schema"],
                    "description": "\"text\" (default) is a compact description. \"json_schema\" is the formal JSON Schema of the node type, which is longer."
                }
            }),
            &[],
        )
    },
    run,
};

fn run(_context: &Context<'_>, args: &Args) -> Result<ToolOutput, ToolError> {
    let format = args
        .choice("format", &["text", "json_schema"])?
        .unwrap_or("text");
    let Some(name) = args.string("type")? else {
        return Ok(ToolOutput::text(type_list()));
    };
    let name = name.trim().to_lowercase();
    if let Some(ty) = NodeType::parse(&name) {
        return Ok(match format {
            "json_schema" => ToolOutput::json(&schema::node_schema(ty)),
            _ => ToolOutput::text(describe_type(ty)),
        });
    }
    let blocks = value_blocks();
    if blocks.iter().any(|block| block.name == name) {
        let wanted = with_references(&blocks, BTreeSet::from([name.as_str()]));
        return Ok(ToolOutput::text(print_blocks(&blocks, &wanted)));
    }

    let mut names: Vec<&str> = NodeType::ALL.iter().map(|ty| ty.as_str()).collect();
    let node_types = names.join(", ");
    names.extend(blocks.iter().map(|block| block.name));
    let hint = closest(&name, &names)
        .map(|close| format!(" Did you mean \"{close}\"?"))
        .unwrap_or_default();
    Err(ToolError::argument(
        "type",
        format!(
            "\"{name}\" is not a node type or a value type.{hint} The node types are: \
             {node_types}. The value types are: {}.",
            blocks
                .iter()
                .map(|block| block.name)
                .collect::<Vec<_>>()
                .join(", ")
        ),
    ))
}

/// The answer without a type: every node type in one line.
fn type_list() -> String {
    let mut text = schema::overview();
    text.push_str(
        "\nCall get_schema with one of these types for its fields, defaults and an example.\n",
    );
    text
}

/// The answer for one node type.
fn describe_type(ty: NodeType) -> String {
    // The summary lists id, parent and index, which apply_edits takes care of.
    let mut text: String = schema::type_summary(ty)
        .lines()
        .filter(|line| {
            !["  id*:", "  parent*:", "  index*:"]
                .iter()
                .any(|managed| line.starts_with(managed))
        })
        .fold(String::new(), |mut text, line| {
            text.push_str(line);
            text.push('\n');
            text
        });
    text.push_str(
        "A * marks a required field. A field you leave out takes its default. \"id\" and the \
         place among siblings are handled by apply_edits (\"parent\", \"position\", \
         \"before\", \"after\" on the operation).\n",
    );

    if ty.family() == Family::Animation && ty != NodeType::AnimationGroup {
        let layers: Vec<&str> = NodeType::ALL
            .iter()
            .filter(|layer| animation_applies_to(ty, **layer))
            .map(|layer| layer.as_str())
            .collect();
        let _ = writeln!(text, "\nWorks on these layer types: {}.", layers.join(", "));
    }

    let blocks = value_blocks();
    let used: BTreeSet<&str> = schema::fields(ty)
        .iter()
        .flat_map(|field| words(&field.value))
        // `stroke`, `shadow` and `corner_radius` name an animation and the
        // value it changes; whoever asks for one may mean the other.
        .chain([ty.as_str()])
        .filter_map(|word| blocks.iter().find(|block| block.name == word))
        .map(|block| block.name)
        .collect();
    let used = with_references(&blocks, used);
    if !used.is_empty() {
        text.push_str("\nValue types:\n");
        text.push_str(&print_blocks(&blocks, &used));
    }

    let _ = writeln!(
        text,
        "\nExample, as an operation for apply_edits:\n{}",
        example(ty)
    );
    text
}

/// A ready-made `add` operation for a node of this type.
fn example(ty: NodeType) -> Value {
    let target = NodeId::from("LAYER_ID");
    let data = match ty.family() {
        Family::Layer => build::default_layer(ty),
        Family::Animation => build::default_animation(ty, &target, 0.0),
    };
    let mut node = Map::new();
    node.insert("type".into(), json!(ty.as_str()));
    if let Some(Value::Object(fields)) = data.map(|data| Node::new(data).to_value()) {
        for (name, value) in fields {
            if !matches!(name.as_str(), "id" | "parent" | "index" | "type") {
                node.insert(name, value);
            }
        }
    }
    let mut operation = Map::new();
    operation.insert("op".into(), json!("add"));
    match ty {
        NodeType::Artboard => {}
        NodeType::AnimationGroup => {
            node.insert("name".into(), json!("Intro"));
            operation.insert("parent".into(), json!("ARTBOARD_ID"));
        }
        _ if ty.family() == Family::Layer => {
            operation.insert("parent".into(), json!("ARTBOARD_OR_GROUP_ID"));
        }
        // An animation goes on the timeline of its target's artboard.
        _ => {}
    }
    operation.insert("node".into(), Value::Object(node));
    Value::Object(operation)
}

/// One value type from core's summary: its name and its lines.
struct Block<'a> {
    name: &'a str,
    lines: Vec<&'a str>,
}

/// Core's summary of the value types, cut into one block per type.
fn value_blocks() -> Vec<Block<'static>> {
    static SUMMARY: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let summary = SUMMARY.get_or_init(schema::value_types_summary);
    let mut blocks: Vec<Block<'static>> = Vec::new();
    for line in summary.lines() {
        match blocks.last_mut() {
            Some(block) if line.starts_with(' ') => block.lines.push(line),
            _ => blocks.push(Block {
                name: line.split(" — ").next().unwrap_or(line).trim(),
                lines: vec![line],
            }),
        }
    }
    blocks
}

/// The lower-case words of a piece of text.
fn words(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !(c.is_ascii_lowercase() || c == '_'))
        .filter(|word| !word.is_empty())
}

/// `wanted` together with every value type those refer to, directly or
/// through others.
fn with_references<'a>(blocks: &[Block<'a>], wanted: BTreeSet<&str>) -> BTreeSet<&'a str> {
    let mut found: BTreeSet<&'a str> = BTreeSet::new();
    let mut pending: Vec<&str> = wanted.into_iter().collect();
    while let Some(name) = pending.pop() {
        let Some(block) = blocks.iter().find(|block| block.name == name) else {
            continue;
        };
        if !found.insert(block.name) {
            continue;
        }
        // Look at what each line defines, not at its explanation.
        for line in block.lines.iter().skip(1) {
            let definition = line.split(" — ").next().unwrap_or(line);
            for piece in definition.split(',') {
                let value = piece.split_once(": ").map_or(piece, |(_, value)| value);
                let value = value.split(" = ").next().unwrap_or(value);
                for word in words(value) {
                    if let Some(other) = blocks.iter().find(|block| block.name == word) {
                        pending.push(other.name);
                    }
                }
            }
        }
    }
    found
}

fn print_blocks(blocks: &[Block<'_>], wanted: &BTreeSet<&str>) -> String {
    let mut text = String::new();
    for block in blocks.iter().filter(|block| wanted.contains(block.name)) {
        for line in &block.lines {
            text.push_str(line);
            text.push('\n');
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::testing::{call, call_error, call_text, demo_session, demo_tools};
    use crate::{Host, Tools};

    #[test]
    fn without_a_type_every_node_type_is_listed() {
        let text = call_text(&demo_tools(), "get_schema", json!({}));
        assert!(text.starts_with("Layers:\n  artboard — "), "{text}");
        for ty in NodeType::ALL {
            assert!(text.contains(&format!("  {ty} — ")), "{ty} is missing");
        }
        assert!(text.contains("Call get_schema with one of these types"));
    }

    #[test]
    fn a_layer_type_comes_with_fields_value_types_and_an_example() {
        let text = call_text(&demo_tools(), "get_schema", json!({ "type": "rect" }));
        assert!(text.starts_with("rect (layer) — "), "{text}");
        assert!(
            text.contains("  corner_radius: corner_radius = 0"),
            "{text}"
        );
        assert!(text.contains("  fill: paint — "), "{text}");
        // What apply_edits manages is not offered as a field to fill in.
        assert!(!text.contains("  index*:"), "{text}");
        assert!(!text.contains("  parent*:"), "{text}");
        assert!(text.contains("\"id\" and the place among siblings are handled by apply_edits"));
        // The value types rect uses, and the ones those use, but no others.
        for used in [
            "paint — ",
            "gradient — ",
            "gradient_stop — ",
            "stroke — ",
            "shadow — ",
        ] {
            assert!(text.contains(&format!("\n{used}")), "{used} is missing");
        }
        assert!(!text.contains("\neasing — "), "{text}");
        assert!(!text.contains("\nfont — "), "{text}");
        assert!(!text.contains("Works on these layer types"));
        assert!(text.contains(
            "{\"op\":\"add\",\"parent\":\"ARTBOARD_OR_GROUP_ID\",\"node\":{\"type\":\"rect\",\"fill\":\"#D9D9D9\"}}"
        ), "{text}");
    }

    #[test]
    fn an_animation_type_says_what_it_works_on() {
        let text = call_text(&demo_tools(), "get_schema", json!({ "type": "text_in" }));
        assert!(
            text.contains("\nWorks on these layer types: text.\n"),
            "{text}"
        );
        assert!(text.contains("\neasing — "), "{text}");
        assert!(text.contains("\"target\":\"LAYER_ID\""), "{text}");
        assert!(!text.contains("\"parent\":"), "{text}");

        let text = call_text(
            &demo_tools(),
            "get_schema",
            json!({ "type": "corner_radius" }),
        );
        assert!(
            text.contains("\nWorks on these layer types: group, rect, image.\n"),
            "{text}"
        );
    }

    #[test]
    fn every_example_is_accepted_by_apply_rules() {
        // The examples are built from core's own defaults, so each must read
        // back as a node of its type once ids are filled in.
        for ty in NodeType::ALL {
            let example = example(*ty);
            let mut node = example["node"].clone();
            assert_eq!(node["type"], ty.as_str());
            node["id"] = json!("n1");
            node["index"] = json!("a0");
            if *ty != NodeType::Artboard {
                node["parent"] = json!("p1");
            }
            if let Err(problem) = schema::read_node(node) {
                panic!("the example for {ty} is not a valid node: {problem}");
            }
            assert_eq!(example["op"], "add");
        }
    }

    #[test]
    fn a_value_type_can_be_looked_up_on_its_own() {
        let text = call_text(&demo_tools(), "get_schema", json!({ "type": "gradient" }));
        assert!(text.starts_with("gradient — A gradient."), "{text}");
        // A gradient has stops, and nothing else comes along.
        assert!(text.contains("\ngradient_stop — "), "{text}");
        assert!(!text.contains("\nshadow — "), "{text}");
        assert!(!text.contains("\npaint — "), "{text}");
    }

    #[test]
    fn a_name_that_is_an_animation_and_a_value_gives_both() {
        let text = call_text(&demo_tools(), "get_schema", json!({ "type": "stroke" }));
        assert!(text.starts_with("stroke (animation) — "), "{text}");
        assert!(
            text.contains("\nstroke — The outline of a layer."),
            "{text}"
        );
        assert!(text.contains("\nstroke_change — "), "{text}");
    }

    #[test]
    fn the_formal_schema_is_available() {
        let schema = call(
            &demo_tools(),
            "get_schema",
            json!({ "type": "fade_in", "format": "json_schema" }),
        );
        assert_eq!(schema["title"], "fade_in");
        assert_eq!(schema["additionalProperties"], false);
        assert!(schema["properties"]["easing"].is_object());
    }

    #[test]
    fn results_fit_the_size_limit() {
        let tools = Tools::new(demo_session(), Host::new());
        let limit = crate::ToolsConfig::default().max_result_chars;
        for ty in NodeType::ALL {
            let text = call_text(&tools, "get_schema", json!({ "type": ty.as_str() }));
            assert!(
                text.chars().count() <= limit,
                "{ty}: {} characters",
                text.chars().count()
            );
        }
    }

    #[test]
    fn an_unknown_type_gets_a_suggestion_and_both_lists() {
        let error = call_error(&demo_tools(), "get_schema", json!({ "type": "rectangle" }));
        assert!(
            error.starts_with(
                "\"rectangle\" is not a node type or a value type. Did you mean \"rect\"?"
            ),
            "{error}"
        );
        assert!(
            error.contains("The node types are: artboard, group,"),
            "{error}"
        );
        assert!(
            error.contains("The value types are: shadow, paint,"),
            "{error}"
        );
        // Case and stray spaces are forgiven.
        let text = call_text(&demo_tools(), "get_schema", json!({ "type": " Rect " }));
        assert!(text.starts_with("rect (layer)"));
    }
}
