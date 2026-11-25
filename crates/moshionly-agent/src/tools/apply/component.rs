//! The operations for components: `make_component` and `detach`.

use moshionly_core::build;
use serde_json::json;

use super::batch::{Batch, Owner};
use super::place::sentence;
use crate::tool::ToolError;
use crate::tools::args::Args;

/// The fields a `make_component` operation takes.
pub(super) const MAKE_FIELDS: &[&str] = &["op", "id", "ids", "temp_id"];
/// The fields a `detach` operation takes.
pub(super) const DETACH_FIELDS: &[&str] = &["op", "id", "ids", "temp_id"];

pub(super) fn make_component(batch: &mut Batch, args: &Args) -> Result<(), ToolError> {
    let ids = batch.targets(
        args,
        "the id of the group to make a component, or \"ids\" for layers to wrap in a new one",
    )?;
    let temp = args.string("temp_id")?;
    let (component, edits) = build::make_component(&batch.scratch, &ids)
        .map_err(|error| ToolError::Rejected(sentence(&error.to_string())))?;
    let in_place = ids == [component.clone()];
    batch.commit(edits, Owner::Operation)?;
    if let Some(temp) = &temp {
        batch.declare(temp, &component, &args.field("temp_id"))?;
    }
    batch.components += 1;
    let how = if in_place {
        "group made a component"
    } else {
        "new"
    };
    batch.created(&component, temp.as_deref(), Some(("component", json!(how))));
    Ok(())
}

pub(super) fn detach(batch: &mut Batch, args: &Args) -> Result<(), ToolError> {
    let ids = batch.targets(
        args,
        "the id of the instance to detach, or \"ids\" for several",
    )?;
    let temp = args.string("temp_id")?;
    if temp.is_some() && ids.len() > 1 {
        return Err(ToolError::argument(
            args.field("temp_id"),
            "\"temp_id\" names the group one instance becomes; give it only with \"id\".",
        ));
    }
    for id in ids {
        let (group, edits) = build::detach(&batch.scratch, &id)
            .map_err(|error| ToolError::Rejected(sentence(&error.to_string())))?;
        batch.commit(edits, Owner::Operation)?;
        if let Some(temp) = &temp {
            batch.declare(temp, &group, &args.field("temp_id"))?;
        }
        batch.detached += 1;
        batch.created(&group, temp.as_deref(), Some(("detached_from", json!(id))));
    }
    Ok(())
}
