//! [`VideoDecoder`]: video frames for the renderer, read with `ffmpeg`.
//!
//! # Which frame shows
//!
//! At a moment of a file, the frame that shows is the last one whose
//! timestamp is at or before that moment; before the first frame, the
//! first. The rule only depends on the file, so the same request gives
//! the same frame whatever was asked before it, which export needs: it
//! renders frames on several threads, out of order.
//!
//! # How frames are read
//!
//! For each file and frame size, up to two `ffmpeg` processes read frames
//! forward (see [`stream`](super::stream)). A request is served by one
//! that has not passed the moment and is not far behind it; otherwise one
//! starts again a little before the moment. Seeking lands on a key frame
//! and can skip the frames just after it, so when the first frame a
//! restarted process gives is already past the moment, it starts again
//! further back. Every frame read is kept (within a memory budget) with
//! the moments it covers, so frames asked for out of order are usually
//! there already.
//!
//! Small boxes get small frames: the size is the video's own, halved as
//! often as it still covers the box (never more than 16 times). A few
//! sizes at most, so a scale animation does not restart decoding every
//! frame.
//!
//! Processes keep reading every file and size asked for lately, so an
//! export or playback that draws many videos on every frame reads each
//! with its own; those not asked for in a while stop when another file
//! needs one.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use moshionly_render::skia_safe as sk;
use moshionly_render::{VideoFrames, VideoRequest};

use super::cache::{AFTER_LAST, FrameCache, StreamKey};
use super::probe::{Probed, Probes};
use super::stream::Stream;
use super::{MediaError, MediaTools, micros};

/// How much memory kept frames may take by default: 256 MB.
pub const DEFAULT_FRAME_BUDGET: usize = 256 << 20;

/// How many `ffmpeg` processes read one file at one size.
const STREAMS_PER_FILE: usize = 2;

/// How many files (at one size each) keep processes reading them even when
/// they have not been asked for in a while.
const FILES: usize = 6;

/// How many files (at one size each) may have processes reading them at
/// most; past this, the one asked for least recently stops for another.
const MAX_FILES: usize = 32;

/// How long a file (at one size) goes unasked for before its processes may
/// stop for another's, when more than [`FILES`] have some.
const IDLE: Duration = Duration::from_secs(5);

/// How far ahead of a process a moment may be for it to read on to it
/// rather than start again there, in microseconds.
const READ_ON: i64 = 2_000_000;

/// How far before the moment asked for a process starts at least, in
/// microseconds.
const LEAD: i64 = 100_000;

/// How far further back a process starts each time the first frame it
/// gives is past the moment, in microseconds (doubling every time).
const BACK: i64 = 1_000_000;

/// How many times a process may start again for one frame.
const ATTEMPTS: usize = 10;

/// Video frames read with `ffmpeg`, for a renderer's
/// [`AssetLoader`](moshionly_render::AssetLoader).
///
/// Share one between every renderer that draws the same files: it keeps
/// the processes and frames it reads. It can be used from any thread.
pub struct VideoDecoder {
    tools: MediaTools,
    probes: Probes,
    files: Mutex<Files>,
    cache: Mutex<FrameCache>,
}

/// The processes reading each file at each size, and when each was last
/// asked for a frame.
#[derive(Default)]
struct Files {
    readers: HashMap<Arc<StreamKey>, (Arc<Mutex<Readers>>, Instant)>,
}

/// The processes reading one file at one size.
#[derive(Default)]
struct Readers {
    streams: Vec<Stream>,
    clock: u64,
}

impl std::fmt::Debug for VideoDecoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VideoDecoder")
            .field("tools", &self.tools)
            .field("memory_used", &self.memory_used())
            .finish_non_exhaustive()
    }
}

impl VideoDecoder {
    /// A decoder that reads files with `tools`, keeping up to
    /// [`DEFAULT_FRAME_BUDGET`] of frames.
    pub fn new(tools: MediaTools) -> Self {
        Self {
            tools,
            probes: Probes::default(),
            files: Mutex::default(),
            cache: Mutex::new(FrameCache::new(DEFAULT_FRAME_BUDGET)),
        }
    }

    /// The same decoder keeping at most `bytes` of frames. The frame in
    /// use is always kept, even when it alone is larger.
    pub fn with_budget(self, bytes: usize) -> Self {
        Self {
            cache: Mutex::new(FrameCache::new(bytes)),
            ..self
        }
    }

    /// Memory taken by kept frames, in bytes.
    pub fn memory_used(&self) -> usize {
        lock(&self.cache).bytes()
    }

    /// The frame of the file at `path` that shows `time_ms` milliseconds
    /// into it, about `width` by `height` pixels where it is drawn.
    pub fn frame_at(
        &self,
        path: &Path,
        time_ms: f64,
        width: f64,
        height: f64,
    ) -> Result<sk::Image, MediaError> {
        let (modified, probed) = self.probes.get(&self.tools, path)?;
        if !probed.info.has_video {
            return Err(MediaError::NoVideo(path.to_owned()));
        }
        let (width, height) = frame_size((probed.info.width, probed.info.height), width, height);
        let key = Arc::new(StreamKey {
            path: path.to_owned(),
            modified,
            width,
            height,
        });
        let time = micros(time_ms);
        let kept = lock(&self.cache).lookup(&key, time);
        if let Some(image) = kept {
            // Asked for, even if kept frames answered.
            lock(&self.files).used(&key, Instant::now());
            return Ok(image);
        }
        let readers = lock(&self.files).readers(&key, Instant::now());
        let mut readers = lock(&readers);
        // Another thread may have read it while this one waited.
        if let Some(image) = lock(&self.cache).lookup(&key, time) {
            return Ok(image);
        }
        readers
            .frame(&self.tools, &probed, &key, time, &self.cache)
            .map_err(|message| MediaError::Failed {
                path: path.to_owned(),
                message,
            })
    }
}

impl Files {
    /// Notes that `key` was asked for at `now`.
    fn used(&mut self, key: &StreamKey, now: Instant) {
        if let Some((_, used)) = self.readers.get_mut(key) {
            *used = now;
        }
    }

    /// The processes reading `key`, asked for at `now`, made room for when
    /// they are new.
    fn readers(&mut self, key: &Arc<StreamKey>, now: Instant) -> Arc<Mutex<Readers>> {
        if let Some((readers, used)) = self.readers.get_mut(key) {
            *used = now;
            return Arc::clone(readers);
        }
        // Past FILES, those not asked for in a while stop, least recently
        // asked for first; those asked for lately keep reading, up to
        // MAX_FILES. The processes stop once whoever is using them is done.
        while self.readers.len() >= FILES {
            let Some((oldest, used)) = self
                .readers
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(key, (_, used))| (Arc::clone(key), *used))
            else {
                break;
            };
            if self.readers.len() < MAX_FILES && now.saturating_duration_since(used) < IDLE {
                break;
            }
            self.readers.remove(&oldest);
        }
        let readers = Arc::new(Mutex::default());
        self.readers
            .insert(Arc::clone(key), (Arc::clone(&readers), now));
        readers
    }
}

impl VideoFrames for VideoDecoder {
    fn frame(&self, request: &VideoRequest<'_>) -> Result<sk::Image, String> {
        self.frame_at(request.path, request.time_ms, request.width, request.height)
            .map_err(|error| error.to_string())
    }
}

/// How large to read frames of a `natural`-sized video for a box `width`
/// by `height` pixels on the output: the video's size, halved as often as
/// it still covers the box, at most 16 times.
fn frame_size(natural: (u32, u32), width: f64, height: f64) -> (u32, u32) {
    let scale = (width / f64::from(natural.0.max(1))).max(height / f64::from(natural.1.max(1)));
    let mut divisor = 1u32;
    if scale.is_finite() && scale > 0.0 {
        while divisor < 16 && scale * f64::from(divisor * 2) <= 1.0 {
            divisor *= 2;
        }
    }
    (
        natural.0.div_ceil(divisor).max(1),
        natural.1.div_ceil(divisor).max(1),
    )
}

/// What reading on from where a process is gave.
enum Reached {
    /// The frame that shows at the moment asked for.
    Frame(sk::Image),
    /// The process started too late: the first frame it gave is past the
    /// moment, and frames before it were skipped.
    TooLate,
}

impl Readers {
    /// The frame that shows at `time`, read by a process that can reach it
    /// or by one started again for it.
    fn frame(
        &mut self,
        tools: &MediaTools,
        probed: &Probed,
        key: &Arc<StreamKey>,
        time: i64,
        cache: &Mutex<FrameCache>,
    ) -> Result<sk::Image, String> {
        // Start at least two frames early, so the frame showing is not
        // skipped when the moment is just after it.
        let lead = probed
            .info
            .frame_rate
            .map_or(LEAD, |rate| ((2.0e6 / rate) as i64).max(LEAD));
        let mut restart_at = None;
        let mut back = BACK;
        for _ in 0..ATTEMPTS {
            self.clock += 1;
            let reader = match restart_at {
                None => self.reaching(time),
                Some(_) => None,
            };
            let reader = match reader {
                Some(reader) => reader,
                None => {
                    let at = restart_at.unwrap_or(time - lead).max(0);
                    let stream = Stream::start(
                        tools,
                        &key.path,
                        probed.start_us,
                        (key.width, key.height),
                        at,
                    )?;
                    self.replace(stream)
                }
            };
            let stream = &mut self.streams[reader];
            stream.used = self.clock;
            match read_on(stream, time, key, cache)? {
                Reached::Frame(image) => return Ok(image),
                Reached::TooLate => {
                    restart_at = Some((stream.seek_us - back).max(0));
                    back *= 2;
                }
            }
        }
        Err("the frame could not be found".into())
    }

    /// The process that can read on to `time`, furthest along first.
    fn reaching(&self, time: i64) -> Option<usize> {
        self.streams
            .iter()
            .enumerate()
            .filter_map(|(index, stream)| {
                let at = match (&stream.prev, &stream.last) {
                    (Some((prev, _)), _) => *prev,
                    (None, Some((last, _))) => *last,
                    (None, None) if stream.ended => return None,
                    (None, None) => stream.seek_us,
                };
                let reachable = time >= at && (stream.ended || time - at <= READ_ON);
                reachable.then_some((index, at))
            })
            .max_by_key(|(_, at)| *at)
            .map(|(index, _)| index)
    }

    /// Puts a new process in, in place of the least recently used one when
    /// there are enough. Returns where it is.
    fn replace(&mut self, stream: Stream) -> usize {
        if self.streams.len() < STREAMS_PER_FILE {
            self.streams.push(stream);
            return self.streams.len() - 1;
        }
        let oldest = (0..self.streams.len())
            .min_by_key(|index| self.streams[*index].used)
            .unwrap_or(0);
        self.streams[oldest] = stream;
        oldest
    }
}

/// Reads frames until the one that shows at `time` is known, keeping each
/// in `cache` with the moments it covers.
fn read_on(
    stream: &mut Stream,
    time: i64,
    key: &Arc<StreamKey>,
    cache: &Mutex<FrameCache>,
) -> Result<Reached, String> {
    loop {
        match (&stream.prev, &stream.last) {
            (_, Some((last, image))) if *last <= time && stream.ended => {
                return Ok(Reached::Frame(image.clone()));
            }
            (_, Some((last, _))) if *last <= time => {}
            (Some((prev, image)), Some(_)) if *prev <= time => {
                return Ok(Reached::Frame(image.clone()));
            }
            (None, Some((_, first))) if stream.seek_us == 0 => {
                // Before the first frame of the file, the first shows.
                return Ok(Reached::Frame(first.clone()));
            }
            (_, Some(_)) => return Ok(Reached::TooLate),
            (_, None) if stream.ended => {
                if stream.seek_us == 0 {
                    return Err("the file has no video frames".into());
                }
                return Ok(Reached::TooLate);
            }
            (_, None) => {}
        }
        match stream.next_frame()? {
            Some((start, image)) => {
                let mut cache = lock(cache);
                if stream.frames == 0 && stream.seek_us == 0 {
                    cache.first(key, start);
                }
                if let Some((last, _)) = &stream.last {
                    cache.link(key, *last, start);
                }
                cache.keep(key, start, image.clone());
                drop(cache);
                stream.prev = stream.last.take();
                stream.last = Some((start, image));
                stream.frames += 1;
            }
            None => {
                stream.ended = true;
                if let Some((last, _)) = &stream.last {
                    lock(cache).link(key, *last, AFTER_LAST);
                }
            }
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

    fn key(name: usize) -> Arc<StreamKey> {
        Arc::new(StreamKey {
            path: format!("/clips/{name}.mp4").into(),
            modified: std::time::SystemTime::UNIX_EPOCH,
            width: 32,
            height: 16,
        })
    }

    fn kept(files: &Files) -> Vec<usize> {
        let mut kept: Vec<usize> = files
            .readers
            .keys()
            .map(|key| {
                key.path
                    .file_stem()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .parse()
                    .unwrap()
            })
            .collect();
        kept.sort_unstable();
        kept
    }

    #[test]
    fn files_asked_for_lately_keep_their_processes() {
        let start = Instant::now();
        let at = |seconds: u64| start + Duration::from_secs(seconds);
        let mut files = Files::default();
        // Eight files asked for in turn, over and over: none stops.
        for second in 0..3 {
            for name in 0..8 {
                files.readers(&key(name), at(second) + Duration::from_millis(name as u64));
            }
        }
        assert_eq!(kept(&files), (0..8).collect::<Vec<_>>());
        // Asked for from kept frames counts too.
        files.used(&key(0), at(9));
        // A while later, a new file: those not asked for lately stop, the
        // least recently asked for first, down to FILES.
        files.readers(&key(8), at(10));
        assert_eq!(kept(&files), [0, 4, 5, 6, 7, 8]);
        // Much later, another new file takes the place of the least recently
        // asked for only: FILES stay, even when none was asked for lately.
        files.readers(&key(9), at(100));
        assert_eq!(kept(&files), [0, 5, 6, 7, 8, 9]);
    }

    #[test]
    fn past_the_ceiling_the_least_recently_asked_for_stops() {
        let start = Instant::now();
        let mut files = Files::default();
        for name in 0..MAX_FILES + 2 {
            files.readers(&key(name), start + Duration::from_millis(name as u64));
        }
        assert_eq!(files.readers.len(), MAX_FILES);
        assert!(!kept(&files).contains(&0) && !kept(&files).contains(&1));
        assert!(kept(&files).contains(&(MAX_FILES + 1)));
    }

    #[test]
    fn small_boxes_get_frames_halved_as_often_as_they_still_cover() {
        let hd = (1920, 1080);
        assert_eq!(frame_size(hd, 1920.0, 1080.0), (1920, 1080));
        assert_eq!(frame_size(hd, 3840.0, 2160.0), (1920, 1080), "never larger");
        assert_eq!(frame_size(hd, 961.0, 100.0), (1920, 1080));
        assert_eq!(frame_size(hd, 960.0, 100.0), (960, 540));
        // A box of another shape that the video covers: the larger side counts.
        assert_eq!(frame_size(hd, 100.0, 540.0), (960, 540));
        assert_eq!(frame_size(hd, 400.0, 200.0), (480, 270));
        assert_eq!(frame_size(hd, 1.0, 1.0), (120, 68), "at most 16 times");
        assert_eq!(frame_size((15, 9), 2.0, 2.0), (4, 3), "rounded up");
        assert_eq!(frame_size(hd, f64::NAN, 0.0), (1920, 1080));
        assert_eq!(frame_size(hd, 0.0, 0.0), (1920, 1080));
    }
}
