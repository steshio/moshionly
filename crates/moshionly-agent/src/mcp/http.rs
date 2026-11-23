//! MCP over HTTP on this machine, at `http://127.0.0.1:47821/mcp`.
//!
//! [`HttpServer::start`] binds the port and serves on a thread of its own,
//! so the app does not need an async runtime. The server only listens on
//! the loopback address, and refuses any request that comes from a web
//! page: agent command-line tools send no `Origin` header, browsers always
//! do, so a request whose `Origin` is not this server's own is turned away
//! before it is read. The `Host` header is checked too, which stops a web
//! page from reaching the port under a domain name of its own (DNS
//! rebinding).
//!
//! Requests are answered with a single JSON response; the server never
//! needs to stream. Clients of the earlier protocol revisions get a
//! session id from `initialize` and end it with `DELETE`.

use std::collections::HashMap;
use std::io;
use std::net::{Ipv4Addr, SocketAddr, TcpListener as StdListener};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::response::Response;
use axum::routing::{any, get};
use moshionly_core::NodeId;
use serde_json::{Value, json};
use tokio::sync::oneshot;

use super::McpServer;
use super::protocol::{Headers, Outcome, Status, code, server_info};

/// The port the app listens on unless told otherwise.
pub const DEFAULT_PORT: u16 = 47821;
/// The path of the MCP endpoint.
pub const MCP_PATH: &str = "/mcp";
/// The path that says the app is running.
pub const HEALTH_PATH: &str = "/health";

/// The most legacy sessions kept at once; the oldest goes first.
const MAX_SESSIONS: usize = 64;
const SESSION_HEADER: &str = "mcp-session-id";

/// How the HTTP server is set up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpConfig {
    /// The port on 127.0.0.1. 0 lets the system choose a free one.
    pub port: u16,
    /// The largest request body accepted, in bytes. Larger requests get
    /// `413 Payload Too Large`.
    pub max_body_bytes: usize,
}

impl Default for HttpConfig {
    /// Port 47821, and bodies up to 64 MB, which leaves room for images
    /// sent as `data:` URIs.
    fn default() -> Self {
        Self {
            port: DEFAULT_PORT,
            max_body_bytes: 64 * 1024 * 1024,
        }
    }
}

/// A running HTTP server. Dropping it stops the server.
#[derive(Debug)]
pub struct HttpServer {
    address: SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl HttpServer {
    /// Binds `127.0.0.1` at the configured port and starts serving `server`
    /// on a thread of its own.
    ///
    /// Fails when the port is taken, for example by another copy of the
    /// app.
    pub fn start(server: McpServer, config: HttpConfig) -> io::Result<HttpServer> {
        let listener = StdListener::bind((Ipv4Addr::LOCALHOST, config.port))?;
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("moshionly-mcp")
            .enable_all()
            .build()?;
        let state = Arc::new(Shared {
            server,
            port: address.port(),
            sessions: Mutex::new(Sessions::default()),
        });
        let app = router(state, config.max_body_bytes);
        let (shutdown, stop) = oneshot::channel::<()>();
        let listener = {
            let _entered = runtime.enter();
            tokio::net::TcpListener::from_std(listener)?
        };
        let thread = std::thread::Builder::new()
            .name("moshionly-mcp-http".into())
            .spawn(move || {
                runtime.block_on(async move {
                    let stopped = async {
                        let _ = stop.await;
                    };
                    if let Err(error) = axum::serve(listener, app)
                        .with_graceful_shutdown(stopped)
                        .await
                    {
                        eprintln!("moshionly: the MCP server stopped: {error}");
                    }
                });
            })?;
        Ok(HttpServer {
            address,
            shutdown: Some(shutdown),
            thread: Some(thread),
        })
    }

    /// The address the server listens on.
    pub fn address(&self) -> SocketAddr {
        self.address
    }

    /// The URL of the MCP endpoint, such as `http://127.0.0.1:47821/mcp`,
    /// to give to agents.
    pub fn url(&self) -> String {
        format!("http://{}{MCP_PATH}", self.address)
    }

    /// Stops the server and waits for it to finish the requests it has.
    pub fn stop(mut self) {
        self.shut_down();
    }

    fn shut_down(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for HttpServer {
    fn drop(&mut self) {
        self.shut_down();
    }
}

/// The legacy sessions, by id, with the version each agreed.
#[derive(Debug, Default)]
struct Sessions {
    versions: HashMap<String, String>,
    order: Vec<String>,
}

impl Sessions {
    fn open(&mut self, version: String) -> String {
        let id = format!(
            "{}{}{}",
            NodeId::generate(),
            NodeId::generate(),
            NodeId::generate()
        );
        if self.order.len() >= MAX_SESSIONS {
            let oldest = self.order.remove(0);
            self.versions.remove(&oldest);
        }
        self.versions.insert(id.clone(), version);
        self.order.push(id.clone());
        id
    }

    fn contains(&self, id: &str) -> bool {
        self.versions.contains_key(id)
    }

    fn close(&mut self, id: &str) -> bool {
        self.order.retain(|open| open != id);
        self.versions.remove(id).is_some()
    }
}

/// What every request handler shares.
struct Shared {
    server: McpServer,
    port: u16,
    sessions: Mutex<Sessions>,
}

impl Shared {
    fn sessions(&self) -> std::sync::MutexGuard<'_, Sessions> {
        self.sessions.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn router(state: Arc<Shared>, max_body_bytes: usize) -> Router {
    Router::new()
        .route(MCP_PATH, any(mcp))
        .route(HEALTH_PATH, get(health))
        .fallback(not_found)
        .layer(DefaultBodyLimit::max(max_body_bytes))
        .with_state(state)
}

/// Every method on `/mcp`.
async fn mcp(
    State(state): State<Arc<Shared>>,
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(refusal) = check_host(&headers, state.port) {
        return refusal;
    }
    if let Some(origin) = header_text(&headers, header::ORIGIN.as_str())
        && !is_own_origin(&origin, state.port)
    {
        return rpc_error(
            StatusCode::FORBIDDEN,
            &Value::Null,
            code::INVALID_REQUEST,
            &format!(
                "Requests from web pages are not accepted (Origin: {origin}). Connect an MCP \
                 client to this address directly."
            ),
        );
    }
    match method {
        Method::POST => post(&state, &headers, &body).await,
        Method::DELETE => delete(&state, &headers),
        // Nothing is ever sent unasked, so there is no stream to open.
        _ => {
            let mut response = plain(StatusCode::METHOD_NOT_ALLOWED, "");
            response
                .headers_mut()
                .insert(header::ALLOW, HeaderValue::from_static("POST, DELETE"));
            response
        }
    }
}

async fn post(state: &Arc<Shared>, headers: &HeaderMap, body: &Bytes) -> Response {
    let message: Value = match serde_json::from_slice(body) {
        Ok(message) => message,
        Err(error) => {
            return rpc_error(
                StatusCode::BAD_REQUEST,
                &Value::Null,
                code::PARSE_ERROR,
                &format!("The body is not JSON: {error}."),
            );
        }
    };
    let is_request = message.get("method").is_some() && message.get("id").is_some();
    let is_initialize = message.get("method").and_then(Value::as_str) == Some("initialize");
    let is_modern = message
        .pointer("/params/_meta")
        .and_then(|meta| meta.get("io.modelcontextprotocol/protocolVersion"))
        .is_some();
    // A request of the earlier revisions belongs to the session that
    // `initialize` opened.
    if is_request && !is_initialize && !is_modern {
        let id = message.get("id").cloned().unwrap_or_default();
        match header_text(headers, SESSION_HEADER) {
            None => {
                return rpc_error(
                    StatusCode::BAD_REQUEST,
                    &id,
                    code::INVALID_REQUEST,
                    "The Mcp-Session-Id header is missing. Start with initialize, or send the \
                     protocol version in params._meta.",
                );
            }
            Some(session) if !state.sessions().contains(&session) => {
                return rpc_error(
                    StatusCode::NOT_FOUND,
                    &id,
                    code::INVALID_REQUEST,
                    "This session has ended. Start a new one with initialize.",
                );
            }
            Some(_) => {}
        }
    }
    let mcp_headers = Headers {
        protocol_version: header_text(headers, "mcp-protocol-version"),
        method: header_text(headers, "mcp-method"),
        name: header_text(headers, "mcp-name"),
    };
    // Tool calls may render or export, so they run off the async threads.
    let shared = Arc::clone(state);
    let outcome = tokio::task::spawn_blocking(move || {
        shared.server.protocol.handle(&message, Some(&mcp_headers))
    })
    .await;
    let outcome = match outcome {
        Ok(outcome) => outcome,
        Err(_) => {
            return rpc_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &Value::Null,
                code::INTERNAL_ERROR,
                "The server failed while answering. This is a bug in Moshionly.",
            );
        }
    };
    match outcome {
        Outcome::Accepted => plain(StatusCode::ACCEPTED, ""),
        Outcome::Reply { response, status } => json_response(
            match status {
                Status::Ok => StatusCode::OK,
                Status::BadRequest => StatusCode::BAD_REQUEST,
                Status::NotFound => StatusCode::NOT_FOUND,
            },
            &response,
        ),
        Outcome::Initialized { response, version } => {
            let session = state.sessions().open(version);
            let mut reply = json_response(StatusCode::OK, &response);
            if let Ok(value) = HeaderValue::from_str(&session) {
                reply.headers_mut().insert(SESSION_HEADER, value);
            }
            reply
        }
    }
}

/// `DELETE /mcp`: a client of the earlier revisions ends its session.
fn delete(state: &Shared, headers: &HeaderMap) -> Response {
    match header_text(headers, SESSION_HEADER) {
        None => plain(
            StatusCode::BAD_REQUEST,
            "The Mcp-Session-Id header is missing.",
        ),
        Some(session) if state.sessions().close(&session) => plain(StatusCode::OK, ""),
        Some(_) => plain(StatusCode::NOT_FOUND, "There is no such session."),
    }
}

/// `GET /health`: whether the app is running, and which version.
async fn health(State(state): State<Arc<Shared>>, headers: HeaderMap) -> Response {
    if let Err(refusal) = check_host(&headers, state.port) {
        return refusal;
    }
    let info = server_info();
    let body = json!({
        "app": "Moshionly",
        "name": info["name"],
        "version": info["version"],
        "mcp": MCP_PATH,
    });
    json_response(StatusCode::OK, &body)
}

async fn not_found() -> Response {
    json_response(
        StatusCode::NOT_FOUND,
        &json!({ "error": format!("Not found. MCP is at {MCP_PATH}.") }),
    )
}

/// Refuses a request whose `Host` is not this machine at this port, which
/// is how a web page would reach the server through a domain it controls.
pub(crate) fn check_host(headers: &HeaderMap, port: u16) -> Result<(), Response> {
    let Some(host) = header_text(headers, header::HOST.as_str()) else {
        return Ok(());
    };
    let (name, given_port) = match host.rsplit_once(':') {
        Some((name, port)) if !name.is_empty() && !port.contains(']') => (name, Some(port)),
        _ => (host.as_str(), None),
    };
    let local = matches!(
        name.to_ascii_lowercase().as_str(),
        "127.0.0.1" | "localhost" | "[::1]"
    );
    let right_port = given_port.is_none_or(|given| given == port.to_string());
    if local && right_port {
        Ok(())
    } else {
        Err(plain(
            StatusCode::FORBIDDEN,
            &format!("This server only answers to 127.0.0.1:{port}, not to \"{host}\"."),
        ))
    }
}

/// Whether `origin` is this server's own address.
pub(crate) fn is_own_origin(origin: &str, port: u16) -> bool {
    let origin = origin.to_ascii_lowercase();
    ["127.0.0.1", "localhost", "[::1]"]
        .iter()
        .any(|host| origin == format!("http://{host}:{port}"))
}

/// A header's value as text, when it is there and is text.
pub(crate) fn header_text(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .map(|value| String::from_utf8_lossy(value.as_bytes()).trim().to_owned())
}

pub(crate) fn json_response(status: StatusCode, body: &Value) -> Response {
    let mut response = Response::new(Body::from(body.to_string()));
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    response
}

fn rpc_error(status: StatusCode, id: &Value, code: i64, message: &str) -> Response {
    let mut body = json!({ "jsonrpc": "2.0", "error": { "code": code, "message": message } });
    if !id.is_null() {
        body["id"] = id.clone();
    }
    json_response(status, &body)
}

pub(crate) fn plain(status: StatusCode, text: &str) -> Response {
    let mut response = Response::new(Body::from(text.to_owned()));
    *response.status_mut() = status;
    if !text.is_empty() {
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/plain; charset=utf-8"),
        );
    }
    response
}

#[cfg(test)]
mod tests;
