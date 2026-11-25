//! Instances of components, which Lottie has no word for: before
//! converting, every instance is detached into a plain group on a copy of
//! the document, so the converter only meets layers it knows.

use std::borrow::Cow;
use std::collections::HashSet;

use moshionly_core::build::{self, DetachLoss};
use moshionly_core::model::NodeData;
use moshionly_core::{Document, Family, NodeId};

use super::warnings;

/// The document to convert: `document` itself when it has no instances,
/// or else a copy with every instance detached. With it come warnings for
/// what the copy shows differently in `artboard`.
pub(crate) fn expanded<'a>(
    document: &'a Document,
    artboard: &NodeId,
) -> (Cow<'a, Document>, Vec<String>) {
    let has_instances = document
        .nodes_unordered()
        .any(|node| node.component().is_some());
    if !has_instances {
        return (Cow::Borrowed(document), Vec::new());
    }
    let (flat, losses) = build::without_instances(document);
    (Cow::Owned(flat), notes(document, artboard, &losses))
}

/// Warnings for the instances `artboard` shows, directly or inside other
/// components, that detaching did not keep exactly: `losses` says which.
fn notes(document: &Document, artboard: &NodeId, losses: &[(NodeId, DetachLoss)]) -> Vec<String> {
    let mut notes = Vec::new();
    let mut seen = HashSet::new();
    let mut pending = vec![artboard.clone()];
    while let Some(root) = pending.pop() {
        for (_, node) in document.walk(&root, Family::Layer) {
            let NodeData::Instance(instance) = &node.data else {
                continue;
            };
            let lost = losses.iter().filter(|(id, _)| *id == node.id);
            for (_, loss) in lost {
                let name = warnings::name(node);
                notes.push(match loss {
                    DetachLoss::Stretched => format!(
                        "{name} is an instance resized out of proportion; here its content is \
                         stretched like a resized group, keeping stroke widths and font sizes."
                    ),
                    DetachLoss::Squeezed => format!(
                        "{name} is an instance that starts {} ms into its component's \
                         animations; here the motion before that is squeezed into what is \
                         left of it.",
                        -instance.time_offset
                    ),
                });
            }
            if seen.insert(instance.component.clone()) {
                pending.push(instance.component.clone());
            }
        }
    }
    notes
}
