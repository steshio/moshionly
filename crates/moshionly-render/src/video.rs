//! Video frames: where the renderer gets the pictures of video layers.
//!
//! The renderer does not decode video. A scene says which moment of which
//! file a video layer shows; the renderer asks a [`VideoFrames`] for the
//! frame and draws it like an image. The provider is installed on the
//! [`AssetLoader`](crate::AssetLoader) with
//! [`with_video`](crate::AssetLoader::with_video), so every renderer made
//! with that loader draws video. `moshionly-export` has one that reads
//! files with `ffmpeg`. Without a provider, or when it has no frame, a
//! placeholder is drawn and the frame carries a
//! [`RenderWarning::MissingVideo`](crate::RenderWarning::MissingVideo).

use std::path::Path;

use skia_safe as sk;

use crate::frame::{self, BYTES_PER_PIXEL};

/// Gives the frame of a video file at a moment of it.
///
/// Frames are asked for from several threads at once and in any order
/// (export renders frames in parallel), so the same request must give the
/// same picture whatever was asked before it.
pub trait VideoFrames: Send + Sync {
    /// The frame of `request.path` that shows at `request.time_ms`, or why
    /// there is none, in words for a person.
    fn frame(&self, request: &VideoRequest<'_>) -> Result<sk::Image, String>;
}

/// What the renderer asks a [`VideoFrames`] for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VideoRequest<'a> {
    /// The video file: the layer's `source` resolved next to the project
    /// file, absolute whenever the project's location is known.
    pub path: &'a Path,
    /// The moment of the file, in milliseconds from its start, 0 or more.
    pub time_ms: f64,
    /// About how many pixels wide the layer's box is where it is drawn,
    /// for a provider that reads smaller frames for small boxes. A frame
    /// never needs to be larger than the video itself.
    pub width: f64,
    /// About how many pixels high the layer's box is where it is drawn.
    pub height: f64,
}

/// A frame for a [`VideoFrames`] to hand out, from `width` by `height`
/// pixels of straight-alpha RGBA, four bytes per pixel, rows from top to
/// bottom. The pixels are taken over, not copied.
pub fn image_from_rgba(width: u32, height: u32, mut rgba: Vec<u8>) -> Result<sk::Image, String> {
    let expected = width as usize * height as usize * BYTES_PER_PIXEL;
    if width == 0 || height == 0 || rgba.len() != expected {
        return Err(format!(
            "a {width} by {height} frame needs {expected} bytes of pixels, and {} came",
            rgba.len()
        ));
    }
    frame::premultiply(&mut rgba);
    let info = sk::ImageInfo::new(
        (width as i32, height as i32),
        sk::ColorType::RGBA8888,
        sk::AlphaType::Premul,
        None,
    );
    sk::images::raster_from_data(
        &info,
        sk::Data::new_with_owner(rgba),
        width as usize * BYTES_PER_PIXEL,
    )
    .ok_or_else(|| "the frame is too large to draw".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixels_become_a_premultiplied_image() {
        let image = image_from_rgba(2, 1, vec![255, 0, 0, 255, 0, 0, 255, 128]).unwrap();
        assert_eq!((image.width(), image.height()), (2, 1));
        let mut pixels = [0u8; 8];
        let info = sk::ImageInfo::new((2, 1), sk::ColorType::RGBA8888, sk::AlphaType::Premul, None);
        assert!(image.read_pixels(&info, &mut pixels, 8, (0, 0), sk::image::CachingHint::Allow));
        assert_eq!(pixels, [255, 0, 0, 255, 0, 0, 128, 128]);
        assert!(image_from_rgba(2, 2, vec![0; 8]).is_err());
        assert!(image_from_rgba(0, 1, Vec::new()).is_err());
    }
}
