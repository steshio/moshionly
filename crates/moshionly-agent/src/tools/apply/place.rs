//! Where an operation puts a node: a parent, and a place among its
//! siblings given as `position`, `before` or `after`.

use moshionly_core::build::{self, BuildError, Place};
use moshionly_core::{Family, NodeId};

use super::batch::Batch;
use crate::tool::ToolError;
use crate::tools::args::Args;

/// The parent an operation names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Parent {
    /// `parent`, or the parent of the sibling in `before` or `after`.
    Given(NodeId),
    /// The sibling in `before` or `after` is an artboard, which has none.
    Root,
    /// Neither says.
    Unset,
}

/// Where an operation puts its node.
#[derive(Debug, Clone)]
pub(super) struct Placement {
    pub parent: Parent,
    pub place: Place,
}

/// Reads `parent`, `position`, `before` and `after`.
pub(super) fn read(batch: &Batch, args: &Args) -> Result<Placement, ToolError> {
    let parent = batch.id_arg(args, "parent")?;
    let position = args.choice("position", &["first", "last"])?;
    let before = batch.id_arg(args, "before")?;
    let after = batch.id_arg(args, "after")?;
    let given = [position.is_some(), before.is_some(), after.is_some()];
    if given.iter().filter(|given| **given).count() > 1 {
        return Err(ToolError::argument(
            args.field("position"),
            "Give one of \"position\", \"before\" and \"after\", not several: they all say where \
             the node goes among its siblings.",
        ));
    }
    let place = match (position, before, after) {
        (_, Some(sibling), _) => Place::Before(sibling),
        (_, _, Some(sibling)) => Place::After(sibling),
        (Some("first"), ..) => Place::First,
        _ => Place::Last,
    };
    let parent = match (parent, &place) {
        (Some(parent), _) => Parent::Given(parent),
        (None, Place::Before(sibling) | Place::After(sibling)) => {
            match batch
                .scratch
                .get(sibling)
                .and_then(|node| node.parent.clone())
            {
                Some(parent) => Parent::Given(parent),
                None => Parent::Root,
            }
        }
        (None, _) => Parent::Unset,
    };
    Ok(Placement { parent, place })
}

/// A fresh index for a node of `family` that goes to `place` under
/// `parent` (`None` for an artboard). `moving` is the node itself when it
/// is already somewhere in the document.
pub(super) fn index(
    batch: &Batch,
    args: &Args,
    parent: Option<&NodeId>,
    family: Family,
    place: &Place,
    moving: Option<&NodeId>,
) -> Result<String, ToolError> {
    build::index_at(&batch.scratch, parent, family, place, moving).map_err(|error| match error {
        BuildError::NotASibling(sibling) => {
            let field = match place {
                Place::Before(_) => args.field("before"),
                _ => args.field("after"),
            };
            let holder = match parent.and_then(|parent| batch.scratch.get(parent)) {
                Some(parent) => parent.label(),
                None => "the canvas".to_owned(),
            };
            let what = if moving == Some(&sibling) {
                "is the node being placed".to_owned()
            } else {
                format!("is not a {} directly in {holder}", family_word(family))
            };
            ToolError::argument(
                field.clone(),
                format!(
                    "\"{field}\": \"{sibling}\" {what}, so nothing can be placed next to it. Name \
                     a sibling in the same parent, or leave out \"parent\" to go next to \
                     \"{sibling}\" wherever it is."
                ),
            )
        }
        other => ToolError::Rejected(format!("No place could be found for the node: {other}.")),
    })
}

fn family_word(family: Family) -> &'static str {
    match family {
        Family::Layer => "layer",
        Family::Animation => "animation",
    }
}
