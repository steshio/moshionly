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

#![warn(missing_docs)]

pub mod format;
pub mod settings;
pub mod timing;

pub use format::{Format, SizeLimits};
pub use settings::{Loop, Moment, Moments, Plan, Quality, Settings, SettingsError, Size};
pub use timing::{FrameRate, FrameTimes, TimingError};

/// What is behind the layers: the artboard's own fill, nothing, or a colour.
pub use moshionly_render::Background;
