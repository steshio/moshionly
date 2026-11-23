//! MCP over HTTP on this machine, at `http://127.0.0.1:47821/mcp`.
//!
//! [`HttpServer::start`] binds the port and serves on a thread of its own,
//! so the app does not need an async runtime. The server only listens on
//! the loopback address, and refuses any request that comes from a web
//! page: agent command-line tools send no `Origin` header, browsers always
//! do, so a request whose `Origin` is not this server's own is turned away
//! before its body is read. The `Host` header is checked too, which stops a
//! web page from reaching the port under a domain name of its own (DNS
//! rebinding).
//!
//! Requests are answered with a single JSON response; the server never
//! needs to stream. Clients of the earlier protocol revisions get a
//! session id from `initialize` and end it with `DELETE`.
//!
//! The same port takes designs from the Figma plugin at `POST /import`
//! and says it is running at `GET /health`. The plugin's panel is a
//! sandboxed page whose origin is `null`, so these two answer that origin's
//! cross-origin requests, including the preflight Chromium sends before a
//! page may reach a private address. Because any sandboxed page looks the
//! same, what arrives is checked and handed to the app as a
//! [`PendingImport`](crate::import::PendingImport) the person accepts or
//! rejects; nothing is applied directly.

use std::collections::HashMap;
use std::io;
use std::net::{Ipv4Addr, SocketAddr, TcpListener as StdListener};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::rejection::BytesRejection;
use axum::extract::{DefaultBodyLimit, FromRequest, Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use moshionly_core::NodeId;
use serde_json::{Value, json};
use tokio::sync::oneshot;

use super::McpServer;
use super::protocol::{Headers, Outcome, Status, code, server_info};
use crate::import::{Delivery, ImportSink, MAX_WAITING, read_envelope};

/// The port the app listens on unless told otherwise.
pub const DEFAULT_PORT: u16 = 47821;
/// The path of the MCP endpoint.
pub const MCP_PATH: &str = "/mcp";
/// The path that says the app is running.
pub const HEALTH_PATH: &str = "/health";
/// The path the Figma plugin sends designs to.
pub const IMPORT_PATH: &str = "/import";

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
    /// app. Designs sent to `/import` are refused; use
    /// [`HttpServer::start_with_imports`] to take them.
    pub fn start(server: McpServer, config: HttpConfig) -> io::Result<HttpServer> {
        Self::serve(server, config, None)
    }

    /// Like [`HttpServer::start`], and hands designs sent to `/import` to
    /// `imports` once they pass the checks.
    pub fn start_with_imports(
        server: McpServer,
        config: HttpConfig,
        imports: ImportSink,
    ) -> io::Result<HttpServer> {
        Self::serve(server, config, Some(imports))
    }

    fn serve(
        server: McpServer,
        config: HttpConfig,
        imports: Option<ImportSink>,
    ) -> io::Result<HttpServer> {
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
            imports,
            port: address.port(),
            max_body_bytes: config.max_body_bytes,
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
    imports: Option<ImportSink>,
    port: u16,
    max_body_bytes: usize,
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
        .route(HEALTH_PATH, any(health))
        .route(IMPORT_PATH, any(import))
        .fallback(not_found)
        .layer(DefaultBodyLimit::max(max_body_bytes))
        .with_state(state)
}

/// Every method on `/mcp`. The request is checked before its body is read,
/// so a refused one costs nothing.
async fn mcp(
    State(state): State<Arc<Shared>>,
    method: Method,
    headers: HeaderMap,
    request: Request,
) -> Response {
    if let Some(refusal) = foreign_host(&headers, state.port) {
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
        Method::POST => match read_body(request).await {
            Ok(body) => post(&state, &headers, &body).await,
            Err(rejection) => rejection.into_response(),
        },
        Method::DELETE => delete(&state, &headers),
        // Nothing is ever sent unasked, so there is no stream to open.
        _ => not_allowed("POST, DELETE"),
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
async fn health(State(state): State<Arc<Shared>>, method: Method, headers: HeaderMap) -> Response {
    let origin = match plugin_origin(&headers, state.port) {
        Ok(origin) => origin,
        Err(refusal) => return *refusal,
    };
    let response = match method {
        Method::OPTIONS => return preflight(&headers, origin.as_deref(), "GET, OPTIONS"),
        Method::GET | Method::HEAD => {
            let info = server_info();
            json_response(
                StatusCode::OK,
                &json!({
                    "app": "Moshionly",
                    "name": info["name"],
                    "version": info["version"],
                    "mcp": MCP_PATH,
                    "import": IMPORT_PATH,
                }),
            )
        }
        _ => not_allowed("GET, OPTIONS"),
    };
    with_cors(response, origin.as_deref())
}

/// `POST /import`: a design from the Figma plugin. The request is checked
/// before its body is read, and every answer to an allowed origin carries
/// the CORS header, so the plugin can show why a design was refused.
async fn import(
    State(state): State<Arc<Shared>>,
    method: Method,
    headers: HeaderMap,
    request: Request,
) -> Response {
    let origin = match plugin_origin(&headers, state.port) {
        Ok(origin) => origin,
        Err(refusal) => return *refusal,
    };
    let response = match method {
        Method::OPTIONS => return preflight(&headers, origin.as_deref(), "POST, OPTIONS"),
        Method::POST => match read_body(request).await {
            Ok(body) => {
                // Checking a design is work for a while, so it runs off the
                // async threads, as tool calls do.
                let shared = Arc::clone(&state);
                tokio::task::spawn_blocking(move || receive(&shared, &body))
                    .await
                    .unwrap_or_else(|_| {
                        refuse(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "Moshionly failed while reading the design. This is a bug in \
                             Moshionly."
                                .to_owned(),
                        )
                    })
            }
            Err(rejection) if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE => refuse(
                StatusCode::PAYLOAD_TOO_LARGE,
                format!(
                    "The design is larger than Moshionly takes ({}). Send fewer layers or \
                     smaller images.",
                    size_text(state.max_body_bytes)
                ),
            ),
            Err(rejection) => refuse(
                rejection.status(),
                format!("The design could not be read: {}.", rejection.body_text()),
            ),
        },
        _ => not_allowed("POST, OPTIONS"),
    };
    with_cors(response, origin.as_deref())
}

/// The body of a request, read up to the server's size limit.
async fn read_body(request: Request) -> Result<Bytes, BytesRejection> {
    Bytes::from_request(request, &()).await
}

/// A refusal at `/import`, in the form the plugin reads.
fn refuse(status: StatusCode, message: String) -> Response {
    json_response(status, &json!({ "ok": false, "error": message }))
}

/// A size in bytes as a person reads it: `64 MB`, `512 KB`.
fn size_text(bytes: usize) -> String {
    const MB: usize = 1024 * 1024;
    if bytes >= MB {
        format!("{} MB", bytes / MB)
    } else {
        format!("{} KB", bytes.div_ceil(1024))
    }
}

/// Checks a design and hands it to the app.
fn receive(state: &Shared, body: &Bytes) -> Response {
    let Some(sink) = &state.imports else {
        return refuse(
            StatusCode::SERVICE_UNAVAILABLE,
            "This Moshionly is not taking designs. Open the Moshionly app and send again."
                .to_owned(),
        );
    };
    let pending = match read_envelope(body) {
        Ok(pending) => pending,
        Err(error) => return refuse(StatusCode::BAD_REQUEST, capitalised(&error.to_string())),
    };
    let (layers, notes) = (pending.nodes.len(), pending.notes.len());
    match sink.deliver(pending) {
        Delivery::Delivered => {}
        Delivery::Full => {
            return refuse(
                StatusCode::TOO_MANY_REQUESTS,
                format!(
                    "Moshionly already has {MAX_WAITING} designs waiting to be accepted. Answer \
                     those in the app, then send again."
                ),
            );
        }
        Delivery::Closed => {
            return refuse(
                StatusCode::SERVICE_UNAVAILABLE,
                "Moshionly is closing and cannot take the design. Send it again once it is \
                 open."
                    .to_owned(),
            );
        }
    }
    json_response(
        StatusCode::ACCEPTED,
        &json!({
            "ok": true,
            "status": "pending",
            "layers": layers,
            "notes": notes,
            "message": "Moshionly received the design and is asking where to put it.",
        }),
    )
}

/// The origin of a request to `/health` or `/import`, when it is one the
/// plugin's panel can have: none, `null`, or this server's own. Anything
/// else is refused.
fn plugin_origin(headers: &HeaderMap, port: u16) -> Result<Option<String>, Box<Response>> {
    if let Some(refusal) = foreign_host(headers, port) {
        return Err(Box::new(refusal));
    }
    match header_text(headers, header::ORIGIN.as_str()) {
        None => Ok(None),
        Some(origin) if origin == "null" || is_own_origin(&origin, port) => Ok(Some(origin)),
        Some(origin) => Err(Box::new(json_response(
            StatusCode::FORBIDDEN,
            &json!({ "ok": false, "error": format!(
                "Requests from {origin} are not accepted. Designs come from the Moshionly \
                 Figma plugin."
            ) }),
        ))),
    }
}

/// The answer to a CORS preflight from an allowed origin, including the
/// private network part Chromium asks for before a page may reach a
/// loopback address.
fn preflight(headers: &HeaderMap, origin: Option<&str>, methods: &'static str) -> Response {
    let mut response = with_cors(plain(StatusCode::NO_CONTENT, ""), origin);
    let out = response.headers_mut();
    out.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static(methods),
    );
    out.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("Content-Type"),
    );
    out.insert(
        header::ACCESS_CONTROL_MAX_AGE,
        HeaderValue::from_static("600"),
    );
    let private_network = header_text(headers, "access-control-request-private-network");
    if private_network.as_deref() == Some("true") {
        out.insert(
            "access-control-allow-private-network",
            HeaderValue::from_static("true"),
        );
    }
    response
}

/// The response with the CORS header that lets `origin` read it.
fn with_cors(mut response: Response, origin: Option<&str>) -> Response {
    if let Some(value) = origin.and_then(|origin| HeaderValue::from_str(origin).ok()) {
        let out = response.headers_mut();
        out.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, value);
        out.insert(header::VARY, HeaderValue::from_static("Origin"));
    }
    response
}

fn not_allowed(methods: &'static str) -> Response {
    let mut response = plain(StatusCode::METHOD_NOT_ALLOWED, "");
    response
        .headers_mut()
        .insert(header::ALLOW, HeaderValue::from_static(methods));
    response
}

fn capitalised(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

async fn not_found() -> Response {
    json_response(
        StatusCode::NOT_FOUND,
        &json!({ "error": format!("Not found. MCP is at {MCP_PATH}.") }),
    )
}

/// The refusal for a request whose `Host` is not this machine at this
/// port, which is how a web page would reach the server through a domain
/// it controls. `None` when the host is fine.
pub(crate) fn foreign_host(headers: &HeaderMap, port: u16) -> Option<Response> {
    let host = header_text(headers, header::HOST.as_str())?;
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
        return None;
    }
    Some(plain(
        StatusCode::FORBIDDEN,
        &format!("This server only answers to 127.0.0.1:{port}, not to \"{host}\"."),
    ))
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
