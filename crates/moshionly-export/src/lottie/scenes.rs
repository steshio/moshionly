//! Whole scenes of the artboard, evaluated at the moments the converter
//! asks about and kept for a while.
//!
//! Most values come straight from the document's properties. Some exist
//! only in an evaluated scene: where a layer is on the artboard after its
//! groups have moved it, which part of it a reveal leaves showing, and how
//! its text is cut into pieces. Those are read from scenes, evaluated with
//! the renderer's fonts so text is measured as it is drawn.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use moshionly_core::{Document, NodeId, Scene, TextMeasurer, evaluate};

use super::clock::Clock;

/// How many scenes are kept before the oldest are forgotten.
const KEPT: usize = 2048;

/// Scenes of one artboard by frame.
pub(crate) struct Scenes<'a> {
    document: &'a Document,
    artboard: &'a NodeId,
    measurer: &'a dyn TextMeasurer,
    clock: Clock,
    kept: RefCell<HashMap<u64, Option<Rc<Scene>>>>,
}

impl<'a> Scenes<'a> {
    /// Scenes of `artboard`, with text laid out by `measurer`.
    pub fn new(
        document: &'a Document,
        artboard: &'a NodeId,
        measurer: &'a dyn TextMeasurer,
        clock: Clock,
    ) -> Self {
        Self {
            document,
            artboard,
            measurer,
            clock,
            kept: RefCell::new(HashMap::new()),
        }
    }

    /// The artboard at frame `frame`. `None` only when it cannot be
    /// evaluated at all, which a checked plan rules out.
    pub fn at(&self, frame: f64) -> Option<Rc<Scene>> {
        let key = frame.to_bits();
        if let Some(scene) = self.kept.borrow().get(&key) {
            return scene.clone();
        }
        let scene = evaluate(
            self.document,
            self.artboard,
            self.clock.ms(frame),
            self.measurer,
        )
        .ok()
        .map(Rc::new);
        let mut kept = self.kept.borrow_mut();
        if kept.len() >= KEPT {
            kept.clear();
        }
        kept.insert(key, scene.clone());
        scene
    }
}
