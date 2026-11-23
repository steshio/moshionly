//! The Model Context Protocol, independent of how messages travel.
//!
//! [`Protocol::handle`] takes one JSON-RPC message and returns what to send
//! back. The HTTP and stdio transports wrap it; they add only what their
//! medium needs (sessions and headers on HTTP, lines on stdio).
//!
//! The server speaks two eras of the protocol, as the specification allows:
//!
//! - **Modern** (`2026-07-28`): no handshake. Every request carries its
//!   protocol version and the client's capabilities in
//!   `params._meta`, and is served on its own.
//! - **Legacy** (`2025-03-26` to `2025-11-25`): the client opens with
//!   `initialize`, which agrees on a version, and then sends
//!   `notifications/initialized`. On HTTP the server gives the client a
//!   session id at that point.
//!
//! A request that carries the modern `_meta` fields is served the modern
//! way; anything else is served as part of a legacy conversation.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde_json::{Map, Value, json};

use crate::tool::{ToolContent, ToolOutput, ToolSpec};
use crate::tools::Tools;

/// The newest protocol version, which every request states for itself.
pub const MODERN_VERSION: &str = "2026-07-28";
/// The versions that start with an `initialize` handshake, newest first.
pub const LEGACY_VERSIONS: [&str; 3] = ["2025-11-25", "2025-06-18", "2025-03-26"];

/// Every protocol version this server speaks, newest first.
pub fn supported_versions() -> Vec<&'static str> {
    std::iter::once(MODERN_VERSION)
        .chain(LEGACY_VERSIONS)
        .collect()
}

/// JSON-RPC and MCP error codes.
pub(crate) mod code {
    /// The message is not JSON.
    pub const PARSE_ERROR: i64 = -32700;
    /// The JSON is not a valid JSON-RPC message.
    pub const INVALID_REQUEST: i64 = -32600;
    /// No such method.
    pub const METHOD_NOT_FOUND: i64 = -32601;
    /// The parameters are wrong, including an unknown tool.
    pub const INVALID_PARAMS: i64 = -32602;
    /// HTTP headers and body disagree.
    pub const HEADER_MISMATCH: i64 = -32020;
    /// The protocol version is not one this server speaks.
    pub const UNSUPPORTED_PROTOCOL_VERSION: i64 = -32022;
}

const META_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
const META_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";
const META_SERVER_INFO: &str = "io.modelcontextprotocol/serverInfo";

/// How long a client may keep the tool list before asking again, in
/// milliseconds. The list only changes with a new version of the app.
const TOOLS_TTL_MS: u64 = 3_600_000;

/// What the HTTP transport read from the request's headers, for the
/// checks the modern protocol asks for. Header names are case-insensitive;
/// the transport has already matched them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Headers {
    /// `MCP-Protocol-Version`.
    pub protocol_version: Option<String>,
    /// `Mcp-Method`.
    pub method: Option<String>,
    /// `Mcp-Name`, as it was sent (possibly Base64 encoded).
    pub name: Option<String>,
}

/// How a reply should be sent over HTTP. stdio ignores it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Status {
    /// 200: a result, or an error that belongs in a result.
    Ok,
    /// 400: the request is malformed or uses an unknown version.
    BadRequest,
    /// 404: no such method (modern requests only).
    NotFound,
}

/// What to send back for one message.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Outcome {
    /// A response to a request.
    Reply {
        /// The JSON-RPC response.
        response: Value,
        /// The HTTP status to send it with.
        status: Status,
    },
    /// A successful `initialize`: the response, and the version agreed,
    /// which starts a legacy conversation.
    Initialized {
        /// The JSON-RPC response.
        response: Value,
        /// The protocol version agreed.
        version: String,
    },
    /// A notification or a response from the client: nothing to send.
    Accepted,
}

impl Outcome {
    fn result(id: &Value, result: Value) -> Self {
        Outcome::Reply {
            response: json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            status: Status::Ok,
        }
    }

    pub(crate) fn error(id: &Value, code: i64, message: impl Into<String>, status: Status) -> Self {
        Self::error_with(id, code, message, None, status)
    }

    fn error_with(
        id: &Value,
        code: i64,
        message: impl Into<String>,
        data: Option<Value>,
        status: Status,
    ) -> Self {
        let mut error = json!({ "code": code, "message": message.into() });
        if let Some(data) = data {
            error["data"] = data;
        }
        Outcome::Reply {
            response: json!({ "jsonrpc": "2.0", "id": id, "error": error }),
            status,
        }
    }

    /// The JSON-RPC message to send, if any.
    pub(crate) fn response(&self) -> Option<&Value> {
        match self {
            Outcome::Reply { response, .. } | Outcome::Initialized { response, .. } => {
                Some(response)
            }
            Outcome::Accepted => None,
        }
    }
}

/// The protocol logic of the Moshionly MCP server.
#[derive(Debug, Clone)]
pub(crate) struct Protocol {
    tools: Tools,
    instructions: String,
}

impl Protocol {
    /// A server for `tools`, which gives clients `instructions` as its
    /// guidance for models.
    pub fn new(tools: Tools, instructions: impl Into<String>) -> Self {
        Self {
            tools,
            instructions: instructions.into(),
        }
    }

    /// The tools the server calls.
    pub fn tools(&self) -> &Tools {
        &self.tools
    }

    /// Handles one message. `headers` are present for HTTP requests.
    pub fn handle(&self, message: &Value, headers: Option<&Headers>) -> Outcome {
        let Some(fields) = message.as_object() else {
            let what = if message.is_array() {
                "Batches are not supported; send one JSON-RPC message at a time."
            } else {
                "A JSON-RPC message must be an object."
            };
            return Outcome::error(
                &Value::Null,
                code::INVALID_REQUEST,
                what,
                Status::BadRequest,
            );
        };
        let id = fields.get("id").cloned().unwrap_or(Value::Null);
        if fields.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return Outcome::error(
                &id,
                code::INVALID_REQUEST,
                "\"jsonrpc\" must be \"2.0\".",
                Status::BadRequest,
            );
        }
        let Some(method) = fields.get("method") else {
            // A response from the client; this server sends no requests.
            return if fields.contains_key("result") || fields.contains_key("error") {
                Outcome::Accepted
            } else {
                Outcome::error(
                    &id,
                    code::INVALID_REQUEST,
                    "A message needs a \"method\".",
                    Status::BadRequest,
                )
            };
        };
        let Some(method) = method.as_str() else {
            return Outcome::error(
                &id,
                code::INVALID_REQUEST,
                "\"method\" must be text.",
                Status::BadRequest,
            );
        };
        let params = fields.get("params").cloned().unwrap_or(json!({}));
        if !fields.contains_key("id") {
            // notifications/initialized, cancellation and every other
            // notification need no answer: calls run to completion.
            return Outcome::Accepted;
        }
        if !matches!(id, Value::String(_) | Value::Number(_)) {
            return Outcome::error(
                &Value::Null,
                code::INVALID_REQUEST,
                "\"id\" must be a string or a number.",
                Status::BadRequest,
            );
        }
        if !params.is_object() {
            return Outcome::error(
                &id,
                code::INVALID_PARAMS,
                "\"params\" must be an object.",
                Status::BadRequest,
            );
        }
        if method == "initialize" {
            return self.initialize(&id, &params);
        }
        match params
            .pointer("/_meta")
            .and_then(|meta| meta.get(META_VERSION))
        {
            Some(version) => self.modern(&id, method, &params, version, headers),
            None => self.legacy(&id, method, &params, headers),
        }
    }

    /// `initialize`: agrees on a legacy version.
    fn initialize(&self, id: &Value, params: &Value) -> Outcome {
        let Some(requested) = params.get("protocolVersion").and_then(Value::as_str) else {
            return Outcome::error(
                id,
                code::INVALID_PARAMS,
                "initialize needs \"protocolVersion\".",
                Status::BadRequest,
            );
        };
        // The client's version when it is one of ours, or else the newest
        // version that has this handshake.
        let version = LEGACY_VERSIONS
            .iter()
            .find(|version| **version == requested)
            .unwrap_or(&LEGACY_VERSIONS[0]);
        let mut result = json!({
            "protocolVersion": version,
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": server_info(),
        });
        if !self.instructions.is_empty() {
            result["instructions"] = json!(self.instructions);
        }
        Outcome::Initialized {
            response: json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            version: (*version).to_owned(),
        }
    }

    /// A request of the modern protocol, which carries what it needs.
    fn modern(
        &self,
        id: &Value,
        method: &str,
        params: &Value,
        version: &Value,
        headers: Option<&Headers>,
    ) -> Outcome {
        let version = version.as_str().unwrap_or_default();
        if version != MODERN_VERSION {
            return Outcome::error_with(
                id,
                code::UNSUPPORTED_PROTOCOL_VERSION,
                "Unsupported protocol version",
                Some(json!({ "supported": supported_versions(), "requested": version })),
                Status::BadRequest,
            );
        }
        let capabilities = params
            .pointer("/_meta")
            .and_then(|meta| meta.get(META_CAPABILITIES));
        if !capabilities.is_some_and(Value::is_object) {
            return Outcome::error(
                id,
                code::INVALID_PARAMS,
                format!("\"_meta\" must hold \"{META_CAPABILITIES}\" as an object."),
                Status::BadRequest,
            );
        }
        if let Some(headers) = headers
            && let Err(message) = check_headers(headers, version, method, params)
        {
            return Outcome::error(id, code::HEADER_MISMATCH, message, Status::BadRequest);
        }
        let result = match method {
            "server/discover" => {
                let mut result = json!({
                    "supportedVersions": supported_versions(),
                    "capabilities": { "tools": {} },
                    "ttlMs": TOOLS_TTL_MS,
                    "cacheScope": "public",
                });
                if !self.instructions.is_empty() {
                    result["instructions"] = json!(self.instructions);
                }
                result
            }
            "tools/list" => {
                let mut result = self.list_tools();
                result["ttlMs"] = json!(TOOLS_TTL_MS);
                result["cacheScope"] = json!("public");
                result
            }
            "tools/call" => match self.call_tool(params) {
                Ok(result) => result,
                Err(message) => {
                    return Outcome::error(id, code::INVALID_PARAMS, message, Status::BadRequest);
                }
            },
            other => {
                return Outcome::error(
                    id,
                    code::METHOD_NOT_FOUND,
                    format!("Method not found: {other}"),
                    Status::NotFound,
                );
            }
        };
        let mut result = result;
        result["resultType"] = json!("complete");
        result["_meta"] = json!({ META_SERVER_INFO: server_info() });
        Outcome::result(id, result)
    }

    /// A request inside a legacy conversation.
    fn legacy(
        &self,
        id: &Value,
        method: &str,
        params: &Value,
        headers: Option<&Headers>,
    ) -> Outcome {
        if let Some(version) = headers.and_then(|headers| headers.protocol_version.as_deref())
            && !LEGACY_VERSIONS.contains(&version)
        {
            let message = if version == MODERN_VERSION {
                format!(
                    "A {MODERN_VERSION} request must carry \"{META_VERSION}\" and \
                     \"{META_CAPABILITIES}\" in params._meta."
                )
            } else {
                format!(
                    "Unsupported protocol version \"{version}\". Supported: {}.",
                    supported_versions().join(", ")
                )
            };
            return Outcome::error(id, code::INVALID_REQUEST, message, Status::BadRequest);
        }
        match method {
            "ping" => Outcome::result(id, json!({})),
            "tools/list" => Outcome::result(id, self.list_tools()),
            "tools/call" => match self.call_tool(params) {
                Ok(result) => Outcome::result(id, result),
                Err(message) => Outcome::error(id, code::INVALID_PARAMS, message, Status::Ok),
            },
            "server/discover" => Outcome::error(
                id,
                code::INVALID_PARAMS,
                format!("server/discover needs \"{META_VERSION}\" in params._meta."),
                Status::BadRequest,
            ),
            other => Outcome::error(
                id,
                code::METHOD_NOT_FOUND,
                format!("Method not found: {other}"),
                Status::Ok,
            ),
        }
    }

    /// The `tools/list` result.
    fn list_tools(&self) -> Value {
        let tools: Vec<Value> = self.tools.list().iter().map(tool_json).collect();
        json!({ "tools": tools })
    }

    /// Runs `tools/call`. An unknown tool or malformed parameters are a
    /// protocol error; everything the tool itself reports is a result.
    fn call_tool(&self, params: &Value) -> Result<Value, String> {
        let Some(name) = params.get("name").and_then(Value::as_str) else {
            return Err("tools/call needs \"name\", the tool to call.".to_owned());
        };
        if !self.tools.has(name) {
            return Err(format!("Unknown tool: {name}"));
        }
        let arguments = match params.get("arguments") {
            None | Some(Value::Null) => json!({}),
            Some(arguments @ Value::Object(_)) => arguments.clone(),
            Some(_) => return Err("\"arguments\" must be an object.".to_owned()),
        };
        Ok(output_json(&self.tools.call(name, arguments)))
    }
}

/// Checks the headers a modern HTTP request must carry against its body.
fn check_headers(
    headers: &Headers,
    version: &str,
    method: &str,
    params: &Value,
) -> Result<(), String> {
    match headers.protocol_version.as_deref() {
        None => return Err("The MCP-Protocol-Version header is missing.".to_owned()),
        Some(header) if header != version => {
            return Err(format!(
                "Header mismatch: MCP-Protocol-Version header value '{header}' does not match \
                 body value '{version}'"
            ));
        }
        Some(_) => {}
    }
    match headers.method.as_deref() {
        None => return Err("The Mcp-Method header is missing.".to_owned()),
        Some(header) if header != method => {
            return Err(format!(
                "Header mismatch: Mcp-Method header value '{header}' does not match body value \
                 '{method}'"
            ));
        }
        Some(_) => {}
    }
    if method == "tools/call" {
        let body = params
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let Some(header) = headers.name.as_deref() else {
            return Err("The Mcp-Name header is missing.".to_owned());
        };
        let decoded = decode_header_value(header)
            .ok_or_else(|| format!("The Mcp-Name header value '{header}' is not valid."))?;
        if decoded != body {
            return Err(format!(
                "Header mismatch: Mcp-Name header value '{decoded}' does not match body value \
                 '{body}'"
            ));
        }
    }
    Ok(())
}

/// A header value, with the `=?base64?...?=` form decoded.
pub(crate) fn decode_header_value(value: &str) -> Option<String> {
    match value
        .strip_prefix("=?base64?")
        .and_then(|rest| rest.strip_suffix("?="))
    {
        Some(encoded) => String::from_utf8(BASE64.decode(encoded).ok()?).ok(),
        None => Some(value.to_owned()),
    }
}

/// Who this server is.
pub(crate) fn server_info() -> Value {
    json!({
        "name": "moshionly",
        "title": "Moshionly",
        "version": env!("CARGO_PKG_VERSION"),
    })
}

/// A tool as `tools/list` describes it.
fn tool_json(spec: &ToolSpec) -> Value {
    let mut annotations = Map::new();
    annotations.insert("title".into(), json!(spec.title));
    annotations.insert("readOnlyHint".into(), json!(spec.read_only));
    if !spec.read_only {
        annotations.insert("destructiveHint".into(), json!(spec.destructive));
    }
    // Every tool works on the open file and the folders next to it.
    annotations.insert("openWorldHint".into(), json!(false));
    json!({
        "name": spec.name,
        "title": spec.title,
        "description": spec.description,
        "inputSchema": spec.input_schema,
        "annotations": annotations,
    })
}

/// A tool's output as a `tools/call` result.
pub(crate) fn output_json(output: &ToolOutput) -> Value {
    let content: Vec<Value> = output
        .content
        .iter()
        .map(|part| match part {
            ToolContent::Text { text } => json!({ "type": "text", "text": text }),
            ToolContent::Image { media_type, data } => json!({
                "type": "image",
                "data": BASE64.encode(data),
                "mimeType": media_type,
            }),
        })
        .collect();
    json!({ "content": content, "isError": output.is_error })
}

#[cfg(test)]
mod tests;
