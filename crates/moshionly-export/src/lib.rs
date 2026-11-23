//! Moshionly export: writes an artboard to an image or a video file.
//!
//! The editor's Export dialog, the agents' `export` tool and the
//! `moshionly export` command all call this crate. PNG, PNG sequences, GIF
//! and MP4 are written in-process; WebM and ProRes by an installed
//! `ffmpeg`. Lottie, which converts the scene model instead of pixels,
//! will join them in `src/lottie/`.
//!
//! # A quick tour
//!
//! ```
//! use std::sync::Arc;
//! use moshionly_core::io;
//! use moshionly_export::{CancelToken, ExportJob, Format, FrameRate, Settings};
//! use moshionly_render::{AssetLoader, FontLibrary, Renderer};
//!
//! let document = Arc::new(io::parse(r##"{"format":"moshionly","version":1,"name":"Hi","nodes":[
//!     {"id":"board","parent":null,"index":"a0","type":"artboard","name":"Board",
//!      "width":160,"height":90,"duration":1000,"fill":"#FFFFFF"},
//!     {"id":"dot","parent":"board","index":"a0","type":"ellipse","name":"Dot",
//!      "x":20,"y":20,"width":50,"height":50,"fill":"#0D99FF"},
//!     {"id":"in","parent":"board","index":"a0","type":"fade_in","target":"dot",
//!      "start":0,"end":500}
//! ]}"##)?);
//! let artboard = document.artboards()[0].clone();
//! let dir = tempfile::tempdir()?;
//!
//! // Settings are checked before anything is written.
//! let settings = Settings::default().with_frame_rate(FrameRate::FPS_25);
//! let job = ExportJob::new(document, artboard, Format::Gif, dir.path().join("dot.gif"))
//!     .with_settings(settings)
//!     // The bundled fonts only, so the result is the same on every machine.
//!     .with_renderer(Renderer::new(FontLibrary::bundled(), AssetLoader::default()));
//! let plan = job.plan()?;
//! assert_eq!((plan.width, plan.height, plan.frame_count()), (160, 90, 25));
//!
//! let exported = job.run(|progress| assert!(progress.fraction() <= 1.0), &CancelToken::new())?;
//! assert_eq!(exported.frames, 25);
//! assert_eq!(exported.duration_ms, Some(1000.0));
//! assert!(std::fs::read(&exported.path)?.starts_with(b"GIF89a"));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! # Where things are
//!
//! | Module | What it is for |
//! |---|---|
//! | [`format`](mod@format) | [`Format`]: the kinds of file, and what each can hold |
//! | [`timing`] | [`FrameRate`] and [`FrameTimes`]: which moments an export captures |
//! | [`settings`] | [`Settings`]: what a person chooses, checked into a [`Plan`] |
//! | [`job`] | [`ExportJob`]: what to export and where; [`ExportJob::run`] does it |
//! | [`progress`] | [`Progress`] reports and the [`CancelToken`] |
//! | [`error`] | [`ExportError`]: why an export stopped |
//! | [`ffmpeg`] | [`Ffmpeg`]: finding the program that writes WebM and ProRes |
//! | [`available`] | [`available_formats`]: what can be written right now, and why not |

#![warn(missing_docs)]

pub mod available;
pub mod error;
pub mod ffmpeg;
pub mod format;
mod frames;
mod gif;
pub mod job;
mod mp4;
mod output;
mod png;
pub mod progress;
pub mod settings;
pub mod timing;

pub use available::{Availability, availability, availability_with, available_formats};
pub use error::ExportError;
pub use ffmpeg::Ffmpeg;
pub use format::{Format, SizeLimits};
pub use job::{ExportJob, Exported};
pub use progress::{CancelToken, Phase, Progress};
pub use settings::{
    Loop, MAX_FRAMES, Moment, Moments, Plan, Quality, Settings, SettingsError, Size,
};
pub use timing::{FrameRate, FrameTimes, TimingError};

/// What is behind the layers: the artboard's own fill, nothing, or a colour.
pub use moshionly_render::Background;
