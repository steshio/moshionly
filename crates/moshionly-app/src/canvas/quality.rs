//! How sharp the canvas is drawn while the animation plays.
//!
//! A picture of an artboard takes a while to render, and while playing a
//! new one is wanted every frame. The renderer never queues more than one
//! picture per artboard, so when it cannot keep up frames are dropped
//! rather than shown late. To drop fewer, [`Quality`] watches how long
//! pictures take and lowers the resolution step by step while they take
//! longer than a frame should, and raises it again when there is time to
//! spare. When playback stops the canvas goes back to full resolution.

use std::time::Duration;

/// The resolutions tried while playing, as fractions of the full one,
/// sharpest first. Each step has about half the pixels of the one before.
pub const LEVELS: [f32; 5] = [1.0, 0.71, 0.5, 0.35, 0.25];

/// How long one picture may take while playing: a frame at 30 frames per
/// second.
pub const BUDGET: Duration = Duration::from_micros(33_333);

/// The resolution of the canvas while playing.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Quality {
    /// Index into [`LEVELS`].
    level: usize,
    /// The recent render time, smoothed, in seconds.
    average: Option<f64>,
    /// How many renders had finished when they were last looked at.
    seen: u64,
    /// Renders still to leave out after a change of level: they were asked
    /// for at the level before.
    settling: u8,
}

/// How many renders after a change of level are left out of the measure:
/// as many as can be under way at the old level.
const SETTLE: u8 = 3;

impl Quality {
    /// Full resolution, with nothing measured yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// The fraction of the full resolution to render at.
    pub fn factor(&self) -> f32 {
        LEVELS[self.level]
    }

    /// Takes in the time of a render that finished, at the current level,
    /// and moves one level down when renders take longer than [`BUDGET`],
    /// or one up when even the sharper level would fit with room to spare.
    ///
    /// The few renders that finish just after a change of level were asked
    /// for before it, so they are left out.
    pub fn observe(&mut self, elapsed: Duration) {
        if self.settling > 0 {
            self.settling -= 1;
            return;
        }
        let sample = elapsed.as_secs_f64();
        let average = match self.average {
            Some(average) => average * 0.7 + sample * 0.3,
            None => sample,
        };
        self.average = Some(average);
        let budget = BUDGET.as_secs_f64();
        if average > budget && self.level + 1 < LEVELS.len() {
            self.level += 1;
            // The next pictures are smaller: start measuring them afresh.
            self.average = None;
            self.settling = SETTLE;
        } else if self.level > 0 {
            // Pixels, and so time, grow with the square of the resolution.
            let sharper = f64::from(LEVELS[self.level - 1] / LEVELS[self.level]).powi(2);
            if average * sharper < budget * 0.6 {
                self.level -= 1;
                self.average = None;
                self.settling = SETTLE;
            }
        }
    }

    /// Takes in the renders that finished since the last call, given the
    /// renderer's count of finished renders and the time of the latest.
    pub fn observe_renders(&mut self, finished: u64, latest: Option<Duration>) {
        if finished > self.seen
            && let Some(latest) = latest
        {
            self.observe(latest);
        }
        self.seen = finished;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    /// Lets the renders asked for before a change of level finish.
    fn settle(quality: &mut Quality) {
        for _ in 0..SETTLE {
            quality.observe(ms(1000));
        }
    }

    #[test]
    fn slow_renders_lower_the_resolution_step_by_step() {
        let mut quality = Quality::new();
        assert_eq!(quality.factor(), 1.0);
        quality.observe(ms(80));
        assert_eq!(quality.factor(), 0.71);
        // Renders asked for at full size are still finishing: ignored.
        settle(&mut quality);
        assert_eq!(quality.factor(), 0.71);
        quality.observe(ms(50));
        assert_eq!(quality.factor(), 0.5);
        // It never goes below the last level.
        for _ in 0..20 {
            quality.observe(ms(200));
        }
        assert_eq!(quality.factor(), 0.25);
    }

    #[test]
    fn quick_renders_raise_it_again_without_flickering() {
        let mut quality = Quality::new();
        quality.observe(ms(80));
        assert_eq!(quality.factor(), 0.71);
        settle(&mut quality);
        // 15 ms at 0.71 is about 30 ms at full size: too close to the
        // budget to go back up.
        quality.observe(ms(15));
        assert_eq!(quality.factor(), 0.71);
        // 8 ms would be 16 ms at full size: once the average says so,
        // there is room.
        for _ in 0..5 {
            quality.observe(ms(8));
        }
        assert_eq!(quality.factor(), 1.0);
        // Renders that fit stay at full resolution.
        for _ in 0..10 {
            quality.observe(ms(12));
        }
        assert_eq!(quality.factor(), 1.0);
    }

    #[test]
    fn only_new_renders_count() {
        let mut quality = Quality::new();
        quality.observe_renders(3, Some(ms(90)));
        assert_eq!(quality.factor(), 0.71);
        // Nothing new finished: the same slow render is not counted twice.
        quality.observe_renders(3, Some(ms(90)));
        assert_eq!(quality.factor(), 0.71);
        quality.observe_renders(4, None);
        assert_eq!(quality.factor(), 0.71);
    }
}
