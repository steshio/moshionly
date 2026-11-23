//! Small things the tools share: finding nodes with helpful errors, and
//! writing numbers, boxes and long strings compactly.

use moshionly_core::{Document, Family, Node, NodeId, NodeType, Rect};
use serde_json::{Value, json};

use crate::tool::ToolError;

/// The node with this id, or an error that helps find the right one.
///
/// Models often pass a layer's name where its id belongs, so when the text
/// is the name of a node the error says which id that node has.
pub(crate) fn node<'d>(
    document: &'d Document,
    id: &str,
    field: &str,
) -> Result<&'d Node, ToolError> {
    if let Some(node) = document.get(&NodeId::from(id)) {
        return Ok(node);
    }
    let named: Vec<String> = document
        .nodes()
        .filter(|node| !id.is_empty() && node.name().eq_ignore_ascii_case(id))
        .take(3)
        .map(|node| format!("{} \"{}\"", node.node_type(), node.id))
        .collect();
    let hint = match named.as_slice() {
        [] => "Ids are case-sensitive and are not names. Find ids with get_overview or \
               query_nodes."
            .to_owned(),
        [only] => format!("\"{id}\" is the name of {only}: pass that id instead."),
        several => format!(
            "\"{id}\" is a name, shared by {}: pass the id of the one you mean.",
            several.join(", ")
        ),
    };
    Err(ToolError::argument(
        field,
        format!("\"{field}\": there is no node with the id \"{id}\". {hint}"),
    ))
}

/// The layer with this id. An animation is refused with its target named.
pub(crate) fn layer<'d>(
    document: &'d Document,
    id: &str,
    field: &str,
) -> Result<&'d Node, ToolError> {
    let node = node(document, id, field)?;
    if node.is_layer() {
        return Ok(node);
    }
    let hint = match node.target() {
        Some(target) => format!(" It animates the layer \"{target}\"; pass that id instead."),
        None => String::new(),
    };
    Err(ToolError::argument(
        field,
        format!(
            "\"{field}\" should be the id of a layer, but {} is an animation.{hint}",
            node.label()
        ),
    ))
}

/// The artboard with this id. Another node is refused with its artboard
/// named.
pub(crate) fn artboard<'d>(
    document: &'d Document,
    id: &str,
    field: &str,
) -> Result<&'d Node, ToolError> {
    let node = node(document, id, field)?;
    if node.node_type() == NodeType::Artboard {
        return Ok(node);
    }
    let hint = match document.artboard_of(&node.id) {
        Some(artboard) => format!(" It is in the artboard \"{}\"; pass that id.", artboard.id),
        None => String::new(),
    };
    Err(ToolError::argument(
        field,
        format!(
            "\"{field}\" should be the id of an artboard, but {} is not one.{hint}",
            node.label()
        ),
    ))
}

/// A number as it goes into a result: two decimals at most, and whole
/// numbers without a fraction.
pub(crate) fn number(value: f64) -> Value {
    if !value.is_finite() {
        return Value::Null;
    }
    let rounded = (value * 100.0).round() / 100.0;
    if rounded.fract() == 0.0 && rounded.abs() < 1e15 {
        json!(rounded as i64)
    } else {
        json!(rounded)
    }
}

/// A number as it is written in a sentence: `1080`, `12.5`.
pub(crate) fn number_text(value: f64) -> String {
    number(value).to_string()
}

/// A box as `{ "x", "y", "width", "height" }`.
pub(crate) fn rect(rect: &Rect) -> Value {
    json!({
        "x": number(rect.x),
        "y": number(rect.y),
        "width": number(rect.width),
        "height": number(rect.height),
    })
}

/// The few words that identify a node: id, type and, when it has one, name.
pub(crate) fn brief(node: &Node) -> Value {
    let mut value = json!({ "id": node.id, "type": node.node_type().as_str() });
    if !node.name().is_empty() {
        value["name"] = json!(node.name());
    }
    value
}

/// `layer` or `animation`.
pub(crate) fn family(node: &Node) -> &'static str {
    match node.family() {
        Family::Layer => "layer",
        Family::Animation => "animation",
    }
}

/// Cuts every string in `value` that is longer than `limit` characters, and
/// says how long it was. Embedded images would otherwise fill a result.
pub(crate) fn shorten_strings(value: &mut Value, limit: usize) {
    match value {
        Value::String(text) => {
            let length = text.chars().count();
            if length > limit {
                let head: String = text.chars().take(limit).collect();
                *text = format!("{head}… ({length} characters in all)");
            }
        }
        Value::Array(items) => items
            .iter_mut()
            .for_each(|item| shorten_strings(item, limit)),
        Value::Object(fields) => fields
            .values_mut()
            .for_each(|field| shorten_strings(field, limit)),
        _ => {}
    }
}

/// The longest string a result shows in full.
pub(crate) const STRING_LIMIT: usize = 300;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::testing::demo_document;

    #[test]
    fn a_name_in_place_of_an_id_is_explained() {
        let document = demo_document();
        assert_eq!(node(&document, "card", "id").unwrap().name(), "Card");
        let error = node(&document, "Card", "id").unwrap_err().to_string();
        assert_eq!(
            error,
            "\"id\": there is no node with the id \"Card\". \"Card\" is the name of group \
             \"card\": pass that id instead."
        );
        let error = node(&document, "zz", "target").unwrap_err().to_string();
        assert!(error.contains("Find ids with get_overview or query_nodes."));
    }

    #[test]
    fn layers_and_artboards_are_told_from_other_nodes() {
        let document = demo_document();
        assert!(layer(&document, "dot", "layer").is_ok());
        let error = layer(&document, "cardfade", "layer")
            .unwrap_err()
            .to_string();
        assert!(error.contains("is an animation"), "{error}");
        assert!(error.contains("It animates the layer \"card\""), "{error}");

        assert!(artboard(&document, "scene", "artboard").is_ok());
        let error = artboard(&document, "dot", "artboard")
            .unwrap_err()
            .to_string();
        assert!(error.contains("It is in the artboard \"scene\""), "{error}");
    }

    #[test]
    fn numbers_are_short() {
        assert_eq!(number(240.0), json!(240));
        assert_eq!(number(0.1 + 0.2), json!(0.3));
        assert_eq!(number(-12.3456), json!(-12.35));
        assert_eq!(number(f64::NAN), Value::Null);
        assert_eq!(
            rect(&Rect::new(1.0, 2.5, 3.0, 4.0)),
            json!({ "x": 1, "y": 2.5, "width": 3, "height": 4 })
        );
    }

    #[test]
    fn long_strings_are_cut_with_their_length() {
        let mut value = json!({ "source": "x".repeat(1000), "list": ["short"] });
        shorten_strings(&mut value, 10);
        assert_eq!(value["source"], "xxxxxxxxxx… (1000 characters in all)");
        assert_eq!(value["list"][0], "short");
    }
}
