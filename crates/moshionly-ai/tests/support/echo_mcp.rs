//! A throwaway MCP server with one trivial tool, for trying subscription
//! providers without the Moshionly app.
//!
//! It speaks just enough of MCP's streamable HTTP transport for an agent CLI
//! to connect, list tools and call one: every request is answered with a
//! plain JSON response, and there are no sessions or server-sent streams.
//! The real server lives in `moshionly-agent`; this one exists so that the
//! live tests and the `chat` example have something to attach.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{Value, json};

/// The name of the only tool.
pub const ECHO_TOOL: &str = "echo";

/// What the tool does, as the model reads it.
pub const ECHO_DESCRIPTION: &str =
    "Returns the given text unchanged. Use it when asked to echo something.";

/// JSON Schema for the tool's input.
pub fn echo_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "text": { "type": "string", "description": "The text to return." }
        },
        "required": ["text"],
        "additionalProperties": false
    })
}

/// What the tool answers for `input`.
pub fn echo_reply(input: &Value) -> String {
    let text = input.get("text").and_then(Value::as_str).unwrap_or("");
    format!("echo: {text}")
}

/// A running echo server. It stops when dropped.
pub struct EchoMcpServer {
    addr: SocketAddr,
    calls: Arc<Mutex<Vec<Value>>>,
    methods: Arc<Mutex<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
}

#[derive(Clone)]
struct Shared {
    calls: Arc<Mutex<Vec<Value>>>,
    methods: Arc<Mutex<Vec<String>>>,
}

impl EchoMcpServer {
    /// Starts the server on a free local port.
    pub async fn start() -> std::io::Result<Self> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let shared = Shared {
            calls: Arc::default(),
            methods: Arc::default(),
        };
        let router = Router::new()
            .route("/mcp", post(handle_post))
            .with_state(shared.clone());
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        Ok(Self {
            addr,
            calls: shared.calls,
            methods: shared.methods,
            task,
        })
    }

    /// The URL to give an agent.
    pub fn url(&self) -> String {
        format!("http://{}/mcp", self.addr)
    }

    /// The arguments of every `echo` call received so far.
    #[allow(dead_code)]
    pub fn calls(&self) -> Vec<Value> {
        self.calls.lock().unwrap().clone()
    }

    /// Every JSON-RPC method received so far, in order.
    #[allow(dead_code)]
    pub fn methods(&self) -> Vec<String> {
        self.methods.lock().unwrap().clone()
    }
}

impl Drop for EchoMcpServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn handle_post(State(shared): State<Shared>, Json(body): Json<Value>) -> Response {
    // Older protocol revisions allow a batch of messages in one POST.
    let replies: Vec<Value> = match &body {
        Value::Array(messages) => messages
            .iter()
            .filter_map(|message| answer(&shared, message))
            .collect(),
        message => answer(&shared, message).into_iter().collect(),
    };
    match (body.is_array(), replies.len()) {
        // Only notifications: nothing to say.
        (_, 0) => StatusCode::ACCEPTED.into_response(),
        (false, _) => Json(replies.into_iter().next()).into_response(),
        (true, _) => Json(replies).into_response(),
    }
}

/// Answers one JSON-RPC message. Notifications (no `id`) get no reply.
fn answer(shared: &Shared, message: &Value) -> Option<Value> {
    let method = message.get("method").and_then(Value::as_str).unwrap_or("");
    shared.methods.lock().unwrap().push(method.to_owned());
    let id = message.get("id")?.clone();
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    let result = match method {
        "initialize" => json!({
            "protocolVersion": params
                .get("protocolVersion")
                .cloned()
                .unwrap_or_else(|| json!("2025-06-18")),
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "moshionly-echo", "version": "0.0.0" }
        }),
        "ping" => json!({}),
        "tools/list" => json!({
            "tools": [{
                "name": ECHO_TOOL,
                "description": ECHO_DESCRIPTION,
                "inputSchema": echo_input_schema()
            }]
        }),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let arguments = params.get("arguments").cloned().unwrap_or(Value::Null);
            if name == ECHO_TOOL {
                shared.calls.lock().unwrap().push(arguments.clone());
                json!({
                    "content": [{ "type": "text", "text": echo_reply(&arguments) }],
                    "isError": false
                })
            } else {
                json!({
                    "content": [{ "type": "text", "text": format!("unknown tool: {name}") }],
                    "isError": true
                })
            }
        }
        _ => {
            return Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32601, "message": format!("method not found: {method}") }
            }));
        }
    };
    Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}
