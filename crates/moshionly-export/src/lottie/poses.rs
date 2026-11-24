//! Poses: where a text layer's box is, and how each of its pieces is moved,
//! at a moment.
//!
//! A text that comes in letter by letter has a keyframed position, scale
//! and opacity for every letter, each fitted by looking at many moments:
//! every whole frame, eight moments a frame while the letter moves, and a
//! few dozen moments of its own around where it starts and stops. Each
//! moment is evaluated for the whole artboard, which shows every letter at
//! once, so what one evaluation finds is kept for the other letters:
//!
//! - Whole frames are looked at by every letter. They are evaluated once,
//!   in order, and each piece keeps its poses as runs of frames over which
//!   it does not change: a letter that holds still before and after it
//!   moves costs a few numbers, however long the text or the animation.
//! - Other moments are kept as rows of every piece's pose. The letters
//!   after the one being fitted look at much the same moments, so as many
//!   rows are kept as one letter looks at, within limits, and the rows
//!   looked at longest ago are forgotten first.
//! - The piece being fitted keeps its own poses at the moments it looked
//!   at, so the fits of its position, scale and opacity evaluate each
//!   moment once, even when not all rows can be kept.
//!
//! The whole frames and the rows take less than a gigabyte together,
//! whatever the text.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::mem::size_of;
use std::rc::Rc;

use moshionly_core::scene::{Content, TextUnit};
use moshionly_core::{Affine, NodeId};

use crate::progress::CancelToken;

use super::scenes::{Kept, Scenes};
use super::timeline::Timeline;

/// How much the runs of whole frames may take, in bytes. Whole frames
/// past it are treated like any other moment.
const WHOLE_BUDGET: usize = 384 << 20;
/// The least the rows of other moments may take, in bytes.
const ROWS_LEAST: usize = 64 << 20;
/// The most the rows of other moments may take, in bytes.
const ROWS_MOST: usize = 512 << 20;
/// Rows kept beyond the moments one piece looked at: room for the next
/// piece's own moments.
const ROWS_SPARE: usize = 256;
/// How much the text's box at other moments may take, in bytes.
const DRAWN_BUDGET: usize = 16 << 20;
/// How many whole frames are evaluated between looks at the cancellation
/// token.
const FRAMES_PER_CANCEL_CHECK: u64 = 16;

/// What a text layer looks like at one moment.
#[derive(Debug, Clone, PartialEq)]
struct Pose {
    /// The text node's box as the scene draws it, or `None` when the text
    /// is not drawn.
    drawn: Option<Drawn>,
    /// How each piece is moved, in the order the pieces were given.
    pieces: Vec<PiecePose>,
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

    /// Whether `other` is the same to the bit, so that either can stand
    /// for the other.
    fn same(&self, other: &PiecePose) -> bool {
        let bits = |pose: &PiecePose| {
            [
                pose.position[0].to_bits(),
                pose.position[1].to_bits(),
                pose.scale.to_bits(),
                pose.opacity.to_bits(),
                pose.blur.to_bits(),
            ]
        };
        bits(self) == bits(other)
    }
}

/// The poses of one text layer by frame, each moment worked out once.
pub(crate) struct Poses<'a> {
    scenes: &'a Scenes<'a>,
    cancel: &'a CancelToken,
    id: NodeId,
    /// Where each piece starts in the text, in bytes.
    starts: Vec<usize>,
    /// Whether `starts` is in reading order.
    starts_sorted: bool,
    /// The last whole frame of the export: its out point.
    last: u64,
    /// The poses at whole frames.
    whole: RefCell<Whole>,
    /// How much `whole` may take, in bytes.
    whole_budget: usize,
    /// The text's box at other moments.
    drawn: Kept<u64, Option<Drawn>>,
    /// Every piece's pose at other moments.
    rows: Kept<u64, Rc<[PiecePose]>>,
    /// The least and the most `rows` may take, in bytes.
    rows_budget: (usize, usize),
    /// The most moments other than whole frames that one piece has looked
    /// at.
    window: Cell<usize>,
    /// The piece being fitted, and its poses at the moments it looked at.
    current: RefCell<(usize, HashMap<u64, PiecePose>)>,
    /// How many moments have been evaluated.
    #[cfg(test)]
    evaluated: Cell<usize>,
}

/// The poses at whole frames, from frame 0 on, evaluated in order.
#[derive(Default)]
struct Whole {
    /// The next frame to evaluate.
    next: u64,
    /// Whether frames from `next` on are left to be evaluated like any
    /// other moment: the export was cancelled, or the runs took their
    /// budget.
    stopped: bool,
    /// The text node's box at each frame.
    drawn: Vec<Option<Drawn>>,
    /// Each piece's poses, as runs of frames over which it holds.
    runs: Vec<Vec<Run>>,
    /// What all this takes, in bytes.
    bytes: usize,
}

/// A piece's pose from a frame until the next run.
#[derive(Debug, Clone, Copy)]
struct Run {
    from: u64,
    pose: PiecePose,
}

impl Whole {
    /// Piece `index`'s pose at frame `at`, which must be before `next`.
    fn piece(&self, at: u64, index: usize) -> PiecePose {
        let runs = &self.runs[index];
        // Every piece has a run from frame 0.
        let after = runs.partition_point(|run| run.from <= at);
        runs[after - 1].pose
    }

    /// Adds frame `next`, posed as `pose`.
    fn push(&mut self, pose: Pose) {
        let frame = self.next;
        let grown = |before: usize, after: usize, size: usize| (after - before) * size;
        let capacity = self.drawn.capacity();
        self.drawn.push(pose.drawn);
        self.bytes += grown(capacity, self.drawn.capacity(), size_of::<Option<Drawn>>());
        if self.runs.is_empty() {
            self.runs = vec![Vec::new(); pose.pieces.len()];
            self.bytes += self.runs.len() * size_of::<Vec<Run>>();
        }
        for (runs, piece) in self.runs.iter_mut().zip(&pose.pieces) {
            if runs.last().is_none_or(|run| !run.pose.same(piece)) {
                let capacity = runs.capacity();
                runs.push(Run {
                    from: frame,
                    pose: *piece,
                });
                self.bytes += grown(capacity, runs.capacity(), size_of::<Run>());
            }
        }
        self.next += 1;
    }
}

impl<'a> Poses<'a> {
    /// The poses of the text layer `id`, for pieces starting at `starts`,
    /// from the scenes of `timeline`.
    pub fn new(timeline: Timeline<'a>, id: NodeId, starts: Vec<usize>) -> Self {
        Self {
            scenes: timeline.scenes,
            cancel: timeline.cancel,
            id,
            starts_sorted: starts.is_sorted(),
            starts,
            last: timeline.clock.frames,
            whole: RefCell::new(Whole::default()),
            whole_budget: WHOLE_BUDGET,
            drawn: Kept::new(),
            rows: Kept::new(),
            rows_budget: (ROWS_LEAST, ROWS_MOST),
            window: Cell::new(0),
            current: RefCell::new((0, HashMap::new())),
            #[cfg(test)]
            evaluated: Cell::new(0),
        }
    }

    /// The text node's box at frame `frame`, or `None` when the text is not
    /// drawn then.
    pub fn drawn(&self, frame: f64) -> Option<Drawn> {
        if let Some(at) = self.whole_frame(frame) {
            return self.whole.borrow().drawn[at as usize];
        }
        if let Some(drawn) = self.drawn.get(&frame.to_bits()) {
            return drawn;
        }
        self.evaluate(frame).0
    }

    /// How piece `index` is moved at frame `frame`.
    pub fn piece(&self, frame: f64, index: usize) -> PiecePose {
        if let Some(at) = self.whole_frame(frame) {
            return self.whole.borrow().piece(at, index);
        }
        let key = frame.to_bits();
        {
            let mut current = self.current.borrow_mut();
            if current.0 != index {
                *current = (index, HashMap::new());
            }
            if let Some(pose) = current.1.get(&key) {
                return *pose;
            }
        }
        let row = match self.rows.get(&key) {
            Some(row) => row,
            None => self.evaluate(frame).1,
        };
        let pose = row[index];
        let mut current = self.current.borrow_mut();
        current.1.insert(key, pose);
        self.window.set(self.window.get().max(current.1.len()));
        pose
    }

    /// The whole frame that `frame` is, when its poses are among the whole
    /// frames kept: evaluates the frames up to it that are not yet.
    fn whole_frame(&self, frame: f64) -> Option<u64> {
        if frame.fract() != 0.0 || !(0.0..=self.last as f64).contains(&frame) {
            return None;
        }
        let at = frame as u64;
        let mut whole = self.whole.borrow_mut();
        while whole.next <= at && !whole.stopped {
            let next = whole.next;
            if (next.is_multiple_of(FRAMES_PER_CANCEL_CHECK) && self.cancel.is_cancelled())
                || whole.bytes > self.whole_budget
            {
                whole.stopped = true;
                break;
            }
            whole.push(self.work_out(next as f64));
        }
        (at < whole.next).then_some(at)
    }

    /// Evaluates frame `frame` and keeps what it finds: the text's box and
    /// every piece's pose.
    fn evaluate(&self, frame: f64) -> (Option<Drawn>, Rc<[PiecePose]>) {
        let pose = self.work_out(frame);
        let key = frame.to_bits();
        let entry = size_of::<(u64, Option<Drawn>, usize, u64)>() + 16;
        self.drawn.insert(key, pose.drawn, entry, DRAWN_BUDGET);
        let row: Rc<[PiecePose]> = pose.pieces.into();
        let size = row.len() * size_of::<PiecePose>() + 64;
        // Room for every moment the piece that looked at most looked at,
        // after the quarter that is freed when the rows are full.
        let wanted = (self.window.get() + ROWS_SPARE).saturating_mul(size) / 3 * 4;
        let (least, most) = self.rows_budget;
        self.rows
            .insert(key, row.clone(), size, wanted.clamp(least, most));
        (pose.drawn, row)
    }

    fn work_out(&self, frame: f64) -> Pose {
        #[cfg(test)]
        self.evaluated.set(self.evaluated.get() + 1);
        let scene = self.scenes.evaluate(frame);
        let found = scene.as_ref().and_then(|scene| scene.find(&self.id));
        let Some((node, Content::Text(content))) = found.map(|node| (node, &node.content)) else {
            return Pose {
                drawn: None,
                pieces: vec![PiecePose::AT_REST; self.starts.len()],
            };
        };
        let units = &content.units;
        let drawn = Some(Drawn {
            transform: node.transform,
            width: node.width,
            height: node.height,
        });
        // Units come in reading order without overlapping, and so do the
        // pieces: walk both together, or look each piece up should that
        // ever change.
        let pieces = if self.starts_sorted && units.is_sorted_by_key(|unit| unit.start) {
            let mut after = 0;
            self.starts
                .iter()
                .map(|at| {
                    while after < units.len() && units[after].start <= *at {
                        after += 1;
                    }
                    units[..after]
                        .last()
                        .filter(|unit| *at < unit.end)
                        .map_or(PiecePose::UNSEEN, PiecePose::of)
                })
                .collect()
        } else {
            self.starts
                .iter()
                .map(|at| {
                    units
                        .iter()
                        .find(|unit| unit.start <= *at && *at < unit.end)
                        .map_or(PiecePose::UNSEEN, PiecePose::of)
                })
                .collect()
        };
        Pose { drawn, pieces }
    }
}

#[cfg(test)]
mod tests {
    use moshionly_core::{Document, MonospaceMeasurer, io};

    use super::super::clock::Clock;
    use super::*;

    const TEXT: &str = "Hello there";

    /// A text that moves while it comes in letter by letter, over most of a
    /// second at 30 frames a second.
    fn document() -> Document {
        io::parse(&format!(
            r##"{{"format":"moshionly","version":1,"name":"Test","nodes":[
            {{"id":"board","parent":null,"index":"a0","type":"artboard","name":"Board",
              "width":400,"height":300,"duration":1000,"fill":"#FFFFFF"}},
            {{"id":"title","parent":"board","index":"a0","type":"text","name":"Title",
              "x":20,"y":20,"width":300,"height":100,"content":"{TEXT}","font_size":20,
              "fill":"#111111"}},
            {{"id":"move","parent":"board","index":"a1","type":"move","target":"title",
              "start":100,"end":900,"to":[60,40]}},
            {{"id":"in","parent":"board","index":"a2","type":"text_in","target":"title",
              "start":0,"end":800,"effect":"slide_up","split":"letters","stagger":40}}
        ]}}"##
        ))
        .expect("the test document is valid")
    }

    const CLOCK: Clock = Clock {
        start_ms: 0.0,
        end_ms: 1000.0,
        fps: 30.0,
        frames: 30,
    };

    /// Runs `check` with the poses of the text's letters, which `cancel`
    /// cancels.
    fn with_poses(check: impl FnOnce(&mut Poses<'_>, &CancelToken)) {
        let document = document();
        let artboard = NodeId::from("board");
        let measurer = MonospaceMeasurer::default();
        let scenes = Scenes::new(&document, &artboard, &measurer, CLOCK);
        let cancel = CancelToken::new();
        let timeline = Timeline {
            document: &document,
            clock: CLOCK,
            scenes: &scenes,
            cancel: &cancel,
        };
        let starts: Vec<usize> = TEXT
            .char_indices()
            .filter(|(_, c)| !c.is_whitespace())
            .map(|(at, _)| at)
            .collect();
        let mut poses = Poses::new(timeline, NodeId::from("title"), starts);
        check(&mut poses, &cancel);
    }

    /// Asks for the box and the letters at whole frames, at moments
    /// between them and past the end, out of order and some twice, letter
    /// by letter and back, and compares them with fresh evaluations.
    fn assert_true_poses(poses: &Poses<'_>) {
        let mut moments: Vec<f64> = (0..=30).map(f64::from).collect();
        moments.extend((0..240).map(|step| f64::from(step) * 0.13));
        moments.extend([30.5, 31.0, 12.0, 0.0, 29.999999, 7.25]);
        moments.reverse();
        for (round, frame) in moments.into_iter().enumerate() {
            let truth = poses.work_out(frame);
            assert_eq!(poses.drawn(frame), truth.drawn, "box at {frame}");
            let index = (round / 7) % truth.pieces.len();
            for index in [index, index, (index + 1) % truth.pieces.len()] {
                assert_eq!(
                    poses.piece(frame, index),
                    truth.pieces[index],
                    "letter {index} at {frame}"
                );
            }
        }
    }

    #[test]
    fn kept_poses_are_the_true_poses() {
        with_poses(|poses, _| {
            assert!(poses.drawn(10.0).is_some(), "the text is drawn");
            assert_true_poses(poses);
        });
    }

    #[test]
    fn poses_stay_true_when_little_can_be_kept() {
        with_poses(|poses, _| {
            // Whole frames past the first, and every row, are forgotten as
            // soon as they are kept.
            poses.whole_budget = 1;
            poses.rows_budget = (0, 0);
            assert_true_poses(poses);
            assert_eq!(poses.whole.borrow().next, 1);
        });
    }

    #[test]
    fn whole_frames_are_evaluated_once_for_all_letters() {
        with_poses(|poses, _| {
            for index in 0..10 {
                for frame in 0..=30 {
                    poses.piece(f64::from(frame), index);
                }
            }
            assert_eq!(poses.evaluated.get(), 31);
            // Letters that hold still are kept as a few runs, not a pose
            // per frame.
            let whole = poses.whole.borrow();
            let runs: usize = whole.runs.iter().map(Vec::len).sum();
            assert!(runs < 10 * 31 / 2, "{runs} runs");
        });
    }

    #[test]
    fn moments_between_frames_are_evaluated_once() {
        let moments: Vec<f64> = (0..200).map(|step| 0.3 + f64::from(step) / 8.0).collect();
        with_poses(|poses, _| {
            // A letter's three fits, then the next letter's.
            for index in [0, 0, 0, 1, 1, 1] {
                for frame in &moments {
                    poses.piece(*frame, index);
                }
            }
            assert_eq!(poses.evaluated.get(), moments.len());
        });
        with_poses(|poses, _| {
            // With no rows kept, each letter still evaluates each of its
            // moments once.
            poses.rows_budget = (0, 0);
            for index in [0, 0, 0, 1, 1, 1] {
                for frame in &moments {
                    poses.piece(*frame, index);
                }
            }
            assert_eq!(poses.evaluated.get(), 2 * moments.len());
        });
    }

    #[test]
    fn rows_make_room_for_what_one_letter_looks_at() {
        let moments: Vec<f64> = (0..1900).map(|step| 0.3 + f64::from(step) / 64.0).collect();
        with_poses(|poses, _| {
            // Room for a few hundred rows at least; one letter looks at
            // more.
            let row = 10 * size_of::<PiecePose>() + 64;
            poses.rows_budget = (300 * row, 100_000 * row);
            for index in [0, 0, 1, 1, 2] {
                for frame in &moments {
                    poses.piece(*frame, index);
                }
            }
            assert_eq!(poses.evaluated.get(), moments.len());
        });
    }

    #[test]
    fn cancelling_stops_the_sweep_through_whole_frames() {
        with_poses(|poses, cancel| {
            cancel.cancel();
            let truth = poses.work_out(30.0);
            assert_eq!(poses.piece(30.0, 3), truth.pieces[3]);
            // The last frame alone, not every frame before it.
            assert_eq!(poses.evaluated.get(), 2);
        });
    }
}
