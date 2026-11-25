//! [`SoundDecoder`]: sound read with `ffmpeg`, and the mix of an artboard.
//!
//! Sound is read as 48 kHz stereo 32-bit float samples, left and right
//! interleaved. Mono sound plays at full level on both sides; more
//! channels are folded down to two by `ffmpeg`. Sample `n` of the timeline
//! is at `n / 48` milliseconds, so a clip starting at `t` milliseconds
//! starts at sample `round(t × 48)` whatever stretch is mixed.
//!
//! A clip is read whole (from `trim_start`, for as long as it plays) and
//! kept, within a memory budget, so mixing another stretch of the same
//! timeline, as playback from another moment does, reads nothing again.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::SystemTime;

use moshionly_core::eval::sounds;
use moshionly_core::io::{Asset, resolve_asset};
use moshionly_core::{Document, NodeId};

use super::probe::{Probes, input};
use super::{CHANNELS, MediaError, MediaTools, SAMPLE_RATE, micros, seconds};
use crate::ffmpeg::last_words;
use crate::progress::CancelToken;

/// How much memory kept sound may take by default: 512 MB, about 46
/// minutes.
pub const DEFAULT_SOUND_BUDGET: usize = 512 << 20;

/// Samples per millisecond, per channel.
const PER_MS: f64 = SAMPLE_RATE as f64 / 1000.0;

/// How much of what `ffmpeg` prints on failure is kept for the message.
const MESSAGE_BYTES: usize = 400;

/// The timeline sample a time in milliseconds falls on.
fn sample_at(time_ms: f64) -> i64 {
    (time_ms * PER_MS).round() as i64
}

/// How many samples (per channel) `length_ms` milliseconds take.
pub(crate) fn frames_in(length_ms: f64) -> usize {
    sample_at(length_ms.max(0.0)).max(0) as usize
}

/// Sound read with `ffmpeg`, and kept.
///
/// Share one between everything that plays or exports the same files. It
/// can be used from any thread.
pub struct SoundDecoder {
    tools: MediaTools,
    probes: Probes,
    cache: Mutex<SoundCache>,
}

impl std::fmt::Debug for SoundDecoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SoundDecoder")
            .field("tools", &self.tools)
            .field("memory_used", &self.memory_used())
            .finish_non_exhaustive()
    }
}

/// The sound of an artboard over a stretch of its timeline.
#[derive(Debug, Clone, PartialEq)]
pub struct Mix {
    /// 48 kHz stereo samples, left and right interleaved, from the start of
    /// the stretch to its end; between -1 and 1.
    pub samples: Vec<f32>,
    /// How many clips were mixed in. 0 means the stretch is silent: there
    /// is no sound in it, or none could be read.
    pub sounds: usize,
    /// The clips that could not be read and were left out, in words for a
    /// person.
    pub warnings: Vec<String>,
}

impl Mix {
    /// Samples per channel.
    pub fn frames(&self) -> usize {
        self.samples.len() / CHANNELS
    }
}

/// What a kept stretch of sound is found by: the file as it was, where
/// the stretch starts and how long it is, in microseconds.
type SoundKey = (PathBuf, SystemTime, i64, i64);

struct SoundCache {
    kept: HashMap<SoundKey, (Arc<[f32]>, u64)>,
    bytes: usize,
    budget: usize,
    clock: u64,
}

impl SoundDecoder {
    /// A decoder that reads files with `tools`, keeping up to
    /// [`DEFAULT_SOUND_BUDGET`] of sound.
    pub fn new(tools: MediaTools) -> Self {
        Self {
            tools,
            probes: Probes::default(),
            cache: Mutex::new(SoundCache {
                kept: HashMap::new(),
                bytes: 0,
                budget: DEFAULT_SOUND_BUDGET,
                clock: 0,
            }),
        }
    }

    /// The same decoder keeping at most `bytes` of sound. The sound in use
    /// is always kept, even when it alone is larger.
    pub fn with_budget(self, bytes: usize) -> Self {
        lock(&self.cache).budget = bytes;
        self
    }

    /// Memory taken by kept sound, in bytes.
    pub fn memory_used(&self) -> usize {
        lock(&self.cache).bytes
    }

    /// Whether the file at `path` has sound.
    pub fn has_sound(&self, path: &Path) -> Result<bool, MediaError> {
        Ok(self.probes.get(&self.tools, path)?.1.info.has_sound)
    }

    /// `length_ms` milliseconds of the sound of the file at `path`, from
    /// `from_ms` milliseconds into it, as 48 kHz stereo samples, left and
    /// right interleaved: [`frames_in`]`(length_ms)` of each, silent where
    /// the file has none (past its end, or a file without sound).
    pub fn decode(
        &self,
        path: &Path,
        from_ms: f64,
        length_ms: f64,
    ) -> Result<Arc<[f32]>, MediaError> {
        let (modified, probed) = self.probes.get(&self.tools, path)?;
        let length = frames_in(length_ms) * CHANNELS;
        if !probed.info.has_sound {
            return Ok(vec![0.0; length].into());
        }
        let key = (
            path.to_owned(),
            modified,
            micros(from_ms),
            micros(length_ms),
        );
        if let Some(kept) = lock(&self.cache).get(&key) {
            return Ok(kept);
        }
        let mut samples = self.read(path, from_ms.max(0.0), length_ms, probed.channels)?;
        samples.resize(length, 0.0);
        let samples: Arc<[f32]> = samples.into();
        lock(&self.cache).keep(key, Arc::clone(&samples));
        Ok(samples)
    }

    /// The sound of `artboard` from `from_ms` to `to_ms` on its timeline,
    /// every audio item and audible video mixed at its volume. Sources are
    /// found next to `project_file`.
    ///
    /// A clip that cannot be read is left out with a warning; the mix
    /// fails only when `cancel` is cancelled.
    pub fn mix(
        &self,
        document: &Document,
        artboard: &NodeId,
        project_file: Option<&Path>,
        from_ms: f64,
        to_ms: f64,
        cancel: &CancelToken,
    ) -> Result<Mix, MediaError> {
        let first = sample_at(from_ms);
        let frames = (sample_at(to_ms) - first).max(0) as usize;
        let mut mix = Mix {
            samples: vec![0.0; frames * CHANNELS],
            sounds: 0,
            warnings: Vec::new(),
        };
        for sound in sounds(document, artboard, from_ms, to_ms) {
            if cancel.is_cancelled() {
                return Err(MediaError::Cancelled);
            }
            let left_out = |reason: String| {
                format!(
                    "the sound of \"{}\" could not be read ({reason}); it was left out",
                    sound.source
                )
            };
            let path = match resolve_asset(&sound.source, project_file) {
                Ok(Asset::File(path)) => path,
                Ok(Asset::Data { .. }) => {
                    let reason = "sound cannot be embedded as a \"data:\" URI".to_owned();
                    mix.warnings.push(left_out(reason));
                    continue;
                }
                Err(error) => {
                    mix.warnings.push(left_out(error.to_string()));
                    continue;
                }
            };
            let clip = match self.has_sound(&path) {
                Ok(false) => continue,
                Ok(true) => self.decode(&path, sound.media_start_ms, sound.duration_ms),
                Err(error) => Err(error),
            };
            match clip {
                Ok(clip) => {
                    add(
                        &mut mix.samples,
                        first,
                        &clip,
                        sample_at(sound.start_ms),
                        sound.volume as f32,
                    );
                    mix.sounds += 1;
                }
                Err(error) => mix.warnings.push(left_out(error.to_string())),
            }
        }
        for sample in &mut mix.samples {
            *sample = sample.clamp(-1.0, 1.0);
        }
        Ok(mix)
    }

    /// Reads sound with `ffmpeg`.
    fn read(
        &self,
        path: &Path,
        from_ms: f64,
        length_ms: f64,
        channels: u64,
    ) -> Result<Vec<f32>, MediaError> {
        let mut command = Command::new(self.tools.ffmpeg());
        command.args(["-hide_banner", "-nostdin", "-nostats", "-loglevel", "error"]);
        let from = micros(from_ms);
        if from > 0 {
            command.args(["-ss", &seconds(from)]);
        }
        command.arg("-i").arg(input(path));
        command.args(["-map", "0:a:0", "-vn", "-sn", "-dn"]);
        command.args(["-t", &seconds(micros(length_ms))]);
        // Mono plays on both sides at full level. The start is padded or
        // trimmed to the moment asked for, and gaps are filled with
        // silence, so samples stay in step with time.
        let mono = if channels == 1 {
            "pan=stereo|c0=c0|c1=c0,"
        } else {
            ""
        };
        let filter = format!("{mono}aresample={SAMPLE_RATE}:async=1:first_pts=0");
        command.args(["-af", &filter, "-ac", "2", "-ar", &SAMPLE_RATE.to_string()]);
        command.args(["-f", "f32le", "pipe:1"]);
        let output = command
            .stdin(Stdio::null())
            .output()
            .map_err(|error| MediaError::Failed {
                path: path.to_owned(),
                message: format!(
                    "{} could not be started ({error})",
                    self.tools.ffmpeg().display()
                ),
            })?;
        if !output.status.success() {
            let said = String::from_utf8_lossy(&output.stderr);
            return Err(MediaError::Failed {
                path: path.to_owned(),
                message: match last_words(&said, MESSAGE_BYTES) {
                    "" => format!("ffmpeg stopped with {}", output.status),
                    said => said.to_owned(),
                },
            });
        }
        Ok(output
            .stdout
            .chunks_exact(4)
            .map(|bytes| f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
            .collect())
    }
}

impl SoundCache {
    fn get(&mut self, key: &SoundKey) -> Option<Arc<[f32]>> {
        self.clock += 1;
        let (samples, used) = self.kept.get_mut(key)?;
        *used = self.clock;
        Some(Arc::clone(samples))
    }

    fn keep(&mut self, key: SoundKey, samples: Arc<[f32]>) {
        self.clock += 1;
        self.bytes += samples.len() * 4;
        if let Some((old, _)) = self.kept.insert(key.clone(), (samples, self.clock)) {
            self.bytes -= old.len() * 4;
        }
        while self.bytes > self.budget {
            let oldest = self
                .kept
                .iter()
                .filter(|(kept, _)| **kept != key)
                .min_by_key(|(_, (_, used))| *used)
                .map(|(kept, _)| kept.clone());
            let Some(oldest) = oldest else {
                break;
            };
            if let Some((samples, _)) = self.kept.remove(&oldest) {
                self.bytes -= samples.len() * 4;
            }
        }
    }
}

/// Adds `clip`, which starts at timeline sample `clip_start`, at `volume`
/// to `mix`, which starts at timeline sample `mix_start`. Both are stereo
/// and interleaved; what falls outside `mix` is left out.
fn add(mix: &mut [f32], mix_start: i64, clip: &[f32], clip_start: i64, volume: f32) {
    let mix_frames = (mix.len() / CHANNELS) as i64;
    let clip_frames = (clip.len() / CHANNELS) as i64;
    let offset = clip_start - mix_start;
    let from = offset.max(0);
    let to = (offset + clip_frames).min(mix_frames);
    for frame in from..to {
        let into = frame as usize * CHANNELS;
        let out_of = (frame - offset) as usize * CHANNELS;
        for channel in 0..CHANNELS {
            mix[into + channel] += clip[out_of + channel] * volume;
        }
    }
}

/// The value behind a mutex, even if a thread panicked while holding it.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clips_land_on_the_sample_of_their_time() {
        assert_eq!(sample_at(0.0), 0);
        assert_eq!(sample_at(1000.0), 48_000);
        assert_eq!(sample_at(0.01), 0);
        assert_eq!(sample_at(0.011), 1);
        assert_eq!(frames_in(250.0), 12_000);
        assert_eq!(frames_in(-5.0), 0);
    }

    #[test]
    fn a_clip_is_added_where_it_falls_in_the_mix() {
        // A mix of 4 frames starting at timeline sample 10.
        let mut mix = vec![0.0; 8];
        // A clip of 3 frames starting at sample 12: two fall in.
        add(&mut mix, 10, &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], 12, 0.5);
        assert_eq!(mix, [0.0, 0.0, 0.0, 0.0, 0.5, 1.0, 1.5, 2.0]);
        // One starting before the mix: its first frame is left out.
        add(&mut mix, 10, &[1.0, 1.0, 1.0, 1.0], 9, 1.0);
        assert_eq!(mix, [1.0, 1.0, 0.0, 0.0, 0.5, 1.0, 1.5, 2.0]);
        // One entirely outside changes nothing.
        add(&mut mix, 10, &[9.0, 9.0], 20, 1.0);
        add(&mut mix, 10, &[9.0, 9.0], 2, 1.0);
        assert_eq!(mix, [1.0, 1.0, 0.0, 0.0, 0.5, 1.0, 1.5, 2.0]);
    }

    #[test]
    fn kept_sound_stays_within_the_budget() {
        let mut cache = SoundCache {
            kept: HashMap::new(),
            bytes: 0,
            budget: 32,
            clock: 0,
        };
        let key = |start| (PathBuf::from("a.wav"), SystemTime::UNIX_EPOCH, start, 4);
        cache.keep(key(0), vec![0.0; 4].into());
        cache.keep(key(1), vec![0.0; 4].into());
        assert!(cache.get(&key(0)).is_some());
        cache.keep(key(2), vec![0.0; 4].into());
        assert_eq!(cache.bytes, 32);
        assert!(cache.get(&key(1)).is_none(), "the least recently used went");
        assert!(cache.get(&key(0)).is_some() && cache.get(&key(2)).is_some());
        cache.keep(key(3), vec![0.0; 16].into());
        assert_eq!(cache.bytes, 64, "what is in use stays, alone");
        assert_eq!(cache.kept.len(), 1);
    }
}
