//! Time in a Lottie file: frames counted from the start of the export.
//!
//! Moshionly counts milliseconds from the start of the artboard. Lottie
//! counts frames at the composition's frame rate, and keyframes may sit
//! between frames, which players handle. Frame 0 is the start of the
//! exported range, so a range that starts later than the artboard plays
//! from its own start.

use crate::timing::FrameTimes;

/// Converts between artboard milliseconds and Lottie frames.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Clock {
    /// The artboard moment of frame 0, in milliseconds.
    pub start_ms: f64,
    /// The moment the export ends, in milliseconds.
    pub end_ms: f64,
    /// Frames per second.
    pub fps: f64,
    /// Frames in the export: the composition's out point.
    pub frames: u64,
}

impl Clock {
    /// The clock of an export that captures `times`.
    pub fn new(times: &FrameTimes) -> Self {
        Self {
            start_ms: times.start_ms(),
            end_ms: times.end_ms(),
            fps: times.rate().fps(),
            frames: times.len(),
        }
    }

    /// The frame at artboard moment `ms`.
    pub fn frame(&self, ms: f64) -> f64 {
        (ms - self.start_ms) * self.fps / 1000.0
    }

    /// The artboard moment of frame `frame`.
    pub fn ms(&self, frame: f64) -> f64 {
        self.start_ms + frame * 1000.0 / self.fps
    }

    /// The composition's out point: the frame after the last one shown.
    pub fn out_point(&self) -> f64 {
        self.frames as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timing::FrameRate;

    #[test]
    fn frames_count_from_the_start_of_the_range() {
        let times = FrameTimes::new(500.0, 1500.0, FrameRate::FPS_25).unwrap();
        let clock = Clock::new(&times);
        assert_eq!(clock.frame(500.0), 0.0);
        assert_eq!(clock.frame(540.0), 1.0);
        assert_eq!(clock.ms(25.0), 1500.0);
        assert_eq!(clock.out_point(), 25.0);
    }
}
