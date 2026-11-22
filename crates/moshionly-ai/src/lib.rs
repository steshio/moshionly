//! Chat providers for Moshionly.
//!
//! This crate gives the app one chat interface over many AI providers, so a
//! user can work with whichever AI they already pay for. It has no user
//! interface and does not depend on the other Moshionly crates.
//!
//! There are two kinds of provider behind the [`ChatProvider`] trait:
//!
//! - **Subscription providers** run an agent CLI that the user has installed
//!   and signed in to. Moshionly starts it non-interactively, attaches the
//!   Moshionly MCP server and streams its output. It never reads, copies or
//!   stores the CLI's credentials; it only runs the binary.
//! - **API providers** call a model API with the user's key and run the tool
//!   loop in-process against a [`ToolExecutor`] that the app supplies.
//!
//! A turn is one call to [`ChatProvider::send`] with a [`ChatRequest`]. The
//! answer arrives as a stream of [`ChatEvent`]s that always ends with
//! [`ChatEvent::Finished`].

mod chat;
mod tool;

pub use chat::{ChatMessage, ChatRequest, ImageData, McpEndpoint, Role};
pub use tool::{ToolContent, ToolExecutor, ToolOutput, ToolSpec};
