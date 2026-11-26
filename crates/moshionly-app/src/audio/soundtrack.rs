//! [`Soundtrack`]: the scene's sound, mixed off the UI thread and played
//! in step with the playhead.
//!
//! Reading sound files takes a while (`ffmpeg` decodes them), so the mix
//! is made on a thread of its own and handed to the output when it is
//! ready; until then playback goes on in silence. One mix is made at a
//! time: when the sound changes while one is being made (a clip dragged
//! on the timeline), the newest wish waits for it and the stale result is
//! dropped. Files once read stay with the [`SoundDecoder`], so mixing the
//! same clips again only adds them up.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, TryRecvError};

use moshionly_core::eval::{Sound, duration_of, sounds};
use moshionly_core::{Document, NodeId};
use moshionly_export::CancelToken;
use moshionly_export::media::{MediaError, Mix, SoundDecoder};

use super::{AudioOut, SilentAudio, Track};

/// What a mix is of: the sound that plays, where its files are found,
/// and how long the scene is.
#[derive(Clone, Debug, PartialEq)]
struct MixKey {
    sounds: Vec<Sound>,
    project: Option<PathBuf>,
    length_ms: f64,
}

/// A mix wished for: what it is of and what it is made from.
struct Request {
    key: MixKey,
    document: Arc<Document>,
    artboard: NodeId,
}

/// A mix being made on its own thread.
struct Job {
    key: MixKey,
    cancel: CancelToken,
    result: Receiver<Result<Mix, MediaError>>,
}

/// The scene's sound played through an [`AudioOut`].
pub struct Soundtrack {
    out: Box<dyn AudioOut>,
    /// Reads and mixes sound; `None` without `ffmpeg`, and then nothing
    /// is heard.
    decoder: Option<Arc<SoundDecoder>>,
    /// What the output's track is the mix of.
    current: Option<MixKey>,
    /// The mix being made.
    job: Option<Job>,
    /// The mix wished for once the one being made is done.
    next: Option<Request>,
    /// Warnings already shown, so a missing file is said once.
    warned: HashSet<String>,
    playing: bool,
    /// The loop the output plays round, as last given to it.
    loop_ms: Option<f64>,
}

impl Default for Soundtrack {
    fn default() -> Self {
        Self::new(Box::new(SilentAudio::default()))
    }
}

impl Soundtrack {
    /// A soundtrack playing through `out`, that reads no files until it is
    /// given a decoder.
    pub fn new(out: Box<dyn AudioOut>) -> Self {
        Self {
            out,
            decoder: None,
            current: None,
            job: None,
            next: None,
            warned: HashSet::new(),
            playing: false,
            loop_ms: None,
        }
    }

    /// Plays through `out` from now on. The old output stops; the new one
    /// gets the track, and plays it from `playhead` when playing.
    pub fn set_output(&mut self, out: Box<dyn AudioOut>, playhead: f64) {
        self.out.stop();
        self.out = out;
        // A new output plays no loop until it is told one.
        if self.loop_ms.is_some() {
            self.out.set_loop(self.loop_ms);
        }
        self.current = None;
        if self.playing {
            self.out.play(playhead);
        }
    }

    /// Reads sound with `decoder` from now on, or with nothing (no
    /// `ffmpeg`): the sound is mixed again.
    pub fn set_decoder(&mut self, decoder: Option<Arc<SoundDecoder>>) {
        self.decoder = decoder;
        self.current = None;
        self.next = None;
        if let Some(job) = self.job.take() {
            job.cancel.cancel();
        }
    }

    /// Whether sound files can be read.
    pub fn can_read_sound(&self) -> bool {
        self.decoder.is_some()
    }

    /// Makes sure the track is the sound of `artboard` in `document`, whose
    /// files are found next to `project`: when that sound changed, it is
    /// mixed again in the background. Cheap when nothing changed; call it
    /// every frame the sound may be needed.
    pub fn follow(&mut self, document: &Arc<Document>, artboard: &NodeId, project: Option<&Path>) {
        let length_ms = duration_of(document, artboard).unwrap_or(0.0);
        let key = MixKey {
            sounds: sounds(document, artboard, 0.0, length_ms),
            project: project.map(Path::to_path_buf),
            length_ms,
        };
        if self.current.as_ref() == Some(&key) {
            // Back to what plays: whatever was on its way is not needed.
            self.next = None;
            if let Some(job) = &self.job {
                job.cancel.cancel();
            }
            return;
        }
        if self.job.as_ref().is_some_and(|job| job.key == key) {
            self.next = None;
            return;
        }
        let request = Request {
            key,
            document: Arc::clone(document),
            artboard: artboard.clone(),
        };
        match &self.job {
            // One mix at a time: this one waits for the running one.
            Some(job) => {
                job.cancel.cancel();
                self.next = Some(request);
            }
            None => self.start(request),
        }
    }

    /// Mixes what `request` asks for: on a thread of its own, or straight
    /// away when there is nothing to read.
    fn start(&mut self, request: Request) {
        let decoder = match &self.decoder {
            Some(decoder) if !request.key.sounds.is_empty() && request.key.length_ms > 0.0 => {
                Arc::clone(decoder)
            }
            // Silence needs no mixing.
            _ => {
                self.install(request.key, None);
                return;
            }
        };
        let (sender, result) = mpsc::channel();
        let cancel = CancelToken::new();
        let job = Job {
            key: request.key.clone(),
            cancel: cancel.clone(),
            result,
        };
        let Request {
            key,
            document,
            artboard,
        } = request;
        let spawned = std::thread::Builder::new()
            .name("moshionly-sound".into())
            .spawn(move || {
                let mix = decoder.mix(
                    &document,
                    &artboard,
                    key.project.as_deref(),
                    0.0,
                    key.length_ms,
                    &cancel,
                );
                // The soundtrack may be gone; then nobody wants the mix.
                let _ = sender.send(mix);
            });
        match spawned {
            Ok(_) => self.job = Some(job),
            // No thread to read sound on: play on in silence.
            Err(_) => self.install(job.key, None),
        }
    }

    /// Gives the output a new track, made for `key`, and carries on
    /// playing where the playhead is.
    fn install(&mut self, key: MixKey, track: Option<Track>) {
        self.current = Some(key);
        self.out.set_track(track);
    }

    /// Takes in a mix that is done, handing it to the output, which goes
    /// on from `playhead` when playing. Returns the warnings about files
    /// that could not be read and were not shown before. Call it every
    /// frame.
    pub fn poll(&mut self, playhead: f64) -> Vec<String> {
        let Some(job) = &self.job else {
            return Vec::new();
        };
        let finished = match job.result.try_recv() {
            Err(TryRecvError::Empty) => return Vec::new(),
            Ok(Ok(mix)) => Some(mix),
            // Cancelled, or the thread is gone.
            Ok(Err(_)) | Err(TryRecvError::Disconnected) => None,
        };
        let Some(job) = self.job.take() else {
            return Vec::new();
        };
        let mut warnings = Vec::new();
        match (self.next.take(), finished) {
            // Something else is wished for now: this mix is stale.
            (Some(next), _) => self.start(next),
            (None, Some(mix)) => {
                for warning in &mix.warnings {
                    if self.warned.insert(warning.clone()) {
                        warnings.push(warning.clone());
                    }
                }
                let track = (mix.sounds > 0).then(|| Track::new(mix.samples));
                self.install(job.key, track);
                if self.playing {
                    self.out.seek(playhead);
                }
            }
            // Cancelled with nothing wished for after it.
            (None, None) => {}
        }
        warnings
    }

    /// Whether a mix is being made.
    pub fn is_mixing(&self) -> bool {
        self.job.is_some()
    }

    /// Whether the output's track is the newest mix asked for, with
    /// nothing left to make.
    pub fn is_ready(&self) -> bool {
        self.job.is_none() && self.next.is_none() && self.current.is_some()
    }

    /// Plays the sound round a loop `loop_ms` long, from the start of the
    /// timeline, while the scene loops; `None` plays on to its end. The
    /// output starts the sound over by itself, so a loop has no gap. Cheap
    /// when nothing changed; call it every frame while playing.
    pub fn set_loop(&mut self, loop_ms: Option<f64>) {
        if self.loop_ms != loop_ms {
            self.loop_ms = loop_ms;
            self.out.set_loop(loop_ms);
        }
    }

    /// Starts playing at `from_ms`.
    pub fn play(&mut self, from_ms: f64) {
        self.playing = true;
        self.out.play(from_ms);
    }

    /// Goes on playing from `to_ms`, when playing.
    pub fn seek(&mut self, to_ms: f64) {
        if self.playing {
            self.out.seek(to_ms);
        }
    }

    /// Stops playing, when playing.
    pub fn stop(&mut self) {
        if std::mem::take(&mut self.playing) {
            self.out.stop();
        }
    }

    /// Whether it is playing.
    pub fn is_playing(&self) -> bool {
        self.playing
    }

    /// Where on the timeline the sound being heard is, while sound plays.
    pub fn position(&self) -> Option<f64> {
        self.playing.then(|| self.out.position()).flatten()
    }
}

#[cfg(test)]
mod tests {
    use moshionly_core::io;

    use super::super::AudioCall;
    use super::*;

    fn document(audio_end: f64) -> Arc<Document> {
        let text = format!(
            r##"{{"format":"moshionly","version":3,"name":"Sound","nodes":[
            {{"id":"ab","parent":null,"index":"a0","type":"artboard","duration":2000}},
            {{"id":"song","parent":"ab","index":"a0","type":"audio","source":"/no/such/song.wav",
              "start":0,"end":{audio_end}}}
            ]}}"##
        );
        Arc::new(io::parse(&text).unwrap())
    }

    #[test]
    fn calls_reach_the_output_only_while_playing() {
        let out = SilentAudio::recording();
        let mut soundtrack = Soundtrack::new(Box::new(out.clone()));
        soundtrack.seek(100.0);
        soundtrack.stop();
        assert!(out.calls().is_empty());
        soundtrack.play(200.0);
        soundtrack.seek(900.0);
        soundtrack.stop();
        soundtrack.stop();
        assert_eq!(
            out.calls(),
            [
                AudioCall::Play(200.0),
                AudioCall::Seek(900.0),
                AudioCall::Stop
            ]
        );
        assert_eq!(soundtrack.position(), None);
    }

    #[test]
    fn the_loop_reaches_the_output_when_it_changes() {
        let first = SilentAudio::recording();
        let mut soundtrack = Soundtrack::new(Box::new(first.clone()));
        soundtrack.set_loop(Some(2000.0));
        soundtrack.set_loop(Some(2000.0));
        soundtrack.set_loop(None);
        assert_eq!(
            first.calls(),
            [AudioCall::Loop(Some(2000.0)), AudioCall::Loop(None)]
        );
        // A new output is told the loop there is, if any.
        let second = SilentAudio::recording();
        soundtrack.set_output(Box::new(second.clone()), 0.0);
        assert!(second.calls().is_empty());
        soundtrack.set_loop(Some(3000.0));
        let third = SilentAudio::recording();
        soundtrack.set_output(Box::new(third.clone()), 0.0);
        assert_eq!(third.calls(), [AudioCall::Loop(Some(3000.0))]);
    }

    #[test]
    fn without_a_decoder_the_track_is_silence_once_per_change() {
        let out = SilentAudio::recording();
        let mut soundtrack = Soundtrack::new(Box::new(out.clone()));
        let artboard = NodeId::from("ab");
        let first = document(1000.0);
        soundtrack.follow(&first, &artboard, None);
        soundtrack.follow(&first, &artboard, None);
        assert!(soundtrack.is_ready() && !soundtrack.is_mixing());
        assert_eq!(out.calls(), [AudioCall::Track(None)]);
        // Moving the clip changes the sound.
        soundtrack.follow(&document(1500.0), &artboard, None);
        assert_eq!(out.calls().len(), 2);
        assert!(soundtrack.poll(0.0).is_empty());
    }
}
