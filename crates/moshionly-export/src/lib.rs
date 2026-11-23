//! Moshionly export: writes an artboard to an image or a video file.
//!
//! The editor's Export dialog, the agents' `export` tool and the
//! `moshionly export` command all call this crate.
//!
//! # Where things are
//!
//! | Module | What it is for |
//! |---|---|
//! | [`format`] | [`Format`]: the kinds of file, and what each can hold |
//! | [`timing`] | [`FrameRate`] and [`FrameTimes`]: which moments an export captures |
//! | [`settings`] | [`Settings`]: what a person chooses, checked into a [`Plan`] |
//! | [`job`] | [`ExportJob`]: what to export and where; [`ExportJob::run`] does it |
//! | [`progress`] | [`Progress`] reports and the [`CancelToken`] |
//! | [`error`] | [`ExportError`]: why an export stopped |

#![warn(missing_docs)]

pub mod error;
pub mod format;
mod frames;
mod gif;
pub mod job;
mod output;
mod png;
pub mod progress;
pub mod settings;
pub mod timing;

pub use error::ExportError;
pub use format::{Format, SizeLimits};
pub use job::{ExportJob, Exported};
pub use progress::{CancelToken, Phase, Progress};
pub use settings::{Loop, Moment, Moments, Plan, Quality, Settings, SettingsError, Size};
pub use timing::{FrameRate, FrameTimes, TimingError};

/// What is behind the layers: the artboard's own fill, nothing, or a colour.
pub use moshionly_render::Background;
