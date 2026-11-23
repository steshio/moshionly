//! The `add` operation: a new layer or animation.

use moshionly_core::build::{self, ARTBOARD_GAP};
use moshionly_core::model::LayerProps;
use moshionly_core::{Edit, Family, NodeId, NodeType, schema};
use serde_json::{Map, Value, json};

use super::batch::{Batch, Owner};
use super::place::{self, Parent};
use super::warn;
use crate::tool::ToolError;
use crate::tools::args::{Args, shown};
use crate::tools::query::node_type;

/// The fields an `add` operation takes.
pub(super) const FIELDS: &[&str] = &[
    "op", "node", "temp_id", "parent", "position", "before", "after",
];

pub(super) fn add(batch: &mut Batch, args: &Args) -> Result<(), ToolError> {
    let node_field = args.field("node");
    let mut node = args.object("node")?.ok_or_else(|| {
        args.missing(
            "node",
            "the node to add, as an object with its \"type\" and the fields to set, such as \
             {\"type\":\"rect\",\"width\":200,\"height\":120,\"fill\":\"#0D99FF\"}",
        )
    })?;
    let ty = read_type(&node, &node_field)?;
    node.insert("type".into(), json!(ty.as_str()));
    if node.contains_key("index") {
        return Err(ToolError::argument(
            format!("{node_field}.index"),
            "\"index\" is chosen by apply_edits. Say where the node goes among its siblings with \
             \"position\" (\"first\" or \"last\"), \"before\" or \"after\" on the operation.",
        ));
    }
    let temp = read_temp_id(args, &mut node, &node_field)?;
    let mut placement = place::read(batch, args)?;
    if let Some(parent) = node.remove("parent").filter(|parent| !parent.is_null()) {
        let field = format!("{node_field}.parent");
        let Value::String(text) = &parent else {
            return Err(ToolError::argument(
                field.clone(),
                format!(
                    "\"{field}\" should be the id of a node, but it is {}.",
                    shown(&parent)
                ),
            ));
        };
        let parent = batch.resolve(text, &field)?;
        match &placement.parent {
            Parent::Unset => placement.parent = Parent::Given(parent),
            Parent::Given(given) if *given == parent => {}
            _ => {
                return Err(ToolError::argument(
                    field,
                    "\"node.parent\" and the operation's \"parent\" (or the parent of its \
                     \"before\" or \"after\") name different nodes. Give the parent once, as \
                     \"parent\" on the operation.",
                ));
            }
        }
    }

    let id = NodeId::generate();
    let parent = match ty.family() {
        Family::Layer if ty == NodeType::Artboard => {
            artboard_parent(batch, args, &placement.parent)?;
            if !node.contains_key("x") && !node.contains_key("y") {
                let (x, y) = free_canvas_spot(batch);
                node.insert("x".into(), json!(x));
                node.insert("y".into(), json!(y));
            }
            None
        }
        Family::Layer => Some(layer_parent(batch, args, ty, placement.parent.clone())?),
        Family::Animation if ty == NodeType::AnimationGroup => match &placement.parent {
            Parent::Given(parent) => Some(parent.clone()),
            _ => {
                return Err(args.missing(
                    "parent",
                    "the id of the artboard (or animation_group) whose timeline the folder goes \
                     in",
                ));
            }
        },
        Family::Animation => {
            let target = read_target(batch, &mut node, &node_field)?;
            fill_in_timing(ty, &mut node);
            Some(animation_parent(batch, args, &placement.parent, &target)?)
        }
    };
    let index = place::index(
        batch,
        args,
        parent.as_ref(),
        ty.family(),
        &placement.place,
        None,
    )?;
    node.insert("id".into(), json!(id));
    node.insert("parent".into(), json!(parent));
    node.insert("index".into(), json!(index));

    if let Some(temp) = &temp {
        batch.declare(temp, &id, &args.field("temp_id"))?;
    }
    let schema_fix = format!(
        "get_schema with {{\"type\":\"{ty}\"}} lists the fields a {ty} takes and the shape of \
         each value."
    );
    let node = schema::read_node(Value::Object(node))
        .map_err(|problem| with_fix(batch.problem(&problem, Owner::Node), &schema_fix))?;
    batch.commit(vec![Edit::add(node)], Owner::Node)?;
    match ty.family() {
        Family::Layer => warn::layer(batch, &id),
        Family::Animation => warn::animation(batch, &id),
    }
    batch.created(&id, temp.as_deref(), None);
    Ok(())
}

/// The node type the `node` object says it is.
fn read_type(node: &Map<String, Value>, node_field: &str) -> Result<NodeType, ToolError> {
    let field = format!("{node_field}.type");
    match node.get("type") {
        Some(Value::String(name)) => node_type(name.trim()).map_err(|error| match error {
            ToolError::Argument { message, .. } => ToolError::Argument {
                field: field.clone(),
                message,
            },
            other => other,
        }),
        other => Err(ToolError::argument(
            field.clone(),
            format!(
                "\"{field}\" is required: the kind of node, such as \"rect\", \"text\" or \
                 \"fade_in\"; get_schema lists them all. It is {}.",
                other.map_or("missing".to_owned(), shown)
            ),
        )),
    }
}

/// The temporary id from `temp_id`, or from `node.id`, where models often
/// put it.
fn read_temp_id(
    args: &Args,
    node: &mut Map<String, Value>,
    node_field: &str,
) -> Result<Option<String>, ToolError> {
    let in_node = node.remove("id").filter(|id| !id.is_null());
    match (args.string("temp_id")?, in_node) {
        (temp, None) => Ok(temp),
        (None, Some(Value::String(id))) => Ok(Some(id)),
        (Some(temp), Some(Value::String(id))) if temp == id => Ok(Some(temp)),
        (Some(_), Some(Value::String(_))) => Err(ToolError::argument(
            args.field("temp_id"),
            format!(
                "\"temp_id\" and \"{node_field}.id\" differ. Real ids are made by apply_edits; \
                 give the name to refer to the new node by once, as \"temp_id\"."
            ),
        )),
        (_, Some(other)) => Err(ToolError::argument(
            format!("{node_field}.id"),
            format!(
                "\"{node_field}.id\" should be text, but it is {}. Real ids are made by \
                 apply_edits; to refer to the new node later in the batch, give it a \
                 \"temp_id\".",
                shown(&other)
            ),
        )),
    }
}

/// Checks that nothing places an artboard inside another node.
fn artboard_parent(batch: &Batch, args: &Args, parent: &Parent) -> Result<(), ToolError> {
    match parent {
        Parent::Given(parent) => {
            let label = batch.node(parent)?.label();
            Err(ToolError::argument(
                args.field("parent"),
                format!(
                    "An artboard is a top-level frame on the canvas and cannot go inside {label}. \
                     Leave out \"parent\"; to make a frame inside an artboard, add a \"group\" \
                     with \"clip\": true."
                ),
            ))
        }
        Parent::Root | Parent::Unset => Ok(()),
    }
}

/// The parent of a new layer that is not an artboard: the one named, or
/// the only artboard there is.
fn layer_parent(
    batch: &Batch,
    args: &Args,
    ty: NodeType,
    parent: Parent,
) -> Result<NodeId, ToolError> {
    match parent {
        Parent::Given(parent) => Ok(parent),
        Parent::Root => Err(ToolError::argument(
            args.field("parent"),
            format!(
                "Only artboards sit next to artboards on the canvas. A {ty} goes in an artboard \
                 or a group: give \"parent\", and \"before\" or \"after\" a layer inside it."
            ),
        )),
        Parent::Unset => match batch.scratch.artboards() {
            [only] => Ok(only.clone()),
            [] => Err(ToolError::argument(
                args.field("parent"),
                "The file has no artboard yet, and every layer lives in one. Add one first in \
                 the same batch, {\"op\":\"add\",\"temp_id\":\"scene\",\"node\":{\"type\":\
                 \"artboard\",\"width\":1080,\"height\":1080}}, and use \"scene\" as the parent.",
            )),
            several => {
                let list: Vec<String> = several
                    .iter()
                    .filter_map(|id| batch.scratch.get(id))
                    .map(|artboard| artboard.label())
                    .collect();
                Err(args.missing(
                    "parent",
                    &format!(
                        "the id of the artboard or group the {ty} goes in. The artboards are {}",
                        list.join(", ")
                    ),
                ))
            }
        },
    }
}

/// The layer `node.target` names, written back as its real id.
fn read_target(
    batch: &Batch,
    node: &mut Map<String, Value>,
    node_field: &str,
) -> Result<NodeId, ToolError> {
    let field = format!("{node_field}.target");
    let target = match node.get("target") {
        Some(Value::String(text)) => batch.resolve(text, &field)?,
        None | Some(Value::Null) => {
            return Err(ToolError::argument(
                field.clone(),
                format!(
                    "\"{field}\" is required: the id of the layer the animation changes (or the \
                     temp_id of a layer added earlier in this batch)."
                ),
            ));
        }
        Some(other) => {
            return Err(ToolError::argument(
                field.clone(),
                format!(
                    "\"{field}\" should be the id of a layer, but it is {}.",
                    shown(other)
                ),
            ));
        }
    };
    node.insert("target".into(), json!(target));
    Ok(target)
}

/// Gives an animation a `start` of 0 and an `end` a typical length after
/// it, when they are left out.
fn fill_in_timing(ty: NodeType, node: &mut Map<String, Value>) {
    let start = match node.get("start") {
        None | Some(Value::Null) => {
            node.insert("start".into(), json!(0));
            0.0
        }
        Some(start) => start.as_f64().unwrap_or(0.0),
    };
    let has_end = schema::field_names(ty).iter().any(|field| field == "end");
    let duration = build::default_duration(ty);
    if has_end && duration > 0.0 && node.get("end").is_none_or(Value::is_null) {
        node.insert("end".into(), json!(start + duration));
    }
}

/// The timeline a new animation goes on: the one named, or the timeline of
/// its target's artboard.
fn animation_parent(
    batch: &Batch,
    args: &Args,
    parent: &Parent,
    target: &NodeId,
) -> Result<NodeId, ToolError> {
    match parent {
        Parent::Given(parent) => Ok(parent.clone()),
        Parent::Root => Err(ToolError::argument(
            args.field("parent"),
            "An animation goes on an artboard's timeline, not next to an artboard. Name another \
             animation in \"before\" or \"after\", or leave both out.",
        )),
        Parent::Unset => batch
            .scratch
            .artboard_of(target)
            .map(|artboard| artboard.id.clone())
            .ok_or_else(|| {
                ToolError::argument(
                    args.field("node.target"),
                    format!("\"{target}\" is not inside an artboard."),
                )
            }),
    }
}

/// Where a new artboard goes when it does not say: to the right of the
/// others, level with the first.
fn free_canvas_spot(batch: &Batch) -> (f64, f64) {
    let frames: Vec<_> = batch
        .scratch
        .artboards()
        .iter()
        .filter_map(|id| batch.scratch.get(id)?.layer())
        .map(LayerProps::frame)
        .collect();
    match frames.first() {
        None => (0.0, 0.0),
        Some(first) => {
            let right = frames
                .iter()
                .map(|frame| frame.right())
                .fold(f64::MIN, f64::max);
            (right + ARTBOARD_GAP, first.y)
        }
    }
}

/// The same failure with `fix` added, when it has none.
fn with_fix(error: ToolError, fix: &str) -> ToolError {
    match error {
        ToolError::Edit(mut failure) => {
            if failure.fix.is_empty() {
                failure.fix = fix.to_owned();
            }
            ToolError::Edit(failure)
        }
        other => other,
    }
}
