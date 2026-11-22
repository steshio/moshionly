//! The Moshionly engine core.
//!
//! This crate holds everything that does not need a window or a graphics
//! library: the document model, the three edits that change it, undo, the
//! `.moshi` file format, the JSON Schema handed to AI agents, easing, and the
//! evaluation of a document at a time into a renderer-agnostic scene.
//!
//! The rules it implements are written down in `docs/architecture.md` and, for
//! the file format, in `docs/file-format.md`.
//!
//! # Where things are
//!
//! | Module | What it is for |
//! |---|---|
//! | [`model`] | Node types: every layer and animation, and the values their properties hold |
//! | [`document`], [`tree`] | The [`Document`] and read-only queries on its tree |
//! | [`edit`] | [`Edit`] and `Document::apply`: the one way to change a document |
//! | [`validate`] | The rules a document keeps, and the messages for breaking them |
//! | [`history`] | [`History`]: undo, redo, gestures and a revision counter |
//! | [`build`] | Helpers that write edits: add, move, group, clone, duplicate |
//! | [`import`] | Bringing in designs from Figma, and re-importing them |
//! | [`io`] | Loading and saving project files, finding image assets |
//! | [`schema`] | JSON Schema and prompt-sized summaries of the node types |
//! | [`easing`] | The easing curves |
//! | [`eval`] | [`evaluate`]: a document and a time in, a [`Scene`] out |
//! | [`scene`] | The scene: one resolved frame, ready to draw |
//! | [`text`] | The [`TextMeasurer`] the renderer supplies, and number parsing for counters |
//! | [`index`] | Fractional indices, which order siblings |
//!
//! # A quick tour
//!
//! ```
//! use moshionly_core::{Document, History, MonospaceMeasurer, NodeType, build, evaluate};
//!
//! let mut document = Document::new("Hello");
//! let mut history = History::new();
//!
//! // Every change is a batch of edits. The helpers in `build` write them.
//! let (artboard, edits) = build::add_artboard(&document, "Scene", 800.0, 600.0)?;
//! history.apply(&mut document, &edits)?;
//!
//! let rect = build::default_layer(NodeType::Rect).expect("rect is a layer type");
//! let (rect, edits) = build::add_layer(&document, &artboard, rect)?;
//! history.apply(&mut document, &edits)?;
//!
//! let fade = build::default_animation(NodeType::FadeIn, &rect, 0.0).expect("an animation type");
//! let (_, edits) = build::add_animation(&document, fade, None)?;
//! history.apply(&mut document, &edits)?;
//!
//! // A frame is a pure function of the document and a time.
//! let scene = evaluate(&document, &artboard, 250.0, &MonospaceMeasurer::default())?;
//! assert_eq!(scene.nodes.len(), 1);
//! assert!(scene.nodes[0].opacity > 0.0 && scene.nodes[0].opacity < 1.0);
//!
//! // Undo takes the fade away again.
//! history.undo(&mut document)?;
//! let scene = evaluate(&document, &artboard, 250.0, &MonospaceMeasurer::default())?;
//! assert_eq!(scene.nodes[0].opacity, 1.0);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

pub mod build;
pub mod color;
mod defaults;
pub mod document;
pub mod easing;
pub mod edit;
pub mod eval;
pub mod geometry;
pub mod history;
pub mod id;
pub mod import;
pub mod index;
pub mod io;
mod json;
pub mod model;
pub mod scene;
pub mod schema;
pub mod text;
pub mod tree;
pub mod validate;

pub use color::Color;
pub use document::Document;
pub use easing::{Easing, ease};
pub use edit::{Edit, EditError};
pub use eval::{EvalError, evaluate, evaluate_at_rest};
pub use geometry::{Affine, Point, Rect};
pub use history::History;
pub use id::NodeId;
pub use model::{Family, Node, NodeData, NodeType};
pub use scene::{Scene, SceneNode};
pub use text::{MonospaceMeasurer, TextMeasurer};
pub use validate::{Problem, ProblemKind};
