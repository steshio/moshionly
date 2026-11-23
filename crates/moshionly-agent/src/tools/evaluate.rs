//! `evaluate`: the values layers have at chosen moments, so a model can
//! check motion as numbers without looking at a picture.

use moshionly_core::eval::{Property, PropertyValue, value_at};
use moshionly_core::model::AutoSize;
use moshionly_core::{NodeData, NodeId};
use serde_json::{Map, Value, json};

use super::args::{Args, closest, short_name};
use super::common::{self, number};
use super::{Context, ToolDef, input_schema};
use crate::tool::{ToolError, ToolOutput};

pub(crate) static EVALUATE: ToolDef = ToolDef {
    name: "evaluate",
    title: "Values over time",
    description: "Returns the values chosen properties of chosen layers have at chosen moments, \
        with every animation applied: a quick, exact way to check motion without a screenshot. \
        For example, to check a fade and a slide, ask for [\"opacity\", \"x\"] at [0, 250, 500, \
        1000]. Each property comes back as a list with one value per time, in the order of \
        \"times\". x and y are the top-left corner in the parent's coordinates before rotation \
        and scale, opacity is 0 to 100, scale a multiplier, rotation degrees.",
    read_only: true,
    destructive: false,
    schema: || {
        input_schema(
            json!({
                "layers": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "The ids of the layers, at most 20."
                },
                "properties": {
                    "type": "array",
                    "items": { "type": "string", "enum": PROPERTY_NAMES },
                    "description": "The properties to read. Default: x, y, scale, rotation, opacity."
                },
                "times": {
                    "type": "array",
                    "items": { "type": "number", "minimum": 0 },
                    "description": "The moments, in milliseconds from the start of the artboard, at most 60."
                }
            }),
            &["layers", "times"],
        )
    },
    run,
};

const PROPERTY_NAMES: [&str; 14] = [
    "x",
    "y",
    "width",
    "height",
    "rotation",
    "scale",
    "opacity",
    "visible",
    "fill",
    "stroke",
    "corner_radius",
    "blur",
    "shadow",
    "content",
];
const DEFAULT_PROPERTIES: [&str; 5] = ["x", "y", "scale", "rotation", "opacity"];
const MAX_LAYERS: usize = 20;
const MAX_TIMES: usize = 60;

fn run(context: &Context<'_>, args: &Args) -> Result<ToolOutput, ToolError> {
    let document = context.session.document();
    let ids = args
        .strings("layers")?
        .filter(|ids| !ids.is_empty())
        .ok_or_else(|| args.missing("layers", "the ids of the layers to read"))?;
    if ids.len() > MAX_LAYERS {
        return Err(ToolError::argument(
            "layers",
            format!(
                "\"layers\" names {} layers; one call reads at most {MAX_LAYERS}.",
                ids.len()
            ),
        ));
    }
    let layers = ids
        .iter()
        .map(|id| common::layer(&document, id, "layers").map(|node| node.id.clone()))
        .collect::<Result<Vec<NodeId>, _>>()?;
    let properties = properties(args)?;
    let times = args
        .numbers("times")?
        .filter(|times| !times.is_empty())
        .ok_or_else(|| {
            args.missing(
                "times",
                "the moments to read, in milliseconds, such as [0, 250, 500]",
            )
        })?;
    if times.len() > MAX_TIMES {
        return Err(ToolError::argument(
            "times",
            format!(
                "\"times\" has {} moments; one call reads at most {MAX_TIMES}.",
                times.len()
            ),
        ));
    }
    if let Some(negative) = times.iter().find(|time| **time < 0.0) {
        return Err(ToolError::argument(
            "times",
            format!("Times start at 0 ms, but {negative} was given."),
        ));
    }

    let mut notes = Vec::new();
    let rows: Vec<Value> = layers
        .iter()
        .map(|id| {
            let node = document.get(id);
            let mut row = Map::new();
            row.insert("id".into(), json!(id));
            if let Some(name) = node.map(|node| node.name()).filter(|name| !name.is_empty()) {
                row.insert("name".into(), json!(name));
            }
            for property in &properties {
                let values: Vec<Option<Value>> = times
                    .iter()
                    .map(|time| value_at(&document, id, *property, *time).map(to_json))
                    .collect();
                if values.iter().all(Option::is_none) {
                    let ty = node.map_or("this layer".into(), |node| node.node_type().to_string());
                    notes.push(format!(
                        "{} does not apply to {ty} \"{id}\".",
                        property.as_str()
                    ));
                    continue;
                }
                let values: Vec<Value> = values
                    .into_iter()
                    .map(|value| value.unwrap_or(Value::Null))
                    .collect();
                row.insert(property.as_str().into(), json!(values));
            }
            if let Some(NodeData::Text(text)) = node.map(|node| &node.data)
                && text.auto_size != AutoSize::Fixed
                && properties
                    .iter()
                    .any(|property| matches!(property, Property::Width | Property::Height))
            {
                notes.push(format!(
                    "The box of text \"{id}\" follows its text; width and height here are the \
                     stored ones. get_node gives the measured box."
                ));
            }
            Value::Object(row)
        })
        .collect();

    let times: Vec<Value> = times.iter().map(|time| number(*time)).collect();
    let mut result = json!({ "times": times, "layers": rows });
    if !notes.is_empty() {
        result["notes"] = json!(notes);
    }
    let text = result.to_string();
    if text.len() > context.config.max_result_chars {
        return Err(ToolError::Rejected(format!(
            "The answer would be {} characters, more than a result may hold ({}). Ask for fewer \
             layers, properties or times.",
            text.len(),
            context.config.max_result_chars
        )));
    }
    Ok(ToolOutput::text(text))
}

/// The properties asked for, or the usual ones.
fn properties(args: &Args) -> Result<Vec<Property>, ToolError> {
    let Some(names) = args.strings("properties")? else {
        return Ok(DEFAULT_PROPERTIES
            .iter()
            .filter_map(|name| Property::parse(name))
            .collect());
    };
    names
        .iter()
        .map(|name| {
            let wanted = match name.trim() {
                "hidden" => "visible",
                "radius" => "corner_radius",
                "text" => "content",
                "color" | "colour" => "fill",
                other => other,
            };
            Property::parse(wanted).ok_or_else(|| {
                let hint = closest(name, &PROPERTY_NAMES)
                    .map(|close| format!(" Did you mean \"{close}\"?"))
                    .unwrap_or_default();
                ToolError::argument(
                    "properties",
                    format!(
                        "\"{}\" is not a property that can be read over time.{hint} The \
                         properties are: {}.",
                        short_name(name),
                        PROPERTY_NAMES.join(", ")
                    ),
                )
            })
        })
        .collect()
}

/// A value as it goes into the result, numbers rounded to two decimals.
fn to_json(value: PropertyValue) -> Value {
    let mut value = match value {
        PropertyValue::Number(number) => return self::number(number),
        PropertyValue::Flag(flag) => json!(flag),
        PropertyValue::Text(text) => json!(text),
        PropertyValue::Paint(paint) => serde_json::to_value(paint).unwrap_or_default(),
        PropertyValue::Stroke(stroke) => serde_json::to_value(stroke).unwrap_or_default(),
        PropertyValue::CornerRadius(radius) => serde_json::to_value(radius).unwrap_or_default(),
        PropertyValue::Shadow(shadow) => serde_json::to_value(shadow).unwrap_or_default(),
    };
    round_numbers(&mut value);
    value
}

fn round_numbers(value: &mut Value) {
    match value {
        Value::Number(n) => {
            if let Some(float) = n.as_f64() {
                *value = number(float);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(round_numbers),
        Value::Object(fields) => fields.values_mut().for_each(round_numbers),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::tools::testing::{call, call_error, demo_tools};

    #[test]
    fn motion_is_read_as_numbers() {
        let result = call(
            &demo_tools(),
            "evaluate",
            json!({ "layers": ["dot", "card"], "properties": ["x", "opacity"],
                    "times": [0, 600, 900, 1200, 2000] }),
        );
        assert_eq!(result["times"], json!([0, 600, 900, 1200, 2000]));
        let dot = &result["layers"][0];
        assert_eq!(dot["id"], "dot");
        assert_eq!(dot["name"], "Dot");
        // The move adds a delta of -100 at its start, easing back to 0.
        let x = dot["x"].as_array().unwrap();
        assert_eq!(x[0], 400);
        assert_eq!(x[1], 400);
        let middle = x[2].as_f64().unwrap();
        assert!(middle > 400.0 && middle < 500.0, "{middle}");
        assert_eq!(x[3], 500);
        assert_eq!(dot["opacity"], json!([100, 100, 100, 100, 100]));
        // The card fades in over its first half second.
        let card = &result["layers"][1];
        assert_eq!(card["opacity"][0], 0);
        assert_eq!(card["opacity"][1], 100);
        assert!(result.get("notes").is_none());
    }

    #[test]
    fn paints_flags_and_text_come_as_values() {
        let result = call(
            &demo_tools(),
            "evaluate",
            json!({ "layers": "title", "properties": ["fill", "visible", "content", "hidden"],
                    "times": 0 }),
        );
        let title = &result["layers"][0];
        assert_eq!(title["fill"], json!(["#111111"]));
        assert_eq!(title["visible"], json!([true]));
        assert_eq!(title["content"], json!(["Launch day"]));
    }

    #[test]
    fn a_property_a_layer_does_not_have_is_noted() {
        let result = call(
            &demo_tools(),
            "evaluate",
            json!({ "layers": ["dot"], "properties": ["corner_radius", "width"],
                    "times": [0] }),
        );
        assert!(result["layers"][0].get("corner_radius").is_none());
        assert_eq!(result["layers"][0]["width"], json!([52]));
        assert_eq!(
            result["notes"],
            json!(["corner_radius does not apply to ellipse \"dot\"."])
        );
    }

    #[test]
    fn mistakes_are_explained() {
        let tools = demo_tools();
        let error = call_error(&tools, "evaluate", json!({ "layers": ["dot"] }));
        assert!(error.starts_with("\"times\" is required"), "{error}");
        let error = call_error(
            &tools,
            "evaluate",
            json!({ "layers": ["dot"], "times": [0], "properties": ["opactiy"] }),
        );
        assert!(error.contains("Did you mean \"opacity\"?"), "{error}");
        let error = call_error(
            &tools,
            "evaluate",
            json!({ "layers": ["cardfade"], "times": [0] }),
        );
        assert!(error.contains("It animates the layer \"card\""), "{error}");
        let error = call_error(
            &tools,
            "evaluate",
            json!({ "layers": ["dot"], "times": [-5] }),
        );
        assert!(error.contains("start at 0"), "{error}");
    }
}
