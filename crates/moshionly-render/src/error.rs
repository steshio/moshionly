//! What can go wrong while rendering, and what can only go slightly wrong.
//!
//! A [`RenderError`] means there is no frame. A [`RenderWarning`] means the
//! frame is complete but something in it was drawn differently from what the
//! scene asked for, such as a placeholder where an image should be.

use moshionly_core::NodeId;
use thiserror::Error;

/// Why a frame could not be produced.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum RenderError {
    /// The output scale is zero, negative or not a number.
    #[error("the scale {0} cannot be used; it must be a number above 0, for example 1 or 0.5")]
    BadScale(f64),
    /// The scene, or the region asked for, has no area.
    #[error(
        "there is nothing to render: the frame would be {width} by {height} pixels, and it needs \
         at least 1 by 1"
    )]
    EmptyFrame {
        /// Width the frame would have had, in pixels.
        width: f64,
        /// Height the frame would have had, in pixels.
        height: f64,
    },
    /// The frame would be larger than the renderer allows.
    #[error(
        "a frame of {width} by {height} pixels is too large; a side can be at most {max_side} \
         pixels and the whole frame at most {max_pixels} pixels. Lower the scale or render a \
         smaller region"
    )]
    FrameTooLarge {
        /// Width asked for, in pixels.
        width: u64,
        /// Height asked for, in pixels.
        height: u64,
        /// Longest side allowed, in pixels.
        max_side: u32,
        /// Largest number of pixels allowed.
        max_pixels: u64,
    },
    /// A pixel buffer does not have the size its width and height call for.
    #[error(
        "a {width} by {height} frame needs {expected} bytes of pixels (4 per pixel), and the \
         buffer holds {actual}"
    )]
    BufferSize {
        /// Width of the frame in pixels.
        width: u32,
        /// Height of the frame in pixels.
        height: u32,
        /// Number of bytes the frame needs.
        expected: usize,
        /// Number of bytes that were given.
        actual: usize,
    },
    /// The graphics library refused to create a drawing surface.
    #[error("a drawing surface of {width} by {height} pixels could not be created")]
    Surface {
        /// Width of the surface in pixels.
        width: u32,
        /// Height of the surface in pixels.
        height: u32,
    },
    /// The frame could not be written as a PNG file.
    #[error("the frame could not be written as PNG: {0}")]
    Png(String),
}

/// Something in a finished frame that is not what the scene asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderWarning {
    /// An image could not be loaded. A placeholder was drawn in its box.
    MissingImage {
        /// The image layer.
        node: NodeId,
        /// The layer's `source`, shortened when it is long (a `data:` URI).
        source: String,
        /// Why the image could not be used.
        reason: String,
    },
    /// A font family is not installed or registered. The text was drawn
    /// with the default family instead.
    MissingFont {
        /// The text layer.
        node: NodeId,
        /// The family the layer asks for.
        family: String,
    },
}

impl std::fmt::Display for RenderWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RenderWarning::MissingImage {
                node,
                source,
                reason,
            } => write!(
                f,
                "the image \"{source}\" of layer \"{node}\" could not be loaded ({reason}); a \
                 placeholder was drawn instead"
            ),
            RenderWarning::MissingFont { node, family } => write!(
                f,
                "the font \"{family}\" of layer \"{node}\" is not available; the default font was \
                 used instead"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_say_what_to_do() {
        let error = RenderError::FrameTooLarge {
            width: 40_000,
            height: 10,
            max_side: 16_384,
            max_pixels: 1 << 27,
        };
        assert!(error.to_string().contains("Lower the scale"), "{error}");
        assert!(RenderError::BadScale(0.0).to_string().contains("above 0"));

        let warning = RenderWarning::MissingImage {
            node: NodeId::from("photo"),
            source: "cat.png".into(),
            reason: "the file does not exist".into(),
        };
        let text = warning.to_string();
        assert!(text.contains("cat.png") && text.contains("photo"), "{text}");
        assert!(text.contains("placeholder"), "{text}");
    }
}
