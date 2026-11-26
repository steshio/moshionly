//! `query_nodes`: find nodes and read chosen fields of them.

use std::collections::{BTreeSet, HashSet};
use std::sync::OnceLock;

use moshionly_core::{Document, Family, Node, NodeId, NodeType, schema};
use serde_json::{Map, Value, json};

use super::args::{Args, closest, short_name};
use super::common::{self, STRING_LIMIT, number, shorten_strings};
use super::{Context, ToolDef, input_schema};
use crate::tool::{ToolError, ToolOutput};

pub(crate) static QUERY_NODES: ToolDef = ToolDef {
    name: "query_nodes",
    title: "Find nodes",
    description: "Finds layers and animations and returns chosen fields of each. All filters \
        are optional and combine with AND; with none, every node is listed. Results come in \
        tree order: layers back to front, then animations in timeline order. Ask narrowly: \
        filter by artboard, parent, type or name, and name only the fields you need. A result \
        that would be too long is cut short and says how to get the rest. Typical calls: \
        {\"artboard\":\"<id>\",\"family\":\"layer\"} for the layers of an artboard, \
        {\"target\":\"<layer id>\"} for what animates a layer, {\"component\":\"<id>\"} for \
        the instances of a component, {\"name\":\"title\"} to find a layer by name.",
    read_only: true,
    destructive: false,
    schema,
    run,
};

/// Fields that are worked out instead of stored.
const COMPUTED: &[(&str, &str)] = &[
    ("family", "layer or animation"),
    ("artboard", "id of the artboard the node is in"),
    ("box", "the layer's box at rest in artboard coordinates"),
    ("child_count", "number of layers directly inside"),
    (
        "animation_count",
        "number of animations that target the layer",
    ),
    (
        "instance_count",
        "number of instances that show the component",
    ),
];

const DEFAULT_LIMIT: i64 = 50;
const MAX_LIMIT: i64 = 500;
const MAX_DEPTH: i64 = 10;

fn schema() -> Value {
    input_schema(
        json!({
            "artboard": {
                "type": "string",
                "description": "Only nodes in this artboard (its id)."
            },
            "parent": {
                "type": "string",
                "description": "Only direct children of this node: the layers in a group or artboard, or the animations in an artboard's timeline or an animation_group."
            },
            "family": {
                "type": "string",
                "enum": ["layer", "animation"],
                "description": "Only layers (things that are drawn) or only animations (things that change layers over time)."
            },
            "type": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Only these node types, for example [\"text\"] or [\"fade_in\", \"slide_in\"]. get_schema lists the types."
            },
            "name": {
                "type": "string",
                "description": "Only nodes whose name contains this text, ignoring case."
            },
            "name_exact": {
                "type": "boolean",
                "description": "With name: match the whole name exactly instead of a part of it. Default false."
            },
            "target": {
                "type": "string",
                "description": "Only animations that change this layer (its id)."
            },
            "component": {
                "type": "string",
                "description": "Only instances that show this component (its id). To list the components themselves, use type [\"component\"]."
            },
            "time_from": {
                "type": "number",
                "description": "Only animations still running at or after this time, in milliseconds. Use with time_to to ask what happens in a stretch of time; give both the same value for one moment."
            },
            "time_to": {
                "type": "number",
                "description": "Only animations that have started at or before this time, in milliseconds."
            },
            "ids": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Only the nodes with these ids."
            },
            "fields": {
                "type": "array",
                "items": { "type": "string" },
                "description": "The fields to return for each node, besides id and type, which always come. Any field of the node's type (see get_schema), or one of the computed fields: family, artboard, box (the layer's box at rest in artboard coordinates), child_count, animation_count, instance_count (for a component). [\"*\"] returns every field that is not at its default. Default: name, parent, x, y, width, height for layers and name, target, start, end for animations."
            },
            "depth": {
                "type": "integer",
                "minimum": 0,
                "maximum": MAX_DEPTH,
                "description": "How many levels of children to nest under each match, as \"children\". Default 0: none."
            },
            "limit": {
                "type": "integer",
                "minimum": 1,
                "maximum": MAX_LIMIT,
                "description": "The most nodes to return. Default 50."
            },
            "offset": {
                "type": "integer",
                "minimum": 0,
                "description": "How many matches to skip, to continue a list that was cut short. Default 0."
            }
        }),
        &[],
    )
}

/// Which nodes are wanted.
struct Filter {
    artboard: Option<NodeId>,
    parent: Option<NodeId>,
    family: Option<Family>,
    types: Option<Vec<NodeType>>,
    name: Option<(String, bool)>,
    target: Option<NodeId>,
    component: Option<NodeId>,
    time: Option<(f64, f64)>,
    ids: Option<HashSet<String>>,
}

impl Filter {
    fn read(document: &Document, args: &Args) -> Result<Self, ToolError> {
        let artboard = match args.string("artboard")? {
            Some(id) => Some(common::artboard(document, &id, "artboard")?.id.clone()),
            None => None,
        };
        let parent = match args.string("parent")? {
            Some(id) => Some(common::node(document, &id, "parent")?.id.clone()),
            None => None,
        };
        let target = match args.string("target")? {
            Some(id) => Some(common::layer(document, &id, "target")?.id.clone()),
            None => None,
        };
        let component = match args.string("component")? {
            Some(id) => Some(common::layer(document, &id, "component")?.id.clone()),
            None => None,
        };
        let family = args
            .choice("family", &["layer", "animation"])?
            .map(|family| match family {
                "layer" => Family::Layer,
                _ => Family::Animation,
            });
        let types = match args.strings("type")? {
            Some(names) => Some(
                names
                    .iter()
                    .map(|name| node_type(name))
                    .collect::<Result<Vec<_>, _>>()?,
            ),
            None => None,
        };
        let exact = args.boolean("name_exact")?.unwrap_or(false);
        let name = args
            .string("name")?
            .map(|name| (name.to_lowercase(), exact));
        let time = match (args.number("time_from")?, args.number("time_to")?) {
            (None, None) => None,
            (from, to) => {
                let (from, to) = (from.unwrap_or(0.0), to.unwrap_or(f64::INFINITY));
                if from > to {
                    return Err(ToolError::argument(
                        "time_from",
                        format!(
                            "\"time_from\" ({from}) is after \"time_to\" ({to}). Give the start \
                             of the stretch of time first."
                        ),
                    ));
                }
                Some((from, to))
            }
        };
        let ids = args
            .strings("ids")?
            .map(|ids| ids.into_iter().collect::<HashSet<_>>());
        Ok(Self {
            artboard,
            parent,
            family,
            types,
            name,
            target,
            component,
            time,
            ids,
        })
    }

    fn matches(&self, document: &Document, node: &Node) -> bool {
        if let Some(ids) = &self.ids
            && !ids.contains(node.id.as_str())
        {
            return false;
        }
        if let Some(artboard) = &self.artboard {
            let inside = document
                .artboard_of(&node.id)
                .is_some_and(|owner| &owner.id == artboard);
            if !inside || &node.id == artboard {
                return false;
            }
        }
        if self.parent.is_some() && node.parent != self.parent {
            return false;
        }
        if self.family.is_some_and(|family| node.family() != family) {
            return false;
        }
        if let Some(types) = &self.types
            && !types.contains(&node.node_type())
        {
            return false;
        }
        if let Some((wanted, exact)) = &self.name {
            let name = node.name().to_lowercase();
            let found = if *exact {
                name == *wanted
            } else {
                name.contains(wanted.as_str())
            };
            if !found {
                return false;
            }
        }
        if self.target.is_some() && node.target() != self.target.as_ref() {
            return false;
        }
        if self.component.is_some() && node.component() != self.component.as_ref() {
            return false;
        }
        if let Some((from, to)) = self.time {
            let overlaps = node
                .data
                .span()
                .is_some_and(|(start, end)| start <= to && end >= from);
            if !overlaps {
                return false;
            }
        }
        true
    }
}

/// A node type by name, or an error that offers the nearest real name.
pub(crate) fn node_type(name: &str) -> Result<NodeType, ToolError> {
    NodeType::parse(name).ok_or_else(|| {
        let names: Vec<&str> = NodeType::ALL.iter().map(|ty| ty.as_str()).collect();
        let hint = closest(name, &names)
            .map(|close| format!(" Did you mean \"{close}\"?"))
            .unwrap_or_default();
        ToolError::argument(
            "type",
            format!(
                "\"{}\" is not a node type.{hint} The types are: {}.",
                short_name(name),
                names.join(", ")
            ),
        )
    })
}

/// Which fields of each node are wanted.
enum Fields {
    /// The usual few for the node's family.
    Usual,
    /// Everything that is not at its default.
    All,
    /// The ones that were named.
    Named(Vec<String>),
}

/// Every name that can be asked for: the fields of all node types and the
/// computed ones.
fn known_fields() -> &'static BTreeSet<&'static str> {
    static KNOWN: OnceLock<BTreeSet<&'static str>> = OnceLock::new();
    KNOWN.get_or_init(|| {
        NodeType::ALL
            .iter()
            .flat_map(|ty| schema::field_names(*ty))
            .map(String::as_str)
            .chain(COMPUTED.iter().map(|(name, _)| *name))
            .collect()
    })
}

impl Fields {
    fn read(args: &Args) -> Result<Self, ToolError> {
        let Some(names) = args.strings("fields")? else {
            return Ok(Fields::Usual);
        };
        if names.iter().any(|name| name == "*") {
            return Ok(Fields::All);
        }
        let known = known_fields();
        for name in &names {
            if !known.contains(name.as_str()) {
                let candidates: Vec<&str> = known.iter().copied().collect();
                let hint = closest(name, &candidates)
                    .map(|close| format!(" Did you mean \"{close}\"?"))
                    .unwrap_or_default();
                return Err(ToolError::argument(
                    "fields",
                    format!(
                        "\"{}\" is not a field of any node type.{hint} Use get_schema with \
                         a type to see its fields; the computed fields are {}.",
                        short_name(name),
                        COMPUTED
                            .iter()
                            .map(|(name, _)| *name)
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                ));
            }
        }
        Ok(Fields::Named(names))
    }
}

/// One node as it goes into the result.
fn entry(document: &Document, node: &Node, fields: &Fields, depth: usize) -> Value {
    let mut entry = Map::new();
    entry.insert("id".into(), json!(node.id));
    entry.insert("type".into(), json!(node.node_type().as_str()));
    match fields {
        Fields::All => {
            if let Value::Object(stored) = node.to_value() {
                for (name, value) in stored {
                    // The index is of no use to an agent: order is the order
                    // of the list, and apply_edits places nodes by name.
                    if name != "index" {
                        entry.entry(name).or_insert(value);
                    }
                }
            }
        }
        Fields::Usual => {
            let full = node.to_full_value();
            let usual: &[&str] = match node.family() {
                Family::Layer => &["name", "parent", "x", "y", "width", "height"],
                Family::Animation => &["name", "target", "start", "end"],
            };
            for name in usual {
                match full.get(*name) {
                    Some(Value::Null) | None => {}
                    Some(Value::String(text)) if text.is_empty() => {}
                    Some(value) => {
                        entry.insert((*name).into(), value.clone());
                    }
                }
            }
        }
        Fields::Named(names) => {
            let full = node.to_full_value();
            for name in names {
                let value = match computed(document, node, name) {
                    Some(value) => value,
                    None => match full.get(name) {
                        Some(value) => value.clone(),
                        // Not a field of this type: leave it out.
                        None => continue,
                    },
                };
                entry.insert(name.clone(), value);
            }
        }
    }
    if depth > 0 {
        let children: Vec<Value> = document
            .child_nodes(&node.id, node.family())
            .map(|child| self::entry(document, child, fields, depth - 1))
            .collect();
        if !children.is_empty() {
            entry.insert("children".into(), Value::Array(children));
        }
    }
    let mut entry = Value::Object(entry);
    shorten_strings(&mut entry, STRING_LIMIT);
    entry
}

/// The value of a computed field, or `None` when `name` is not one or does
/// not apply to this node.
fn computed(document: &Document, node: &Node, name: &str) -> Option<Value> {
    Some(match name {
        "family" => json!(common::family(node)),
        "artboard" => json!(document.artboard_of(&node.id)?.id),
        "box" => {
            let bounds = document.world_bounds(&node.id)?;
            json!([
                number(bounds.x),
                number(bounds.y),
                number(bounds.width),
                number(bounds.height)
            ])
        }
        "child_count" if node.is_layer() => {
            json!(document.children(&node.id, Family::Layer).len())
        }
        "animation_count" if node.is_layer() => {
            json!(document.animations_targeting(&node.id).len())
        }
        "instance_count" if node.node_type() == NodeType::Component => {
            json!(document.instances_of(&node.id).len())
        }
        _ => return None,
    })
}

fn run(context: &Context<'_>, args: &Args) -> Result<ToolOutput, ToolError> {
    let document = context.session.document();
    let filter = Filter::read(&document, args)?;
    let fields = Fields::read(args)?;
    let depth = args.integer("depth", 0..=MAX_DEPTH)?.unwrap_or(0) as usize;
    let limit = args
        .integer("limit", 1..=MAX_LIMIT)?
        .unwrap_or(DEFAULT_LIMIT) as usize;
    let offset = args
        .integer("offset", 0..=i64::from(u32::MAX))?
        .unwrap_or(0) as usize;

    let matched: Vec<&Node> = document
        .nodes()
        .filter(|node| filter.matches(&document, node))
        .collect();

    // Take nodes until the limit or the size budget is reached.
    let budget = context.config.max_result_chars.saturating_sub(400);
    let mut nodes = Vec::new();
    let mut size = 0;
    let mut too_long = false;
    for node in matched.iter().skip(offset).take(limit) {
        let entry = entry(&document, node, &fields, depth);
        size += entry.to_string().len() + 1;
        if size > budget {
            too_long = true;
            break;
        }
        nodes.push(entry);
    }
    let returned = nodes.len();
    let next = offset + returned;

    let mut result = json!({ "matched": matched.len(), "nodes": nodes });
    if too_long && returned == 0 {
        return Err(ToolError::Rejected(format!(
            "The first matching node alone is longer than a result may be ({} characters). Ask \
             for less of it: name a few \"fields\" instead of \"*\", and lower \"depth\".",
            context.config.max_result_chars
        )));
    }
    if next < matched.len() {
        result["next_offset"] = json!(next);
        result["note"] = json!(if too_long {
            format!(
                "Cut short to keep the result small: {returned} of {} matches are here. Narrow \
                 the query (artboard, parent, type, name), ask for fewer fields or a lower \
                 depth, or continue with \"offset\": {next}.",
                matched.len()
            )
        } else {
            format!(
                "{returned} of {} matches are here. Continue with \"offset\": {next}, or narrow \
                 the query.",
                matched.len()
            )
        });
    }
    if let Some(ids) = &filter.ids {
        let mut missing: Vec<&str> = ids
            .iter()
            .map(String::as_str)
            .filter(|id| !document.contains(&NodeId::from(*id)))
            .collect();
        if !missing.is_empty() {
            missing.sort_unstable();
            result["not_found"] = json!(missing);
        }
    }
    Ok(ToolOutput::json(&result))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::testing::{call, call_error, demo_session, demo_tools};
    use crate::tools::{Tools, ToolsConfig};
    use crate::{Host, Origin};
    use moshionly_core::Edit;

    fn ids(result: &Value) -> Vec<&str> {
        result["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|node| node["id"].as_str().unwrap())
            .collect()
    }

    #[test]
    fn without_filters_every_node_comes_in_tree_order() {
        let result = call(&demo_tools(), "query_nodes", json!({}));
        assert_eq!(result["matched"], 11);
        assert_eq!(
            ids(&result),
            [
                "scene",
                "card",
                "accent",
                "title",
                "dot",
                "badge",
                "cardfade",
                "dotmove",
                "extras",
                "titlefade",
                "second"
            ]
        );
        assert!(result.get("next_offset").is_none());
    }

    #[test]
    fn the_usual_fields_differ_by_family() {
        let result = call(
            &demo_tools(),
            "query_nodes",
            json!({ "ids": ["dot", "dotmove"] }),
        );
        assert_eq!(
            result["nodes"],
            json!([
                { "id": "dot", "type": "ellipse", "name": "Dot", "parent": "card",
                  "x": 500, "y": 300, "width": 52, "height": 52 },
                { "id": "dotmove", "type": "move", "target": "dot", "start": 600, "end": 1200 },
            ])
        );
    }

    #[test]
    fn filters_combine() {
        let tools = demo_tools();
        let layers = call(
            &tools,
            "query_nodes",
            json!({ "artboard": "scene", "family": "layer" }),
        );
        assert_eq!(ids(&layers), ["card", "accent", "title", "dot", "badge"]);

        let children = call(&tools, "query_nodes", json!({ "parent": "card" }));
        assert_eq!(ids(&children), ["accent", "title", "dot"]);

        let timeline = call(
            &tools,
            "query_nodes",
            json!({ "parent": "scene", "family": "animation" }),
        );
        assert_eq!(ids(&timeline), ["cardfade", "dotmove", "extras"]);

        let rects = call(&tools, "query_nodes", json!({ "type": ["rect", "text"] }));
        assert_eq!(ids(&rects), ["accent", "title", "badge"]);
        // One type on its own does not need a list.
        let texts = call(&tools, "query_nodes", json!({ "type": "text" }));
        assert_eq!(ids(&texts), ["title"]);

        let named = call(&tools, "query_nodes", json!({ "name": "CARD" }));
        assert_eq!(ids(&named), ["card", "cardfade"]);
        let exact = call(
            &tools,
            "query_nodes",
            json!({ "name": "card", "name_exact": true }),
        );
        assert_eq!(ids(&exact), ["card"]);

        let onto = call(&tools, "query_nodes", json!({ "target": "title" }));
        assert_eq!(ids(&onto), ["titlefade"]);
    }

    #[test]
    fn a_time_range_finds_the_animations_running_in_it() {
        let tools = demo_tools();
        let at = |from: i64, to: i64| {
            call(
                &tools,
                "query_nodes",
                json!({ "time_from": from, "time_to": to }),
            )
        };
        assert_eq!(ids(&at(0, 0)), ["cardfade"]);
        assert_eq!(ids(&at(500, 600)), ["cardfade", "dotmove", "titlefade"]);
        assert_eq!(ids(&at(1300, 1400)), ["titlefade"]);
        assert_eq!(ids(&at(3000, 4000)), [] as [&str; 0]);
        let from_only = call(&tools, "query_nodes", json!({ "time_from": 1300 }));
        assert_eq!(ids(&from_only), ["titlefade"]);
        let error = call_error(
            &tools,
            "query_nodes",
            json!({ "time_from": 900, "time_to": 100 }),
        );
        assert!(error.contains("is after \"time_to\""), "{error}");
    }

    #[test]
    fn named_and_computed_fields() {
        let result = call(
            &demo_tools(),
            "query_nodes",
            json!({
                "ids": ["dot", "card", "cardfade"],
                "fields": ["fill", "opacity", "box", "artboard", "family", "child_count",
                           "animation_count", "easing"]
            }),
        );
        assert_eq!(
            result["nodes"],
            json!([
                { "id": "card", "type": "group", "fill": "#FFFFFF", "opacity": 100,
                  "box": [240, 340, 600, 400], "artboard": "scene", "family": "layer",
                  "child_count": 3, "animation_count": 1 },
                { "id": "dot", "type": "ellipse", "fill": "#0D99FF", "opacity": 100,
                  "box": [740, 640, 52, 52], "artboard": "scene", "family": "layer",
                  "child_count": 0, "animation_count": 1 },
                { "id": "cardfade", "type": "fade_in", "artboard": "scene",
                  "family": "animation", "easing": { "type": "slow_down", "intensity": 50 } },
            ])
        );
    }

    #[test]
    fn a_star_returns_what_is_not_at_its_default() {
        let result = call(
            &demo_tools(),
            "query_nodes",
            json!({ "ids": ["badge"], "fields": ["*"] }),
        );
        assert_eq!(
            result["nodes"][0],
            json!({ "id": "badge", "type": "rect", "parent": "scene", "name": "Badge",
                    "x": 40, "y": 40, "width": 120, "height": 48, "fill": "#111111",
                    "corner_radius": 24 })
        );
    }

    #[test]
    fn depth_nests_children() {
        let result = call(
            &demo_tools(),
            "query_nodes",
            json!({ "ids": ["scene", "extras"], "fields": ["name"], "depth": 2 }),
        );
        assert_eq!(
            result["nodes"],
            json!([
                { "id": "scene", "type": "artboard", "name": "Launch", "children": [
                    { "id": "card", "type": "group", "name": "Card", "children": [
                        { "id": "accent", "type": "rect", "name": "Accent" },
                        { "id": "title", "type": "text", "name": "Title" },
                        { "id": "dot", "type": "ellipse", "name": "Dot" },
                    ]},
                    { "id": "badge", "type": "rect", "name": "Badge" },
                ]},
                { "id": "extras", "type": "animation_group", "name": "Extras", "children": [
                    { "id": "titlefade", "type": "opacity", "name": "" },
                ]},
            ])
        );
    }

    #[test]
    fn limit_and_offset_page_through_matches() {
        let tools = demo_tools();
        let first = call(&tools, "query_nodes", json!({ "limit": 4 }));
        assert_eq!(ids(&first), ["scene", "card", "accent", "title"]);
        assert_eq!(first["next_offset"], 4);
        assert!(
            first["note"]
                .as_str()
                .unwrap()
                .starts_with("4 of 11 matches are here.")
        );
        let rest = call(&tools, "query_nodes", json!({ "limit": 50, "offset": 9 }));
        assert_eq!(ids(&rest), ["titlefade", "second"]);
        assert!(rest.get("next_offset").is_none());
    }

    #[test]
    fn a_long_result_is_cut_and_says_how_to_narrow() {
        let tools = Tools::with_config(
            demo_session(),
            Host::new(),
            ToolsConfig {
                max_result_chars: 800,
                ..ToolsConfig::default()
            },
        );
        let result = call(&tools, "query_nodes", json!({}));
        let returned = result["nodes"].as_array().unwrap().len();
        assert!(returned > 0 && returned < 11, "{returned}");
        assert_eq!(result["matched"], 11);
        assert_eq!(result["next_offset"], returned);
        let note = result["note"].as_str().unwrap();
        assert!(
            note.starts_with("Cut short to keep the result small"),
            "{note}"
        );
        assert!(note.contains(&format!("\"offset\": {returned}")), "{note}");
        assert!(result.to_string().len() <= 800);

        let error = call_error(
            &tools,
            "query_nodes",
            json!({ "ids": ["scene"], "fields": ["*"], "depth": 5 }),
        );
        assert!(
            error.starts_with("The first matching node alone is longer than a result may be"),
            "{error}"
        );
    }

    #[test]
    fn embedded_images_do_not_fill_the_result() {
        let session = demo_session();
        let source = format!("data:image/png;base64,{}", "A".repeat(20_000));
        let image = moshionly_core::Node::from_value(json!({
            "id": "photo", "parent": "scene", "index": "a5", "type": "image", "source": source
        }))
        .unwrap();
        session
            .apply(Origin::User, "Add image", &[Edit::add(image)])
            .unwrap();
        let tools = Tools::new(session, Host::new());
        let result = call(
            &tools,
            "query_nodes",
            json!({ "ids": ["photo"], "fields": ["source"] }),
        );
        let shown = result["nodes"][0]["source"].as_str().unwrap();
        assert!(shown.len() < 400, "{}", shown.len());
        assert!(shown.ends_with("(20022 characters in all)"), "{shown}");
    }

    #[test]
    fn mistakes_get_directions() {
        let tools = demo_tools();
        let error = call_error(&tools, "query_nodes", json!({ "type": ["rectangle"] }));
        assert!(
            error.starts_with("\"rectangle\" is not a node type. Did you mean \"rect\"?"),
            "{error}"
        );
        let error = call_error(&tools, "query_nodes", json!({ "fields": ["colour"] }));
        assert!(error.contains("is not a field of any node type"), "{error}");
        let error = call_error(&tools, "query_nodes", json!({ "fields": ["opacty"] }));
        assert!(error.contains("Did you mean \"opacity\"?"), "{error}");
        let error = call_error(&tools, "query_nodes", json!({ "artboard": "card" }));
        assert!(error.contains("It is in the artboard \"scene\""), "{error}");
        let error = call_error(&tools, "query_nodes", json!({ "parent": "Card" }));
        assert!(error.contains("is the name of group \"card\""), "{error}");
        let error = call_error(&tools, "query_nodes", json!({ "limit": 0 }));
        assert!(error.contains("a whole number from 1 to 500"), "{error}");

        let result = call(&tools, "query_nodes", json!({ "ids": ["dot", "gone"] }));
        assert_eq!(ids(&result), ["dot"]);
        assert_eq!(result["not_found"], json!(["gone"]));
    }
}
