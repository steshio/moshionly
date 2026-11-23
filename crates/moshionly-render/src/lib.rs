//! The Moshionly renderer: turns a [`Scene`](moshionly_core::Scene) into
//! pixels.
//!
//! `moshionly-core` evaluates a document at a time into a scene, a plain
//! description of one frame. This crate draws that description with Skia on
//! a CPU surface. The same code serves the editor preview, export and the
//! screenshots handed to AI agents, so a frame depends on nothing but the
//! scene and the options: no window, no GPU, no clock.

#![warn(missing_docs)]

pub mod error;
pub mod frame;

pub use error::{RenderError, RenderWarning};
pub use frame::Frame;
