//! The Moshionly renderer: turns a [`Scene`](moshionly_core::Scene) into
//! pixels.
//!
//! `moshionly-core` evaluates a document at a time into a scene, a plain
//! description of one frame. This crate draws that description with Skia on
//! a CPU surface. The same code serves the editor preview, export and the
//! screenshots handed to AI agents, so a frame depends on nothing but the
//! scene and the options: no window, no GPU, no clock.

#![warn(missing_docs)]

mod convert;
mod draw;
pub mod error;
pub mod fonts;
pub mod frame;
pub mod options;
pub mod renderer;

pub use error::{RenderError, RenderWarning};
pub use fonts::{FontError, FontFace, FontFamily, FontLibrary};
pub use frame::Frame;
pub use options::{Background, RenderOptions};
pub use renderer::Renderer;

/// The Skia binding this crate draws with, for callers of
/// [`Renderer::render_into`] who need its canvas type.
pub use skia_safe;
