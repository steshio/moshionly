//! The Moshionly desktop editor.
//!
//! This crate is the only part of Moshionly that knows about a window. It is split
//! into a library, so that every visual piece can be rendered headless in tests,
//! and a thin binary (`moshionly`) that opens the window.
//!
//! See "The app" in `docs/architecture.md` for how the editor is laid out.

pub mod app;
pub mod commands;
pub mod dialogs;
pub mod key;
pub mod settings;
pub mod shell;
pub mod theme;
pub mod widgets;

/// The name shown in the window title and used for the app's settings folder.
pub const APP_NAME: &str = "Moshionly";
