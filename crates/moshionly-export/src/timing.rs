//! Frame rates, and which moments of an artboard an export captures.
//!
//! # Which moments
//!
//! Frame `i` shows the artboard at `start + i * 1000 / fps` milliseconds.
//! Frames are taken from the start up to, **but not including**, the end:
//! each frame stands for the stretch of time until the next one, so `n`
//! frames last exactly `n / fps` seconds and a 4 second artboard at 30 fps
//! is 120 frames lasting 4 seconds.
//!
//! Leaving the end out is what makes a loop seamless. The moment at the end
//! of a looping animation is the moment at its start, so a frame there would
//! show the same picture twice in a row each time the loop comes round,
//! a visible stutter. The cost is that the very last instant is not shown on
//! its own; the last frame is one frame step before the end.
//!
//! When the length is not a whole number of frames, the last frame still
//! starts before the end, so the export is up to one frame longer than the
//! range rather than shorter: nothing of the animation is cut off.

use std::fmt;

use thiserror::Error;

/// Frames per second as an exact fraction, so that the television rates
/// 29.97 (30000/1001) and 23.976 (24000/1001) keep their timing over long
/// exports. Always in lowest terms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrameRate {
    numerator: u32,
    denominator: u32,
}

/// Why a frame rate or a time range cannot be used.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum TimingError {
    /// The frame rate is zero, negative, not a number or absurdly high.
    #[error(
        "{0} frames per second cannot be used; choose a rate above 0, for example 24, 30 or 60"
    )]
    BadFrameRate(f64),
    /// The range ends before it starts, or is not made of numbers.
    #[error(
        "the time range from {start_ms} ms to {end_ms} ms is empty; the end must come after \
         the start"
    )]
    EmptyRange {
        /// Where the range starts, in milliseconds.
        start_ms: f64,
        /// Where the range ends, in milliseconds.
        end_ms: f64,
    },
}

impl FrameRate {
    /// 12 frames per second: a deliberately choppy, hand-made look.
    pub const FPS_12: FrameRate = FrameRate::whole(12);
    /// 15 frames per second: small GIFs.
    pub const FPS_15: FrameRate = FrameRate::whole(15);
    /// 20 frames per second: smooth GIFs.
    pub const FPS_20: FrameRate = FrameRate::whole(20);
    /// 23.976 (24000/1001) frames per second: film shown on NTSC television.
    pub const FPS_23_976: FrameRate = FrameRate::exact(24_000, 1001);
    /// 24 frames per second: film.
    pub const FPS_24: FrameRate = FrameRate::whole(24);
    /// 25 frames per second: PAL television, and the usual rate for GIFs.
    pub const FPS_25: FrameRate = FrameRate::whole(25);
    /// 29.97 (30000/1001) frames per second: NTSC television.
    pub const FPS_29_97: FrameRate = FrameRate::exact(30_000, 1001);
    /// 30 frames per second: the usual rate for video on the web.
    pub const FPS_30: FrameRate = FrameRate::whole(30);
    /// 50 frames per second: the smoothest GIFs.
    pub const FPS_50: FrameRate = FrameRate::whole(50);
    /// 59.94 (60000/1001) frames per second: NTSC high frame rate.
    pub const FPS_59_94: FrameRate = FrameRate::exact(60_000, 1001);
    /// 60 frames per second: smooth motion on screens.
    pub const FPS_60: FrameRate = FrameRate::whole(60);
    /// 120 frames per second: high refresh rate screens and slow motion.
    pub const FPS_120: FrameRate = FrameRate::whole(120);

    /// The common rates, slowest first.
    pub const PRESETS: [FrameRate; 12] = [
        FrameRate::FPS_12,
        FrameRate::FPS_15,
        FrameRate::FPS_20,
        FrameRate::FPS_23_976,
        FrameRate::FPS_24,
        FrameRate::FPS_25,
        FrameRate::FPS_29_97,
        FrameRate::FPS_30,
        FrameRate::FPS_50,
        FrameRate::FPS_59_94,
        FrameRate::FPS_60,
        FrameRate::FPS_120,
    ];

    /// The highest rate that can be described: one frame a millisecond.
    pub const MAX_FPS: f64 = 1000.0;

    const fn whole(fps: u32) -> Self {
        Self::exact(fps, 1)
    }

    /// For constants that are already in lowest terms.
    const fn exact(numerator: u32, denominator: u32) -> Self {
        Self {
            numerator,
            denominator,
        }
    }

    /// `numerator / denominator` frames per second, for example
    /// `FrameRate::new(30_000, 1001)` for 29.97.
    pub fn new(numerator: u32, denominator: u32) -> Result<Self, TimingError> {
        let fps = f64::from(numerator) / f64::from(denominator);
        if numerator == 0 || denominator == 0 || fps > Self::MAX_FPS {
            return Err(TimingError::BadFrameRate(fps));
        }
        let divisor = gcd(numerator, denominator);
        Ok(Self::exact(numerator / divisor, denominator / divisor))
    }

    /// A rate given as a decimal number. The television rates written
    /// with two or three decimals (23.976, 29.97, 59.94, 119.88 and their
    /// longer spellings) become their exact `/1001` fractions; anything else
    /// is kept to a thousandth of a frame per second.
    pub fn from_fps(fps: f64) -> Result<Self, TimingError> {
        if !(fps.is_finite() && fps > 0.0 && fps <= Self::MAX_FPS) {
            return Err(TimingError::BadFrameRate(fps));
        }
        for base in [24_u32, 30, 48, 60, 120] {
            let television = f64::from(base) * 1000.0 / 1001.0;
            if (fps - television).abs() < 0.006 {
                return Self::new(base * 1000, 1001);
            }
        }
        let thousandths = (fps * 1000.0).round();
        if thousandths < 1.0 {
            return Err(TimingError::BadFrameRate(fps));
        }
        Self::new(thousandths as u32, 1000)
    }

    /// The top of the fraction.
    pub fn numerator(self) -> u32 {
        self.numerator
    }

    /// The bottom of the fraction.
    pub fn denominator(self) -> u32 {
        self.denominator
    }

    /// Frames per second as a decimal number.
    pub fn fps(self) -> f64 {
        f64::from(self.numerator) / f64::from(self.denominator)
    }

    /// How long one frame lasts, in milliseconds.
    pub fn frame_ms(self) -> f64 {
        1000.0 * f64::from(self.denominator) / f64::from(self.numerator)
    }

    /// When frame `index` starts, in milliseconds from frame 0.
    ///
    /// Computed from the index each time rather than by adding up frame
    /// lengths, so it does not drift.
    pub fn time_of(self, index: u64) -> f64 {
        (index as f64 * 1000.0 * f64::from(self.denominator)) / f64::from(self.numerator)
    }

    /// How many frames it takes to cover `span_ms` milliseconds: every
    /// frame that starts before the end. At least 1.
    pub fn frames_in(self, span_ms: f64) -> u64 {
        let frames =
            span_ms.max(0.0) * f64::from(self.numerator) / (1000.0 * f64::from(self.denominator));
        // A span that is a whole number of frames, give or take rounding in
        // the last bits, is that many frames and not one more.
        let whole = frames.round();
        let count = if (frames - whole).abs() < 1e-9 * whole.max(1.0) {
            whole
        } else {
            frames.ceil()
        };
        (count as u64).max(1)
    }
}

impl Default for FrameRate {
    /// 30 frames per second.
    fn default() -> Self {
        Self::FPS_30
    }
}

impl fmt::Display for FrameRate {
    /// `30 fps`, or `29.97 fps` for fractional rates.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.denominator == 1 {
            write!(f, "{} fps", self.numerator)
        } else {
            let text = format!("{:.3}", self.fps());
            let text = text.trim_end_matches('0').trim_end_matches('.');
            write!(f, "{text} fps")
        }
    }
}

fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// The moments an animated export captures: a frame every `1000 / fps`
/// milliseconds from `start_ms`, for every frame that starts before
/// `end_ms`. See the [module docs](self) for why the end is left out.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameTimes {
    start_ms: f64,
    end_ms: f64,
    rate: FrameRate,
    count: u64,
}

impl FrameTimes {
    /// The frames from `start_ms` up to `end_ms`, both in milliseconds from
    /// the start of the artboard.
    pub fn new(start_ms: f64, end_ms: f64, rate: FrameRate) -> Result<Self, TimingError> {
        if !(start_ms.is_finite() && end_ms.is_finite() && end_ms > start_ms) {
            return Err(TimingError::EmptyRange { start_ms, end_ms });
        }
        Ok(Self {
            start_ms,
            end_ms,
            rate,
            count: rate.frames_in(end_ms - start_ms),
        })
    }

    /// The frames of a whole artboard that lasts `duration_ms`.
    pub fn whole(duration_ms: f64, rate: FrameRate) -> Result<Self, TimingError> {
        Self::new(0.0, duration_ms, rate)
    }

    /// The number of frames. At least 1.
    pub fn len(&self) -> u64 {
        self.count
    }

    /// Always false: there is at least one frame.
    pub fn is_empty(&self) -> bool {
        false
    }

    /// The frame rate.
    pub fn rate(&self) -> FrameRate {
        self.rate
    }

    /// Where the range starts, in milliseconds from the start of the artboard.
    pub fn start_ms(&self) -> f64 {
        self.start_ms
    }

    /// Where the range ends, in milliseconds from the start of the artboard.
    pub fn end_ms(&self) -> f64 {
        self.end_ms
    }

    /// The moment frame `index` shows, in milliseconds from the start of
    /// the artboard. `None` past the last frame.
    pub fn time(&self, index: u64) -> Option<f64> {
        (index < self.count).then(|| self.start_ms + self.rate.time_of(index))
    }

    /// Every moment, in order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = f64> + '_ {
        (0..self.count as usize).map(|index| self.start_ms + self.rate.time_of(index as u64))
    }

    /// How long the frames play for, in milliseconds: the number of frames
    /// times the length of one.
    pub fn duration_ms(&self) -> f64 {
        self.rate.time_of(self.count)
    }

    /// The display time of each frame in hundredths of a second, the unit
    /// GIF stores. Rounding each frame on its own would make a 30 fps GIF
    /// run at 33.3 fps; instead every frame ends at the hundredth nearest
    /// its true end, so the error never builds up and the delays add up to
    /// the length of the whole export, rounded to a hundredth.
    pub fn centisecond_delays(&self) -> impl ExactSizeIterator<Item = u32> + '_ {
        let end_of = move |frames: u64| {
            let hundredths = frames as f64 * 100.0 * f64::from(self.rate.denominator)
                / f64::from(self.rate.numerator);
            hundredths.round() as u64
        };
        (0..self.count as usize).map(move |index| {
            let index = index as u64;
            (end_of(index + 1) - end_of(index)) as u32
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rates_are_exact_fractions() {
        assert_eq!(FrameRate::FPS_30.fps(), 30.0);
        let ntsc = FrameRate::from_fps(29.97).unwrap();
        assert_eq!(ntsc, FrameRate::FPS_29_97);
        assert_eq!((ntsc.numerator(), ntsc.denominator()), (30_000, 1001));
        assert_eq!(FrameRate::from_fps(23.976).unwrap(), FrameRate::FPS_23_976);
        assert_eq!(FrameRate::from_fps(23.98).unwrap(), FrameRate::FPS_23_976);
        assert_eq!(FrameRate::from_fps(59.94).unwrap(), FrameRate::FPS_59_94);
        assert_eq!(FrameRate::from_fps(30.0).unwrap(), FrameRate::FPS_30);
        assert_eq!(
            FrameRate::from_fps(12.5).unwrap(),
            FrameRate::new(25, 2).unwrap()
        );
        assert_eq!(FrameRate::new(60, 2).unwrap(), FrameRate::FPS_30);
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY, 1e6, 0.0001] {
            assert!(FrameRate::from_fps(bad).is_err(), "{bad}");
        }
        assert!(FrameRate::new(30, 0).is_err());
        assert_eq!(FrameRate::FPS_29_97.to_string(), "29.97 fps");
        assert_eq!(FrameRate::FPS_23_976.to_string(), "23.976 fps");
        assert_eq!(FrameRate::FPS_24.to_string(), "24 fps");
        assert!(
            FrameRate::PRESETS
                .windows(2)
                .all(|w| w[0].fps() < w[1].fps())
        );
    }

    #[test]
    fn frames_cover_the_range_without_the_end() {
        let times = FrameTimes::whole(4000.0, FrameRate::FPS_30).unwrap();
        assert_eq!(times.len(), 120);
        assert_eq!(times.time(0), Some(0.0));
        assert_eq!(times.time(3), Some(100.0));
        assert_eq!(times.time(119), Some(3966.6666666666665));
        assert_eq!(times.time(120), None);
        assert_eq!(times.duration_ms(), 4000.0);
        assert_eq!(times.iter().len(), 120);
        assert!(times.iter().all(|time| time < 4000.0));

        // A length that is not a whole number of frames gets one more
        // frame, which still starts before the end.
        let times = FrameTimes::whole(1010.0, FrameRate::FPS_30).unwrap();
        assert_eq!(times.len(), 31);
        assert_eq!(times.time(30), Some(1000.0));

        // Never no frames at all.
        let times = FrameTimes::whole(1.0, FrameRate::FPS_30).unwrap();
        assert_eq!(times.len(), 1);
    }

    #[test]
    fn television_rates_do_not_drift() {
        let times = FrameTimes::whole(1000.0, FrameRate::FPS_29_97).unwrap();
        // 29.97 frames fit in a second, so the 30th starts just before it.
        assert_eq!(times.len(), 30);
        assert!((times.time(29).unwrap() - 967.633).abs() < 0.001);
        // After ten minutes the frame times are still exact.
        let long = FrameTimes::whole(600_600.0, FrameRate::FPS_29_97).unwrap();
        assert_eq!(long.len(), 18_000);
        assert!((long.time(17_999).unwrap() - 600_566.633_333).abs() < 1e-6);
        assert_eq!(long.duration_ms(), 600_600.0);
        let film = FrameTimes::whole(1001.0, FrameRate::FPS_23_976).unwrap();
        assert_eq!(film.len(), 24);
        assert_eq!(film.duration_ms(), 1001.0);
    }

    #[test]
    fn ranges_start_and_end_where_asked() {
        let times = FrameTimes::new(500.0, 1500.0, FrameRate::FPS_25).unwrap();
        assert_eq!(times.len(), 25);
        assert_eq!(times.time(0), Some(500.0));
        assert_eq!(times.time(1), Some(540.0));
        assert_eq!(times.time(24), Some(1460.0));
        assert_eq!((times.start_ms(), times.end_ms()), (500.0, 1500.0));
        let ntsc = FrameTimes::new(1000.0, 2001.0, FrameRate::FPS_29_97).unwrap();
        assert_eq!(ntsc.len(), 30);
        assert_eq!(ntsc.time(0), Some(1000.0));
        assert!((ntsc.time(1).unwrap() - 1033.3667).abs() < 0.001);

        for (start, end) in [
            (10.0, 10.0),
            (10.0, 5.0),
            (f64::NAN, 10.0),
            (0.0, f64::INFINITY),
        ] {
            assert!(FrameTimes::new(start, end, FrameRate::FPS_30).is_err());
        }
    }

    #[test]
    fn gif_delays_carry_their_rounding() {
        // 30 fps is 3.33 hundredths a frame: 3, 4, 3, 3, 4, 3 ...
        let times = FrameTimes::whole(1000.0, FrameRate::FPS_30).unwrap();
        let delays: Vec<u32> = times.centisecond_delays().collect();
        assert_eq!(delays.len(), 30);
        assert_eq!(delays.iter().sum::<u32>(), 100);
        assert!(delays.iter().all(|delay| (3..=4).contains(delay)));
        assert_eq!(&delays[..6], [3, 4, 3, 3, 4, 3]);

        // 25 fps is exactly 4 hundredths.
        let times = FrameTimes::whole(4000.0, FrameRate::FPS_25).unwrap();
        assert!(times.centisecond_delays().all(|delay| delay == 4));
        assert_eq!(times.centisecond_delays().sum::<u32>(), 400);

        // 29.97 fps over ten seconds is 10.01 seconds of frames.
        let times = FrameTimes::whole(10_000.0, FrameRate::FPS_29_97).unwrap();
        assert_eq!(times.len(), 300);
        assert_eq!(times.centisecond_delays().sum::<u32>(), 1001);
    }
}
