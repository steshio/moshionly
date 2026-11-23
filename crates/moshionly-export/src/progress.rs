//! How far an export has come, and how to stop it.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// What an export is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Phase {
    /// Checking the destination, starting encoders, choosing a GIF's
    /// colours.
    Preparing,
    /// Drawing frames and handing them to the encoder, in order.
    Rendering,
    /// Every frame is drawn; the encoder is finishing its work.
    Encoding,
    /// Writing the last parts of the file and moving it into place.
    Finishing,
}

/// How far an export has come.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    /// What the export is doing.
    pub phase: Phase,
    /// Frames handed to the encoder so far.
    pub frames_done: u64,
    /// Frames there will be in all.
    pub frames_total: u64,
}

impl Progress {
    /// The part of the frames done, from 0 to 1.
    pub fn fraction(&self) -> f64 {
        if self.frames_total == 0 {
            return 0.0;
        }
        (self.frames_done as f64 / self.frames_total as f64).clamp(0.0, 1.0)
    }
}

/// Stops an export from another thread.
///
/// Clones share one flag: give the export one clone and keep another to call
/// [`cancel`](Self::cancel) on. The export notices within a frame or so,
/// stops its encoder, removes what it had written and returns
/// [`ExportError::Cancelled`](crate::ExportError::Cancelled).
#[derive(Debug, Clone, Default)]
pub struct CancelToken {
    cancelled: Arc<AtomicBool>,
}

impl CancelToken {
    /// A token that has not been cancelled.
    pub fn new() -> Self {
        Self::default()
    }

    /// Asks the export to stop.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    /// Whether [`cancel`](Self::cancel) has been called on this token or a
    /// clone of it.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clones_share_the_flag() {
        let token = CancelToken::new();
        let other = token.clone();
        assert!(!other.is_cancelled());
        token.cancel();
        assert!(other.is_cancelled());
    }

    #[test]
    fn fractions_stay_between_zero_and_one() {
        let progress = |done, total| Progress {
            phase: Phase::Rendering,
            frames_done: done,
            frames_total: total,
        };
        assert_eq!(progress(0, 0).fraction(), 0.0);
        assert_eq!(progress(30, 120).fraction(), 0.25);
        assert_eq!(progress(130, 120).fraction(), 1.0);
    }
}
