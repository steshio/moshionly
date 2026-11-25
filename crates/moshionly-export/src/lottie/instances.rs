//! Instances of components, which Lottie has no word for: before
//! converting, every instance is detached into a plain group on a copy of
//! the document, so the converter only meets layers it knows.

use std::borrow::Cow;
use std::collections::HashSet;

use moshionly_core::model::{LayerProps, NodeData};
use moshionly_core::{Document, Family, NodeId, build};

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
    (
        Cow::Owned(build::without_instances(document).0),
        notes(document, artboard),
    )
}

/// Warnings for the instances `artboard` shows, directly or inside other
/// components, that detaching cannot keep exactly.
fn notes(document: &Document, artboard: &NodeId) -> Vec<String> {
    let mut notes = Vec::new();
    let mut seen = HashSet::new();
    let mut pending = vec![artboard.clone()];
    while let Some(root) = pending.pop() {
        for (_, node) in document.walk(&root, Family::Layer) {
            let NodeData::Instance(instance) = &node.data else {
                continue;
            };
            let Some(main) = document.get(&instance.component) else {
                continue;
            };
            if let Some(layer) = main.layer()
                && out_of_proportion(layer, &instance.layer)
            {
                notes.push(format!(
                    "{} is an instance resized out of proportion; here its content is \
                     stretched like a resized group, keeping stroke widths and font sizes.",
                    warnings::name(node)
                ));
            }
            if seen.insert(main.id.clone()) {
                pending.push(main.id.clone());
            }
        }
    }
    notes
}

/// Whether an instance's box is the component's scaled unevenly.
fn out_of_proportion(component: &LayerProps, instance: &LayerProps) -> bool {
    let ratio = |size: f64, main: f64| (main > 0.0).then(|| size / main);
    match (
        ratio(instance.width, component.width),
        ratio(instance.height, component.height),
    ) {
        (Some(sx), Some(sy)) if sx.is_finite() && sy.is_finite() => {
            (sx - sy).abs() > 1e-9 * sx.abs().max(sy.abs()).max(1.0)
        }
        _ => false,
    }
}
