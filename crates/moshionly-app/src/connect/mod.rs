//! The app's connections to the outside: agents over MCP and designs sent
//! from Figma.
//!
//! - [`server`]: the MCP server on `127.0.0.1:47821` over the open
//!   document, and its status.
//! - [`clients`]: how to add Moshionly to Claude Code, Cursor and others.
//! - [`import`]: the choices of the dialog that asks where a design sent
//!   from Figma goes.
//!
//! The dialogs themselves are drawn by the shell.

pub mod clients;
pub mod import;
pub mod server;

pub use server::{AgentServer, DEFAULT_PORT, ServerStatus};
