//! Things worth telling the model about a node it just placed, although
//! nothing is wrong with the file: a shape nobody will see, an animation
//! that runs past the end of the timeline, a detached instance that does
//! not look quite the same.

use moshionly_core::build::DetachLoss;
use moshionly_core::eval::duration_of;
use moshionly_core::{NodeData, NodeId, NodeType};

use super::batch::Batch;

/// Checks a layer that was added or copied.
pub(super) fn layer(batch: &mut Batch, id: &NodeId) {
    let Some(node) = batch.scratch.get(id) else {
        return;
    };
    let label = node.label();
    let drawn_by_paint = matches!(
        node.node_type(),
        NodeType::Rect | NodeType::Ellipse | NodeType::Star | NodeType::Path | NodeType::Text
    );
    if drawn_by_paint && node.data.fill().is_none() && node.data.stroke().is_none() {
        batch.warnings.push(format!(
            "{label} has no fill and no stroke, so nothing of it is drawn. Give it a \"fill\" \
             such as \"#0D99FF\" to see it."
        ));
    }
    if let NodeData::Text(text) = &node.data
        && text.content.trim().is_empty()
    {
        batch
            .warnings
            .push(format!("{label} has no \"content\", so it shows nothing."));
    }
    if node.node_type() == NodeType::Artboard {
        return;
    }
    let artboard = batch.scratch.artboard_of(id).and_then(|artboard| {
        let layer = artboard.layer()?;
        Some((layer.width, layer.height))
    });
    if let (Some((width, height)), Some(bounds)) = (artboard, batch.scratch.world_bounds(id))
        && (bounds.right() <= 0.0
            || bounds.bottom() <= 0.0
            || bounds.x >= width
            || bounds.y >= height)
    {
        batch.warnings.push(format!(
            "{label} is entirely outside its {width} by {height} artboard (its box is at {}, {}, \
             {} by {}), so it cannot be seen there. Positions are relative to the parent's \
             top-left corner.",
            bounds.x, bounds.y, bounds.width, bounds.height
        ));
    }
}

/// Checks an animation that was added, copied or changed.
pub(super) fn animation(batch: &mut Batch, id: &NodeId) {
    let Some(node) = batch.scratch.get(id) else {
        return;
    };
    let (Some((_, end)), Some(artboard)) = (node.data.span(), batch.scratch.artboard_of(id)) else {
        return;
    };
    let Some(duration) = duration_of(&batch.scratch, &artboard.id) else {
        return;
    };
    if end > duration {
        let message = format!(
            "{} ends at {end} ms, after the end of the artboard's {duration} ms timeline, so its \
             last part is never played. Make it end earlier, or lengthen the timeline with \
             set_duration.",
            node.label()
        );
        batch.warnings.push(message);
    }
}

/// Why the group a detached instance became does not look quite the same,
/// in one clause.
pub(super) fn losses(losses: &[DetachLoss]) -> String {
    let reasons: Vec<String> = losses.iter().map(ToString::to_string).collect();
    reasons.join(", and ")
}
