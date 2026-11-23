//! Playing a scene in real time.
//!
//! [`Playback`] ties the playhead to a clock. When play is pressed it notes
//! the moment and the time in the scene, and every frame after that the
//! playhead is worked out from how long ago that was. Nothing is added up
//! frame by frame, so a slow frame never makes playback lag behind the
//! clock and many fast ones never make it run ahead. The clock is any
//! number of seconds that goes up (the window's own time in the app, a
//! made-up one in tests).

/// The frame rate the frame-by-frame steps use, in frames per second.
pub const FRAME_RATE: f64 = 60.0;

/// The playhead driven by a clock.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Playback {
    /// While playing: the clock reading, in seconds, at which the playhead
    /// was at the scene time, in milliseconds.
    anchor: Option<(f64, f64)>,
    /// The time handed out last, to notice the playhead being moved by
    /// something else.
    last: f64,
}

/// Where the playhead is after a [`Playback::tick`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tick {
    /// The scene time to show, in milliseconds.
    pub time: f64,
    /// Whether playback goes on.
    pub playing: bool,
}

impl Playback {
    /// Stopped.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether it is playing.
    pub fn is_playing(&self) -> bool {
        self.anchor.is_some()
    }

    /// Starts playing at the clock reading `now` from the scene time
    /// `time`. At the end of a scene `duration` long it starts over from
    /// the beginning. Returns the time it plays from.
    pub fn play(&mut self, now: f64, time: f64, duration: f64) -> f64 {
        let from = if duration > 0.0 && time >= duration {
            0.0
        } else {
            time.max(0.0)
        };
        self.anchor = Some((now, from));
        self.last = from;
        from
    }

    /// Stops where it is.
    pub fn pause(&mut self) {
        self.anchor = None;
    }

    /// The playhead at the clock reading `now`, for a scene `duration` long.
    ///
    /// `playhead` is where the playhead is now: when something else moved it
    /// since the last tick (a click in the ruler while playing), playback
    /// carries on from there. Past the end, a loop starts over and keeps
    /// time with the clock; otherwise playback stops at the end.
    pub fn tick(&mut self, now: f64, playhead: f64, duration: f64, looping: bool) -> Tick {
        let Some((mut since, mut from)) = self.anchor else {
            return Tick {
                time: playhead,
                playing: false,
            };
        };
        if playhead != self.last {
            (since, from) = (now, playhead.max(0.0));
        }
        if duration <= 0.0 {
            self.anchor = None;
            return Tick {
                time: 0.0,
                playing: false,
            };
        }
        let mut time = from + (now - since).max(0.0) * 1000.0;
        if time >= duration {
            if looping {
                time %= duration;
                // Start counting again from the loop's start, so the phase
                // stays tied to the clock.
                (since, from) = (now - time / 1000.0, 0.0);
            } else {
                self.anchor = None;
                self.last = duration;
                return Tick {
                    time: duration,
                    playing: false,
                };
            }
        }
        self.anchor = Some((since, from));
        self.last = time;
        Tick {
            time,
            playing: true,
        }
    }
}

/// How near a time must be to a frame, as a fraction of a frame, to count
/// as on it. The times [`step_frames`] hands out are rounded to a
/// thousandth of a millisecond, which puts them up to about 3e-5 of a frame
/// either side of the frame they stand for; this is well above that and
/// well below anything a person could place by hand.
const ON_FRAME: f64 = 1e-3;

/// The time `frames` frames (at [`FRAME_RATE`]) after `time`, or before it
/// for a negative count, within a scene `duration` long. Steps land on
/// frame boundaries: one step from between two frames goes to the next
/// frame in that direction.
pub fn step_frames(time: f64, frames: i32, duration: f64) -> f64 {
    let frame = 1000.0 / FRAME_RATE;
    // Work in whole frames. A time on a frame (give or take the rounding
    // below) counts from that frame; one between two frames counts from
    // the frame behind it in the direction of the step, so one step
    // reaches the next frame that way.
    let here = time / frame;
    let nearest = here.round();
    let from = if (here - nearest).abs() < ON_FRAME {
        nearest
    } else if frames >= 0 {
        here.floor()
    } else {
        here.ceil()
    };
    let target = from + f64::from(frames);
    // Frame boundaries are fractions of a millisecond; keep them tidy.
    let there = (target * frame * 1000.0).round() / 1000.0;
    there.clamp(0.0, duration.max(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_playhead_follows_the_clock_without_drifting() {
        let mut playback = Playback::new();
        assert_eq!(playback.play(10.0, 500.0, 4000.0), 500.0);
        let mut time = 500.0;
        // Uneven frames: a slow one among quick ones.
        for now in [10.016, 10.033, 10.2, 10.216] {
            let tick = playback.tick(now, time, 4000.0, false);
            assert!(tick.playing);
            time = tick.time;
        }
        // 216 milliseconds after pressing play, whatever the frames were.
        assert!((time - 716.0).abs() < 1e-6, "{time}");
        // A thousand tiny frames still land exactly on the clock.
        let mut now = 10.216;
        for _ in 0..1000 {
            now += 0.001;
            time = playback.tick(now, time, 1e9, false).time;
        }
        assert!((time - 1716.0).abs() < 1e-6, "{time}");
    }

    #[test]
    fn playback_stops_at_the_end_unless_it_loops() {
        let mut playback = Playback::new();
        playback.play(0.0, 3900.0, 4000.0);
        let tick = playback.tick(0.2, 3900.0, 4000.0, false);
        assert_eq!(
            tick,
            Tick {
                time: 4000.0,
                playing: false
            }
        );
        assert!(!playback.is_playing());

        playback.play(0.0, 3900.0, 4000.0);
        let tick = playback.tick(0.25, 3900.0, 4000.0, true);
        assert!(tick.playing);
        assert!((tick.time - 150.0).abs() < 1e-6);
        // And keeps time with the clock after the loop.
        let tick = playback.tick(0.35, tick.time, 4000.0, true);
        assert!((tick.time - 250.0).abs() < 1e-6);
        // A long stall wraps as many times as needed.
        let tick = playback.tick(9.35, tick.time, 4000.0, true);
        assert!((tick.time - 1250.0).abs() < 1e-6, "{}", tick.time);
    }

    #[test]
    fn play_at_the_end_starts_over() {
        let mut playback = Playback::new();
        assert_eq!(playback.play(1.0, 4000.0, 4000.0), 0.0);
        assert_eq!(playback.play(1.0, -50.0, 4000.0), 0.0);
    }

    #[test]
    fn moving_the_playhead_while_playing_carries_on_from_there() {
        let mut playback = Playback::new();
        playback.play(0.0, 0.0, 4000.0);
        let tick = playback.tick(0.1, 0.0, 4000.0, false);
        assert!((tick.time - 100.0).abs() < 1e-6);
        // Someone clicked the ruler at 2 s.
        let tick = playback.tick(0.2, 2000.0, 4000.0, false);
        assert_eq!(tick.time, 2000.0);
        let tick = playback.tick(0.3, tick.time, 4000.0, false);
        assert!((tick.time - 2100.0).abs() < 1e-6);
    }

    #[test]
    fn a_paused_or_empty_scene_does_not_move() {
        let mut playback = Playback::new();
        assert_eq!(
            playback.tick(5.0, 1200.0, 4000.0, true),
            Tick {
                time: 1200.0,
                playing: false
            }
        );
        playback.play(0.0, 0.0, 0.0);
        assert!(!playback.tick(1.0, 0.0, 0.0, true).playing);
        playback.play(0.0, 0.0, 4000.0);
        playback.pause();
        assert!(!playback.is_playing());
    }

    #[test]
    fn steps_go_frame_by_frame_within_the_scene() {
        let frame: f64 = 1000.0 / 60.0;
        assert_eq!(
            step_frames(0.0, 1, 4000.0),
            (frame * 1000.0).round() / 1000.0
        );
        assert_eq!(step_frames(0.0, -1, 4000.0), 0.0);
        // From between frames, a step lands on a frame.
        let next = step_frames(25.0, 1, 4000.0);
        assert!((next - 2.0 * frame).abs() < 1e-3, "{next}");
        assert_eq!(step_frames(4000.0, 1, 4000.0), 4000.0);
        let back = step_frames(4000.0, -10, 4000.0);
        assert!((back - 230.0 * frame).abs() < 1e-3, "{back}");
    }

    /// Steps `frames` at a time from `time` until the end of a 4 second
    /// scene in that direction, as the keys do: each step starts from the
    /// time the one before handed out. Checks every step lands one step's
    /// worth of frames further on, and returns how many steps it took.
    fn step_through(mut time: f64, frames: i32) -> usize {
        let frame: f64 = 1000.0 / 60.0;
        let end = if frames > 0 { 4000.0 } else { 0.0 };
        let mut steps = 0;
        while time != end {
            let next = step_frames(time, frames, 4000.0);
            let expected = ((time / frame).round() + f64::from(frames)) * frame;
            let expected = expected.clamp(0.0, 4000.0);
            assert!(
                (next - expected).abs() < 1e-3,
                "step {steps} of {frames} from {time} went to {next}, not {expected}"
            );
            time = next;
            steps += 1;
            assert!(steps <= 300, "stuck at {time}");
        }
        steps
    }

    #[test]
    fn steps_never_get_stuck_on_frames_that_round_badly() {
        // 4 seconds at 60 frames a second is 240 frames.
        assert_eq!(step_through(0.0, 1), 240);
        assert_eq!(step_through(4000.0, -1), 240);
        assert_eq!(step_through(0.0, 10), 24);
        assert_eq!(step_through(4000.0, -10), 24);
        // The times that once stuck, both ways.
        assert_eq!(step_frames(33.333, 1, 4000.0), 50.0);
        assert_eq!(step_frames(16.667, -1, 4000.0), 0.0);
        assert_eq!(step_frames(3966.667, -1, 4000.0), 3950.0);
        assert_eq!(step_frames(33.333, 10, 4000.0), 200.0);
    }

    #[test]
    fn steps_from_between_frames_land_on_the_next_frame_that_way() {
        let frame: f64 = 1000.0 / 60.0;
        // 25 ms is between frame 1 (16.667) and frame 2 (33.333).
        assert!((step_frames(25.0, 1, 4000.0) - 2.0 * frame).abs() < 1e-3);
        assert!((step_frames(25.0, -1, 4000.0) - frame).abs() < 1e-3);
        assert!((step_frames(25.0, 10, 4000.0) - 11.0 * frame).abs() < 1e-3);
        // Just off a frame by more than rounding counts as between frames.
        assert!((step_frames(33.4, -1, 4000.0) - 2.0 * frame).abs() < 1e-3);
        assert!((step_frames(33.3, 1, 4000.0) - 2.0 * frame).abs() < 1e-3);
        // A scene that does not end on a frame: the end is as far as it goes,
        // and a step back from it lands on the frame before.
        assert_eq!(step_frames(4000.0, 1, 4010.0), 4010.0);
        assert_eq!(step_frames(4010.0, 1, 4010.0), 4010.0);
        assert_eq!(step_frames(4010.0, -1, 4010.0), 4000.0);
    }
}
