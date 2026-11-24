//! The box auto-sized text is drawn in, and making its stored box match.
//!
//! Text whose box follows its words (`auto_size` of `width_and_height` or
//! `height`) keeps a `width` and `height` in the file that are only a hint:
//! the engine measures the words and grows the box from the side the text
//! is aligned to. So the box such a layer is drawn in is not the box its
//! `x`, `y`, `width` and `height` describe.
//!
//! Moving, resizing, turning and grouping work on the box that is drawn.
//! Before they do, the text is *settled*: its drawn box at rest is written
//! back as its `x`, `y`, `width` and `height`. That changes nothing on the
//! canvas (the words measure the same, and a box already their size grows
//! by nothing), and afterwards every rule that works on a layer's stored
//! box sees the real one.

use std::collections::HashSet;
use std::sync::Arc;

use moshionly_core::model::{AutoSize, LayerProps, TextLayer};
use moshionly_core::{Affine, Edit, Family, Node, NodeId, Point, Scene, evaluate_at_rest};
use serde_json::Value;

use super::Editor;
use super::inspect::{text_of, tidy};

/// A layer's box before it is turned: its top-left corner in its parent's
/// coordinates, and its size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    /// Left edge.
    pub x: f64,
    /// Top edge.
    pub y: f64,
    /// Width.
    pub width: f64,
    /// Height.
    pub height: f64,
}

impl Frame {
    /// The box a layer's properties describe.
    pub fn of(layer: &LayerProps) -> Self {
        Self {
            x: layer.x,
            y: layer.y,
            width: layer.width,
            height: layer.height,
        }
    }
}

/// Whether a text layer's box follows its words.
pub fn is_auto_sized(text: &TextLayer) -> bool {
    text.auto_size != AutoSize::Fixed
}

/// The box `text` is drawn in at rest when its words measure `measured`
/// (width and height): the box grows from the point its alignment names,
/// along the layer's own axes, as the engine grows it.
pub fn settled(text: &TextLayer, measured: (f64, f64)) -> Frame {
    let layer = &text.layer;
    let (width, height) = match text.auto_size {
        AutoSize::WidthAndHeight => measured,
        AutoSize::Height => (layer.width, measured.1),
        AutoSize::Fixed => (layer.width, layer.height),
    };
    let anchor = (text.align.factor(), text.vertical_align.factor());
    // How far the centre moves, along the layer's resting axes.
    let shift = Point::new(
        -(anchor.0 - 0.5) * (width - layer.width),
        -(anchor.1 - 0.5) * (height - layer.height),
    );
    let moved = (Affine::rotate(layer.rotation) * Affine::scale(layer.scale, layer.scale))
        .apply_vector(shift);
    let center = layer.center();
    Frame {
        x: center.x + moved.x - width / 2.0,
        y: center.y + moved.y - height / 2.0,
        width,
        height,
    }
}

/// The edit that writes `frame` as the box of `node`, or `None` when that
/// is its box already (to a hundredth).
pub fn settle_edit(node: &Node, frame: Frame) -> Option<Edit> {
    let stored = Frame::of(node.layer()?);
    let tidied = Frame {
        x: tidy(frame.x),
        y: tidy(frame.y),
        width: tidy(frame.width),
        height: tidy(frame.height),
    };
    let same = |a: f64, b: f64| (a - b).abs() < 0.005;
    if same(stored.x, tidied.x)
        && same(stored.y, tidied.y)
        && same(stored.width, tidied.width)
        && same(stored.height, tidied.height)
    {
        return None;
    }
    Some(Edit::update(
        node.id.clone(),
        [
            ("x", Value::from(tidied.x)),
            ("y", Value::from(tidied.y)),
            ("width", Value::from(tidied.width)),
            ("height", Value::from(tidied.height)),
        ],
    ))
}

impl Editor {
    /// The scene of `artboard` at rest, evaluated once per revision: what
    /// the Design tab shows, and what text is measured in.
    pub fn rest_scene(&mut self, artboard: &NodeId) -> Option<Arc<Scene>> {
        if self.view_time().is_none() {
            return self.scene(artboard);
        }
        let version = self.artboard_version(artboard);
        if let Some((cached, scene)) = self.rest_scenes.get(artboard)
            && *cached == version
        {
            return scene.clone();
        }
        let scene = evaluate_at_rest(&self.document, artboard, self.renderer.fonts())
            .ok()
            .map(Arc::new);
        self.rest_scenes
            .insert(artboard.clone(), (version, scene.clone()));
        scene
    }

    /// The box of the layer `id` at rest, as drawn: for text whose box
    /// follows its words, the box they are measured in; for anything else,
    /// the box its properties describe.
    pub fn frame_at_rest(&mut self, id: &NodeId) -> Option<Frame> {
        let document = Arc::clone(&self.document);
        let node = document.get(id)?;
        let layer = node.layer()?;
        let Some(text) = text_of(node).filter(|text| is_auto_sized(text)) else {
            return Some(Frame::of(layer));
        };
        let artboard = document.artboard_of(id)?.id.clone();
        let measured = self
            .rest_scene(&artboard)
            .and_then(|scene| scene.find(id).map(|drawn| (drawn.width, drawn.height)));
        // Text that is not drawn (hidden, or empty) keeps its box.
        Some(measured.map_or(Frame::of(layer), |measured| settled(text, measured)))
    }

    /// The edits that settle the auto-sized text among `ids`, and with
    /// `inside` also the text inside them: each gets the box it is drawn in
    /// as its own. Nothing changes on the canvas.
    pub fn settle_edits(&mut self, ids: &[NodeId], inside: bool) -> Vec<Edit> {
        let document = Arc::clone(&self.document);
        let mut texts: Vec<NodeId> = Vec::new();
        for id in ids {
            texts.push(id.clone());
            if inside {
                texts.extend(document.walk(id, Family::Layer).map(|(_, n)| n.id.clone()));
            }
        }
        let mut seen: HashSet<NodeId> = HashSet::new();
        let mut edits = Vec::new();
        for id in texts {
            if !seen.insert(id.clone()) {
                continue;
            }
            let Some(node) = document.get(&id) else {
                continue;
            };
            if !text_of(node).is_some_and(is_auto_sized) {
                continue;
            }
            if let Some(frame) = self.frame_at_rest(&id)
                && let Some(edit) = settle_edit(node, frame)
            {
                edits.push(edit);
            }
        }
        edits
    }

    /// Settles the auto-sized text among `ids` (and inside them, with
    /// `inside`) in the document, as part of the step being made.
    pub(crate) fn settle(
        &mut self,
        summary: &str,
        ids: &[NodeId],
        inside: bool,
    ) -> Result<(), moshionly_core::EditError> {
        let edits = self.settle_edits(ids, inside);
        self.apply(summary, &edits).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use moshionly_core::NodeType;
    use moshionly_core::model::{NodeData, TextAlign};

    use super::super::view::tests::editor;
    use super::*;

    /// Adds a text layer with `content` to the test editor's first
    /// artboard, set up by `setup`, and returns its id.
    fn add_text(editor: &mut Editor, content: &str, setup: impl FnOnce(&mut TextLayer)) -> NodeId {
        let mut data = moshionly_core::build::default_layer(NodeType::Text).unwrap();
        if let NodeData::Text(text) = &mut data {
            text.content = content.to_owned();
            text.layer.x = 100.0;
            text.layer.y = 500.0;
            setup(text);
        }
        let artboard = NodeId::from("ab");
        let (id, edits) =
            moshionly_core::build::add_layer(editor.document(), &artboard, data).unwrap();
        editor.apply("Add text", &edits).unwrap();
        id
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 0.05
    }

    #[test]
    fn the_frame_of_auto_sized_text_is_the_box_it_is_drawn_in() {
        let mut editor = editor();
        // Centred and turned, so that the box grows about a point that is
        // not its corner, along turned axes.
        let id = add_text(&mut editor, "Hello world", |text| {
            text.align = TextAlign::Center;
            text.layer.rotation = 30.0;
        });
        let frame = editor.frame_at_rest(&id).unwrap();
        let stored = Frame::of(editor.document().get(&id).unwrap().layer().unwrap());
        assert!(frame.width > stored.width, "{frame:?} {stored:?}");
        // The frame, as a fixed box, is where the renderer draws the text.
        let drawn = editor.box_of(&id).unwrap();
        let settle = editor.settle_edits(std::slice::from_ref(&id), false);
        assert_eq!(settle.len(), 1);
        editor.apply("Settle", &settle).unwrap();
        let after = editor.box_of(&id).unwrap();
        assert!(
            (after.center - drawn.center).length() < 0.05,
            "{after:?} {drawn:?}"
        );
        assert!((after.size - drawn.size).length() < 0.05);
        let layer = editor.document().get(&id).unwrap().layer().unwrap().clone();
        assert!(close(layer.width, frame.width) && close(layer.x, frame.x));
        // Settled text needs no more settling.
        assert!(editor.settle_edits(&[id], false).is_empty());
    }

    #[test]
    fn only_text_that_follows_its_words_is_settled() {
        let mut editor = editor();
        let fixed = add_text(&mut editor, "Hello", |text| {
            text.auto_size = AutoSize::Fixed;
        });
        assert!(
            editor
                .settle_edits(&[fixed, NodeId::from("loose")], false)
                .is_empty()
        );
        // Text inside a group is settled with it when asked.
        let card = NodeId::from("card");
        let mut data = moshionly_core::build::default_layer(NodeType::Text).unwrap();
        if let NodeData::Text(text) = &mut data {
            text.content = "Inside".to_owned();
        }
        let (inner, edits) =
            moshionly_core::build::add_layer(editor.document(), &card, data).unwrap();
        editor.apply("Add text", &edits).unwrap();
        assert!(
            editor
                .settle_edits(std::slice::from_ref(&card), false)
                .is_empty()
        );
        let edits = editor.settle_edits(std::slice::from_ref(&card), true);
        assert_eq!(edits.len(), 1);
        assert!(format!("{edits:?}").contains(inner.as_str()));
    }
}
