//! The operations that change or remove what is there: `update`, `remove`
//! and `set_duration`.

use moshionly_core::build::{self, DetachLoss};
use moshionly_core::{Document, Edit, Family, Node};
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

    let mut edits: Vec<Edit> = ids.iter().map(|id| Edit::update(id, set.clone())).collect();
    // Instances as large as a component that is resized take its new size,
    // so what they show is not squeezed. Those that already have it are
    // not resized.
    let mut follow = build::instances_follow_mains(&batch.scratch, &edits);
    follow.retain(|edit| resizes(&batch.scratch, edit));
    let followed = follow.len();
    edits.extend(follow);
    batch.commit(edits, Owner::Set)?;
    batch.updated += ids.len();
    batch.followed += followed;
    for id in &ids {
        warn::late(batch, id);
    }
    Ok(())
}

/// Whether `edit` gives a layer of `document` a width or a height other
/// than the one it has.
fn resizes(document: &Document, edit: &Edit) -> bool {
    let Edit::Update { id, patch } = edit else {
        return false;
    };
    let Some(layer) = document.get(id).and_then(Node::layer) else {
        return false;
    };
    [("width", layer.width), ("height", layer.height)]
        .into_iter()
        .any(|(side, size)| patch.get(side).is_some_and(|to| to.as_f64() != Some(size)))
}

pub(super) fn remove(batch: &mut Batch, args: &Args) -> Result<(), ToolError> {
    let ids = batch.targets(args, "the id of the node to remove, or \"ids\" for several")?;
    let before = batch.scratch.len();
    // The instances of a component that goes are detached first, which
    // adds the groups that take their place.
    let (edits, losses) = build::remove_with_losses(&batch.scratch, &ids);
    let added = edits
        .iter()
        .filter(|edit| matches!(edit, Edit::Add { .. }))
        .count();
    // With what each group cannot keep of its instance's look. An instance
    // copied out of another one while detaching is part of that one's
    // group, and what it loses counts toward it.
    let detached: Vec<(String, Vec<DetachLoss>)> = losses
        .into_iter()
        .filter_map(|(id, lost)| Some((batch.scratch.get(&id)?.label(), lost)))
        .collect();
    batch.commit(edits, Owner::Operation)?;
    // A detached instance is counted as detached, not as removed.
    batch.removed += (before + added)
        .saturating_sub(batch.scratch.len())
        .saturating_sub(detached.len());
    if !detached.is_empty() {
        batch.detached += detached.len();
        let names: Vec<&str> = detached.iter().map(|(name, _)| name.as_str()).collect();
        let differ: Vec<String> = detached
            .iter()
            .filter(|(_, losses)| !losses.is_empty())
            .map(|(name, losses)| format!("{name}: {}", warn::losses(losses)))
            .collect();
        let message = if differ.is_empty() {
            format!(
                "A removed component was still shown by instances, so they were detached into \
                 plain groups that look the same: {}.",
                names.join(", ")
            )
        } else {
            format!(
                "A removed component was still shown by instances, so they were detached into \
                 plain groups: {}. They look the same, except {}.",
                names.join(", "),
                differ.join("; ")
            )
        };
        batch.warnings.push(message);
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

    let runs_past = |node: &Node| warn::end_of(node).is_some_and(|end| end > duration);
    let late: Vec<String> = batch
        .scratch
        .walk(&artboard, Family::Animation)
        .chain(batch.scratch.walk(&artboard, Family::Layer))
        .filter(|(_, node)| runs_past(node))
        .map(|(_, node)| node.label())
        .take(5)
        .collect();
    if !late.is_empty() {
        batch.warnings.push(format!(
            "These animations, sounds or videos run past the new end at {duration} ms and are \
             cut off: {}.",
            late.join(", ")
        ));
    }
    Ok(())
}
