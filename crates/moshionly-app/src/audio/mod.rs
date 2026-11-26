//! Sound while the editor plays.
//!
//! The scene's sound (its audio items and the sound of its videos) is
//! mixed into one [`Track`] off the UI thread by a [`Soundtrack`], which
//! hands it to an [`AudioOut`] and tells it when to play, jump and stop.
//! The window installs the computer's speakers ([`DeviceAudio`]); without
//! that, as in tests and [`Shell::headless`](crate::shell::Shell::headless),
//! the output is [`SilentAudio`], which plays nothing.
//!
//! The output also says where on the timeline the sound being heard is,
//! so that while sound plays the picture follows it rather than the
//! window's clock, and the two stay in step.

mod device;
mod soundtrack;

use std::sync::{Arc, Mutex, PoisonError};

pub use device::DeviceAudio;
pub use moshionly_export::media::{CHANNELS, SAMPLE_RATE};
pub use soundtrack::Soundtrack;

/// The sound of a scene, from the start of its timeline: 48 kHz stereo
/// samples, left and right interleaved, between -1 and 1.
#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    samples: Arc<[f32]>,
}

impl Track {
    /// A track of `samples`, which start at time 0 of the timeline.
    pub fn new(samples: impl Into<Arc<[f32]>>) -> Self {
        Self {
            samples: samples.into(),
        }
    }

    /// The samples, left and right interleaved.
    pub fn samples(&self) -> &Arc<[f32]> {
        &self.samples
    }

    /// How long the track plays, in milliseconds.
    pub fn duration_ms(&self) -> f64 {
        (self.samples.len() / CHANNELS) as f64 * 1000.0 / f64::from(SAMPLE_RATE)
    }
}

/// Where the editor's sound goes.
///
/// The editor calls these in the order things happen: [`play`](Self::play)
/// when playback starts, [`seek`](Self::seek) when the playhead jumps while
/// playing (a click on the ruler, a loop starting over when the output
/// cannot say what is heard), and [`stop`](Self::stop) when playback stops.
/// The track and the loop can change at any time, for example when a clip
/// is moved while playing.
pub trait AudioOut {
    /// The sound to play from now on, or `None` for silence.
    fn set_track(&mut self, track: Option<Track>);
    /// While the scene loops, `Some` of the loop's length in milliseconds:
    /// at that moment the track starts over from the beginning of the
    /// timeline by itself, with no gap, and [`position`](Self::position)
    /// says where in the loop the sound heard is. `None`, as at first,
    /// plays on to the end of the track.
    fn set_loop(&mut self, loop_ms: Option<f64>);
    /// Starts playing at `from_ms` on the timeline.
    fn play(&mut self, from_ms: f64);
    /// Goes on playing from `to_ms` on the timeline.
    fn seek(&mut self, to_ms: f64);
    /// Stops playing.
    fn stop(&mut self);
    /// Where on the timeline the sound being heard is, in milliseconds,
    /// while sound is playing; `None` when nothing is heard, and then the
    /// window's clock moves the playhead.
    fn position(&self) -> Option<f64>;
}

/// What an [`AudioOut`] was asked to do, as a [`SilentAudio`] that
/// records writes it down.
#[derive(Clone, Debug, PartialEq)]
pub enum AudioCall {
    /// A track of this many milliseconds, or silence.
    Track(Option<f64>),
    /// A loop this many milliseconds long, or none.
    Loop(Option<f64>),
    /// Play from this time.
    Play(f64),
    /// Jump to this time.
    Seek(f64),
    /// Stop.
    Stop,
}

/// An output that plays nothing: the editor's default, so tests and
/// windows without speakers stay quiet. One made with
/// [`recording`](Self::recording) writes down what it is asked.
#[derive(Clone, Debug, Default)]
pub struct SilentAudio {
    log: Option<Arc<Mutex<Vec<AudioCall>>>>,
}

impl SilentAudio {
    /// A silent output that writes down every call, readable through
    /// [`calls`](Self::calls) on a clone of it.
    pub fn recording() -> Self {
        Self {
            log: Some(Arc::default()),
        }
    }

    /// The calls written down so far, in order; empty unless recording.
    pub fn calls(&self) -> Vec<AudioCall> {
        self.log.as_ref().map_or_else(Vec::new, |log| {
            log.lock().unwrap_or_else(PoisonError::into_inner).clone()
        })
    }

    fn note(&self, call: AudioCall) {
        if let Some(log) = &self.log {
            log.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(call);
        }
    }
}

impl AudioOut for SilentAudio {
    fn set_track(&mut self, track: Option<Track>) {
        self.note(AudioCall::Track(track.map(|track| track.duration_ms())));
    }

    fn set_loop(&mut self, loop_ms: Option<f64>) {
        self.note(AudioCall::Loop(loop_ms));
    }

    fn play(&mut self, from_ms: f64) {
        self.note(AudioCall::Play(from_ms));
    }

    fn seek(&mut self, to_ms: f64) {
        self.note(AudioCall::Seek(to_ms));
    }

    fn stop(&mut self) {
        self.note(AudioCall::Stop);
    }

    fn position(&self) -> Option<f64> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_track_knows_its_length() {
        let second = Track::new(vec![0.0; SAMPLE_RATE as usize * CHANNELS]);
        assert_eq!(second.duration_ms(), 1000.0);
        assert_eq!(Track::new(Vec::new()).duration_ms(), 0.0);
    }

    #[test]
    fn a_recording_output_writes_down_its_calls_in_order() {
        let mut out = SilentAudio::recording();
        let log = out.clone();
        out.play(250.0);
        out.set_track(Some(Track::new(vec![0.0; 96])));
        out.set_loop(Some(4000.0));
        out.seek(1000.0);
        out.stop();
        assert_eq!(out.position(), None);
        assert_eq!(
            log.calls(),
            [
                AudioCall::Play(250.0),
                AudioCall::Track(Some(1.0)),
                AudioCall::Loop(Some(4000.0)),
                AudioCall::Seek(1000.0),
                AudioCall::Stop
            ]
        );
        // The default one keeps nothing.
        let mut quiet = SilentAudio::default();
        quiet.play(0.0);
        assert!(quiet.calls().is_empty());
    }
}
