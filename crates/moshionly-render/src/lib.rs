//! The Moshionly renderer: turns a [`Scene`](moshionly_core::Scene) into
//! pixels.
//!
//! `moshionly-core` evaluates a document at a time into a scene, a plain
//! description of one frame. This crate draws that description with Skia on
//! a CPU surface. The same code serves the editor preview, export and the
//! screenshots handed to AI agents, so a frame depends on nothing but the
//! scene and the options: no window, no GPU, no clock.
//!
//! # Where things are
//!
//! | Module | What it is for |
//! |---|---|
//! | [`renderer`] | [`Renderer`]: a scene in, a [`Frame`] out; hit testing and layer bounds |
//! | [`options`] | [`RenderOptions`]: output scale, background, region of the artboard |
//! | [`frame`] | [`Frame`]: the pixels, premultiplied or not, and PNG encoding |
//! | [`fonts`] | [`FontLibrary`]: bundled, registered and installed fonts; also core's text measurer |
//! | [`assets`] | [`AssetLoader`]: finds, decodes and keeps images |
//! | [`hit`] | [`LayerBounds`] and the hit-test tolerance |
//! | [`error`] | [`RenderError`] (no frame) and [`RenderWarning`] (a frame, not quite as asked) |
//!
//! # A quick tour
//!
//! ```
//! use moshionly_core::{evaluate, io, Point};
//! use moshionly_render::{AssetLoader, FontLibrary, RenderOptions, Renderer};
//!
//! let document = io::parse(r##"{"format":"moshionly","version":1,"name":"Hi","nodes":[
//!     {"id":"board","parent":null,"index":"a0","type":"artboard","name":"Board",
//!      "width":200,"height":100,"fill":"#FFFFFF"},
//!     {"id":"hello","parent":"board","index":"a0","type":"text","name":"Hello",
//!      "x":20,"y":20,"content":"Hello","font_size":32,"fill":"#0D99FF"}
//! ]}"##)?;
//! let artboard = &document.artboards()[0];
//!
//! // The font library draws text and is also the measurer core lays text
//! // out with, so text is split exactly where it is drawn. `bundled` uses
//! // the fonts that come with Moshionly only; the app adds installed ones.
//! let fonts = FontLibrary::bundled();
//! let renderer = Renderer::new(fonts.clone(), AssetLoader::default());
//!
//! let scene = evaluate(&document, artboard, 0.0, &fonts)?;
//! let frame = renderer.render(&scene, &RenderOptions::new().with_scale(2.0))?;
//! assert_eq!((frame.width(), frame.height()), (400, 200));
//! let png = frame.to_png()?;
//! assert!(png.starts_with(b"\x89PNG"));
//!
//! // What is under a point, and where the text layer is.
//! let hit = renderer.hit_test(&scene, Point::new(40.0, 40.0));
//! assert_eq!(hit[0].as_str(), "hello");
//! let bounds = renderer.bounds_of(&scene, &hit[0]).expect("the text is drawn");
//! assert_eq!(bounds.corners[0], Point::new(20.0, 20.0));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

#![warn(missing_docs)]

pub mod assets;
mod convert;
mod draw;
pub mod error;
pub mod fonts;
pub mod frame;
pub mod hit;
pub mod options;
pub mod renderer;
mod text;

pub use assets::AssetLoader;
pub use error::{RenderError, RenderWarning};
pub use fonts::{FontError, FontFace, FontFamily, FontLibrary};
pub use frame::Frame;
pub use hit::{HIT_TOLERANCE, LayerBounds};
pub use options::{Background, RenderOptions};
pub use renderer::Renderer;

/// The Skia binding this crate draws with, for callers of
/// [`Renderer::render_into`] who need its canvas type.
pub use skia_safe;
