//! Images: finding, decoding and keeping them.
//!
//! An image layer's `source` is a path relative to the project file, an
//! absolute path or a `data:` URI; core's
//! [`moshionly_core::io::resolve_asset`] says which. The
//! [`AssetLoader`] reads and decodes the image once and keeps the decoded
//! pixels, up to a memory budget, so frames after the first cost nothing.
//! A file that changes on disk is read again. A kept image is found by a
//! short sample of its source, so a `data:` URI megabytes long is not
//! hashed whole on every frame.
//!
//! Decoding uses the `image` crate, which is the same code on every
//! platform. Photos are turned upright by their orientation tag.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::SystemTime;

use moshionly_core::io::{Asset, resolve_asset};
use skia_safe as sk;

use crate::frame;

/// How much memory decoded images may take by default: 256 MB.
pub const DEFAULT_BUDGET: usize = 256 << 20;

/// The longest side of an image that is decoded.
const MAX_SIDE: u32 = 16_384;

/// The most pixels an image that is decoded may have: as many as the
/// largest frame, 512 MB once decoded. A small file can claim far more,
/// so the size is checked before any pixel is decoded.
const MAX_PIXELS: u64 = frame::MAX_FRAME_PIXELS;

/// The largest image file that is read, in bytes: 256 MB.
const MAX_FILE_BYTES: u64 = 256 << 20;

/// Finds, decodes and keeps the images of image layers.
///
/// Cheap to clone; clones share what they keep. It can be used from any
/// thread.
#[derive(Clone)]
pub struct AssetLoader {
    shared: Arc<Mutex<Store>>,
}

impl std::fmt::Debug for AssetLoader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let store = self.store();
        f.debug_struct("AssetLoader")
            .field("project_file", &store.project_file)
            .field("images", &store.images.len())
            .field("bytes", &store.bytes)
            .field("budget", &store.budget)
            .finish()
    }
}

impl Default for AssetLoader {
    /// A loader for a project that has not been saved: relative paths are
    /// taken as they are.
    fn default() -> Self {
        Self::new(None)
    }
}

/// A decoded image and what it was decoded from.
#[derive(Clone)]
struct Entry {
    /// The source the image was decoded from.
    source: String,
    image: sk::Image,
    /// For files, when the file was last changed, to notice edits.
    modified: Option<SystemTime>,
    /// The memory the image and its source take.
    bytes: usize,
    used: u64,
}

/// What a kept image is found by: the length of its source and a hash of a
/// sample of it. See [`key`].
type Key = (usize, u64);

/// The key of `source`. A `data:` URI can be megabytes long, and hashing
/// all of it would cost more on every frame than drawing the image, so
/// only its start, its end and a few short runs between are hashed. The
/// source kept with the image is compared whole to confirm a match.
fn key(source: &str) -> Key {
    const RUN: usize = 64;
    const RUNS: usize = 64;
    let bytes = source.as_bytes();
    let mut hasher = DefaultHasher::new();
    if bytes.len() <= RUN * (RUNS + 2) {
        bytes.hash(&mut hasher);
    } else {
        let step = (bytes.len() - RUN) / (RUNS + 1);
        for run in 0..=RUNS + 1 {
            let start = (run * step).min(bytes.len() - RUN);
            bytes[start..start + RUN].hash(&mut hasher);
        }
        bytes[bytes.len() - RUN..].hash(&mut hasher);
    }
    (bytes.len(), hasher.finish())
}

struct Store {
    project_file: Option<PathBuf>,
    images: HashMap<Key, Entry>,
    bytes: usize,
    budget: usize,
    clock: u64,
}

impl AssetLoader {
    /// A loader that finds relative paths next to `project_file`, the
    /// `.moshi` file the images belong to.
    pub fn new(project_file: Option<PathBuf>) -> Self {
        Self {
            shared: Arc::new(Mutex::new(Store {
                project_file,
                images: HashMap::new(),
                bytes: 0,
                budget: DEFAULT_BUDGET,
                clock: 0,
            })),
        }
    }

    /// The same loader keeping at most `bytes` of decoded images. The image
    /// in use is always kept, even when it alone is larger.
    pub fn with_budget(self, bytes: usize) -> Self {
        {
            let mut store = self.store();
            store.budget = bytes;
            store.trim(None);
        }
        self
    }

    /// Changes the project file relative paths are found next to, for
    /// example after "Save as". Kept images are forgotten.
    pub fn set_project_file(&self, project_file: Option<PathBuf>) {
        let mut store = self.store();
        store.project_file = project_file;
        store.images.clear();
        store.bytes = 0;
    }

    /// Forgets every kept image.
    pub fn clear(&self) {
        let mut store = self.store();
        store.images.clear();
        store.bytes = 0;
    }

    /// Memory taken by kept images, in bytes.
    pub fn memory_used(&self) -> usize {
        self.store().bytes
    }

    /// The width and height in pixels of the image a `source` refers to,
    /// upright.
    pub fn natural_size(&self, source: &str) -> Result<(u32, u32), String> {
        let image = self.load(source)?;
        Ok((image.width() as u32, image.height() as u32))
    }

    /// The decoded image for `source`, or why there is none, in words for
    /// a person.
    pub(crate) fn load(&self, source: &str) -> Result<sk::Image, String> {
        // A `data:` URI is its own content: look it up before decoding its
        // text. A file is looked up by when it last changed.
        let embedded = source
            .trim_start()
            .get(..5)
            .is_some_and(|scheme| scheme.eq_ignore_ascii_case("data:"));
        if embedded && let Some(image) = self.store().kept(source, None) {
            return Ok(image);
        }
        let project_file = self.store().project_file.clone();
        let asset = resolve_asset(source, project_file.as_deref()).map_err(|e| e.to_string())?;
        let (bytes, modified) = match asset {
            Asset::File(path) => {
                let modified = modified(&path)?;
                if let Some(image) = self.store().kept(source, Some(modified)) {
                    return Ok(image);
                }
                (read_file(&path, MAX_FILE_BYTES)?, Some(modified))
            }
            Asset::Data { bytes, .. } => (bytes, None),
        };
        // Decoding happens without the lock, so other threads can draw
        // images that are kept meanwhile.
        let image = decode(&bytes)?;
        self.store().keep(source, image.clone(), modified);
        Ok(image)
    }

    fn store(&self) -> MutexGuard<'_, Store> {
        self.shared.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Store {
    /// The kept image for `source`, if it was decoded from the same
    /// version of the file.
    fn kept(&mut self, source: &str, modified: Option<SystemTime>) -> Option<sk::Image> {
        self.clock += 1;
        let clock = self.clock;
        let entry = self.images.get_mut(&key(source))?;
        if entry.modified != modified || entry.source != source {
            return None;
        }
        entry.used = clock;
        Some(entry.image.clone())
    }

    /// Keeps a decoded image, making room for it. Another source with the
    /// same key gives way.
    fn keep(&mut self, source: &str, image: sk::Image, modified: Option<SystemTime>) {
        self.clock += 1;
        let bytes = image_bytes(&image) + source.len();
        let entry = Entry {
            source: source.to_owned(),
            image,
            modified,
            bytes,
            used: self.clock,
        };
        let key = key(source);
        if let Some(old) = self.images.insert(key, entry) {
            self.bytes -= old.bytes;
        }
        self.bytes += bytes;
        self.trim(Some(key));
    }

    /// Forgets the least recently used images until the rest fit the
    /// budget, keeping `keep`.
    fn trim(&mut self, keep: Option<Key>) {
        while self.bytes > self.budget {
            let oldest = self
                .images
                .iter()
                .filter(|(key, _)| Some(**key) != keep)
                .min_by_key(|(_, entry)| entry.used)
                .map(|(key, _)| *key);
            let Some(oldest) = oldest else {
                break;
            };
            if let Some(entry) = self.images.remove(&oldest) {
                self.bytes -= entry.bytes;
            }
        }
    }
}

/// When a file was last changed. Anything but a plain file, such as a
/// folder, a device or a pipe, is refused here, before it is opened:
/// reading `/dev/zero` never ends, and opening a pipe can wait forever.
fn modified(path: &Path) -> Result<SystemTime, String> {
    let metadata = std::fs::metadata(path).map_err(|error| describe_io(path, &error))?;
    if !metadata.is_file() {
        return Err(format!("{} is not an image file", path.display()));
    }
    metadata
        .modified()
        .map_err(|error| describe_io(path, &error))
}

/// The bytes of the file at `path`, refused when it is not a plain file or
/// holds more than `limit` bytes. The limit holds while reading too, in
/// case the file changes after it was looked at.
fn read_file(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let too_large = || {
        format!(
            "{} is larger than an image file may be ({} MB)",
            path.display(),
            limit >> 20
        )
    };
    let file = std::fs::File::open(path).map_err(|error| describe_io(path, &error))?;
    let metadata = file.metadata().map_err(|error| describe_io(path, &error))?;
    if !metadata.is_file() {
        return Err(format!("{} is not an image file", path.display()));
    }
    if metadata.len() > limit {
        return Err(too_large());
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| describe_io(path, &error))?;
    if bytes.len() as u64 > limit {
        return Err(too_large());
    }
    Ok(bytes)
}

fn describe_io(path: &Path, error: &std::io::Error) -> String {
    match error.kind() {
        std::io::ErrorKind::NotFound => format!("there is no file at {}", path.display()),
        _ => format!("{} could not be read: {error}", path.display()),
    }
}

/// Memory a decoded image takes, with its smaller copies for drawing it
/// small.
fn image_bytes(image: &sk::Image) -> usize {
    image.width() as usize * image.height() as usize * frame::BYTES_PER_PIXEL * 4 / 3
}

/// Decodes an image file's bytes into an upright, premultiplied Skia image
/// with smaller copies for drawing it small.
fn decode(bytes: &[u8]) -> Result<sk::Image, String> {
    use image::ImageDecoder;

    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| error.to_string())?;
    if reader.format().is_none() {
        return Err(
            "the data is not an image in a known format (PNG, JPEG, GIF, WebP, BMP)".into(),
        );
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(MAX_PIXELS * frame::BYTES_PER_PIXEL as u64);
    reader.limits(limits);
    let mut decoder = reader
        .into_decoder()
        .map_err(|error| format!("the image cannot be decoded: {error}"))?;
    let (width, height) = decoder.dimensions();
    if u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err(format!(
            "the image is {width} by {height} pixels, more than the {} million an image may have",
            MAX_PIXELS / 1_000_000
        ));
    }
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut decoded = image::DynamicImage::from_decoder(decoder)
        .map_err(|error| format!("the image cannot be decoded: {error}"))?;
    decoded.apply_orientation(orientation);
    let rgba = decoded.into_rgba8();
    let (width, height) = rgba.dimensions();
    let mut pixels = rgba.into_raw();
    frame::premultiply(&mut pixels);
    let info = sk::ImageInfo::new(
        (width as i32, height as i32),
        sk::ColorType::RGBA8888,
        sk::AlphaType::Premul,
        None,
    );
    // Skia takes the pixels over, so they are never held twice.
    let image = sk::images::raster_from_data(
        &info,
        sk::Data::new_with_owner(pixels),
        width as usize * frame::BYTES_PER_PIXEL,
    )
    .ok_or("the image is too large to draw")?;
    Ok(image.with_default_mipmaps().unwrap_or(image))
}

/// A source short enough to show in a message: `data:` URIs can be
/// megabytes long.
pub(crate) fn shorten(source: &str) -> String {
    const LIMIT: usize = 60;
    if source.len() <= LIMIT {
        return source.to_owned();
    }
    let cut = (0..=LIMIT)
        .rev()
        .find(|at| source.is_char_boundary(*at))
        .unwrap_or(0);
    format!("{}…", &source[..cut])
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use moshionly_core::io::data_uri;

    /// A PNG file's bytes: `width` by `height`, each pixel from `colour`.
    pub(crate) fn png(width: u32, height: u32, colour: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
        let mut pixels = Vec::new();
        for y in 0..height {
            for x in 0..width {
                pixels.extend(colour(x, y));
            }
        }
        crate::Frame::from_unpremultiplied(width, height, pixels)
            .unwrap()
            .to_png()
            .unwrap()
    }

    fn folder(name: &str) -> PathBuf {
        let folder =
            std::env::temp_dir().join(format!("moshionly-render-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        folder
    }

    #[test]
    fn data_uris_relative_and_absolute_paths_load() {
        let loader = AssetLoader::default();
        let uri = data_uri("image/png", &png(3, 2, |_, _| [255, 0, 0, 255]));
        assert_eq!(loader.natural_size(&uri), Ok((3, 2)));

        let folder = folder("paths");
        std::fs::write(folder.join("dot.png"), png(5, 4, |_, _| [0, 0, 0, 255])).unwrap();
        let project = AssetLoader::new(Some(folder.join("project.moshi")));
        assert_eq!(project.natural_size("dot.png"), Ok((5, 4)));
        let absolute = folder.join("dot.png");
        assert_eq!(loader.natural_size(absolute.to_str().unwrap()), Ok((5, 4)));
        // Without a project file, a relative path is not found.
        assert!(loader.natural_size("dot.png").is_err());
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn problems_are_explained() {
        let loader = AssetLoader::default();
        let missing = loader.natural_size("/no/such/picture.png").unwrap_err();
        assert!(missing.contains("no file"), "{missing}");
        let link = loader
            .natural_size("https://example.com/a.png")
            .unwrap_err();
        assert!(link.contains("link"), "{link}");
        let junk = loader
            .natural_size(&data_uri("image/png", b"not an image"))
            .unwrap_err();
        assert!(junk.contains("not an image"), "{junk}");
        assert!(loader.natural_size("").is_err());
    }

    #[test]
    fn only_plain_files_of_a_sensible_size_are_read() {
        let folder = folder("plain");
        let loader = AssetLoader::default();
        let error = loader.natural_size(folder.to_str().unwrap()).unwrap_err();
        assert!(error.ends_with("is not an image file"), "{error}");
        // A device that never ends is refused before it is opened.
        #[cfg(unix)]
        {
            let error = loader.natural_size("/dev/zero").unwrap_err();
            assert!(error.ends_with("is not an image file"), "{error}");
        }

        let path = folder.join("big.png");
        std::fs::write(&path, [0u8; 20]).unwrap();
        assert_eq!(read_file(&path, 20).unwrap().len(), 20);
        let error = read_file(&path, 19).unwrap_err();
        assert!(
            error.contains("larger than an image file may be"),
            "{error}"
        );
        std::fs::remove_dir_all(folder).unwrap();
    }

    /// The header of an uncompressed BMP file of `width` by `height`
    /// pixels, with none of its pixels.
    fn bmp_header(width: i32, height: i32) -> Vec<u8> {
        let mut bytes = b"BM".to_vec();
        bytes.extend(54u32.to_le_bytes()); // file size
        bytes.extend([0; 4]);
        bytes.extend(54u32.to_le_bytes()); // where the pixels start
        bytes.extend(40u32.to_le_bytes()); // size of this header
        bytes.extend(width.to_le_bytes());
        bytes.extend(height.to_le_bytes());
        bytes.extend(1u16.to_le_bytes()); // planes
        bytes.extend(24u16.to_le_bytes()); // bits per pixel
        bytes.extend([0; 24]);
        bytes
    }

    #[test]
    fn an_image_with_too_many_pixels_is_refused_before_decoding() {
        let loader = AssetLoader::default();
        // 54 bytes that claim 144 million pixels.
        let error = loader
            .natural_size(&data_uri("image/bmp", &bmp_header(12_000, 12_000)))
            .unwrap_err();
        assert_eq!(
            error,
            "the image is 12000 by 12000 pixels, more than the 134 million an image may have"
        );
        // A header of a sensible size gets as far as its missing pixels.
        let error = loader
            .natural_size(&data_uri("image/bmp", &bmp_header(4, 4)))
            .unwrap_err();
        assert!(error.starts_with("the image cannot be decoded"), "{error}");
    }

    #[test]
    fn images_are_kept_within_the_budget() {
        let loader = AssetLoader::default();
        // 10 by 10 is 400 bytes, 533 with the smaller copies, and the
        // source is kept too.
        let a = data_uri("image/png", &png(10, 10, |_, _| [1, 2, 3, 255]));
        let b = data_uri("image/png", &png(10, 10, |_, _| [4, 5, 6, 255]));
        let first = loader.load(&a).unwrap();
        assert_eq!(loader.memory_used(), 533 + a.len());
        assert_eq!(
            loader.load(&a).unwrap().unique_id(),
            first.unique_id(),
            "kept"
        );
        let loader = loader.with_budget(600);
        loader.load(&b).unwrap();
        assert_eq!(
            loader.memory_used(),
            533 + b.len(),
            "the older image made room"
        );
        assert_ne!(loader.load(&a).unwrap().unique_id(), first.unique_id());
        loader.clear();
        assert_eq!(loader.memory_used(), 0);
    }

    #[test]
    fn long_sources_that_differ_only_between_their_samples_are_told_apart() {
        let loader = AssetLoader::default();
        // The same length, start and end; the images inside differ.
        let source = |width, height| {
            let uri = data_uri("image/png", &png(width, height, |_, _| [9, 9, 9, 255]));
            format!("{}{uri}{}", " ".repeat(20_000), " ".repeat(20_000))
        };
        let (wide, tall) = (source(4, 2), source(2, 4));
        assert_eq!(wide.len(), tall.len());
        assert_eq!(key(&wide), key(&tall), "the samples miss the middle");
        assert_eq!(loader.natural_size(&wide), Ok((4, 2)));
        assert_eq!(loader.natural_size(&tall), Ok((2, 4)));
        assert_eq!(loader.natural_size(&wide), Ok((4, 2)));
        assert_ne!(key(&wide), key(&wide.replacen(' ', "x", 1)));
    }

    #[test]
    fn a_changed_file_is_read_again() {
        let folder = folder("changed");
        let path = folder.join("grow.png");
        std::fs::write(&path, png(2, 2, |_, _| [0, 0, 0, 255])).unwrap();
        let loader = AssetLoader::new(Some(folder.join("p.moshi")));
        assert_eq!(loader.natural_size("grow.png"), Ok((2, 2)));
        std::fs::write(&path, png(6, 3, |_, _| [0, 0, 0, 255])).unwrap();
        // Some file systems only keep whole seconds; make the change show.
        let later = SystemTime::now() + std::time::Duration::from_secs(5);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .and_then(|file| file.set_modified(later))
            .unwrap();
        assert_eq!(loader.natural_size("grow.png"), Ok((6, 3)));
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn pixels_come_out_premultiplied() {
        let loader = AssetLoader::default();
        let uri = data_uri("image/png", &png(1, 1, |_, _| [255, 0, 0, 128]));
        let image = loader.load(&uri).unwrap();
        let mut pixel = [0u8; 4];
        let info = sk::ImageInfo::new((1, 1), sk::ColorType::RGBA8888, sk::AlphaType::Premul, None);
        assert!(image.read_pixels(&info, &mut pixel, 4, (0, 0), sk::image::CachingHint::Allow));
        assert_eq!(pixel, [128, 0, 0, 128]);
    }

    #[test]
    fn long_sources_are_shortened_for_messages() {
        assert_eq!(shorten("cat.png"), "cat.png");
        let long = format!("data:image/png;base64,{}", "é".repeat(100));
        let short = shorten(&long);
        assert!(short.ends_with('…') && short.len() < 70, "{short}");
    }
}
