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
//! | [`bridge`] | [`ToolBridge`]: the same tools as plain data, for the built-in chat |
//! | [`guide`] | [`guide::GUIDE`] and [`guide::system_prompt`]: how to work in a file, written for a model |
//! | [`import`] | [`import::PendingImport`]: designs sent from Figma, checked and ready to add as one undo step |
//! | [`mcp`] | [`mcp::McpServer`]: the Model Context Protocol server, for agents outside the app |
//! | [`session`] | [`Session`] and [`SessionHandle`]: the open document, its undo history, and who changed what |
//! | [`host`] | [`Host`]: what the embedding app supplies, as traits: [`FrameSource`], [`Exporter`], [`FontSource`] |
//! | [`tools`] | [`Tools`]: every tool, with `list` and `call` |
//! | [`tool`] | What a tool looks like from outside: [`ToolSpec`], [`ToolOutput`], [`ToolError`] |

#![warn(missing_docs)]

pub mod bridge;
pub mod guide;
pub mod host;
pub mod import;
pub mod mcp;
pub mod session;
pub mod tool;
pub mod tools;

pub use bridge::{BridgePart, BridgeResult, BridgeTool, ToolBridge};
pub use host::{
    ExportFormat, ExportOptions, ExportProgress, ExportRequest, Exported, Exporter, FontFamily,
    FontSource, Frame, FrameRequest, FrameSource, Host, HostError, MediaInfo, MediaProbe,
    SharedMeasurer,
};
pub use session::{
    Change, ChangeKind, Origin, Session, SessionHandle, SessionSaveError, Step, Subscription,
};
pub use tool::{EditFailure, ToolContent, ToolError, ToolOutput, ToolSpec};
pub use tools::{Tools, ToolsConfig};
