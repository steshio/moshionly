//! The Moshionly desktop editor.
//!
//! This crate is the only part of Moshionly that knows about a window. It is
//! split into a library, so that every visual piece can be rendered headless
//! in tests, and a thin binary (`moshionly`) that opens the window.
//!
//! The pieces, from the look inwards:
//!
//! - [`theme`]: every colour, size and typeface, for light and dark.
//! - [`widgets`]: the controls the editor is built from.
//! - [`timeline`] and [`viewport`]: the two large widgets. Both are driven by
//!   plain data and report what the user did as events, so they can be bound
//!   to the document engine without knowing about it. They name things with
//!   a [`key::Key`].
//! - [`commands`]: every command with its name and shortcut, and the command
//!   palette.
//! - [`editor`]: the open document, held in a shared session, and the
//!   editor's state around it: selection, current artboard, playhead, tool.
//!   Its rules (what a click selects, how a drag becomes edits) are plain
//!   functions that are tested without a window.
//! - [`canvas`]: pictures of the artboards, rendered on worker threads.
//! - [`chat`]: the AI chat panel and the AI settings, over `moshionly-ai`
//!   and the agent tools.
//! - [`host`]: what the app does for the agent tools: screenshots, fonts
//!   and exports.
//! - [`shell`]: the window's layout and its panels.
//! - [`app`]: the `eframe` application, with [`settings`] and [`dialogs`].
//!
//! See "The app" in `docs/architecture.md` for how the editor is laid out.

pub mod app;
pub mod canvas;
pub mod chat;
pub mod commands;
pub mod dialogs;
pub mod editor;
pub mod host;
pub mod key;
pub mod raster;
pub mod settings;
pub mod shell;
pub mod theme;
pub mod timeline;
pub mod viewport;
pub mod widgets;

/// The name shown in the window title and used for the app's settings folder.
pub const APP_NAME: &str = "Moshionly";
