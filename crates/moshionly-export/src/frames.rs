//! Turning moments into rendered frames, on every core, in order.
//!
//! Each frame is a pure function of the document and its moment, so frames
//! can be drawn in any order on any thread and still come out the same.
//! [`render_in_order`] draws them on a few worker threads and hands them to
//! the encoder strictly in order. Workers may only run a bounded number of
//! frames ahead of the encoder, so memory stays bounded however long the
//! export is.

use std::collections::BTreeMap;
use std::num::NonZeroUsize;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Condvar, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use moshionly_core::{Document, NodeId, Scene, evaluate, evaluate_at_rest};
use moshionly_render::{Frame, RenderOptions, RenderWarning, Renderer};

use crate::error::ExportError;
use crate::job::Reporter;
use crate::progress::Phase;
use crate::settings::{Moment, Moments};

/// Renders frames of one artboard. Cheap to copy and safe to share between
/// threads: each frame is a pure function of the document and its moment.
#[derive(Clone, Copy)]
pub(crate) struct FrameSource<'a> {
    /// The document.
    pub document: &'a Document,
    /// The artboard drawn.
    pub artboard: &'a NodeId,
    /// Draws the scenes; its fonts also lay text out.
    pub renderer: &'a Renderer,
}

impl FrameSource<'_> {
    /// The artboard at `moment`.
    pub fn scene(&self, moment: Moment) -> Result<Scene, ExportError> {
        let fonts = self.renderer.fonts();
        Ok(match moment {
            Moment::AtRest => evaluate_at_rest(self.document, self.artboard, fonts)?,
            Moment::At(time_ms) => evaluate(self.document, self.artboard, time_ms, fonts)?,
        })
    }

    /// Frame `index`, which shows the artboard at `moment`.
    pub fn render(
        &self,
        index: u64,
        moment: Moment,
        options: &RenderOptions,
    ) -> Result<Frame, ExportError> {
        let scene = self.scene(moment)?;
        self.renderer
            .render(&scene, options)
            .map_err(|source| ExportError::Render {
                frame: index,
                source,
            })
    }
}

/// How much memory frames waiting to be encoded may take: 512 MB.
const MEMORY_BUDGET: usize = 512 << 20;

/// The most worker threads used, whatever the number of cores.
const MAX_WORKERS: usize = 16;

/// One run of frames through the workers.
pub(crate) struct Pass {
    /// The moment of each frame, in order. Each is worked out when its
    /// frame is drawn, so no list of every frame is ever made.
    pub moments: Moments,
    /// How each frame is rendered.
    pub options: RenderOptions,
    /// Roughly how many bytes one rendered frame and its prepared form
    /// take, to bound memory.
    pub frame_bytes: usize,
    /// Whether to report each frame handed over as rendering progress.
    /// Off for a pass that only looks at the animation, such as choosing a
    /// GIF's colours.
    pub report_progress: bool,
}

/// Renders every frame of `pass` and hands it to `consume` in order.
///
/// `prepare` runs on the worker that rendered the frame, so per-frame work
/// such as compressing it or converting its colours uses every core too.
/// `consume` runs on the calling thread, once per frame, strictly in order.
///
/// Stops at the first error, from rendering, `prepare` or `consume`, and
/// soon after the export is cancelled, without waiting for frames that are
/// not yet needed.
pub(crate) fn render_in_order<T: Send>(
    source: FrameSource<'_>,
    pass: &Pass,
    reporter: &mut Reporter<'_>,
    prepare: &(dyn Fn(u64, Frame) -> Result<T, ExportError> + Sync),
    consume: &mut dyn FnMut(u64, T) -> Result<(), ExportError>,
) -> Result<(), ExportError> {
    let count = pass.moments.len();
    let cores = thread::available_parallelism().map_or(4, NonZeroUsize::get);
    let window = (MEMORY_BUDGET / pass.frame_bytes.max(1)).clamp(2, 2 * cores.max(1));
    let workers = cores
        .clamp(1, MAX_WORKERS)
        .min(window)
        .min(usize::try_from(count).unwrap_or(usize::MAX))
        .max(1);
    let gate = Gate::new(window as u64);
    let cancel = reporter.cancel_token().clone();

    thread::scope(|scope| {
        let (sender, receiver) = mpsc::channel();
        for _ in 0..workers {
            let sender = sender.clone();
            let (gate, cancel) = (&gate, &cancel);
            scope.spawn(move || {
                while let Some(index) = gate.claim(count) {
                    if cancel.is_cancelled() {
                        gate.stop();
                        break;
                    }
                    // Never past the end: claims stop at the count.
                    let Some(moment) = pass.moments.get(index) else {
                        break;
                    };
                    let result = source
                        .render(index, moment, &pass.options)
                        .and_then(|frame| {
                            let warnings = frame.warnings().to_vec();
                            prepare(index, frame).map(|value| (value, warnings))
                        });
                    if sender.send((index, result)).is_err() {
                        break;
                    }
                }
            });
        }
        drop(sender);

        let mut hand_over = || -> Result<(), ExportError> {
            let mut waiting: BTreeMap<u64, (T, Vec<RenderWarning>)> = BTreeMap::new();
            let mut next = 0;
            while next < count {
                reporter.check()?;
                match receiver.recv_timeout(Duration::from_millis(50)) {
                    Ok((index, result)) => {
                        waiting.insert(index, result?);
                    }
                    Err(RecvTimeoutError::Timeout) => continue,
                    // Every worker stopped before the last frame, which only
                    // happens when the export is cancelled.
                    Err(RecvTimeoutError::Disconnected) => {
                        reporter.check()?;
                        return Err(ExportError::Cancelled);
                    }
                }
                while let Some((value, warnings)) = waiting.remove(&next) {
                    reporter.warn(&warnings);
                    consume(next, value)?;
                    next += 1;
                    gate.advance(next);
                    if pass.report_progress {
                        reporter.report(Phase::Rendering, next);
                    }
                }
            }
            Ok(())
        };
        let outcome = hand_over();
        // Wakes workers waiting for room so they see there is nothing
        // more to do; the scope then waits for them to finish.
        gate.stop();
        outcome
    })
}

/// Hands out frame numbers to workers, no further ahead of the encoder
/// than the window allows.
struct Gate {
    state: Mutex<GateState>,
    changed: Condvar,
    window: u64,
}

struct GateState {
    /// The next frame to hand out.
    next: u64,
    /// Frames the encoder has taken.
    consumed: u64,
    /// No more frames are wanted.
    stopped: bool,
}

impl Gate {
    fn new(window: u64) -> Self {
        Self {
            state: Mutex::new(GateState {
                next: 0,
                consumed: 0,
                stopped: false,
            }),
            changed: Condvar::new(),
            window,
        }
    }

    /// The next frame to render, waiting while the workers are a whole
    /// window ahead of the encoder. `None` when there is nothing more to do.
    fn claim(&self, count: u64) -> Option<u64> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            if state.stopped || state.next >= count {
                return None;
            }
            if state.next < state.consumed + self.window {
                state.next += 1;
                return Some(state.next - 1);
            }
            state = self
                .changed
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// The encoder has taken `consumed` frames.
    fn advance(&self, consumed: u64) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.consumed = consumed;
        self.changed.notify_all();
    }

    /// No more frames are wanted.
    fn stop(&self) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.stopped = true;
        self.changed.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_gate_keeps_workers_within_the_window() {
        let gate = Gate::new(2);
        assert_eq!(gate.claim(10), Some(0));
        assert_eq!(gate.claim(10), Some(1));
        // A third claim would wait; free a slot from another thread first.
        thread::scope(|scope| {
            scope.spawn(|| {
                thread::sleep(Duration::from_millis(20));
                gate.advance(1);
            });
            assert_eq!(gate.claim(10), Some(2));
        });
        gate.advance(3);
        assert_eq!(gate.claim(4), Some(3));
        assert_eq!(gate.claim(4), None, "all frames handed out");
        let stopped = Gate::new(2);
        stopped.stop();
        assert_eq!(stopped.claim(10), None);
    }
}
