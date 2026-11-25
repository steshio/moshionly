//! The operations that change or remove what is there: `update`, `remove`
//! and `set_duration`.

use moshionly_core::{Edit, Family, NodeType, build};
use serde_json::{Value, json};

use super::batch::{Batch, Owner};
use super::warn;
use crate::tool::ToolError;
use crate::tools::args::{Args, shown};
use crate::tools::common;

/// The fields an `update` operation takes.
pub(super) const UPDATE_FIELDS: &[&str] = &["op", "id", "ids", "set"];
/// The fields a `remove` operation takes.
pub(super) const REMOVE_FIELDS: &[&str] = &["op", "id", "ids"];
/// The fields a `set_duration` operation takes.
pub(super) const DURATION_FIELDS: &[&str] = &["op", "artboard", "duration"];

pub(super) fn update(batch: &mut Batch, args: &Args) -> Result<(), ToolError> {
    let ids = batch.targets(args, "the id of the node to change, or \"ids\" for several")?;
    let mut set = args.object("set")?.ok_or_else(|| {
        args.missing(
            "set",
            "the fields to change and their new values, such as {\"x\": 120, \"fill\": \
             \"#0D99FF\"}; null puts a field back to its default",
        )
    })?;
    if set.is_empty() {
        return Err(args.missing("set", "at least one field to change; it is empty"));
    }
    let set_field = args.field("set");
    for managed in ["parent", "index"] {
        if set.contains_key(managed) {
            return Err(ToolError::argument(
                format!("{set_field}.{managed}"),
                format!(
                    "\"{managed}\" is not changed with update. Use the move operation, with \
                     \"parent\" and \"position\", \"before\" or \"after\", which keeps the layer \
                     where it is on the artboard and takes its animations along."
                ),
            ));
        }
    }
    // Fields that hold an id may hold a temporary id from this batch.
    for (name, what) in [("target", "a layer"), ("component", "a component")] {
        let Some(value) = set.get(name) else {
            continue;
        };
        let field = format!("{set_field}.{name}");
        let Value::String(text) = value else {
            return Err(ToolError::argument(
                field.clone(),
                format!(
                    "\"{field}\" should be the id of {what}, but it is {}.",
                    shown(value)
                ),
            ));
        };
        let real = batch.resolve(text, &field)?;
        set.insert(name.into(), json!(real));
    }

    let edits: Vec<Edit> = ids.iter().map(|id| Edit::update(id, set.clone())).collect();
    batch.commit(edits, Owner::Set)?;
    batch.updated += ids.len();
    for id in &ids {
        let animation = batch
            .scratch
            .get(id)
            .is_some_and(|node| node.is_animation());
        if animation {
            warn::animation(batch, id);
        }
    }
    Ok(())
}

pub(super) fn remove(batch: &mut Batch, args: &Args) -> Result<(), ToolError> {
    let ids = batch.targets(args, "the id of the node to remove, or \"ids\" for several")?;
    let before = batch.scratch.len();
    let edits = build::remove(&batch.scratch, &ids);
    // The instances of a component that goes are detached first, which
    // adds the groups that take their place.
    let added = edits
        .iter()
        .filter(|edit| matches!(edit, Edit::Add { .. }))
        .count();
    let detached: Vec<String> = edits
        .iter()
        .filter_map(|edit| match edit {
            Edit::Remove { id } => batch.scratch.get(id),
            _ => None,
        })
        .filter(|node| node.node_type() == NodeType::Instance && !ids.contains(&node.id))
        .map(|node| node.label())
        .collect();
    batch.commit(edits, Owner::Operation)?;
    // A detached instance is counted as detached, not as removed.
    batch.removed += (before + added)
        .saturating_sub(batch.scratch.len())
        .saturating_sub(detached.len());
    if !detached.is_empty() {
        batch.detached += detached.len();
        batch.warnings.push(format!(
            "A removed component was still shown by instances, so they were detached into \
             plain groups that look the same: {}.",
            detached.join(", ")
        ));
    }
    Ok(())
}

pub(super) fn set_duration(batch: &mut Batch, args: &Args) -> Result<(), ToolError> {
    let text = args.required_string("artboard", "the id of the artboard whose timeline to set")?;
    let field = args.field("artboard");
    let artboard = batch.resolve(&text, &field)?;
    common::artboard(&batch.scratch, artboard.as_str(), &field)?;
    let duration = args
        .number("duration")?
        .ok_or_else(|| args.missing("duration", "the length of the timeline in milliseconds"))?;
    if duration < 100.0 {
        batch.warnings.push(format!(
            "The timeline is now {duration} ms long. Durations are in milliseconds: 3 seconds \
             is 3000."
        ));
    }
    batch.commit(
        vec![Edit::set(&artboard, "duration", duration)],
        Owner::Operation,
    )?;
    batch.durations += 1;

    let late: Vec<String> = batch
        .scratch
        .walk(&artboard, Family::Animation)
        .filter(|(_, node)| node.data.span().is_some_and(|(_, end)| end > duration))
        .map(|(_, node)| node.label())
        .take(5)
        .collect();
    if !late.is_empty() {
        batch.warnings.push(format!(
            "These animations run past the new end at {duration} ms and are cut off: {}.",
            late.join(", ")
        ));
    }
    Ok(())
}
