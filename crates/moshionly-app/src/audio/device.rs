//! [`DeviceAudio`]: sound through the computer's default output, with
//! `cpal` (CoreAudio on macOS, ALSA on Linux).
//!
//! The device asks for samples on a thread of its own. The track and the
//! place in it are shared with that thread behind a lock that the device's
//! thread only ever tries: when the editor holds it for a moment, the
//! device plays a few milliseconds of silence rather than wait. The device
//! is opened the first time there is sound to play, and paused, not
//! closed, when playback stops.
//!
//! Each time the device asks for samples it also says when they will be
//! heard. From that the output knows which moment of the timeline is
//! being heard, and the editor moves the playhead with it.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError, TryLockError};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, StreamConfig, SupportedStreamConfig};

use super::{AudioOut, CHANNELS, SAMPLE_RATE, Track};

/// How long the device may go without asking for samples before its clock
/// is no longer trusted, and the window's clock moves the playhead again.
const STALL: Duration = Duration::from_millis(250);

/// Sound played through the default output device.
pub struct DeviceAudio {
    shared: Arc<Mutex<Shared>>,
    stream: Option<cpal::Stream>,
    /// Opening the device failed during this playback: it is tried again
    /// at the next [`play`](AudioOut::play).
    failed: bool,
}

/// What the editor and the device's thread share.
#[derive(Default)]
struct Shared {
    track: Option<Arc<[f32]>>,
    playing: bool,
    /// The next frame of the track to play: in track frames (48 kHz),
    /// fractional when the device runs at another rate.
    cursor: f64,
    /// Track frames per device frame.
    step: f64,
    /// The moment of the timeline the last samples handed over start at,
    /// in milliseconds, and when they are heard.
    heard: Option<(f64, Instant)>,
    /// Where playback last started or jumped to, in milliseconds: the
    /// moment heard is never reported as earlier than that.
    floor_ms: f64,
    /// When playback last started or jumped.
    since: Option<Instant>,
    /// When the device last asked for samples.
    last_call: Option<Instant>,
    /// The device reported an error, for example it was unplugged: it is
    /// opened again at the next play.
    broken: bool,
}

impl Default for DeviceAudio {
    fn default() -> Self {
        Self::new()
    }
}

impl DeviceAudio {
    /// An output for the default device, which is opened when there is
    /// first something to hear.
    pub fn new() -> Self {
        Self {
            shared: Arc::new(Mutex::new(Shared {
                step: 1.0,
                ..Shared::default()
            })),
            stream: None,
            failed: false,
        }
    }

    /// Opens the device and starts it, when sound is to be heard and it is
    /// not running yet.
    fn run(&mut self) {
        let (playing, has_track, broken) = {
            let shared = lock(&self.shared);
            (shared.playing, shared.track.is_some(), shared.broken)
        };
        if !playing || !has_track || self.failed {
            return;
        }
        if broken {
            self.stream = None;
            lock(&self.shared).broken = false;
        }
        if let Some(stream) = &self.stream {
            if stream.play().is_ok() {
                return;
            }
            self.stream = None;
        }
        match open(&self.shared) {
            Ok(stream) => self.stream = Some(stream),
            Err(problem) => {
                // Played in silence; the picture follows the window's clock.
                eprintln!("moshionly: sound cannot be played: {problem}");
                self.failed = true;
            }
        }
    }

    /// Moves to `ms` on the timeline: the device plays from there next.
    fn jump(&self, ms: f64) {
        let mut shared = lock(&self.shared);
        let ms = ms.max(0.0);
        shared.cursor = ms * f64::from(SAMPLE_RATE) / 1000.0;
        shared.floor_ms = ms;
        shared.heard = None;
        shared.since = Some(Instant::now());
    }
}

impl AudioOut for DeviceAudio {
    fn set_track(&mut self, track: Option<Track>) {
        lock(&self.shared).track = track.map(|track| Arc::clone(track.samples()));
        self.run();
    }

    fn play(&mut self, from_ms: f64) {
        self.failed = false;
        self.jump(from_ms);
        lock(&self.shared).playing = true;
        self.run();
    }

    fn seek(&mut self, to_ms: f64) {
        self.jump(to_ms);
        self.run();
    }

    fn stop(&mut self) {
        {
            let mut shared = lock(&self.shared);
            shared.playing = false;
            shared.heard = None;
        }
        if let Some(stream) = &self.stream
            && stream.pause().is_err()
        {
            self.stream = None;
        }
    }

    fn position(&self) -> Option<f64> {
        self.stream.as_ref()?;
        let shared = lock(&self.shared);
        if !shared.playing || shared.track.is_none() || shared.broken {
            return None;
        }
        heard_at(&shared, Instant::now())
    }
}

/// The moment of the timeline heard at `now`, from what the device last
/// said; `None` when the device has stopped asking for samples.
fn heard_at(shared: &Shared, now: Instant) -> Option<f64> {
    let quiet_since = shared.last_call.max(shared.since)?;
    if now.saturating_duration_since(quiet_since) > STALL {
        return None;
    }
    let Some((ms, at)) = shared.heard else {
        // Just started or jumped: nothing new is heard yet.
        return Some(shared.floor_ms);
    };
    let heard = if now >= at {
        ms + now.duration_since(at).as_secs_f64() * 1000.0
    } else {
        ms - at.duration_since(now).as_secs_f64() * 1000.0
    };
    Some(heard.max(shared.floor_ms))
}

/// The value behind a mutex, even if a thread panicked while holding it.
fn lock(shared: &Mutex<Shared>) -> MutexGuard<'_, Shared> {
    shared.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Opens the default output device and starts it.
fn open(shared: &Arc<Mutex<Shared>>) -> Result<cpal::Stream, String> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or("there is no sound output device")?;
    let supported = pick_config(&device)?;
    let config = supported.config();
    lock(shared).step = f64::from(SAMPLE_RATE) / f64::from(config.sample_rate.max(1));
    let stream = match supported.sample_format() {
        SampleFormat::F32 => build::<f32>(&device, config, shared),
        SampleFormat::I16 => build::<i16>(&device, config, shared),
        SampleFormat::U16 => build::<u16>(&device, config, shared),
        SampleFormat::I32 => build::<i32>(&device, config, shared),
        other => Err(format!(
            "the device wants {other} samples, which are not supported"
        )),
    }?;
    stream.play().map_err(|error| error.to_string())?;
    Ok(stream)
}

/// Stereo floats at 48 kHz when the device offers them, so nothing needs
/// converting; otherwise the device's own choice.
fn pick_config(device: &cpal::Device) -> Result<SupportedStreamConfig, String> {
    if let Ok(configs) = device.supported_output_configs() {
        let wanted = configs
            .filter(|range| {
                usize::from(range.channels()) == CHANNELS
                    && range.sample_format() == SampleFormat::F32
            })
            .find_map(|range| range.try_with_sample_rate(SAMPLE_RATE));
        if let Some(config) = wanted {
            return Ok(config);
        }
    }
    device
        .default_output_config()
        .map_err(|error| error.to_string())
}

/// An output stream of samples of type `T` that plays the shared track.
fn build<T>(
    device: &cpal::Device,
    config: StreamConfig,
    shared: &Arc<Mutex<Shared>>,
) -> Result<cpal::Stream, String>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = usize::from(config.channels).max(1);
    let data = Arc::clone(shared);
    let errors = Arc::clone(shared);
    device
        .build_output_stream(
            config,
            move |out: &mut [T], info: &cpal::OutputCallbackInfo| {
                let stamp = info.timestamp();
                let latency = stamp.playback.duration_since(stamp.callback);
                let mut shared = match data.try_lock() {
                    Ok(shared) => shared,
                    Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
                    // The editor is changing something: a moment of silence.
                    Err(TryLockError::WouldBlock) => {
                        out.fill(T::from_sample(0.0f32));
                        return;
                    }
                };
                fill(&mut shared, out, channels, Instant::now(), latency);
            },
            move |_error| {
                lock(&errors).broken = true;
            },
            None,
        )
        .map_err(|error| error.to_string())
}

/// Fills `out`, `channels` samples per frame, with the track from the
/// cursor on, and notes when the first of them is heard: `latency` after
/// `now`.
fn fill<T>(shared: &mut Shared, out: &mut [T], channels: usize, now: Instant, latency: Duration)
where
    T: SizedSample + FromSample<f32>,
{
    shared.last_call = Some(now);
    let track = match (&shared.track, shared.playing) {
        (Some(track), true) => Arc::clone(track),
        _ => {
            out.fill(T::from_sample(0.0f32));
            return;
        }
    };
    let start_ms = shared.cursor * 1000.0 / f64::from(SAMPLE_RATE);
    shared.heard = Some((start_ms, now + latency));
    for frame in out.chunks_mut(channels) {
        let (left, right) = stereo_at(&track, shared.cursor);
        match frame {
            [mono] => *mono = T::from_sample((left + right) * 0.5),
            [first, second, rest @ ..] => {
                *first = T::from_sample(left);
                *second = T::from_sample(right);
                rest.fill(T::from_sample(0.0f32));
            }
            [] => {}
        }
        shared.cursor += shared.step;
    }
}

/// The left and right sample of `track` at fractional frame `at`, read
/// between the frames on either side; silence outside the track.
fn stereo_at(track: &[f32], at: f64) -> (f32, f32) {
    if !(at >= 0.0) {
        return (0.0, 0.0);
    }
    let frames = track.len() / CHANNELS;
    let index = at.floor() as usize;
    if index >= frames {
        return (0.0, 0.0);
    }
    let fraction = (at - at.floor()) as f32;
    let frame = |n: usize| -> (f32, f32) {
        if n < frames {
            (track[n * CHANNELS], track[n * CHANNELS + 1])
        } else {
            (0.0, 0.0)
        }
    };
    let (a, b) = (frame(index), frame(index + 1));
    (a.0 + (b.0 - a.0) * fraction, a.1 + (b.1 - a.1) * fraction)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn playing(track: Vec<f32>) -> Shared {
        Shared {
            track: Some(track.into()),
            playing: true,
            step: 1.0,
            ..Shared::default()
        }
    }

    #[test]
    fn samples_between_frames_are_read_in_between() {
        let track = [0.0, 1.0, 1.0, -1.0];
        assert_eq!(stereo_at(&track, 0.0), (0.0, 1.0));
        assert_eq!(stereo_at(&track, 0.5), (0.5, 0.0));
        assert_eq!(stereo_at(&track, 1.0), (1.0, -1.0));
        // Past the end, before the start: silence.
        assert_eq!(stereo_at(&track, 1.5), (0.5, -0.5));
        assert_eq!(stereo_at(&track, 2.0), (0.0, 0.0));
        assert_eq!(stereo_at(&track, -1.0), (0.0, 0.0));
        assert_eq!(stereo_at(&track, f64::NAN), (0.0, 0.0));
    }

    #[test]
    fn the_device_gets_the_track_from_the_cursor_on() {
        let mut shared = playing(vec![0.1, 0.2, 0.3, 0.4, 0.5, 0.6]);
        shared.cursor = 1.0;
        let now = Instant::now();
        let mut out = [9.0f32; 6];
        fill(&mut shared, &mut out, 2, now, Duration::from_millis(10));
        assert_eq!(out, [0.3, 0.4, 0.5, 0.6, 0.0, 0.0]);
        assert_eq!(shared.cursor, 4.0);
        // Heard 10 ms from now, from frame 1 of the track.
        let (ms, at) = shared.heard.unwrap();
        assert!((ms - 1000.0 / 48_000.0).abs() < 1e-9);
        assert_eq!(at, now + Duration::from_millis(10));
        // A mono device hears both sides; a wider one hears them on its
        // first two speakers.
        shared.cursor = 0.0;
        let mut mono = [0.0f32; 1];
        fill(&mut shared, &mut mono, 1, now, Duration::ZERO);
        assert!((mono[0] - 0.15).abs() < 1e-6);
        let mut wide = [9.0f32; 4];
        fill(&mut shared, &mut wide, 4, now, Duration::ZERO);
        assert_eq!(wide, [0.3, 0.4, 0.0, 0.0]);
    }

    #[test]
    fn stopped_or_trackless_output_is_silent() {
        let mut shared = playing(vec![0.5; 8]);
        shared.playing = false;
        let mut out = [1i16; 4];
        fill(&mut shared, &mut out, 2, Instant::now(), Duration::ZERO);
        assert_eq!(out, [0; 4]);
        assert_eq!(shared.heard, None);
        let mut shared = Shared {
            playing: true,
            ..Shared::default()
        };
        let mut out = [1.0f32; 4];
        fill(&mut shared, &mut out, 2, Instant::now(), Duration::ZERO);
        assert_eq!(out, [0.0; 4]);
    }

    #[test]
    fn the_moment_heard_follows_the_device() {
        let start = Instant::now();
        let mut shared = playing(vec![0.0; 96_000]);
        shared.floor_ms = 500.0;
        shared.since = Some(start);
        // Before the device asks for anything: where playback started.
        assert_eq!(heard_at(&shared, start), Some(500.0));
        // It asked, and what it got is heard 20 ms later.
        shared.last_call = Some(start);
        shared.heard = Some((500.0, start + Duration::from_millis(20)));
        assert_eq!(
            heard_at(&shared, start),
            Some(500.0),
            "never before the start"
        );
        let later = start + Duration::from_millis(120);
        let heard = heard_at(&shared, later).unwrap();
        assert!((heard - 600.0).abs() < 1e-6, "{heard}");
        // A device that stopped asking is not trusted.
        assert_eq!(heard_at(&shared, start + STALL * 2), None);
        assert_eq!(heard_at(&Shared::default(), start), None);
    }

    #[test]
    fn a_new_output_opens_no_device_until_there_is_sound() {
        let mut out = DeviceAudio::new();
        out.play(0.0);
        out.seek(100.0);
        assert!(out.stream.is_none());
        assert_eq!(out.position(), None);
        out.stop();
        assert!(!lock(&out.shared).playing);
    }
}
