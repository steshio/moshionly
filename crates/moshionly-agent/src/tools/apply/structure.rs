//! The operations that rearrange the tree: `move` and `clone`.

use std::collections::{HashMap, HashSet};

use moshionly_core::build::{self, Place};
use moshionly_core::{Edit, Family, Node, NodeData, NodeId, NodeType};
use serde_json::{Map, Value, json};

use super::batch::{Batch, Owner};
use super::place::{self, Parent};
use super::warn;
use crate::tool::ToolError;
use crate::tools::args::Args;

/// The fields a `move` operation takes.
pub(super) const MOVE_FIELDS: &[&str] = &[
    "op",
    "id",
    "parent",
    "position",
    "before",
    "after",
    "keep_position",
];
/// The fields a `clone` operation takes.
pub(super) const CLONE_FIELDS: &[&str] = &[
    "op",
    "id",
    "temp_id",
    "parent",
    "position",
    "before",
    "after",
    "offset",
    "time_offset",
    "target",
];

/// The most copied ids a result lists one by one.
const MAX_LISTED_COPIES: usize = 40;

/// Answers the usual mix-up of `move` with moving a layer on the artboard,
/// before the fields are checked.
pub(super) fn check_move(args: &Args) -> Result<(), ToolError> {
    for field in ["x", "y", "by", "dx", "dy", "offset", "to"] {
        if args.has(field) {
            return Err(ToolError::argument(
                args.field(field),
                "The move operation changes where a node sits in the layer tree or the timeline \
                 (its parent or its order). To shift a layer on the artboard, use update with \
                 \"set\": {\"x\": ..., \"y\": ...}; to animate it moving, add a \"move\" \
                 animation.",
            ));
        }
    }
    Ok(())
}

pub(super) fn move_node(batch: &mut Batch, args: &Args) -> Result<(), ToolError> {
    let text = args.required_string("id", "the id of the node to move")?;
    let id = batch.resolve(&text, &args.field("id"))?;
    let node = batch.node(&id)?;
    let (family, current_parent, is_artboard) = (
        node.family(),
        node.parent.clone(),
        node.node_type() == NodeType::Artboard,
    );
    let placement = place::read(batch, args)?;
    let keep_position = args.boolean("keep_position")?.unwrap_or(true);

    let edits = if is_artboard {
        // Artboards only change their order on the canvas.
        if let Parent::Given(parent) = &placement.parent {
            let label = batch.node(parent)?.label();
            return Err(ToolError::argument(
                args.field("parent"),
                format!(
                    "An artboard is a top-level frame and cannot go inside {label}. To put its \
                     layers somewhere else, move them one by one, or clone the artboard."
                ),
            ));
        }
        let index = place::index(batch, args, None, family, &placement.place, Some(&id))?;
        vec![Edit::set(&id, "index", index)]
    } else {
        let parent = match placement.parent {
            Parent::Given(parent) => parent,
            Parent::Unset => current_parent.ok_or_else(|| {
                ToolError::Rejected(format!("\"{id}\" has no parent to stay in."))
            })?,
            Parent::Root => {
                return Err(ToolError::argument(
                    args.field("parent"),
                    "Only artboards sit next to artboards on the canvas. Name the artboard or \
                     group to move the node into as \"parent\".",
                ));
            }
        };
        build::move_node(
            &batch.scratch,
            &id,
            &parent,
            &placement.place,
            keep_position,
        )
        .map_err(|error| {
            place::explain(
                batch,
                args,
                error,
                Some(&parent),
                family,
                &placement.place,
                Some(&id),
            )
        })?
    };
    batch.commit(edits, Owner::Operation)?;
    batch.moved += 1;
    Ok(())
}

pub(super) fn clone(batch: &mut Batch, args: &Args) -> Result<(), ToolError> {
    let text = args.required_string("id", "the id of the layer or animation to copy")?;
    let id = batch.resolve(&text, &args.field("id"))?;
    let temp = args.string("temp_id")?;
    let time_offset = args.number("time_offset")?.unwrap_or(0.0);
    let node = batch.node(&id)?;
    let (mut edits, copies) = if node.is_layer() {
        clone_layer(batch, args, &id)?
    } else {
        clone_animation(batch, args, &id)?
    };
    if time_offset != 0.0 {
        let copied: HashSet<&NodeId> = copies.values().collect();
        shift_animations(args, &mut edits, time_offset, &copied)?;
    }
    let Some(top) = edits.first().map(|edit| edit.node_id().clone()) else {
        return Err(ToolError::Rejected("There was nothing to copy.".to_owned()));
    };
    if let Some(temp) = &temp {
        batch.declare(temp, &top, &args.field("temp_id"))?;
    }
    let added: Vec<NodeId> = edits.iter().map(|edit| edit.node_id().clone()).collect();
    batch.commit(edits, Owner::Operation)?;

    let top_is_layer = batch.scratch.get(&top).is_some_and(Node::is_layer);
    if top_is_layer {
        warn::layer(batch, &top);
    }
    // One warning for the copy that ends latest is enough.
    let latest = added
        .iter()
        .filter_map(|id| Some((id, warn::end_of(batch.scratch.get(id)?)?)))
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(id, _)| id.clone());
    if let Some(latest) = latest {
        warn::late(batch, &latest);
    }
    let listed = if copies.len() <= MAX_LISTED_COPIES {
        let mut map = Map::new();
        let mut pairs: Vec<(&NodeId, &NodeId)> = copies.iter().collect();
        pairs.sort_by_key(|(_, copy)| added.iter().position(|id| id == *copy));
        for (original, copy) in pairs {
            map.insert(original.to_string(), json!(copy));
        }
        Value::Object(map)
    } else {
        json!(copies.len())
    };
    batch.created(&top, temp.as_deref(), Some(("copies", listed)));
    Ok(())
}

/// The edits that copy a layer with what is inside it and the animations
/// on all of it, and the id of every copy by the id of its original.
fn clone_layer(
    batch: &Batch,
    args: &Args,
    id: &NodeId,
) -> Result<(Vec<Edit>, HashMap<NodeId, NodeId>), ToolError> {
    if args.has("target") {
        return Err(ToolError::argument(
            args.field("target"),
            "\"target\" is for copying an animation onto another layer. A copied layer takes \
             copies of its own animations along.",
        ));
    }
    let node = batch.node(id)?;
    let is_artboard = node.node_type() == NodeType::Artboard;
    if is_artboard && args.has("parent") {
        return Err(ToolError::argument(
            args.field("parent"),
            "A copy of an artboard is a new artboard on the canvas, to the right of the others. \
             Leave out \"parent\".",
        ));
    }
    let placement = place::read(batch, args)?;
    let own_parent = || node.parent.clone().unwrap_or_else(|| id.clone());
    let (parent, place) = match (placement.parent, placement.place) {
        // The copy of an artboard goes to the right of the others; the
        // helper does not look at the parent it is given.
        _ if is_artboard => (id.clone(), Place::Last),
        (Parent::Given(parent), place) => (parent, place),
        // Without directions the copy goes just in front of the original.
        (Parent::Unset, Place::Last) if !args.has("position") => {
            (own_parent(), Place::After(id.clone()))
        }
        (Parent::Unset, place) => (own_parent(), place),
        (Parent::Root, _) => {
            return Err(ToolError::argument(
                args.field("parent"),
                "Only artboards sit next to artboards on the canvas. Name the artboard or group \
                 the copy goes in as \"parent\".",
            ));
        }
    };
    let (mut edits, copies) =
        build::clone_subtree(&batch.scratch, id, &parent, &place).map_err(|error| {
            place::explain(
                batch,
                args,
                error,
                Some(&parent),
                Family::Layer,
                &place,
                None,
            )
        })?;
    let (dx, dy) = read_offset(args)?;
    if (dx, dy) != (0.0, 0.0)
        && let Some(Edit::Add { node }) = edits.first_mut()
        && let Some(layer) = node.data.layer_mut()
    {
        layer.x += dx;
        layer.y += dy;
    }
    Ok((edits, copies))
}

/// The edits that copy an animation (or a timeline folder with what is in
/// it) onto the row below it, aimed at `target` when one is given.
fn clone_animation(
    batch: &Batch,
    args: &Args,
    id: &NodeId,
) -> Result<(Vec<Edit>, HashMap<NodeId, NodeId>), ToolError> {
    for field in ["parent", "position", "before", "after", "offset"] {
        if args.has(field) {
            return Err(ToolError::argument(
                args.field(field),
                format!(
                    "\"{field}\" is for copying a layer. A copied animation goes on the row \
                     below the original; to put it elsewhere, give it a temp_id and move it in a \
                     later operation."
                ),
            ));
        }
    }
    let (mut edits, copies) = build::duplicate(&batch.scratch, std::slice::from_ref(id))
        .map_err(|error| ToolError::Rejected(place::sentence(&error.to_string())))?;
    let Some(text) = args.string("target")? else {
        return Ok((edits, copies));
    };
    let field = args.field("target");
    let target = batch.resolve(&text, &field)?;
    let Some(Edit::Add { node }) = edits.first_mut() else {
        return Ok((edits, copies));
    };
    if node.node_type() == NodeType::Audio {
        return Err(ToolError::argument(
            field,
            "\"target\" cannot be used when copying an audio item: sound changes no layer.",
        ));
    }
    if node.target().is_none() {
        return Err(ToolError::argument(
            field,
            "\"target\" cannot be used when copying an animation_group: aim the animations \
             inside it one by one after copying.",
        ));
    }
    node.data.set_target(target.clone());
    // An animation lives on the timeline of its target's artboard.
    let artboard = batch
        .scratch
        .artboard_of(&target)
        .map(|artboard| artboard.id.clone());
    let current = node
        .parent
        .as_ref()
        .and_then(|parent| batch.scratch.artboard_of(parent))
        .map(|artboard| artboard.id.clone());
    if let Some(artboard) = artboard
        && current.as_ref() != Some(&artboard)
    {
        node.index = place::index(
            batch,
            args,
            Some(&artboard),
            Family::Animation,
            &Place::Last,
            None,
        )?;
        node.parent = Some(artboard);
    }
    Ok((edits, copies))
}

/// `offset` as a shift in x and y.
fn read_offset(args: &Args) -> Result<(f64, f64), ToolError> {
    let Some(value) = args.get("offset") else {
        return Ok((0.0, 0.0));
    };
    let offset = Args::nested(args.field("offset"), value, &["x", "y"])?;
    Ok((
        offset.number("x")?.unwrap_or(0.0),
        offset.number("y")?.unwrap_or(0.0),
    ))
}

/// Moves every copied animation, video clip and sound `by` milliseconds
/// later (earlier when negative), and so what every copied instance plays:
/// `copied` holds the ids of the copies.
fn shift_animations(
    args: &Args,
    edits: &mut [Edit],
    by: f64,
    copied: &HashSet<&NodeId>,
) -> Result<(), ToolError> {
    for edit in edits.iter_mut() {
        let Edit::Add { node } = edit else { continue };
        let what = match node.name() {
            "" => node.node_type().to_string(),
            name => format!("{} ({name})", node.node_type()),
        };
        let start = match &mut node.data {
            // A copied instance plays its component's animations later too,
            // unless that component was copied along: its animations moved
            // already.
            NodeData::Instance(instance) => {
                if !copied.contains(&instance.component) {
                    instance.time_offset += by;
                }
                continue;
            }
            NodeData::Show(instant) | NodeData::Hide(instant) => {
                instant.start += by;
                if let Some(end) = &mut instant.end {
                    *end += by;
                }
                instant.start
            }
            // Video layers and sound play their clip later too.
            data if data.clip().is_some() => match data.clip_mut() {
                Some(clip) => {
                    clip.start += by;
                    clip.end += by;
                    clip.start
                }
                None => continue,
            },
            data => match data.timing_mut() {
                Some(timing) => {
                    timing.start += by;
                    timing.end += by;
                    timing.start
                }
                None => continue,
            },
        };
        if start < 0.0 {
            return Err(ToolError::argument(
                args.field("time_offset"),
                format!(
                    "\"time_offset\" {by} would make the copied {what} start before 0 ms. Use \
                     a smaller shift."
                ),
            ));
        }
    }
    Ok(())
}
