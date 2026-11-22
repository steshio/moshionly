//! The schema of nodes, derived from the Rust definitions in [`crate::model`].
//!
//! Three audiences read it:
//!
//! - programs, as JSON Schema: [`node_schema`];
//! - AI agents, as short text that fits in a prompt: [`overview`],
//!   [`type_summary`] and [`value_types_summary`];
//! - people and agents who got a node wrong: [`read_node`] turns a JSON
//!   value into a node or into a message that says what to fix.
//!
//! Nothing here is written by hand per type. A new property on a node type
//! shows up everywhere as soon as it is added to the struct.

mod reference;

use std::collections::{BTreeSet, HashMap};
use std::fmt::Write as _;
use std::sync::OnceLock;

use schemars::generate::SchemaSettings;
use serde_json::{Map, Value, json};

use crate::defaults;
use crate::id::NodeId;
use crate::json::tidy_numbers;
use crate::model::{AnimationKind, Family, Node, NodeType};
use crate::validate::{Problem, ProblemKind};

pub use reference::reference;

/// One node type, for pickers and prompts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TypeInfo {
    /// The type.
    pub node_type: NodeType,
    /// Its name in the `type` field, for example `"mask_group"`.
    pub name: &'static str,
    /// Whether nodes of the type are layers or animations.
    pub family: Family,
    /// The kind of animation, or `None` for a layer type.
    pub kind: Option<AnimationKind>,
    /// One line saying what the type is.
    pub description: &'static str,
}

/// Every node type: layers first, then animations.
pub fn node_types() -> Vec<TypeInfo> {
    NodeType::ALL
        .iter()
        .map(|ty| TypeInfo {
            node_type: *ty,
            name: ty.as_str(),
            family: ty.family(),
            kind: ty.animation_kind(),
            description: ty.description(),
        })
        .collect()
}

/// One field of a node type.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldInfo {
    /// The field's name in JSON.
    pub name: String,
    /// Whether the field has to be present.
    pub required: bool,
    /// What the field holds, in a few words: `number`, `paint`,
    /// `"left" | "center" | "right"`.
    pub value: String,
    /// The value the field has when it is left out, when it has one.
    pub default: Option<Value>,
    /// What the field means, with its unit and range.
    pub description: String,
}

/// The schema of [`Node`] as schemars generates it, cleaned up: one `oneOf`
/// branch per node type, shared value types under `$defs`.
fn raw() -> &'static Value {
    static RAW: OnceLock<Value> = OnceLock::new();
    RAW.get_or_init(|| {
        // Defaults are normally left out when serialising; the schema wants them.
        let schema = defaults::with_defaults(|| {
            SchemaSettings::draft2020_12()
                .into_generator()
                .into_root_schema_for::<Node>()
        });
        let mut value = schema.to_value();
        clean(&mut value);
        tidy_numbers(&mut value);
        value
    })
}

/// Drops machine-level number formats, and the line breaks and code samples
/// that doc comments carry.
fn clean(value: &mut Value) {
    match value {
        Value::Object(fields) => {
            if fields.get("format").is_some_and(|format| {
                matches!(
                    format.as_str(),
                    Some("double" | "float" | "uint16" | "uint32")
                )
            }) {
                fields.remove("format");
            }
            if let Some(Value::String(description)) = fields.get_mut("description") {
                // Code samples in doc comments are for rustdoc, not for prompts.
                let prose = description.split("```").next().unwrap_or_default();
                *description = prose.split_whitespace().collect::<Vec<_>>().join(" ");
            }
            fields.values_mut().for_each(clean);
        }
        Value::Array(items) => items.iter_mut().for_each(clean),
        _ => {}
    }
}

fn defs() -> &'static Map<String, Value> {
    static EMPTY: OnceLock<Map<String, Value>> = OnceLock::new();
    raw()
        .get("$defs")
        .and_then(Value::as_object)
        .unwrap_or_else(|| EMPTY.get_or_init(Map::new))
}

/// The name a `$ref` points at.
fn ref_name(schema: &Value) -> Option<&str> {
    schema.get("$ref")?.as_str()?.rsplit('/').next()
}

/// The JSON Schema (draft 2020-12) of one node type.
///
/// It describes a whole node: `id`, `parent`, `index`, `type` and every
/// property of the type, with descriptions, ranges and defaults. Unknown
/// fields are not allowed. Shared value types such as paints and easings are
/// under `$defs`.
pub fn node_schema(ty: NodeType) -> Value {
    built(ty).clone()
}

/// The schema of every node type, built once.
fn built(ty: NodeType) -> &'static Value {
    static BUILT: OnceLock<HashMap<NodeType, Value>> = OnceLock::new();
    static NOTHING: Value = Value::Null;
    BUILT
        .get_or_init(|| NodeType::ALL.iter().map(|ty| (*ty, build(*ty))).collect())
        .get(&ty)
        .unwrap_or(&NOTHING)
}

fn build(ty: NodeType) -> Value {
    let raw = raw();
    let branch = raw["oneOf"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|branch| branch["properties"]["type"]["const"] == ty.as_str());
    let mut properties = Map::new();
    properties.insert("id".into(), raw["properties"]["id"].clone());
    properties.insert("parent".into(), parent_schema(ty));
    properties.insert("index".into(), raw["properties"]["index"].clone());
    let mut required = vec![json!("id"), json!("index")];
    if ty != NodeType::Artboard {
        required.push(json!("parent"));
    }
    if let Some(branch) = branch {
        if let Some(own) = branch["properties"].as_object() {
            properties.extend(own.clone());
        }
        required.extend(branch["required"].as_array().into_iter().flatten().cloned());
    }
    let mut schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": ty.as_str(),
        "description": ty.description(),
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    });
    let used = reachable_defs(&schema);
    if !used.is_empty() {
        schema["$defs"] = Value::Object(used);
    }
    schema
}

fn parent_schema(ty: NodeType) -> Value {
    match (ty, ty.family()) {
        (NodeType::Artboard, _) => json!({
            "type": "null",
            "description": "An artboard is a root: null, or left out.",
            "default": null
        }),
        (_, Family::Layer) => json!({
            "$ref": "#/$defs/NodeId",
            "description": "Id of the artboard, group or mask_group this layer is in."
        }),
        (_, Family::Animation) => json!({
            "$ref": "#/$defs/NodeId",
            "description": "Id of the artboard whose timeline this is on, or of an animation_group in that artboard. Not the layer it changes: that goes in \"target\"."
        }),
    }
}

/// The shared definitions a schema refers to, directly or through others.
fn reachable_defs(schema: &Value) -> Map<String, Value> {
    fn collect(value: &Value, found: &mut BTreeSet<String>) {
        match value {
            Value::Object(fields) => {
                if let Some(name) = ref_name(value)
                    && found.insert(name.to_owned())
                    && let Some(definition) = defs().get(name)
                {
                    collect(definition, found);
                }
                fields.values().for_each(|field| collect(field, found));
            }
            Value::Array(items) => items.iter().for_each(|item| collect(item, found)),
            _ => {}
        }
    }
    let mut found = BTreeSet::new();
    collect(schema, &mut found);
    defs()
        .iter()
        .filter(|(name, _)| found.contains(*name))
        .map(|(name, definition)| (name.clone(), definition.clone()))
        .collect()
}

/// Splits `X or null` into `X` and whether null was allowed.
fn without_null(schema: &Value) -> (&Value, bool) {
    if let Some(options) = schema.get("anyOf").and_then(Value::as_array)
        && let [only] = options
            .iter()
            .filter(|option| option["type"] != "null")
            .collect::<Vec<_>>()[..]
        && options.len() == 2
    {
        return (only, true);
    }
    let nullable = schema["type"]
        .as_array()
        .is_some_and(|types| types.iter().any(|ty| ty == "null"));
    (schema, nullable)
}

/// The values of an enum of strings, when the schema is one.
fn string_choices(schema: &Value) -> Option<Vec<&str>> {
    let options = schema.get("oneOf")?.as_array()?;
    options
        .iter()
        .map(|option| option.get("const")?.as_str())
        .collect()
}

/// `CornerRadius` to `corner_radius`.
fn snake_case(name: &str) -> String {
    let mut result = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            result.push('_');
        }
        result.extend(c.to_lowercase());
    }
    result
}

/// A few words for what a schema accepts.
fn describe_value(schema: &Value) -> String {
    let (schema, _) = without_null(schema);
    if let Some(name) = ref_name(schema) {
        if name == "NodeId" {
            return "id".into();
        }
        return match defs().get(name).and_then(string_choices) {
            Some(choices) => choices
                .iter()
                .map(|choice| format!("\"{choice}\""))
                .collect::<Vec<_>>()
                .join(" | "),
            None => snake_case(name),
        };
    }
    if let Some(constant) = schema.get("const") {
        return constant.to_string();
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
            let item = describe_value(&schema["items"]);
            match (schema["minItems"].as_u64(), schema["maxItems"].as_u64()) {
                (Some(min), Some(max)) if min == max && max <= 4 => {
                    format!("[{}]", vec![item; max as usize].join(", "))
                }
                _ => format!("list of {item}"),
            }
        }
        "integer" => "whole number".into(),
        other => other.to_owned(),
    }
}

/// The description without a closing "Default ..." sentence, which the
/// summaries show as a value instead.
fn without_default_sentence(description: &str) -> &str {
    match description.rfind("Default") {
        Some(at) if at == 0 || description[..at].ends_with(". ") => description[..at].trim_end(),
        _ => description,
    }
}

fn fields_of_object(schema: &Value) -> Vec<FieldInfo> {
    let required: Vec<&str> = schema["required"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    schema["properties"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(name, property)| {
            let default = property
                .get("default")
                .filter(|value| !value.is_null())
                .cloned();
            let description = property["description"].as_str().unwrap_or_default();
            FieldInfo {
                name: name.clone(),
                required: required.contains(&name.as_str()),
                value: describe_value(property),
                description: if default.is_some() {
                    without_default_sentence(description).to_owned()
                } else {
                    description.to_owned()
                },
                default,
            }
        })
        .collect()
}

/// The fields of one node type, in the order they are written to files.
pub fn fields(ty: NodeType) -> Vec<FieldInfo> {
    fields_of_object(built(ty))
}

/// The names of the fields of one node type, in the order they are written
/// to files.
pub fn field_names(ty: NodeType) -> &'static [String] {
    static NAMES: OnceLock<HashMap<NodeType, Vec<String>>> = OnceLock::new();
    NAMES
        .get_or_init(|| {
            NodeType::ALL
                .iter()
                .map(|ty| {
                    let names = built(*ty)["properties"]
                        .as_object()
                        .map(|properties| properties.keys().cloned().collect())
                        .unwrap_or_default();
                    (*ty, names)
                })
                .collect()
        })
        .get(&ty)
        .map_or(&[], Vec::as_slice)
}

fn write_field(text: &mut String, field: &FieldInfo) {
    let _ = write!(text, "  {}", field.name);
    if field.required {
        text.push('*');
    }
    let _ = write!(text, ": {}", field.value);
    if let Some(default) = &field.default {
        let _ = write!(text, " = {default}");
    }
    if !field.description.is_empty() {
        let _ = write!(text, " — {}", field.description);
    }
    text.push('\n');
}

/// The list of node types as text for an AI prompt: one line per type with
/// its family and what it is.
pub fn overview() -> String {
    let mut text = String::new();
    let mut heading = "";
    for info in node_types() {
        let group = match info.kind {
            None => "Layers",
            Some(AnimationKind::Property) => "Property animations",
            Some(AnimationKind::Instant) => "Instant animations",
            Some(AnimationKind::Preset) => "Preset animations",
            Some(AnimationKind::Text) => "Text animations",
            Some(AnimationKind::Structure) => "Timeline structure",
        };
        if group != heading {
            let _ = writeln!(text, "{group}:");
            heading = group;
        }
        let _ = writeln!(text, "  {} — {}", info.name, info.description);
    }
    text
}

/// One node type as text for an AI prompt: what it is, then one line per
/// field with what it holds, its default and its meaning.
///
/// Required fields are marked with `*`. Value types written as one word
/// (`paint`, `easing`, `stroke`) are explained by [`value_types_summary`].
pub fn type_summary(ty: NodeType) -> String {
    let mut text = format!("{} ({}) — {}\n", ty.as_str(), ty.family(), ty.description());
    for field in fields(ty) {
        write_field(&mut text, &field);
    }
    text
}

/// The value types that node fields share (paint, gradient, stroke, shadow,
/// easing, corner radius, font, path data and the like) as text for an AI
/// prompt.
pub fn value_types_summary() -> String {
    let mut text = String::new();
    for (name, definition) in defs() {
        if name == "NodeId" || string_choices(definition).is_some() {
            continue;
        }
        let description = definition["description"].as_str().unwrap_or_default();
        let _ = writeln!(text, "{} — {description}", snake_case(name));
        if definition.get("properties").is_some() {
            for field in fields_of_object(definition) {
                write_field(&mut text, &field);
            }
        }
        let options = definition
            .get("oneOf")
            .or_else(|| definition.get("anyOf"))
            .and_then(Value::as_array);
        for option in options.into_iter().flatten() {
            let description = option["description"].as_str().unwrap_or_default();
            if option.get("properties").is_some() {
                let fields: Vec<String> = fields_of_object(option)
                    .iter()
                    .map(|field| {
                        let default = field.default.as_ref().map(|d| format!(" = {d}"));
                        format!(
                            "{}{}: {}{}",
                            field.name,
                            if field.required { "*" } else { "" },
                            field.value,
                            default.unwrap_or_default()
                        )
                    })
                    .collect();
                let _ = writeln!(text, "  either {{ {} }} — {description}", fields.join(", "));
            } else {
                let _ = writeln!(text, "  either {} — {description}", describe_value(option));
            }
        }
    }
    text
}

/// Number of single-character edits between two short strings.
fn edit_distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, x) in a.iter().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, y) in b.iter().enumerate() {
            let substitution = diagonal + usize::from(x != y);
            diagonal = row[j + 1];
            row[j + 1] = substitution.min(row[j] + 1).min(row[j + 1] + 1);
        }
    }
    row[b.len()]
}

/// The candidate that a misspelt `word` most likely meant, if any is close.
fn closest<'a>(word: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let word = word.to_lowercase();
    candidates
        .into_iter()
        .filter_map(|candidate| {
            let shorter = word.chars().count().min(candidate.chars().count());
            let distance =
                if shorter >= 3 && (candidate.contains(&word) || word.contains(candidate)) {
                    1
                } else {
                    edit_distance(&word, candidate)
                };
            // Allow one slip per three letters, and never a full rewrite.
            (distance <= (shorter / 3).max(1) && distance < shorter)
                .then_some((distance, candidate))
        })
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, candidate)| candidate)
}

/// `a rect`, `an artboard`.
fn with_article(name: &str) -> String {
    let article = if name.starts_with(['a', 'e', 'i', 'o', 'u']) {
        "an"
    } else {
        "a"
    };
    format!("{article} {name}")
}

/// Common names from other tools for fields that are called something else
/// here.
fn alias(field: &str) -> Option<&'static str> {
    Some(match field.to_lowercase().as_str() {
        "color" | "colour" | "background" | "background_color" | "fill_color" => "fill",
        "border" | "outline" | "stroke_color" => "stroke",
        "radius" | "border_radius" | "corner" => "corner_radius",
        "w" => "width",
        "h" => "height",
        "angle" | "rotate" => "rotation",
        "alpha" | "transparency" => "opacity",
        "visible" => "hidden",
        "text" | "characters" | "value" => "content",
        "size" => "font_size",
        "duration" => "end",
        "delay" | "begin" | "start_time" => "start",
        "end_time" => "end",
        "layer" | "target_id" | "node" => "target",
        "ease" | "curve" | "timing" => "easing",
        "z" | "z_index" | "order" => "index",
        "parent_id" => "parent",
        "src" | "url" | "path" | "image" => "source",
        _ => return None,
    })
}

/// Reads a node from JSON, with an explanation when it is not one.
///
/// This accepts exactly what [`Node::from_value`] accepts. The difference is
/// the error: it names the node, says which field or type is unknown, offers
/// the nearest real name and lists what the type does have.
pub fn read_node(value: Value) -> Result<Node, Problem> {
    let id = NodeId::from(value.get("id").and_then(Value::as_str).unwrap_or_default());
    let problem = |field: Option<&str>, message: String| Problem {
        kind: ProblemKind::Malformed,
        node: (!id.as_str().is_empty()).then(|| id.clone()),
        field: field.map(str::to_owned),
        message,
    };
    let Some(fields) = value.as_object() else {
        return Err(problem(
            None,
            format!(
                "A node must be a JSON object with \"id\", \"index\" and \"type\", but this is {value}."
            ),
        ));
    };
    let type_names = || NodeType::ALL.iter().map(|ty| ty.as_str());
    let ty = match fields.get("type") {
        Some(Value::String(name)) => NodeType::parse(name).ok_or_else(|| {
            let hint = closest(name, type_names())
                .map(|close| format!(" Did you mean \"{close}\"?"))
                .unwrap_or_default();
            problem(
                Some("type"),
                format!(
                    "node \"{id}\": \"{name}\" is not a node type.{hint} The types are: {}.",
                    type_names().collect::<Vec<_>>().join(", ")
                ),
            )
        })?,
        other => {
            return Err(problem(
                Some("type"),
                format!(
                    "node \"{id}\": \"type\" is {}, but every node needs one of these types: {}.",
                    other.map_or("missing".into(), Value::to_string),
                    type_names().collect::<Vec<_>>().join(", ")
                ),
            ));
        }
    };
    let label = match fields.get("name").and_then(Value::as_str) {
        Some(name) if !name.is_empty() => format!("{ty} \"{id}\" ({name})"),
        _ => format!("{ty} \"{id}\""),
    };
    let known = field_names(ty);
    if let Some(unknown) = fields.keys().find(|field| !known.contains(field)) {
        let suggestion = alias(unknown)
            .filter(|field| known.iter().any(|name| name == field))
            .or_else(|| closest(unknown, known.iter().map(String::as_str)));
        let hint = suggestion
            .map(|field| format!(" Did you mean \"{field}\"?"))
            .unwrap_or_default();
        let own: Vec<&str> = known
            .iter()
            .map(String::as_str)
            .filter(|field| !matches!(*field, "id" | "parent" | "index" | "type"))
            .collect();
        return Err(problem(
            Some(unknown),
            format!(
                "{label}: \"{unknown}\" is not a field of {ty}.{hint} Besides id, parent, index \
                 and type, {} has: {}.",
                with_article(ty.as_str()),
                own.join(", ")
            ),
        ));
    }
    Node::from_value(value.clone())
        .map_err(|error| problem(None, format!("{label}: this is not a valid {ty}: {error}.")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Timing;

    #[test]
    fn every_type_is_listed_with_family_and_description() {
        let types = node_types();
        assert_eq!(types.len(), NodeType::ALL.len());
        assert_eq!(types[0].name, "artboard");
        assert_eq!(types[0].family, Family::Layer);
        assert_eq!(types[0].kind, None);
        let slide = types.iter().find(|info| info.name == "slide_in").unwrap();
        assert_eq!(slide.family, Family::Animation);
        assert_eq!(slide.kind, Some(AnimationKind::Preset));
        assert!(
            slide
                .description
                .contains("travels into its resting position")
        );

        let overview = overview();
        assert!(overview.starts_with("Layers:\n  artboard — "), "{overview}");
        for heading in [
            "Property animations:",
            "Instant animations:",
            "Preset animations:",
            "Text animations:",
            "Timeline structure:",
        ] {
            assert!(overview.contains(heading), "{overview}");
        }
        assert_eq!(overview.lines().count(), NodeType::ALL.len() + 6);
    }

    #[test]
    fn a_node_schema_describes_the_whole_node() {
        let schema = node_schema(NodeType::Rect);
        assert_eq!(schema["title"], "rect");
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(schema["required"], json!(["id", "index", "parent", "type"]));
        let properties = schema["properties"].as_object().unwrap();
        let names: Vec<&str> = properties.keys().map(String::as_str).collect();
        assert_eq!(
            names,
            [
                "id",
                "parent",
                "index",
                "type",
                "name",
                "x",
                "y",
                "width",
                "height",
                "rotation",
                "scale",
                "opacity",
                "hidden",
                "locked",
                "blend_mode",
                "blur",
                "shadow",
                "source_id",
                "fill",
                "stroke",
                "corner_radius"
            ]
        );
        assert_eq!(properties["type"]["const"], "rect");
        // Ranges, defaults and descriptions come from the Rust definitions.
        assert_eq!(properties["opacity"]["minimum"], 0);
        assert_eq!(properties["opacity"]["maximum"], 100);
        assert_eq!(properties["opacity"]["default"], 100);
        assert_eq!(properties["width"]["default"], 100);
        assert_eq!(properties["blend_mode"]["default"], "normal");
        assert_eq!(
            properties["opacity"]["description"],
            "Opacity from 0 (invisible) to 100 (opaque). Default 100."
        );
        assert!(properties["x"].get("format").is_none());
        // Only the shared types a rect uses are attached.
        let defs: Vec<&str> = schema["$defs"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            defs,
            [
                "NodeId",
                "BlendMode",
                "Shadow",
                "Paint",
                "Gradient",
                "GradientStop",
                "Stroke",
                "StrokePosition",
                "StrokeCap",
                "StrokeJoin",
                "CornerRadius"
            ]
        );
    }

    #[test]
    fn schemas_differ_by_family() {
        let artboard = node_schema(NodeType::Artboard);
        assert_eq!(artboard["required"], json!(["id", "index", "type"]));
        assert_eq!(artboard["properties"]["parent"]["type"], "null");
        assert_eq!(artboard["properties"]["duration"]["default"], 4000);

        let fade = node_schema(NodeType::FadeIn);
        assert_eq!(
            fade["required"],
            json!(["id", "index", "parent", "type", "target", "start", "end"])
        );
        assert_eq!(
            fade["properties"]["easing"]["default"],
            json!({ "type": "smooth", "intensity": 50 })
        );
        assert!(
            fade["properties"]["parent"]["description"]
                .as_str()
                .unwrap()
                .contains("timeline")
        );
        assert!(fade["$defs"].get("Easing").is_some());
        assert!(fade["$defs"].get("Paint").is_none());

        let show = node_schema(NodeType::Show);
        assert_eq!(
            show["required"],
            json!(["id", "index", "parent", "type", "target", "start"])
        );
        let image = node_schema(NodeType::Image);
        assert!(
            image["required"]
                .as_array()
                .unwrap()
                .contains(&json!("source"))
        );
    }

    #[test]
    fn every_schema_accepts_the_default_node_of_its_type() {
        // A light check that the schema and the type agree: every field the
        // type writes out is a property, and every required property is
        // written.
        for ty in NodeType::ALL {
            let data = crate::build::default_layer(*ty)
                .or_else(|| crate::build::default_animation(*ty, &NodeId::from("t"), 0.0))
                .unwrap_or_else(|| crate::model::AnimationGroup::default().into());
            let node = Node::new(data).with_parent("p");
            let schema = node_schema(*ty);
            let properties = schema["properties"].as_object().unwrap();
            let written = node.to_full_value();
            for field in written.as_object().unwrap().keys() {
                assert!(
                    properties.contains_key(field),
                    "{ty}: {field} is not in the schema"
                );
            }
            for required in schema["required"].as_array().unwrap() {
                let field = required.as_str().unwrap();
                assert!(
                    written.get(field).is_some()
                        || (field == "parent" && *ty == NodeType::Artboard),
                    "{ty}: {field} is required but not written"
                );
            }
        }
    }

    #[test]
    fn fields_carry_types_defaults_and_meaning() {
        let fields = fields(NodeType::Text);
        let field = |name: &str| fields.iter().find(|field| field.name == name).unwrap();
        assert!(field("id").required && field("type").required && !field("x").required);
        assert_eq!(field("type").value, "\"text\"");
        assert_eq!(field("parent").value, "id");
        assert_eq!(field("font_size").value, "number");
        assert_eq!(field("font_size").default, Some(json!(48)));
        assert_eq!(
            field("font_size").description,
            "Font size in pixels, more than 0."
        );
        assert_eq!(field("align").value, "\"left\" | \"center\" | \"right\"");
        assert_eq!(field("align").default, Some(json!("left")));
        assert_eq!(field("fill").value, "paint");
        assert_eq!(field("fill").default, None);
        assert_eq!(
            field("fill").description,
            "Paint of the glyphs. Default: none."
        );
        assert_eq!(field("font").value, "font");
        assert_eq!(field("hidden").value, "boolean");
        assert_eq!(field("source_id").value, "string");

        let star = super::fields(NodeType::Star);
        let points = star.iter().find(|field| field.name == "points").unwrap();
        assert_eq!(
            (points.value.as_str(), &points.default),
            ("whole number", &Some(json!(5)))
        );
        let resize = super::fields(NodeType::Resize);
        let from = resize.iter().find(|field| field.name == "from").unwrap();
        assert_eq!(from.value, "size_change");
        let radius = super::fields(NodeType::CornerRadius);
        assert_eq!(
            radius
                .iter()
                .find(|field| field.name == "to")
                .unwrap()
                .value,
            "corner_radius"
        );
        assert_eq!(
            field_names(NodeType::AnimationGroup),
            ["id", "parent", "index", "type", "name"]
        );
    }

    #[test]
    fn summaries_are_compact_and_complete() {
        let summary = type_summary(NodeType::SlideIn);
        let lines: Vec<&str> = summary.lines().collect();
        assert_eq!(
            lines[0],
            "slide_in (animation) — The layer travels into its resting position from a distance, fading in."
        );
        assert!(lines.contains(&"  target*: id — Id of the layer this animation changes. It must be in the same artboard as the animation."), "{summary}");
        assert!(lines.contains(&"  direction: \"left\" | \"right\" | \"up\" | \"down\" = \"up\" — The direction the layer travels in."), "{summary}");
        assert!(
            lines.contains(&"  distance: number = 100 — How far it travels, in pixels, 0 or more."),
            "{summary}"
        );
        assert!(lines.contains(&"  fade: boolean = true — Whether the layer also fades. Without it the layer is visible the whole time."), "{summary}");
        assert_eq!(lines.len(), 1 + field_names(NodeType::SlideIn).len());
        // Every type fits comfortably in a prompt.
        for ty in NodeType::ALL {
            let summary = type_summary(*ty);
            assert!(summary.len() < 4000, "{ty} is {} bytes", summary.len());
            assert!(!summary.contains('\t') && !summary.contains("  \n"));
        }
    }

    #[test]
    fn shared_value_types_are_explained_once() {
        let summary = value_types_summary();
        for expected in [
            "paint — What an area or an outline is painted with",
            "  either colour — One colour.",
            "  either gradient — A linear or radial gradient.",
            "  either { type*: \"linear_gradient\", start: [number, number] = [0.5,0], end: [number, number] = [0.5,1], stops*: list of gradient_stop } — Colours change along a line.",
            "stroke — The outline of a layer.",
            "  width: number = 1 — Thickness in pixels, 0 or more.",
            "  position: \"inside\" | \"center\" | \"outside\" = \"center\"",
            "  either { type*: \"smooth\", intensity: number = 50 } — Eases in and out evenly. The default.",
            "  either { type*: \"bezier\", x1*: number, y1*: number, x2*: number, y2*: number }",
            "corner_radius — Corner rounding",
            "  either [number, number, number, number] — Top-left, top-right, bottom-right, bottom-left.",
            "easing — The pacing of an animation.\n",
            "offset — A distance on both axes, in pixels.",
            "font — Which font draws a text layer.",
        ] {
            assert!(
                summary.contains(expected),
                "missing {expected:?} in:\n{summary}"
            );
        }
        assert!(
            !summary.contains("blend_mode —"),
            "enums are spelt out where they are used"
        );
        assert!(summary.len() < 12_000, "{} bytes", summary.len());
    }

    #[test]
    fn read_node_accepts_what_from_value_accepts() {
        let value = json!({ "id": "cd34", "parent": "ab12", "index": "a0", "type": "rect", "fill": "#0D99FF" });
        assert_eq!(
            read_node(value.clone()).unwrap(),
            Node::from_value(value).unwrap()
        );
        let fade = Node::new(crate::NodeData::FadeIn(crate::model::FadePreset {
            timing: Timing::new("cd34", 0.0, 500.0),
        }))
        .with_parent("ab12");
        assert_eq!(read_node(fade.to_full_value()).unwrap(), fade);
    }

    #[test]
    fn read_node_explains_unknown_fields() {
        let problem = read_node(json!({
            "id": "cd34", "parent": "ab12", "index": "a0", "type": "rect", "name": "Card", "colour": "#0D99FF"
        }))
        .unwrap_err();
        assert_eq!(problem.kind, ProblemKind::Malformed);
        assert_eq!(problem.node, Some(NodeId::from("cd34")));
        assert_eq!(problem.field.as_deref(), Some("colour"));
        assert_eq!(
            problem.message,
            "rect \"cd34\" (Card): \"colour\" is not a field of rect. Did you mean \"fill\"? \
             Besides id, parent, index and type, a rect has: name, x, y, width, height, rotation, \
             scale, opacity, hidden, locked, blend_mode, blur, shadow, source_id, fill, stroke, \
             corner_radius."
        );
        let hint = |ty: &str, field: &str| {
            let mut value = json!({ "id": "n", "parent": "p", "index": "a0", "type": ty });
            value[field] = json!(1);
            read_node(value).unwrap_err().message
        };
        assert!(hint("rect", "opacty").contains("Did you mean \"opacity\"?"));
        assert!(hint("rect", "cornerRadius").contains("Did you mean \"corner_radius\"?"));
        assert!(hint("fade_in", "duration").contains("Did you mean \"end\"?"));
        assert!(hint("fade_in", "delay").contains("Did you mean \"start\"?"));
        assert!(hint("text", "text").contains("Did you mean \"content\"?"));
        // An alias for a field the type does not have is not offered.
        assert!(!hint("ellipse", "radius").contains("corner_radius\"?"));
        assert!(!hint("rect", "zzzzzz").contains("Did you mean"));
    }

    #[test]
    fn read_node_explains_types_and_shapes() {
        let unknown =
            read_node(json!({ "id": "n", "index": "a0", "type": "rectangle" })).unwrap_err();
        assert_eq!(unknown.field.as_deref(), Some("type"));
        assert!(
            unknown
                .message
                .contains("\"rectangle\" is not a node type. Did you mean \"rect\"?"),
            "{unknown}"
        );
        assert!(unknown.message.contains("mask_group"), "{unknown}");

        let missing = read_node(json!({ "id": "n", "index": "a0" })).unwrap_err();
        assert!(missing.message.contains("\"type\" is missing"), "{missing}");
        let not_text = read_node(json!({ "id": "n", "index": "a0", "type": 3 })).unwrap_err();
        assert!(not_text.message.contains("\"type\" is 3"), "{not_text}");
        let not_object = read_node(json!([1, 2])).unwrap_err();
        assert_eq!(not_object.node, None);
        assert!(
            not_object.message.contains("must be a JSON object"),
            "{not_object}"
        );

        let wrong_shape = read_node(
            json!({ "id": "n", "parent": "p", "index": "a0", "type": "rect", "x": "left" }),
        )
        .unwrap_err();
        assert!(
            wrong_shape
                .message
                .starts_with("rect \"n\": this is not a valid rect: invalid type"),
            "{wrong_shape}"
        );
        let no_target = read_node(json!({ "id": "n", "parent": "p", "index": "a0", "type": "fade_in", "start": 0, "end": 1 })).unwrap_err();
        assert!(
            no_target.message.contains("missing field `target`"),
            "{no_target}"
        );
        let nested = read_node(json!({ "id": "n", "parent": "p", "index": "a0", "type": "rect", "stroke": { "colour": "#000000" } })).unwrap_err();
        assert!(
            nested
                .message
                .contains("unknown field `colour`, expected one of `paint`"),
            "{nested}"
        );
    }

    #[test]
    fn near_misses_are_found() {
        assert_eq!(edit_distance("kitten", "sitting"), 3);
        assert_eq!(edit_distance("", "abc"), 3);
        assert_eq!(closest("opacty", ["opacity", "scale"]), Some("opacity"));
        assert_eq!(closest("fade", ["fade_in", "move"]), Some("fade_in"));
        assert_eq!(closest("qqq", ["opacity", "scale"]), None);
    }
}
