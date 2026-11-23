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

#![warn(missing_docs)]

pub mod format;

pub use format::{Format, SizeLimits};
