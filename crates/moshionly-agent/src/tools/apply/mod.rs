//! `apply_edits`: change the document with one atomic batch of operations.
//!
//! The operations of a call are run one after the other on a scratch copy
//! of the document, so each one sees what the ones before it did, a new
//! node can be referred to by a temporary id, and a failure names the
//! operation and the field at fault. Only when every operation has worked
//! are all their edits applied to the session, together, as one undo step.

mod add;
mod batch;
mod change;
mod component;
mod place;
mod structure;
mod warn;

use serde_json::{Value, json};

use super::args::{Args, closest, short_name};
use super::{Context, ToolDef, input_schema};
use crate::session::Origin;
use crate::tool::{EditFailure, ToolError, ToolOutput};
use batch::Batch;

pub(crate) static APPLY_EDITS: ToolDef = ToolDef {
    name: "apply_edits",
    title: "Edit the file",
    description: "Changes the file. Put every operation of one change in one call: the batch is \
        checked as a whole and applied completely or not at all, and the person can undo it in \
        one step. Operations run in order, each seeing what the ones before it did.\n\
        - add: a new layer or animation. \"node\" holds its \"type\" and the fields to set \
        (get_schema shows them). A layer goes in \"parent\", an artboard or group (with one \
        artboard it may be left out). An animation needs \"node.target\", the layer it changes, \
        and goes on that layer's artboard timeline; \"start\" defaults to 0 and \"end\" to a \
        typical length for the type. Give \"temp_id\" to use the new node in later operations \
        of the same call, wherever an id goes.\n\
        - update: \"set\" fields of \"id\" (or of every node in \"ids\"). null puts a field back \
        to its default; an object merges into the one there, except an easing or a gradient, \
        which is replaced whole.\n\
        - remove: \"id\" or \"ids\", with everything inside them and every animation that \
        targets them.\n\
        - move: put \"id\" in another \"parent\" or at another place among its siblings. A \
        layer keeps its spot on the artboard and takes its animations along. (To shift a layer \
        on the artboard, update its x and y.)\n\
        - clone: copy the layer \"id\" with everything inside it and the animations on all of \
        it, just in front of the original unless placed otherwise, shifted by \"offset\" \
        {x, y} and its animations (so also what copied instances play) by \"time_offset\" \
        ms; good for repeating an element with a stagger. Cloning an animation copies it onto the row below, onto \"target\" if given. \
        \"temp_id\" names the copy; the result maps every original id to its copy.\n\
        - set_duration: the length of an \"artboard\"'s timeline, in milliseconds.\n\
        - make_component: make the group \"id\" a component in place, or wrap the layers in \
        \"ids\" in a new component; \"temp_id\" names it. Then add instances of it with add \
        {\"type\":\"instance\",\"component\":<its id or temp_id>}, sized like the component \
        unless given a width and height: each shows the component's layers and their \
        animations, and changes when the component does. Resizing the component resizes the \
        instances that were its size with it, each side on its own; one sized otherwise keeps \
        its size.\n\
        - detach: turn the instance \"id\" (or every one of \"ids\") into a plain group that \
        looks the same, with copies of the component's layers and animations; \"temp_id\" \
        names the group. Removing a component detaches its instances the same way. Two cases \
        do not look quite the same, and the result warns about them: an instance resized out \
        of proportion (stroke widths and font sizes stay), and a negative time_offset while \
        the component's animations are under way (they are squeezed after 0 ms).\n\
        Where a node goes among its siblings: \"position\" \"first\" or \"last\" (the default: \
        frontmost layer, bottom timeline row), or \"before\" or \"after\" a sibling's id. Units: \
        pixels, degrees clockwise, milliseconds, opacity 0 to 100, colours \"#RRGGBB\". The \
        result gives the ids of what was made; on failure nothing is applied and the error names \
        the operation and the field to fix.",
    read_only: false,
    destructive: true,
    schema,
    run,
};

/// The most operations one call may hold.
const MAX_OPERATIONS: usize = 500;
/// The most new nodes a result lists one by one.
const MAX_LISTED: usize = 100;

/// Looks at an operation before its fields are checked.
type Check = fn(&Args) -> Result<(), ToolError>;

/// One kind of operation: its name, the fields it takes, and what it does.
struct Operation {
    name: &'static str,
    fields: &'static [&'static str],
    /// Looks at the operation before its fields are checked, to answer a
    /// common mistake better than "not a field" would.
    check: Option<Check>,
    run: fn(&mut Batch, &Args) -> Result<(), ToolError>,
}

const OPERATIONS: &[Operation] = &[
    Operation {
        name: "add",
        fields: add::FIELDS,
        check: None,
        run: add::add,
    },
    Operation {
        name: "update",
        fields: change::UPDATE_FIELDS,
        check: None,
        run: change::update,
    },
    Operation {
        name: "remove",
        fields: change::REMOVE_FIELDS,
        check: None,
        run: change::remove,
    },
    Operation {
        name: "move",
        fields: structure::MOVE_FIELDS,
        check: Some(structure::check_move),
        run: structure::move_node,
    },
    Operation {
        name: "clone",
        fields: structure::CLONE_FIELDS,
        check: None,
        run: structure::clone,
    },
    Operation {
        name: "set_duration",
        fields: change::DURATION_FIELDS,
        check: None,
        run: change::set_duration,
    },
    Operation {
        name: "make_component",
        fields: component::MAKE_FIELDS,
        check: None,
        run: component::make_component,
    },
    Operation {
        name: "detach",
        fields: component::DETACH_FIELDS,
        check: None,
        run: component::detach,
    },
];

/// Names models reach for, and the operation each one means.
const ALIASES: &[(&str, &str)] = &[
    ("create", "add"),
    ("insert", "add"),
    ("new", "add"),
    ("set", "update"),
    ("change", "update"),
    ("modify", "update"),
    ("edit", "update"),
    ("patch", "update"),
    ("delete", "remove"),
    ("destroy", "remove"),
    ("duplicate", "clone"),
    ("copy", "clone"),
    ("reparent", "move"),
    ("reorder", "move"),
    ("duration", "set_duration"),
    ("component", "make_component"),
    ("create_component", "make_component"),
    ("componentize", "make_component"),
    ("detach_instance", "detach"),
    ("unlink", "detach"),
];

fn schema() -> Value {
    let names: Vec<&str> = OPERATIONS.iter().map(|operation| operation.name).collect();
    let mut fields = json!({
        "op": {
            "type": "string",
            "enum": names,
            "description": "What the operation does."
        },
        "node": {
            "type": "object",
            "description": "add: the new node, with \"type\" and the fields to set, for example {\"type\":\"rect\",\"name\":\"Card\",\"x\":40,\"y\":40,\"width\":320,\"height\":200,\"fill\":\"#FFFFFF\",\"corner_radius\":24} or {\"type\":\"fade_in\",\"target\":\"<layer id>\",\"start\":0,\"end\":500}."
        },
        "temp_id": {
            "type": "string",
            "description": "add, clone, make_component, detach: a name of your choosing for the new node, copy, component or group, such as \"title\". Later operations of this call can use it wherever an id goes. The result gives the real id."
        },
        "id": {
            "type": "string",
            "description": "The node the operation works on: an id, or a temp_id given earlier in this call."
        },
        "ids": {
            "type": "array",
            "items": { "type": "string" },
            "description": "update, remove, detach: several nodes at once, in place of \"id\". make_component: the layers to wrap in a new component."
        },
        "set": {
            "type": "object",
            "description": "update: the fields to change and their new values, for example {\"opacity\":50,\"stroke\":{\"width\":4}}. null puts a field back to its default."
        },
        "parent": {
            "type": "string",
            "description": "add, move, clone: the artboard or group a layer goes in, or the artboard or animation_group an animation goes in."
        },
        "position": {
            "type": "string",
            "enum": ["first", "last"],
            "description": "Where among the siblings: \"first\" is the backmost layer or top timeline row, \"last\" (the default) the frontmost layer or bottom row."
        },
        "before": {
            "type": "string",
            "description": "Place the node just before this sibling (behind it, for layers)."
        },
        "after": {
            "type": "string",
            "description": "Place the node just after this sibling (in front of it, for layers)."
        },
        "keep_position": {
            "type": "boolean",
            "description": "move: keep the layer where it is on the artboard, adjusting x, y, rotation and scale for the new parent. Default true."
        },
        "offset": {
            "type": "object",
            "properties": { "x": { "type": "number" }, "y": { "type": "number" } },
            "additionalProperties": false,
            "description": "clone: how far to shift the copy from the original, in pixels, for example {\"x\": 0, \"y\": 120}."
        },
        "time_offset": {
            "type": "number",
            "description": "clone: milliseconds to shift the copied animations, and what copied instances play, by, so copies play one after another."
        },
        "target": {
            "type": "string",
            "description": "clone of an animation: the layer the copy animates."
        },
        "artboard": {
            "type": "string",
            "description": "set_duration: the artboard whose timeline to set."
        },
        "duration": {
            "type": "number",
            "minimum": 1,
            "description": "set_duration: the length of the timeline in milliseconds."
        }
    });
    // Only the fields some operation takes are offered.
    if let Value::Object(properties) = &mut fields {
        properties.retain(|name, _| {
            OPERATIONS
                .iter()
                .any(|operation| operation.fields.contains(&name.as_str()))
        });
    }
    input_schema(
        json!({
            "operations": {
                "type": "array",
                "minItems": 1,
                "maxItems": MAX_OPERATIONS,
                "description": "The operations, run in order and applied together.",
                "items": {
                    "type": "object",
                    "properties": fields,
                    "required": ["op"],
                    "additionalProperties": false
                }
            },
            "label": {
                "type": "string",
                "description": "A few words for the undo menu saying what this change does, such as \"Fade in the title\". Default: a list of what the operations did."
            },
            "dry_run": {
                "type": "boolean",
                "description": "Check the operations without applying them. Default false."
            }
        }),
        &["operations"],
    )
}

fn run(context: &Context<'_>, args: &Args) -> Result<ToolOutput, ToolError> {
    let operations = read_operations(args)?;
    let dry_run = args.boolean("dry_run")?.unwrap_or(false);
    let label = args
        .string("label")?
        .map(|label| label.trim().to_owned())
        .filter(|label| !label.is_empty());

    // The whole call holds the session, so nobody changes the document
    // between the checks and the edits.
    context.session.write(|session| {
        let mut batch = Batch::new(session.document().clone());
        for (index, operation) in operations.iter().enumerate() {
            batch.begin(index, "");
            if let Err(error) = run_operation(&mut batch, index, operation) {
                return Err(ToolError::Edit(Box::new(batch.to_failure(error))));
            }
        }
        let summary = label.unwrap_or_else(|| batch.summary());
        if dry_run {
            return Ok(report(&batch, operations.len(), &summary, None));
        }
        match session.apply(Origin::Agent, summary.clone(), &batch.edits) {
            Ok(changed) => {
                let revision = session.revision();
                Ok(report(
                    &batch,
                    operations.len(),
                    &summary,
                    Some((changed, revision)),
                ))
            }
            // Each operation was checked on its own, so the whole batch
            // should not fail. If it does, it is still reported properly.
            Err(error) => Err(ToolError::Edit(Box::new(EditFailure {
                operation: batch.operation_of_edit(error.index),
                op: String::new(),
                field: error.problem.field.clone(),
                message: error.problem.message.clone(),
                fix: String::new(),
            }))),
        }
    })
}

/// The `operations` argument: a list, or one operation on its own.
fn read_operations(args: &Args) -> Result<Vec<Value>, ToolError> {
    let operations = match args.get("operations") {
        Some(Value::Object(_)) => vec![args.get("operations").cloned().unwrap_or_default()],
        _ => args.array("operations")?.ok_or_else(|| {
            args.missing(
                "operations",
                "the list of operations to apply, such as [{\"op\":\"update\",\"id\":\"<id>\",\
                 \"set\":{\"opacity\":50}}]",
            )
        })?,
    };
    if operations.is_empty() {
        return Err(ToolError::argument(
            "operations",
            "\"operations\" is empty, so there is nothing to do.",
        ));
    }
    if operations.len() > MAX_OPERATIONS {
        return Err(ToolError::argument(
            "operations",
            format!(
                "\"operations\" holds {} operations, more than the {MAX_OPERATIONS} one call may \
                 hold. Split the change into several calls.",
                operations.len()
            ),
        ));
    }
    Ok(operations)
}

/// Runs operation `index` of the batch.
fn run_operation(batch: &mut Batch, index: usize, value: &Value) -> Result<(), ToolError> {
    let path = format!("operations[{index}]");
    let names: Vec<&str> = OPERATIONS.iter().map(|operation| operation.name).collect();
    let generic = Args::open(path.as_str(), value)?;
    let name = generic.required_string("op", &format!("one of {}", names.join(", ")))?;
    let wanted = ALIASES
        .iter()
        .find(|(alias, _)| *alias == name)
        .map_or(name.as_str(), |(_, meant)| meant);
    let Some(operation) = OPERATIONS.iter().find(|operation| operation.name == wanted) else {
        let hint = closest(&name, &names)
            .map(|close| format!(" Did you mean \"{close}\"?"))
            .unwrap_or_default();
        return Err(ToolError::argument(
            generic.field("op"),
            format!(
                "\"{}\" is not an operation.{hint} The operations are: {}.",
                short_name(&name),
                names.join(", ")
            ),
        ));
    };
    batch.begin(index, operation.name);
    if let Some(check) = operation.check {
        check(&generic)?;
    }
    let args = Args::nested(path, value, operation.fields)?;
    (operation.run)(batch, &args)
}

/// The result of a batch that worked. `applied` is `None` for a dry run,
/// and otherwise whether anything changed and the revision after.
fn report(
    batch: &Batch,
    operations: usize,
    summary: &str,
    applied: Option<(bool, u64)>,
) -> ToolOutput {
    let mut result = match applied {
        Some(_) => json!({ "applied": operations, "summary": summary }),
        None => json!({ "dry_run": true, "valid": true, "operations": operations,
                        "summary": summary }),
    };
    if !batch.created.is_empty() {
        let mut created: Vec<Value> = batch.created.iter().take(MAX_LISTED).cloned().collect();
        if applied.is_none() {
            // A dry run makes nothing, so its ids would mean nothing.
            for entry in &mut created {
                if let Value::Object(fields) = entry {
                    fields.remove("id");
                }
            }
        }
        result["created"] = json!(created);
        if batch.created.len() > MAX_LISTED {
            result["created_note"] = json!(format!(
                "The first {MAX_LISTED} of {} new nodes. query_nodes finds the rest.",
                batch.created.len()
            ));
        }
    }
    if batch.removed > 0 {
        result["removed"] = json!(batch.removed);
    }
    if !batch.warnings.is_empty() {
        result["warnings"] = json!(batch.warnings);
    }
    if let Some((changed, revision)) = applied {
        if !changed {
            result["note"] = json!(
                "Nothing changed: everything was already as asked, so no undo step was recorded."
            );
        }
        result["revision"] = json!(revision);
    }
    ToolOutput::json(&result)
}

#[cfg(test)]
mod tests;
