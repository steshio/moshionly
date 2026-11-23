//! `get_node`: everything about one node.

use moshionly_core::model::AutoSize;
use moshionly_core::{
    Affine, Document, Family, Node, NodeData, NodeId, NodeType, Rect, SceneNode, evaluate_at_rest,
    scene::Content,
};
use serde_json::{Value, json};

use super::args::Args;
use super::common::{self, STRING_LIMIT, brief, rect, shorten_strings};
use super::{Context, ToolDef, input_schema};
use crate::host::Host;
use crate::tool::{ToolError, ToolOutput};

pub(crate) static GET_NODE: ToolDef = ToolDef {
    name: "get_node",
    title: "Read a node",
    description: "Returns one node in full: its fields, the artboard and parents it sits in, \
        and for a layer its box at rest in artboard coordinates, its children, and every \
        animation that targets it in the order they take effect. Use it before changing a \
        layer you have only seen in a list, and to learn what already animates it so new \
        motion fits in. For an animation it also names the layer it changes.",
    read_only: true,
    destructive: false,
    schema: || {
        input_schema(
            json!({
                "id": {
                    "type": "string",
                    "description": "The id of the layer or animation. An id, not a name."
                },
                "defaults": {
                    "type": "boolean",
                    "description": "Also show the fields that are at their default value. Default false, which keeps the result short."
                }
            }),
            &["id"],
        )
    },
    run,
};

/// The most children and animations listed for one node.
const LIST_LIMIT: usize = 60;

fn run(context: &Context<'_>, args: &Args) -> Result<ToolOutput, ToolError> {
    let document = context.session.document();
    let id = args.required_string("id", "the id of the node to read")?;
    let node = common::node(&document, &id, "id")?;
    let with_defaults = args.boolean("defaults")?.unwrap_or(false);

    let mut fields = if with_defaults {
        node.to_full_value()
    } else {
        node.to_value()
    };
    if let Value::Object(fields) = &mut fields {
        // Order is told by the lists below; the index itself is noise.
        fields.remove("index");
    }
    let mut result = json!({ "node": fields, "family": common::family(node) });
    if let Some(artboard) = document.artboard_of(&node.id)
        && artboard.id != node.id
    {
        result["artboard"] = brief(artboard);
    }
    let ancestors: Vec<&NodeId> = document
        .ancestors(&node.id)
        .filter(|ancestor| ancestor.node_type() != NodeType::Artboard)
        .map(|ancestor| &ancestor.id)
        .collect();
    if !ancestors.is_empty() {
        result["inside"] = json!(ancestors);
    }

    if node.is_layer() {
        describe_layer(&document, context.host, node, &mut result);
    } else if let Some(target) = node.target().and_then(|target| document.get(target)) {
        result["target"] = brief(target);
    }
    if node.node_type() == NodeType::AnimationGroup {
        let rows: Vec<Value> = document
            .child_nodes(&node.id, Family::Animation)
            .take(LIST_LIMIT)
            .map(brief)
            .collect();
        result["children"] = json!(rows);
    }
    shorten_strings(&mut result, STRING_LIMIT);
    fit(&mut result, context.config.max_result_chars)?;
    Ok(ToolOutput::json(&result))
}

/// Makes `result` fit in `limit` characters. A path's outline is summed up
/// first, as it is what makes a node long; then the animations are named
/// rather than shown whole. A node still too long is refused with a way to
/// read it in parts.
fn fit(result: &mut Value, limit: usize) -> Result<(), ToolError> {
    let fits = |result: &Value| result.to_string().len() <= limit;
    if fits(result) {
        return Ok(());
    }
    if let Some(path) = result.get_mut("node").and_then(|node| node.get_mut("path"))
        && let Some(Value::Array(subpaths)) = path.get("subpaths")
    {
        let count = subpaths.len();
        let anchors: usize = subpaths
            .iter()
            .filter_map(|subpath| subpath.get("anchors").and_then(Value::as_array))
            .map(Vec::len)
            .sum();
        path["subpaths"] = json!(count);
        path["anchors"] = json!(anchors);
        let subpaths = if count == 1 { "subpath" } else { "subpaths" };
        result["path_note"] = json!(format!(
            "The outline is too long to show: {count} {subpaths} with {anchors} anchors in all. \
             \"path\" here counts them instead. To change the shape, set a new \"path\" whole \
             with apply_edits."
        ));
    }
    if !fits(result)
        && let Some(Value::Array(animations)) = result.get_mut("animations")
    {
        for animation in animations.iter_mut() {
            let fields = ["id", "type", "name", "start", "end"]
                .into_iter()
                .filter_map(|key| Some((key.to_owned(), animation.get(key)?.clone())))
                .collect();
            *animation = Value::Object(fields);
        }
        result["animations_note"] = json!(
            "Only named, to keep the result small. get_node on an animation shows it in full."
        );
    }
    if fits(result) {
        return Ok(());
    }
    Err(ToolError::Rejected(format!(
        "This node is longer than a result may be ({limit} characters), even shortened. Read \
         it in parts with query_nodes: its id in \"ids\" and a few \"fields\"."
    )))
}

fn describe_layer(document: &Document, host: &Host, node: &Node, result: &mut Value) {
    if let Some((bounds, estimated)) = resting_box(document, host, node) {
        result["box"] = rect(&bounds);
        if estimated {
            result["box_note"] = json!(
                "The size of this text box follows its text and was estimated without fonts. \
                 Take a screenshot to see its real extent."
            );
        }
    }
    let children = document.children(&node.id, Family::Layer);
    if !children.is_empty() {
        result["children"] = children
            .iter()
            .take(LIST_LIMIT)
            .filter_map(|child| document.get(child))
            .map(brief)
            .collect();
        if children.len() > LIST_LIMIT {
            result["children_note"] = json!(format!(
                "The first {LIST_LIMIT} of {} children, back to front. query_nodes with \
                 \"parent\" lists them all.",
                children.len()
            ));
        }
    }
    let animations = document.animations_targeting(&node.id);
    if !animations.is_empty() {
        result["animations"] = animations
            .iter()
            .take(LIST_LIMIT)
            .map(|animation| {
                let mut value = animation.to_value();
                if let Value::Object(fields) = &mut value {
                    for implied in ["index", "target"] {
                        fields.remove(implied);
                    }
                }
                value
            })
            .collect();
    }
    if node.node_type() == NodeType::Artboard {
        result["timeline_rows"] = json!(document.walk(&node.id, Family::Animation).count());
    }
}

/// The box around a layer at rest, in artboard coordinates, and whether it
/// had to be estimated.
///
/// A text layer whose box follows its text is measured, with the host's
/// fonts when it has them. Everything else has its box stored.
pub(crate) fn resting_box(document: &Document, host: &Host, node: &Node) -> Option<(Rect, bool)> {
    let stored = document.world_bounds(&node.id)?;
    let follows_text =
        matches!(&node.data, NodeData::Text(text) if text.auto_size != AutoSize::Fixed);
    if !follows_text {
        return Some((stored, false));
    }
    let artboard = document.artboard_of(&node.id)?;
    let (scene, real) =
        host.with_text_measurer(|measurer| evaluate_at_rest(document, &artboard.id, measurer));
    let measured = scene
        .ok()
        .and_then(|scene| find_bounds(&scene.nodes, &node.id, Affine::IDENTITY));
    // A hidden layer is not in the scene; its stored box is the best there is.
    Some((measured.unwrap_or(stored), !real))
}

/// The box around the scene node of layer `id`, searching `nodes` and what
/// is inside them. `outer` maps the nodes' parent into the artboard.
pub(crate) fn find_bounds(nodes: &[SceneNode], id: &NodeId, outer: Affine) -> Option<Rect> {
    nodes.iter().find_map(|node| {
        let transform = outer * node.transform;
        if &node.id == id {
            let own = Rect::new(0.0, 0.0, node.width, node.height);
            return Some(transform.bounds_of(&own));
        }
        let inside = match &node.content {
            Content::Group { children } => find_bounds(children, id, transform),
            _ => None,
        };
        inside.or_else(|| {
            let mask = node.mask.as_deref()?;
            find_bounds(std::slice::from_ref(mask), id, transform)
        })
    })
}

#[cfg(test)]
mod tests {
    use crate::host::Host;
    use crate::session::{Session, SessionHandle};
    use crate::tools::testing::{DEMO, call, call_error, demo_session, demo_tools};
    use crate::tools::{Tools, ToolsConfig};
    use serde_json::{Value, json};

    /// The demo project with a path layer `outline` of `anchors` anchors.
    fn tools_with_path(anchors: usize) -> Tools {
        let mut project: Value = serde_json::from_str(DEMO).unwrap();
        let anchors: Vec<Value> = (0..anchors)
            .map(|n| json!({ "point": [n as f64 / anchors as f64, 0.5] }))
            .collect();
        project["nodes"].as_array_mut().unwrap().push(json!({
            "id": "outline", "parent": "scene", "index": "a2", "type": "path",
            "width": 100, "height": 100,
            "path": { "subpaths": [{ "closed": true, "anchors": anchors }] }
        }));
        let document = moshionly_core::io::parse(&project.to_string()).unwrap();
        Tools::new(SessionHandle::new(Session::new(document)), Host::new())
    }

    #[test]
    fn a_long_outline_is_summed_up_to_keep_the_result_small() {
        let short = call(&tools_with_path(4), "get_node", json!({ "id": "outline" }));
        assert_eq!(
            short["node"]["path"]["subpaths"][0]["anchors"]
                .as_array()
                .unwrap()
                .len(),
            4
        );
        assert!(short.get("path_note").is_none());

        let tools = tools_with_path(3000);
        let output = tools.call("get_node", json!({ "id": "outline" }));
        let text = output.joined_text();
        assert!(!output.is_error, "{text}");
        assert!(
            text.len() <= ToolsConfig::default().max_result_chars,
            "{}",
            text.len()
        );
        let long: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(long["node"]["path"]["subpaths"], 1);
        assert_eq!(long["node"]["path"]["anchors"], 3000);
        assert!(
            long["path_note"]
                .as_str()
                .unwrap()
                .contains("1 subpath with 3000 anchors"),
            "{long}"
        );
    }

    #[test]
    fn a_node_too_long_even_shortened_is_refused_with_a_way_to_read_less() {
        let tools = Tools::with_config(
            demo_session(),
            Host::new(),
            ToolsConfig {
                max_result_chars: 200,
                ..ToolsConfig::default()
            },
        );
        let error = call_error(&tools, "get_node", json!({ "id": "card" }));
        assert!(
            error.contains("Read it in parts with query_nodes"),
            "{error}"
        );
    }

    #[test]
    fn a_layer_comes_with_its_box_children_and_animations() {
        let node = call(&demo_tools(), "get_node", json!({ "id": "card" }));
        assert_eq!(
            node["node"],
            json!({ "id": "card", "parent": "scene", "type": "group", "name": "Card",
                    "x": 240, "y": 340, "width": 600, "height": 400, "fill": "#FFFFFF",
                    "corner_radius": 32, "clip": true })
        );
        assert_eq!(node["family"], "layer");
        assert_eq!(
            node["artboard"],
            json!({ "id": "scene", "type": "artboard", "name": "Launch" })
        );
        assert_eq!(
            node["box"],
            json!({ "x": 240, "y": 340, "width": 600, "height": 400 })
        );
        assert_eq!(
            node["children"],
            json!([
                { "id": "accent", "type": "rect", "name": "Accent" },
                { "id": "title", "type": "text", "name": "Title" },
                { "id": "dot", "type": "ellipse", "name": "Dot" },
            ])
        );
        assert_eq!(
            node["animations"],
            json!([{ "id": "cardfade", "parent": "scene", "type": "fade_in",
                     "name": "Card fades in", "start": 0, "end": 500,
                     "easing": { "type": "slow_down" } }])
        );
        assert!(node.get("inside").is_none());
    }

    #[test]
    fn a_nested_layer_has_its_box_in_artboard_coordinates() {
        let node = call(&demo_tools(), "get_node", json!({ "id": "dot" }));
        assert_eq!(
            node["box"],
            json!({ "x": 740, "y": 640, "width": 52, "height": 52 })
        );
        assert_eq!(node["inside"], json!(["card"]));
        assert_eq!(node["animations"][0]["from"], json!({ "x": -100 }));
    }

    #[test]
    fn a_text_box_that_follows_its_text_is_measured_and_marked_as_an_estimate() {
        let node = call(&demo_tools(), "get_node", json!({ "id": "title" }));
        // Ten letters at 0.6 of 64 pixels, one line of 120 percent.
        assert_eq!(
            node["box"],
            json!({ "x": 288, "y": 412, "width": 384, "height": 76.8 })
        );
        assert!(node["box_note"].as_str().unwrap().contains("estimated"));
    }

    #[test]
    fn defaults_can_be_shown() {
        let tools = demo_tools();
        let short = call(&tools, "get_node", json!({ "id": "badge" }));
        assert!(short["node"].get("opacity").is_none());
        let full = call(
            &tools,
            "get_node",
            json!({ "id": "badge", "defaults": true }),
        );
        assert_eq!(full["node"]["opacity"], 100);
        assert_eq!(full["node"]["rotation"], 0);
        assert!(full["node"].get("index").is_none());
    }

    #[test]
    fn an_animation_names_its_target_and_a_folder_its_rows() {
        let tools = demo_tools();
        let animation = call(&tools, "get_node", json!({ "id": "titlefade" }));
        assert_eq!(animation["family"], "animation");
        assert_eq!(
            animation["target"],
            json!({ "id": "title", "type": "text", "name": "Title" })
        );
        assert_eq!(animation["inside"], json!(["extras"]));
        assert!(animation.get("box").is_none());

        let folder = call(&tools, "get_node", json!({ "id": "extras" }));
        assert_eq!(
            folder["children"],
            json!([{ "id": "titlefade", "type": "opacity" }])
        );
    }

    #[test]
    fn an_artboard_reports_its_timeline() {
        let artboard = call(&demo_tools(), "get_node", json!({ "id": "scene" }));
        assert_eq!(
            artboard["box"],
            json!({ "x": 0, "y": 0, "width": 1080, "height": 1080 })
        );
        assert_eq!(artboard["timeline_rows"], 4);
        assert!(artboard.get("artboard").is_none());
    }

    #[test]
    fn a_missing_or_unknown_id_is_explained() {
        let tools = demo_tools();
        assert_eq!(
            call_error(&tools, "get_node", json!({})),
            "\"id\" is required: the id of the node to read."
        );
        let error = call_error(&tools, "get_node", json!({ "id": "Title" }));
        assert!(error.contains("is the name of text \"title\""), "{error}");
    }
}
