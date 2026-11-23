//! The frame: the pixels a render produces.
//!
//! A frame stores 8-bit RGBA with **premultiplied** alpha, which is what the
//! renderer draws into and what compositing and GPU textures want. File
//! formats and most image libraries want **unpremultiplied** (straight)
//! alpha instead. The two are different numbers for every pixel that is
//! partly transparent, so every accessor here says in its name which one it
//! hands out.

use std::fmt;
use std::io::Cursor;

use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::{ExtendedColorType, ImageEncoder};

use crate::error::{RenderError, RenderWarning};

/// Bytes per pixel: red, green, blue, alpha.
pub const BYTES_PER_PIXEL: usize = 4;

/// The longest side a frame may have, in pixels.
pub const MAX_FRAME_SIDE: u32 = 16_384;

/// The largest number of pixels a frame may have (about 134 million, which
/// is half a gigabyte of memory).
pub const MAX_FRAME_PIXELS: u64 = 1 << 27;

/// One rendered picture: `width` by `height` pixels of 8-bit RGBA, rows from
/// top to bottom, no padding between rows.
#[derive(Clone, PartialEq, Eq)]
pub struct Frame {
    width: u32,
    height: u32,
    /// Premultiplied RGBA.
    pixels: Vec<u8>,
    warnings: Vec<RenderWarning>,
}

impl fmt::Debug for Frame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Frame")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("warnings", &self.warnings)
            .finish_non_exhaustive()
    }
}

/// Checks a frame size and returns the number of bytes its pixels take.
pub(crate) fn byte_len(width: u32, height: u32) -> Result<usize, RenderError> {
    if width == 0 || height == 0 {
        return Err(RenderError::EmptyFrame {
            width: f64::from(width),
            height: f64::from(height),
        });
    }
    let pixels = u64::from(width) * u64::from(height);
    if width > MAX_FRAME_SIDE || height > MAX_FRAME_SIDE || pixels > MAX_FRAME_PIXELS {
        return Err(RenderError::FrameTooLarge {
            width: u64::from(width),
            height: u64::from(height),
            max_side: MAX_FRAME_SIDE,
            max_pixels: MAX_FRAME_PIXELS,
        });
    }
    Ok(pixels as usize * BYTES_PER_PIXEL)
}

impl Frame {
    /// A fully transparent frame.
    pub fn new(width: u32, height: u32) -> Result<Self, RenderError> {
        let len = byte_len(width, height)?;
        Ok(Self {
            width,
            height,
            pixels: vec![0; len],
            warnings: Vec::new(),
        })
    }

    /// Wraps premultiplied RGBA pixels, four bytes per pixel, rows from top
    /// to bottom.
    pub fn from_premultiplied(
        width: u32,
        height: u32,
        pixels: Vec<u8>,
    ) -> Result<Self, RenderError> {
        let expected = byte_len(width, height)?;
        if pixels.len() != expected {
            return Err(RenderError::BufferSize {
                width,
                height,
                expected,
                actual: pixels.len(),
            });
        }
        Ok(Self {
            width,
            height,
            pixels,
            warnings: Vec::new(),
        })
    }

    /// Wraps unpremultiplied (straight alpha) RGBA pixels, such as a decoded
    /// PNG, converting them to the premultiplied form a frame stores.
    pub fn from_unpremultiplied(
        width: u32,
        height: u32,
        mut pixels: Vec<u8>,
    ) -> Result<Self, RenderError> {
        premultiply(&mut pixels);
        Self::from_premultiplied(width, height, pixels)
    }

    /// Width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// The pixels as premultiplied RGBA: each colour channel is already
    /// multiplied by the pixel's alpha. This is the form to upload as a
    /// texture or to composite with.
    pub fn premultiplied(&self) -> &[u8] {
        &self.pixels
    }

    /// Gives up the frame for its premultiplied RGBA pixels, without copying.
    pub fn into_premultiplied(self) -> Vec<u8> {
        self.pixels
    }

    /// The pixels as unpremultiplied (straight alpha) RGBA: the form image
    /// files and most image libraries use. This converts, so keep the result
    /// when it is needed more than once.
    pub fn to_unpremultiplied(&self) -> Vec<u8> {
        let mut pixels = self.pixels.clone();
        unpremultiply(&mut pixels);
        pixels
    }

    /// One pixel as premultiplied `[r, g, b, a]`, or `None` outside the frame.
    pub fn premultiplied_pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let at = (y as usize * self.width as usize + x as usize) * BYTES_PER_PIXEL;
        self.pixels[at..at + BYTES_PER_PIXEL].try_into().ok()
    }

    /// One pixel as unpremultiplied `[r, g, b, a]`, or `None` outside the
    /// frame.
    pub fn unpremultiplied_pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        let mut pixel = self.premultiplied_pixel(x, y)?;
        unpremultiply(&mut pixel);
        Some(pixel)
    }

    /// Whether every pixel is fully opaque.
    pub fn is_opaque(&self) -> bool {
        let (pixels, _) = self.pixels.as_chunks::<BYTES_PER_PIXEL>();
        pixels.iter().all(|pixel| pixel[3] == 255)
    }

    /// What was drawn differently from what the scene asked for. Empty for a
    /// frame that is exactly what was asked.
    pub fn warnings(&self) -> &[RenderWarning] {
        &self.warnings
    }

    /// Encodes the frame as a PNG file with an alpha channel.
    pub fn to_png(&self) -> Result<Vec<u8>, RenderError> {
        let mut file = Vec::new();
        PngEncoder::new_with_quality(
            Cursor::new(&mut file),
            CompressionType::Default,
            FilterType::Adaptive,
        )
        .write_image(
            &self.to_unpremultiplied(),
            self.width,
            self.height,
            ExtendedColorType::Rgba8,
        )
        .map_err(|error| RenderError::Png(error.to_string()))?;
        Ok(file)
    }

    /// The pixels, for the renderer to draw into.
    pub(crate) fn premultiplied_mut(&mut self) -> &mut [u8] {
        &mut self.pixels
    }

    /// Attaches the warnings gathered while drawing.
    pub(crate) fn set_warnings(&mut self, warnings: Vec<RenderWarning>) {
        self.warnings = warnings;
    }
}

/// Converts RGBA pixels from unpremultiplied to premultiplied in place.
///
/// Each colour channel becomes `channel * alpha / 255`, rounded to nearest.
pub fn premultiply(pixels: &mut [u8]) {
    let (pixels, _) = pixels.as_chunks_mut::<BYTES_PER_PIXEL>();
    for pixel in pixels {
        let alpha = u32::from(pixel[3]);
        if alpha == 255 {
            continue;
        }
        for channel in &mut pixel[..3] {
            *channel = ((u32::from(*channel) * alpha + 127) / 255) as u8;
        }
    }
}

/// Converts RGBA pixels from premultiplied to unpremultiplied in place.
///
/// Each colour channel becomes `channel * 255 / alpha`, rounded to nearest.
/// Fully transparent pixels become transparent black: their colour is gone.
pub fn unpremultiply(pixels: &mut [u8]) {
    let (pixels, _) = pixels.as_chunks_mut::<BYTES_PER_PIXEL>();
    for pixel in pixels {
        let alpha = u32::from(pixel[3]);
        match alpha {
            255 => {}
            0 => pixel[..3].fill(0),
            _ => {
                for channel in &mut pixel[..3] {
                    *channel = ((u32::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_checked() {
        assert!(matches!(
            Frame::new(0, 10),
            Err(RenderError::EmptyFrame { .. })
        ));
        assert!(matches!(
            Frame::new(MAX_FRAME_SIDE + 1, 1),
            Err(RenderError::FrameTooLarge { .. })
        ));
        assert!(matches!(
            Frame::new(MAX_FRAME_SIDE, MAX_FRAME_SIDE),
            Err(RenderError::FrameTooLarge { .. })
        ));
        assert!(matches!(
            Frame::from_premultiplied(2, 2, vec![0; 15]),
            Err(RenderError::BufferSize {
                expected: 16,
                actual: 15,
                ..
            })
        ));
        let frame = Frame::new(3, 2).unwrap();
        assert_eq!((frame.width(), frame.height()), (3, 2));
        assert_eq!(frame.premultiplied().len(), 24);
        assert!(!frame.is_opaque());
    }

    #[test]
    fn the_two_alpha_forms_are_told_apart() {
        // Half-transparent pure red, and opaque blue.
        let straight = vec![255, 0, 0, 128, 0, 0, 255, 255];
        let frame = Frame::from_unpremultiplied(2, 1, straight.clone()).unwrap();
        assert_eq!(frame.premultiplied(), [128, 0, 0, 128, 0, 0, 255, 255]);
        assert_eq!(frame.premultiplied_pixel(0, 0), Some([128, 0, 0, 128]));
        assert_eq!(frame.unpremultiplied_pixel(0, 0), Some([255, 0, 0, 128]));
        assert_eq!(frame.unpremultiplied_pixel(1, 0), Some([0, 0, 255, 255]));
        assert_eq!(frame.premultiplied_pixel(2, 0), None);
        assert_eq!(frame.to_unpremultiplied(), straight);
        assert_eq!(frame.clone().into_premultiplied(), frame.premultiplied());
    }

    #[test]
    fn conversion_round_trips_wherever_alpha_keeps_the_colour() {
        // With alpha 255 every value survives; with lower alpha the values
        // that are multiples of what alpha can represent do.
        for alpha in [255u8, 128, 51, 1] {
            for value in [0u8, 255] {
                let mut pixel = [value, 255 - value, value, alpha];
                premultiply(&mut pixel);
                assert!(pixel[..3].iter().all(|channel| *channel <= alpha));
                unpremultiply(&mut pixel);
                assert_eq!(pixel, [value, 255 - value, value, alpha]);
            }
        }
        let mut gone = [200, 100, 50, 0];
        premultiply(&mut gone);
        assert_eq!(gone, [0, 0, 0, 0]);
        // Colour that is brighter than its alpha allows is clamped, not wrapped.
        let mut too_bright = [200, 10, 0, 100];
        unpremultiply(&mut too_bright);
        assert_eq!(too_bright, [255, 26, 0, 100]);
    }

    #[test]
    fn png_holds_the_unpremultiplied_pixels() {
        let frame =
            Frame::from_unpremultiplied(2, 1, vec![255, 0, 0, 128, 10, 20, 30, 255]).unwrap();
        let file = frame.to_png().unwrap();
        assert_eq!(&file[..8], b"\x89PNG\r\n\x1a\n");
        let decoded = image::load_from_memory(&file).unwrap().into_rgba8();
        assert_eq!((decoded.width(), decoded.height()), (2, 1));
        assert_eq!(decoded.into_raw(), [255, 0, 0, 128, 10, 20, 30, 255]);
    }
}
