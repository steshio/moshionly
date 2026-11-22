//! The Moshionly engine core.
//!
//! This crate holds everything that does not need a window or a graphics
//! library: the document model, the three edits that change it, undo, the
//! `.moshi` file format, the JSON Schema handed to AI agents, easing, and the
//! evaluation of a document at a time into a renderer-agnostic scene.
//!
//! The rules it implements are written down in `docs/architecture.md` and, for
//! the file format, in `docs/file-format.md`.

pub mod id;

pub use id::NodeId;
