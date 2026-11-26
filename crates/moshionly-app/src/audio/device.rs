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
//!
//! While the scene loops, the device starts the track over by itself at
//! the end of the loop, so no silence is heard between loops. The moment
//! heard then keeps rising through the loops, and is folded back into the
//! scene when the editor asks for it: the start of the next loop is on its
//! way to the speakers while the end of this one is still heard.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError, TryLockError};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{
    FromSample, SampleFormat, SizedSample, StreamConfig, SupportedStreamConfig,
    SupportedStreamConfigRange,
};

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
    /// While the scene loops, how long the loop is, in milliseconds: the
    /// track starts over from the beginning there.
    loop_ms: Option<f64>,
    /// How far round the loop the track has gone since playback last
    /// started or jumped, in milliseconds. The moments heard, and
    /// [`floor_ms`](Self::floor_ms), count these on top of the timeline.
    laps_ms: f64,
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
        count_from(&mut shared, ms);
    }
}

/// Counts the moments heard afresh from `ms` on the timeline, where the
/// device plays from next: nothing is heard before it.
fn count_from(shared: &mut Shared, ms: f64) {
    shared.floor_ms = ms;
    shared.laps_ms = 0.0;
    shared.heard = None;
    shared.since = Some(Instant::now());
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

    fn set_loop(&mut self, loop_ms: Option<f64>) {
        let mut shared = lock(&self.shared);
        if shared.loop_ms == loop_ms {
            return;
        }
        shared.loop_ms = loop_ms;
        // The laps gone round the old loop would fold the moments heard
        // wrongly: count afresh from where the device is, which goes on
        // playing from there.
        if shared.laps_ms > 0.0 {
            let ms = shared.cursor * 1000.0 / f64::from(SAMPLE_RATE);
            count_from(&mut shared, ms);
        }
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
        let heard = heard_at(&shared, Instant::now())?;
        Some(folded(&shared, heard))
    }
}

/// The moment of the timeline that `heard`, counted on through the loops,
/// stands for. Without a loop there are no laps to take off: a new loop
/// length counts afresh.
fn folded(shared: &Shared, heard: f64) -> f64 {
    match shared.loop_ms {
        Some(length) if length > 0.0 => heard.rem_euclid(length),
        _ => heard,
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

/// How sound is sent to `device`: see [`choose_config`].
fn pick_config(device: &cpal::Device) -> Result<SupportedStreamConfig, String> {
    let default = device
        .default_output_config()
        .map_err(|error| error.to_string())?;
    let offered = device
        .supported_output_configs()
        .map(Iterator::collect)
        .unwrap_or_else(|_| Vec::new());
    Ok(choose_config(offered, default))
}

/// Stereo floats at the rate the device runs at, `default`'s, when it
/// offers them; otherwise the device's own choice, `default`. Asking for
/// any other rate would switch the device itself (on a Mac, for every app,
/// with the window waiting while it switches); [`fill`] converts the
/// 48 kHz track to the device's rate instead.
fn choose_config(
    offered: Vec<SupportedStreamConfigRange>,
    default: SupportedStreamConfig,
) -> SupportedStreamConfig {
    offered
        .into_iter()
        .filter(|range| {
            usize::from(range.channels()) == CHANNELS && range.sample_format() == SampleFormat::F32
        })
        .find_map(|range| range.try_with_sample_rate(default.sample_rate()))
        .unwrap_or(default)
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
/// `now`. While the scene loops, the track starts over at the loop's end.
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
    let rate = f64::from(SAMPLE_RATE);
    let end = shared
        .loop_ms
        .map(|ms| ms * rate / 1000.0)
        .filter(|&end| end > 0.0);
    let start_ms = shared.laps_ms + shared.cursor * 1000.0 / rate;
    shared.heard = Some((start_ms, now + latency));
    for frame in out.chunks_mut(channels) {
        if let Some(end) = end
            && shared.cursor >= end
        {
            // Round the loop again, however far past its end the cursor
            // is (a loop just made shorter).
            let laps = (shared.cursor / end).floor();
            shared.cursor -= laps * end;
            shared.laps_ms += laps * end * 1000.0 / rate;
        }
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

    /// Fills `frames` stereo frames, the device asking `latency` before
    /// they are heard, and returns their left samples.
    fn left(shared: &mut Shared, frames: usize, now: Instant, latency: Duration) -> Vec<f32> {
        let mut out = vec![9.0f32; frames * 2];
        fill(shared, &mut out, 2, now, latency);
        out.chunks(2).map(|frame| frame[0]).collect()
    }

    #[test]
    fn a_looping_scene_starts_its_sound_over_with_no_gap() {
        // A scene of 4 frames, whose sound is 1, 2, 3, 4 on the left.
        let ms = 1000.0 / f64::from(SAMPLE_RATE);
        let mut shared = playing(vec![1.0, 0.0, 2.0, 0.0, 3.0, 0.0, 4.0, 0.0]);
        shared.loop_ms = Some(4.0 * ms);
        shared.cursor = 2.0;
        let now = Instant::now();
        assert_eq!(
            left(&mut shared, 7, now, Duration::ZERO),
            [3.0, 4.0, 1.0, 2.0, 3.0, 4.0, 1.0]
        );
        assert_eq!(shared.cursor, 1.0);
        // The moment heard keeps rising through the loops...
        assert_eq!(shared.laps_ms, 8.0 * ms);
        left(&mut shared, 1, now, Duration::ZERO);
        let (heard, _) = shared.heard.unwrap();
        assert!((heard - 9.0 * ms).abs() < 1e-9, "{heard}");
        // ...and is told as a moment of the scene.
        assert!((folded(&shared, heard) - ms).abs() < 1e-9);
        // Without a loop the sound ends with the track.
        shared.loop_ms = None;
        shared.cursor = 3.0;
        shared.laps_ms = 0.0;
        assert_eq!(left(&mut shared, 3, now, Duration::ZERO), [4.0, 0.0, 0.0]);
    }

    #[test]
    fn the_loop_start_is_heard_on_time_through_the_output_delay() {
        // A second-long looping scene, played with 150 ms of delay in the
        // output: just before the end of the loop is heard, the start of
        // the next one is already on its way.
        let start = Instant::now();
        let mut shared = playing(vec![0.5; SAMPLE_RATE as usize * CHANNELS]);
        shared.loop_ms = Some(1000.0);
        shared.since = Some(start);
        shared.cursor = 0.9 * f64::from(SAMPLE_RATE);
        let latency = Duration::from_millis(150);
        let mut at = start;
        let mut silent = 0;
        let mut moments = Vec::new();
        // 512-frame buffers, each asked for when the one before is used up.
        let buffer = Duration::from_secs_f64(512.0 / f64::from(SAMPLE_RATE));
        for _ in 0..40 {
            silent += left(&mut shared, 512, at, latency)
                .iter()
                .filter(|&&sample| sample == 0.0)
                .count();
            moments.push(folded(&shared, heard_at(&shared, at).unwrap()));
            at += buffer;
        }
        assert_eq!(silent, 0, "no gap of silence at the loop");
        // Heard: 750 ms (900 less the delay), on to the end of the loop,
        // then from its start, a buffer at a time.
        assert!((moments[0] - 750.0).abs() < 1e-6, "{moments:?}");
        let wrap = moments
            .windows(2)
            .position(|pair| pair[1] < pair[0])
            .expect("the loop started over");
        let step = 512.0 * 1000.0 / f64::from(SAMPLE_RATE);
        for (index, pair) in moments.windows(2).enumerate() {
            let expected = if index == wrap { step - 1000.0 } else { step };
            assert!(
                (pair[1] - pair[0] - expected).abs() < 1e-6,
                "{index}: {moments:?}"
            );
        }
    }

    #[test]
    fn a_new_loop_length_counts_afresh_from_where_the_device_is() {
        let mut out = DeviceAudio::new();
        {
            let mut shared = lock(&out.shared);
            shared.loop_ms = Some(1000.0);
            shared.laps_ms = 3000.0;
            shared.cursor = 0.2 * f64::from(SAMPLE_RATE);
        }
        // The loop's length changes, or the loop is turned off: the laps
        // gone round the old loop no longer count.
        out.set_loop(Some(1500.0));
        let shared = lock(&out.shared);
        assert_eq!(shared.loop_ms, Some(1500.0));
        assert_eq!(shared.laps_ms, 0.0);
        assert!((shared.floor_ms - 200.0).abs() < 1e-9);
        assert!((shared.cursor - 0.2 * f64::from(SAMPLE_RATE)).abs() < 1e-6);
        drop(shared);
        out.set_loop(None);
        assert_eq!(lock(&out.shared).loop_ms, None);
        // Playing from somewhere counts the laps from there.
        lock(&out.shared).laps_ms = 2000.0;
        out.seek(500.0);
        assert_eq!(lock(&out.shared).laps_ms, 0.0);
    }

    #[test]
    fn the_device_is_asked_for_the_rate_it_already_runs_at() {
        use cpal::SupportedBufferSize::Unknown;
        let range = |channels, format| {
            SupportedStreamConfigRange::new(channels, 44_100, 96_000, Unknown, format)
        };
        // A device running at 44.1 kHz that could also run at 48 kHz: asking
        // for 48 kHz would switch it, for every app on a Mac.
        let default = SupportedStreamConfig::new(2, 44_100, Unknown, SampleFormat::I32);
        let offered = vec![
            range(1, SampleFormat::F32),
            range(2, SampleFormat::I32),
            range(2, SampleFormat::F32),
        ];
        let chosen = choose_config(offered, default.clone());
        assert_eq!(
            (
                chosen.channels(),
                chosen.sample_rate(),
                chosen.sample_format()
            ),
            (2, 44_100, SampleFormat::F32)
        );
        // Without stereo floats at that rate, the device's own choice.
        let mono = vec![range(1, SampleFormat::F32)];
        assert_eq!(choose_config(mono, default.clone()), default);
        assert_eq!(choose_config(Vec::new(), default.clone()), default);
    }

    #[test]
    fn a_device_at_another_rate_hears_the_track_at_its_pace() {
        // A device at 96 kHz plays two frames for every frame of the
        // track, read in between; round a loop of 3 frames too.
        let ms = 1000.0 / f64::from(SAMPLE_RATE);
        let mut shared = playing(vec![0.0, 0.0, 1.0, 0.0, 2.0, 0.0, 3.0, 0.0]);
        shared.step = 0.5;
        shared.loop_ms = Some(3.0 * ms);
        let heard = left(&mut shared, 8, Instant::now(), Duration::ZERO);
        assert_eq!(heard, [0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 0.0, 0.5]);
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
