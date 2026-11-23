//! The kinds of file an export can write, and what each of them can hold.

use std::fmt;

/// A kind of file an export writes.
///
/// Raster formats are made of rendered frames. Lottie, which describes the
/// animation as vectors instead of pixels, will join this list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    /// One frame as a PNG image: a moment of the animation, or the design at
    /// rest.
    Png,
    /// Every frame as a numbered PNG image in a folder, for editing software
    /// that imports image sequences.
    PngSequence,
    /// An animated GIF: plays everywhere, up to 256 colours a frame, and
    /// pixels that are either fully see-through or fully solid.
    Gif,
    /// An MP4 video with H.264: the format for social media, slides and
    /// websites. Never transparent.
    Mp4,
    /// A WebM video with VP9, which can be transparent: for websites.
    /// Needs `ffmpeg`.
    WebM,
    /// A QuickTime video with ProRes 4444, which can be transparent: for
    /// video editing software. Needs `ffmpeg`.
    ProRes,
}

impl Format {
    /// Every format, in the order a menu lists them.
    pub const ALL: [Format; 6] = [
        Format::Png,
        Format::PngSequence,
        Format::Gif,
        Format::Mp4,
        Format::WebM,
        Format::ProRes,
    ];

    /// The short name tools and the command line use, such as `png_sequence`.
    pub fn as_str(self) -> &'static str {
        match self {
            Format::Png => "png",
            Format::PngSequence => "png_sequence",
            Format::Gif => "gif",
            Format::Mp4 => "mp4",
            Format::WebM => "webm",
            Format::ProRes => "prores",
        }
    }

    /// Looks a format up by its [short name](Self::as_str), ignoring case.
    /// The file extensions `mov` and `apng` are not names; `mov` is
    /// accepted for ProRes since that is the only QuickTime format.
    pub fn parse(name: &str) -> Option<Format> {
        let name = name.trim().to_ascii_lowercase();
        let name = name.replace(['-', ' '], "_");
        match name.as_str() {
            "mov" => Some(Format::ProRes),
            "pngs" | "png_seq" => Some(Format::PngSequence),
            _ => Self::ALL.into_iter().find(|format| format.as_str() == name),
        }
    }

    /// The name people read, such as "ProRes 4444".
    pub fn label(self) -> &'static str {
        match self {
            Format::Png => "PNG",
            Format::PngSequence => "PNG sequence",
            Format::Gif => "GIF",
            Format::Mp4 => "MP4",
            Format::WebM => "WebM",
            Format::ProRes => "ProRes 4444",
        }
    }

    /// The file extension, without the dot. `None` for a PNG sequence,
    /// which is written as a folder.
    pub fn extension(self) -> Option<&'static str> {
        match self {
            Format::Png => Some("png"),
            Format::PngSequence => None,
            Format::Gif => Some("gif"),
            Format::Mp4 => Some("mp4"),
            Format::WebM => Some("webm"),
            Format::ProRes => Some("mov"),
        }
    }

    /// Whether the export is a folder of files rather than one file.
    pub fn writes_folder(self) -> bool {
        self == Format::PngSequence
    }

    /// Whether the format holds movement: every format but a single PNG.
    pub fn is_animated(self) -> bool {
        self != Format::Png
    }

    /// Whether the format can keep see-through parts see-through. A GIF
    /// can, but only fully: partly transparent pixels become either solid
    /// or invisible.
    pub fn supports_transparency(self) -> bool {
        self != Format::Mp4
    }

    /// Whether the format is written by running `ffmpeg`, which must then
    /// be installed.
    pub fn needs_ffmpeg(self) -> bool {
        matches!(self, Format::WebM | Format::ProRes)
    }

    /// Whether the video codec needs an even width and height. Sizes are
    /// rounded down to even numbers for these formats.
    pub fn needs_even_size(self) -> bool {
        matches!(self, Format::Mp4 | Format::WebM | Format::ProRes)
    }

    /// The smallest and largest frames the format is written at.
    pub fn size_limits(self) -> SizeLimits {
        use moshionly_render::frame::{MAX_FRAME_PIXELS, MAX_FRAME_SIDE};
        match self {
            Format::Png | Format::PngSequence => SizeLimits {
                min_side: 1,
                max_long_side: MAX_FRAME_SIDE,
                max_short_side: MAX_FRAME_SIDE,
                max_pixels: MAX_FRAME_PIXELS,
            },
            // Larger GIFs exist, but they are tens of megabytes a second and
            // many viewers give up on them.
            Format::Gif => SizeLimits {
                min_side: 1,
                max_long_side: 4096,
                max_short_side: 4096,
                max_pixels: 4096 * 4096,
            },
            // The limit of the H.264 encoder: 4K in either orientation.
            Format::Mp4 => SizeLimits {
                min_side: 16,
                max_long_side: 3840,
                max_short_side: 2160,
                max_pixels: 3840 * 2160,
            },
            Format::WebM | Format::ProRes => SizeLimits {
                min_side: 2,
                max_long_side: 8192,
                max_short_side: 8192,
                max_pixels: 8192 * 4320,
            },
        }
    }

    /// The highest frame rate the format is written at, in frames per
    /// second. GIF frame delays are whole hundredths of a second and most
    /// viewers slow down delays below two hundredths, so GIFs stop at 50.
    pub fn max_fps(self) -> f64 {
        match self {
            Format::Gif => 50.0,
            _ => 240.0,
        }
    }
}

impl fmt::Display for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// The frame sizes a format can be written at, in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SizeLimits {
    /// Neither side may be shorter.
    pub min_side: u32,
    /// The longer side may be at most this long.
    pub max_long_side: u32,
    /// The shorter side may be at most this long.
    pub max_short_side: u32,
    /// Width times height may be at most this.
    pub max_pixels: u64,
}

impl SizeLimits {
    /// Whether a frame of `width` by `height` pixels is within the limits.
    pub fn allows(&self, width: u32, height: u32) -> bool {
        let (long, short) = (width.max(height), width.min(height));
        short >= self.min_side
            && long <= self.max_long_side
            && short <= self.max_short_side
            && u64::from(width) * u64::from(height) <= self.max_pixels
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip() {
        for format in Format::ALL {
            assert_eq!(Format::parse(format.as_str()), Some(format));
            assert_eq!(Format::parse(&format.as_str().to_uppercase()), Some(format));
        }
        assert_eq!(Format::parse("PNG sequence"), Some(Format::PngSequence));
        assert_eq!(Format::parse("png-sequence"), Some(Format::PngSequence));
        assert_eq!(Format::parse("mov"), Some(Format::ProRes));
        assert_eq!(Format::parse("WebM"), Some(Format::WebM));
        assert_eq!(Format::parse("lottie"), None);
        assert_eq!(Format::parse("jpeg"), None);
    }

    #[test]
    fn what_each_format_can_hold() {
        assert!(!Format::Mp4.supports_transparency());
        assert!(Format::Gif.supports_transparency());
        assert!(Format::WebM.needs_ffmpeg() && Format::ProRes.needs_ffmpeg());
        assert!(!Format::Mp4.needs_ffmpeg());
        assert!(Format::PngSequence.writes_folder());
        assert_eq!(Format::PngSequence.extension(), None);
        assert_eq!(Format::ProRes.extension(), Some("mov"));
        assert!(!Format::Png.is_animated());
        assert_eq!(Format::Gif.max_fps(), 50.0);
    }

    #[test]
    fn size_limits_know_portrait_from_landscape() {
        let mp4 = Format::Mp4.size_limits();
        assert!(mp4.allows(3840, 2160));
        assert!(mp4.allows(2160, 3840));
        assert!(!mp4.allows(3840, 2162));
        assert!(!mp4.allows(3842, 100));
        assert!(!mp4.allows(8, 100));
        let png = Format::Png.size_limits();
        assert!(png.allows(1, 1));
        assert!(!png.allows(16_384, 16_384), "too many pixels in all");
    }
}
