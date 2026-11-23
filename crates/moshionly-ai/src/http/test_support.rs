//! A local HTTP server that plays scripted responses, so the API providers
//! can be tested from request to events without a key or a network.
//!
//! Each request gets the next response in the script, whatever its path, and
//! is recorded so a test can check what was sent.

use std::collections::{HashMap, VecDeque};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::{Request, State};
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum::response::Response;
use serde_json::Value;

/// One scripted response.
#[derive(Debug, Clone)]
pub(crate) struct MockResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: MockBody,
}

#[derive(Debug, Clone)]
enum MockBody {
    Whole(String),
    /// Sent piece by piece; when `hang` is set the response never ends.
    Stream {
        pieces: Vec<String>,
        hang: bool,
    },
}

impl MockResponse {
    /// A JSON response with the given status.
    pub fn json(status: u16, body: Value) -> Self {
        Self {
            status,
            headers: vec![("content-type".to_owned(), "application/json".to_owned())],
            body: MockBody::Whole(body.to_string()),
        }
    }

    /// A plain text response with the given status.
    pub fn text(status: u16, body: &str) -> Self {
        Self {
            status,
            headers: vec![("content-type".to_owned(), "text/plain".to_owned())],
            body: MockBody::Whole(body.to_owned()),
        }
    }

    /// An event stream, sent in the given pieces and then closed.
    pub fn sse<S: AsRef<str>>(pieces: &[S]) -> Self {
        Self::stream(pieces, false)
    }

    /// An event stream that sends the given pieces and then stays open
    /// without sending anything more, like a model that has stalled.
    pub fn sse_then_hang<S: AsRef<str>>(pieces: &[S]) -> Self {
        Self::stream(pieces, true)
    }

    fn stream<S: AsRef<str>>(pieces: &[S], hang: bool) -> Self {
        Self {
            status: 200,
            headers: vec![("content-type".to_owned(), "text/event-stream".to_owned())],
            body: MockBody::Stream {
                pieces: pieces
                    .iter()
                    .map(|piece| piece.as_ref().to_owned())
                    .collect(),
                hang,
            },
        }
    }

    /// Adds a response header.
    pub fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }
}

/// Formats one server-sent event carrying `data` as JSON.
pub(crate) fn sse_event(event: &str, data: &Value) -> String {
    if event.is_empty() {
        format!("data: {data}\n\n")
    } else {
        format!("event: {event}\ndata: {data}\n\n")
    }
}

/// A request the server received.
#[derive(Debug, Clone)]
pub(crate) struct RecordedRequest {
    /// The HTTP method.
    pub method: String,
    /// The path with its query string.
    pub path: String,
    /// Header names in lower case.
    pub headers: HashMap<String, String>,
    /// The body as JSON, or `Null` when it was not JSON.
    pub body: Value,
}

impl RecordedRequest {
    /// The value of a header, or the empty string.
    pub fn header(&self, name: &str) -> &str {
        self.headers.get(name).map_or("", String::as_str)
    }
}

#[derive(Clone)]
struct Shared {
    script: Arc<Mutex<VecDeque<MockResponse>>>,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
}

/// A running mock server. It stops when dropped.
pub(crate) struct MockServer {
    addr: SocketAddr,
    shared: Shared,
    task: tokio::task::JoinHandle<()>,
}

impl MockServer {
    /// Starts a server that answers with `script`, one response per request.
    /// A request beyond the end of the script gets a 500.
    pub async fn start(script: Vec<MockResponse>) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let shared = Shared {
            script: Arc::new(Mutex::new(script.into())),
            requests: Arc::default(),
        };
        let router = Router::new().fallback(handle).with_state(shared.clone());
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        Self { addr, shared, task }
    }

    /// The server's base URL, without a trailing slash.
    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Every request received so far, in order.
    pub fn requests(&self) -> Vec<RecordedRequest> {
        self.shared.requests.lock().unwrap().clone()
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn handle(State(shared): State<Shared>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let bytes = axum::body::to_bytes(body, usize::MAX)
        .await
        .unwrap_or_default();
    let headers = parts
        .headers
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_owned(),
                value.to_str().unwrap_or("").to_owned(),
            )
        })
        .collect();
    shared.requests.lock().unwrap().push(RecordedRequest {
        method: parts.method.to_string(),
        path: parts.uri.path_and_query().map_or_else(
            || parts.uri.path().to_owned(),
            |path| path.as_str().to_owned(),
        ),
        headers,
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    });

    let next = shared.script.lock().unwrap().pop_front();
    let Some(scripted) = next else {
        return respond(
            500,
            &[],
            Body::from("the test script has no more responses"),
        );
    };
    let body = match scripted.body {
        MockBody::Whole(text) => Body::from(text),
        MockBody::Stream { pieces, hang } => Body::from_stream(piece_stream(pieces, hang)),
    };
    respond(scripted.status, &scripted.headers, body)
}

fn respond(status: u16, headers: &[(String, String)], body: Body) -> Response {
    let mut response = Response::new(body);
    *response.status_mut() = StatusCode::from_u16(status).unwrap();
    for (name, value) in headers {
        response.headers_mut().insert(
            HeaderName::from_bytes(name.as_bytes()).unwrap(),
            HeaderValue::from_str(value).unwrap(),
        );
    }
    response
}

/// Sends the pieces one at a time with a short pause between them, so that
/// they reach the client as separate chunks.
fn piece_stream(
    pieces: Vec<String>,
    hang: bool,
) -> impl futures_util::Stream<Item = Result<Bytes, Infallible>> {
    let pieces: VecDeque<String> = pieces.into();
    futures_util::stream::unfold(pieces, move |mut pieces| async move {
        match pieces.pop_front() {
            Some(piece) => {
                tokio::time::sleep(Duration::from_millis(2)).await;
                Some((Ok(Bytes::from(piece)), pieces))
            }
            None if hang => std::future::pending().await,
            None => None,
        }
    })
}
