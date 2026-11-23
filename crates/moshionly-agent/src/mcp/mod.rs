//! The Model Context Protocol server: the tools, for agents outside the app.
//!
//! [`McpServer`] answers MCP messages with the [`Tools`] of a session. It
//! speaks both eras of the protocol: the stateless `2026-07-28` revision,
//! where every request carries its version, and the earlier revisions
//! (`2025-03-26` to `2025-11-25`) that open with an `initialize`
//! handshake. Only tools are offered; there are no resources or prompts.
//!
//! [`http::HttpServer`] serves it on this machine's loopback address, and
//! [`stdio::serve`] on standard input and output for `moshionly mcp`.

pub mod http;
mod protocol;
pub mod stdio;

use serde_json::Value;

pub use protocol::{LEGACY_VERSIONS, MODERN_VERSION, supported_versions};

use crate::guide::GUIDE;
use crate::tools::Tools;
use protocol::{Outcome, Protocol, Status, code};

/// An MCP server for the tools of one session.
///
/// Cloning is cheap; clones share the session.
///
/// ```
/// use moshionly_agent::mcp::McpServer;
/// use moshionly_agent::{Host, SessionHandle, Tools};
/// use moshionly_core::Document;
/// use serde_json::json;
///
/// let tools = Tools::new(SessionHandle::from_document(Document::new("Hello")), Host::new());
/// let server = McpServer::new(tools);
/// let reply = server
///     .handle(&json!({
///         "jsonrpc": "2.0", "id": 1, "method": "tools/list",
///         "params": { "_meta": {
///             "io.modelcontextprotocol/protocolVersion": "2026-07-28",
///             "io.modelcontextprotocol/clientCapabilities": {}
///         }}
///     }))
///     .unwrap();
/// assert_eq!(reply["result"]["tools"][0]["name"], "get_overview");
/// ```
#[derive(Debug, Clone)]
pub struct McpServer {
    protocol: Protocol,
}

impl McpServer {
    /// A server for `tools`, which gives clients the [agent guide](GUIDE)
    /// as its instructions.
    pub fn new(tools: Tools) -> Self {
        Self::with_instructions(tools, GUIDE)
    }

    /// A server for `tools` with other instructions for models. Empty
    /// instructions are left out.
    pub fn with_instructions(tools: Tools, instructions: impl Into<String>) -> Self {
        Self {
            protocol: Protocol::new(tools, instructions),
        }
    }

    /// The tools the server calls.
    pub fn tools(&self) -> &Tools {
        self.protocol.tools()
    }

    /// Answers one JSON-RPC message, as over stdio. Returns the response to
    /// send, or `None` for a notification.
    pub fn handle(&self, message: &Value) -> Option<Value> {
        self.protocol.handle(message, None).response().cloned()
    }

    /// Answers one JSON-RPC message given as text, as it arrives on a line
    /// of stdio. Text that is not JSON gets a parse error. Returns the
    /// response as compact JSON on one line, or `None` for a notification.
    pub fn handle_text(&self, text: &str) -> Option<String> {
        let outcome = match serde_json::from_str::<Value>(text) {
            Ok(message) => self.protocol.handle(&message, None),
            Err(error) => Outcome::error(
                &Value::Null,
                code::PARSE_ERROR,
                format!("The message is not JSON: {error}."),
                Status::BadRequest,
            ),
        };
        outcome.response().map(Value::to_string)
    }
}
