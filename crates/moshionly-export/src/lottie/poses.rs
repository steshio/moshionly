//! Poses: where a text layer's box is, and how each of its pieces is moved,
//! at a moment.
//!
//! A text that comes in letter by letter has a keyframed position, scale
//! and opacity for every letter, and each is fitted by looking at many
//! moments. Every letter of the text moves in the same scene, so each
//! moment is evaluated once, for all letters and all their properties
//! together, and kept in a compact table: a few numbers per letter rather
//! than a whole scene.

use std::mem::size_of;
use std::rc::Rc;

use moshionly_core::scene::{Content, TextUnit};
use moshionly_core::{Affine, NodeId};

use super::scenes::{Kept, Scenes};

/// How much the poses of one text may take, in bytes, before the moments
/// looked at longest ago are forgotten.
const BUDGET: usize = 64 << 20;

/// What a text layer looks like at one moment.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Pose {
    /// The text node's box as the scene draws it, or `None` when the text
    /// is not drawn.
    pub drawn: Option<Drawn>,
    /// How each piece is moved, in the order the pieces were given.
    pub pieces: Vec<PiecePose>,
}

/// A text node's box in a scene.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Drawn {
    /// From the box's own space to its parent's.
    pub transform: Affine,
    /// Width of the box.
    pub width: f64,
    /// Height of the box.
    pub height: f64,
}

/// How a piece of text is moved from where layout puts it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PiecePose {
    /// Its shift, `[x, y]`, in pixels.
    pub position: [f64; 2],
    /// Its scale; 1 is its own size.
    pub scale: f64,
    /// Its opacity from 0 to 1.
    pub opacity: f64,
    /// Its blur in pixels.
    pub blur: f64,
}

impl PiecePose {
    /// Where layout puts it, as when the text is not drawn at all: the
    /// layer's own opacity hides it then.
    const AT_REST: PiecePose = PiecePose {
        position: [0.0, 0.0],
        scale: 1.0,
        opacity: 1.0,
        blur: 0.0,
    };

    /// Not drawn: no unit of the text covers it at that moment.
    const UNSEEN: PiecePose = PiecePose {
        opacity: 0.0,
        ..PiecePose::AT_REST
    };

    fn of(unit: &TextUnit) -> Self {
        let [a, _, _, _, e, f] = unit.transform.0;
        PiecePose {
            position: [e, f],
            scale: a,
            opacity: unit.opacity,
            blur: unit.blur,
        }
    }
}

/// The poses of one text layer by frame, worked out once per moment.
pub(crate) struct Poses<'a> {
    scenes: &'a Scenes<'a>,
    id: NodeId,
    /// Where each piece starts in the text, in bytes.
    starts: Vec<usize>,
    kept: Kept<u64, Rc<Pose>>,
}

impl<'a> Poses<'a> {
    /// The poses of the text layer `id`, for pieces starting at `starts`,
    /// from scenes of `scenes`.
    pub fn new(scenes: &'a Scenes<'a>, id: NodeId, starts: Vec<usize>) -> Self {
        Self {
            scenes,
            id,
            starts,
            kept: Kept::new(),
        }
    }

    /// The pose at frame `frame`.
    pub fn at(&self, frame: f64) -> Rc<Pose> {
        let key = frame.to_bits();
        if let Some(pose) = self.kept.get(&key) {
            return pose;
        }
        let pose = Rc::new(self.work_out(frame));
        let size = size_of::<Pose>() + pose.pieces.len() * size_of::<PiecePose>() + 64;
        self.kept.insert(key, pose.clone(), size, BUDGET);
        pose
    }

    fn work_out(&self, frame: f64) -> Pose {
        let scene = self.scenes.evaluate(frame);
        let found = scene.as_ref().and_then(|scene| scene.find(&self.id));
        let Some((node, Content::Text(content))) = found.map(|node| (node, &node.content)) else {
            return Pose {
                drawn: None,
                pieces: vec![PiecePose::AT_REST; self.starts.len()],
            };
        };
        let units = &content.units;
        // Units come in reading order without overlapping; look them up by
        // where they start, and one by one should that ever change.
        let sorted = units.is_sorted_by_key(|unit| unit.start);
        let covering = |at: usize| -> Option<&TextUnit> {
            if sorted {
                let after = units.partition_point(|unit| unit.start <= at);
                units[..after].last().filter(|unit| at < unit.end)
            } else {
                units.iter().find(|unit| unit.start <= at && at < unit.end)
            }
        };
        Pose {
            drawn: Some(Drawn {
                transform: node.transform,
                width: node.width,
                height: node.height,
            }),
            pieces: self
                .starts
                .iter()
                .map(|at| covering(*at).map_or(PiecePose::UNSEEN, PiecePose::of))
                .collect(),
        }
    }
}
