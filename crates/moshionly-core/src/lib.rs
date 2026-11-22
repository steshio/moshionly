//! The Moshionly engine core.
//!
//! This crate holds everything that does not need a window or a graphics
//! library: the document model, the three edits that change it, undo, the
//! `.moshi` file format, the JSON Schema handed to AI agents, easing, and the
//! evaluation of a document at a time into a renderer-agnostic scene.
//!
//! The rules it implements are written down in `docs/architecture.md` and, for
//! the file format, in `docs/file-format.md`.

pub mod color;
mod defaults;
pub mod easing;
pub mod geometry;
pub mod id;
mod json;
pub mod model;

pub use color::Color;
pub use easing::{Easing, ease};
pub use geometry::{Affine, Point, Rect};
pub use id::NodeId;
pub use model::{Family, Node, NodeData, NodeType};
