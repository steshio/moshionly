//! The tool surface that lets an AI agent work in a Moshionly document, and
//! the MCP server that exposes it.
//!
//! The built-in chat, external agents such as Claude Code, and headless
//! scripts all reach a document through this crate, and through the same
//! door as the editor itself: batches of edits that are validated as a whole
//! and can be undone.
//!
//! The crate depends on `moshionly-core` only. Rendering and export belong
//! to the host, which hands them in through traits, so the tools run just as
//! well in a test or on a server without a window.
//!
//! # Where things are
//!
//! | Module | What it is for |
//! |---|---|
//! | [`session`] | [`Session`] and [`SessionHandle`]: the open document, its undo history, and who changed what |

#![warn(missing_docs)]

pub mod session;

pub use session::{
    Change, ChangeKind, Origin, Session, SessionHandle, SessionSaveError, Step, Subscription,
};
