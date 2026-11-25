//! The frames a [`VideoDecoder`](super::VideoDecoder) has read, kept within
//! a memory budget, and which moments each one covers.
//!
//! A frame shows from its own timestamp until the next frame's, so a
//! frame is only found for a time once the frame after it has been read
//! (or the file is known to end after it). Before the first frame of a
//! file, the first frame shows.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use moshionly_render::skia_safe as sk;

/// One way of reading a file's frames: the file, as it was when it was
/// read, at one size.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct StreamKey {
    /// The file.
    pub path: PathBuf,
    /// When the file was last changed.
    pub modified: SystemTime,
    /// Width of the frames read, in pixels.
    pub width: u32,
    /// Height of the frames read, in pixels.
    pub height: u32,
}

/// The `next` of the last frame of a file: it shows from then on.
pub(crate) const AFTER_LAST: i64 = i64::MAX;

/// Frames read, by file and size, then by when they start in
/// microseconds.
pub(crate) struct FrameCache {
    files: HashMap<Arc<StreamKey>, Frames>,
    /// Frames holding pictures, least recently used first.
    order: BTreeMap<u64, (Arc<StreamKey>, i64)>,
    bytes: usize,
    budget: usize,
    clock: u64,
}

#[derive(Default)]
struct Frames {
    slots: BTreeMap<i64, Slot>,
    /// When the file's first frame starts, once it is known.
    first: Option<i64>,
}

#[derive(Default)]
struct Slot {
    /// When the next frame starts, once it is known.
    next: Option<i64>,
    /// The picture, and when it was last used; `None` once it made room.
    image: Option<(sk::Image, u64)>,
}

impl FrameCache {
    /// An empty cache keeping at most `budget` bytes of pictures.
    pub fn new(budget: usize) -> Self {
        Self {
            files: HashMap::new(),
            order: BTreeMap::new(),
            bytes: 0,
            budget,
            clock: 0,
        }
    }

    /// Memory the kept pictures take, in bytes.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// The kept frame that shows at `time` microseconds, if it is known.
    pub fn lookup(&mut self, key: &StreamKey, time: i64) -> Option<sk::Image> {
        let frames = self.files.get_mut(key)?;
        let start = match frames.first {
            Some(first) if time < first => first,
            _ => {
                let (start, slot) = frames.slots.range(..=time).next_back()?;
                if !slot.next.is_some_and(|next| time < next) {
                    return None;
                }
                *start
            }
        };
        let (image, used) = frames.slots.get_mut(&start)?.image.as_mut()?;
        self.clock += 1;
        if let Some(entry) = self.order.remove(used) {
            self.order.insert(self.clock, entry);
        }
        *used = self.clock;
        Some(image.clone())
    }

    /// Keeps the frame starting at `start`, making room for it.
    pub fn keep(&mut self, key: &Arc<StreamKey>, start: i64, image: sk::Image) {
        self.clock += 1;
        let clock = self.clock;
        let slot = self
            .files
            .entry(Arc::clone(key))
            .or_default()
            .slots
            .entry(start)
            .or_default();
        if let Some((old, used)) = slot.image.take() {
            self.bytes -= bytes_of(&old);
            self.order.remove(&used);
        }
        self.bytes += bytes_of(&image);
        slot.image = Some((image, clock));
        self.order.insert(clock, (Arc::clone(key), start));
        while self.bytes > self.budget {
            let Some((&oldest, _)) = self.order.first_key_value() else {
                break;
            };
            if oldest == clock {
                // The frame just kept stays, even when it alone is larger.
                break;
            }
            if let Some((file, start)) = self.order.remove(&oldest)
                && let Some(slot) = self
                    .files
                    .get_mut(&file)
                    .and_then(|frames| frames.slots.get_mut(&start))
                && let Some((image, _)) = slot.image.take()
            {
                self.bytes -= bytes_of(&image);
            }
        }
    }

    /// Says that the frame starting at `start` shows until `next`
    /// ([`AFTER_LAST`] for the last frame of the file).
    pub fn link(&mut self, key: &Arc<StreamKey>, start: i64, next: i64) {
        self.files
            .entry(Arc::clone(key))
            .or_default()
            .slots
            .entry(start)
            .or_default()
            .next = Some(next);
    }

    /// Says that the file's first frame starts at `start`.
    pub fn first(&mut self, key: &Arc<StreamKey>, start: i64) {
        self.files.entry(Arc::clone(key)).or_default().first = Some(start);
    }
}

/// Memory a picture takes.
fn bytes_of(image: &sk::Image) -> usize {
    image.width().max(0) as usize * image.height().max(0) as usize * 4
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(width: u32) -> Arc<StreamKey> {
        Arc::new(StreamKey {
            path: "/clips/a.mp4".into(),
            modified: SystemTime::UNIX_EPOCH,
            width,
            height: 1,
        })
    }

    /// A `width` by 1 opaque picture whose red is `mark`.
    fn picture(width: u32, mark: u8) -> sk::Image {
        let pixels = [mark, 0, 0, 255].repeat(width as usize);
        moshionly_render::video::image_from_rgba(width, 1, pixels).unwrap()
    }

    fn mark(image: sk::Image) -> u8 {
        let mut pixel = [0u8; 4];
        let info = sk::ImageInfo::new((1, 1), sk::ColorType::RGBA8888, sk::AlphaType::Premul, None);
        image.read_pixels(&info, &mut pixel, 4, (0, 0), sk::image::CachingHint::Allow);
        pixel[0]
    }

    #[test]
    fn a_frame_covers_the_time_until_the_next_one() {
        let mut cache = FrameCache::new(1 << 20);
        let file = key(2);
        cache.keep(&file, 0, picture(2, 10));
        cache.keep(&file, 100, picture(2, 20));
        // Until the frame after it is known, a frame covers nothing.
        assert!(cache.lookup(&file, 50).is_none());
        cache.link(&file, 0, 100);
        assert_eq!(cache.lookup(&file, 0).map(mark), Some(10));
        assert_eq!(cache.lookup(&file, 99).map(mark), Some(10));
        assert!(cache.lookup(&file, 100).is_none());
        cache.link(&file, 100, AFTER_LAST);
        assert_eq!(cache.lookup(&file, 100).map(mark), Some(20));
        assert_eq!(cache.lookup(&file, 1_000_000).map(mark), Some(20));
        // Before the first frame, once it is known to be the first.
        assert!(cache.lookup(&file, -5).is_none());
        cache.first(&file, 0);
        assert_eq!(cache.lookup(&file, -5).map(mark), Some(10));
        // Another size of the same file is another set of frames.
        assert!(cache.lookup(&key(4), 50).is_none());
    }

    #[test]
    fn the_least_recently_used_pictures_make_room() {
        // Room for two 2 by 1 pictures.
        let mut cache = FrameCache::new(16);
        let file = key(2);
        for (start, mark) in [(0, 1), (10, 2), (20, 3)] {
            cache.keep(&file, start, picture(2, mark));
            cache.link(&file, start, start + 10);
            if start == 10 {
                // Using the first keeps it; the second goes instead.
                assert!(cache.lookup(&file, 5).is_some());
            }
        }
        assert_eq!(cache.bytes(), 16);
        assert_eq!(cache.lookup(&file, 5).map(mark), Some(1));
        assert!(cache.lookup(&file, 15).is_none());
        assert_eq!(cache.lookup(&file, 25).map(mark), Some(3));
        // A picture larger than the budget is still kept, alone.
        cache.keep(&file, 30, picture(8, 4));
        assert_eq!(cache.bytes(), 32);
        assert!(cache.lookup(&file, 5).is_none());
    }
}
