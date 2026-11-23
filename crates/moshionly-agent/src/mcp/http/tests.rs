use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use serde_json::{Value, json};

use super::*;
use crate::tools::testing::{demo_session, demo_tools};

/// A started server on a free port.
fn start() -> HttpServer {
    let config = HttpConfig {
        port: 0,
        ..HttpConfig::default()
    };
    HttpServer::start(McpServer::with_instructions(demo_tools(), "Hello."), config)
        .expect("a free port can be bound")
}

/// What came back from the server.
#[derive(Debug)]
struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl Reply {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or_else(|error| panic!("{error}: {}", self.body))
    }
}

/// Sends one HTTP/1.1 request and reads the whole answer.
fn send(
    server: &HttpServer,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> Reply {
    let mut stream = TcpStream::connect(server.address()).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut request = format!("{method} {path} HTTP/1.1\r\nConnection: close\r\n");
    if !headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("host"))
    {
        request.push_str(&format!("Host: {}\r\n", server.address()));
    }
    for (name, value) in headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str(&format!("Content-Length: {}\r\n\r\n{body}", body.len()));
    stream.write_all(request.as_bytes()).unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).unwrap();
    let raw = String::from_utf8(raw).unwrap();
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
    let mut lines = head.lines();
    let status = lines
        .next()
        .unwrap()
        .split(' ')
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let headers = lines
        .filter_map(|line| line.split_once(": "))
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect();
    Reply {
        status,
        headers,
        body: body.to_owned(),
    }
}

const JSON: (&str, &str) = ("Content-Type", "application/json");
const ACCEPT: (&str, &str) = ("Accept", "application/json, text/event-stream");

fn post(server: &HttpServer, extra: &[(&str, &str)], body: &Value) -> Reply {
    let mut headers = vec![JSON, ACCEPT];
    headers.extend_from_slice(extra);
    send(server, "POST", "/mcp", &headers, &body.to_string())
}

#[test]
fn a_legacy_client_initialises_lists_calls_and_ends_its_session() {
    let server = start();
    assert!(server.url().starts_with("http://127.0.0.1:"));
    assert!(server.url().ends_with("/mcp"));

    let reply = post(
        &server,
        &[],
        &json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-06-18", "capabilities": {},
            "clientInfo": { "name": "test", "version": "1" } } }),
    );
    assert_eq!(reply.status, 200, "{}", reply.body);
    assert_eq!(reply.header("content-type"), Some("application/json"));
    let session = reply.header("mcp-session-id").unwrap().to_owned();
    assert_eq!(reply.json()["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(reply.json()["result"]["instructions"], "Hello.");

    let with_session = [
        ("Mcp-Session-Id", session.as_str()),
        ("MCP-Protocol-Version", "2025-06-18"),
    ];
    let reply = post(
        &server,
        &with_session,
        &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
    );
    assert_eq!((reply.status, reply.body.as_str()), (202, ""));

    let reply = post(
        &server,
        &with_session,
        &json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
    );
    assert_eq!(reply.status, 200);
    assert!(reply.json()["result"]["tools"].as_array().unwrap().len() > 5);

    let reply = post(
        &server,
        &with_session,
        &json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call",
                 "params": { "name": "apply_edits", "arguments": { "operations": [
                     { "op": "update", "id": "dot", "set": { "x": 12 } } ] } } }),
    );
    assert_eq!(reply.json()["result"]["isError"], false, "{}", reply.body);

    let reply = send(
        &server,
        "DELETE",
        "/mcp",
        &[("Mcp-Session-Id", &session)],
        "",
    );
    assert_eq!(reply.status, 200);
    let reply = post(
        &server,
        &with_session,
        &json!({ "jsonrpc": "2.0", "id": 4, "method": "ping" }),
    );
    assert_eq!(reply.status, 404, "an ended session is gone");
    let reply = send(
        &server,
        "DELETE",
        "/mcp",
        &[("Mcp-Session-Id", &session)],
        "",
    );
    assert_eq!(reply.status, 404);
    let reply = send(&server, "DELETE", "/mcp", &[], "");
    assert_eq!(reply.status, 400);
}

#[test]
fn a_legacy_request_needs_its_session() {
    let server = start();
    let reply = post(
        &server,
        &[],
        &json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }),
    );
    assert_eq!(reply.status, 400);
    assert_eq!(reply.json()["id"], 1);
    assert!(
        reply.json()["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Mcp-Session-Id")
    );
    let reply = post(
        &server,
        &[("Mcp-Session-Id", "made-up")],
        &json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }),
    );
    assert_eq!(reply.status, 404);
}

#[test]
fn a_modern_client_needs_no_session() {
    let server = start();
    let meta = json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientCapabilities": {}
    });
    let headers = [
        ("MCP-Protocol-Version", "2026-07-28"),
        ("Mcp-Method", "tools/call"),
        ("Mcp-Name", "get_overview"),
        // A stray session id is ignored.
        ("Mcp-Session-Id", "ignored"),
    ];
    let reply = post(
        &server,
        &headers,
        &json!({ "jsonrpc": "2.0", "id": "c1", "method": "tools/call",
                 "params": { "name": "get_overview", "_meta": meta } }),
    );
    assert_eq!(reply.status, 200, "{}", reply.body);
    assert!(reply.header("mcp-session-id").is_none());
    let result = &reply.json()["result"];
    assert_eq!(result["resultType"], "complete");
    let overview: Value =
        serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(overview["name"], "Launch day");

    let reply = post(
        &server,
        &[
            ("MCP-Protocol-Version", "2026-07-28"),
            ("Mcp-Method", "tools/list"),
        ],
        &json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call",
                 "params": { "name": "get_overview", "_meta": meta } }),
    );
    assert_eq!(reply.status, 400);
    assert_eq!(reply.json()["error"]["code"], -32020);

    let reply = post(
        &server,
        &[
            ("MCP-Protocol-Version", "2026-07-28"),
            ("Mcp-Method", "ping"),
        ],
        &json!({ "jsonrpc": "2.0", "id": 3, "method": "ping", "params": { "_meta": meta } }),
    );
    assert_eq!(reply.status, 404);
    assert_eq!(reply.json()["error"]["code"], -32601);
}

#[test]
fn web_pages_are_refused() {
    let server = start();
    let ping = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize",
                       "params": { "protocolVersion": "2025-11-25" } });
    for origin in ["https://evil.example", "null", "http://localhost:1"] {
        let reply = post(&server, &[("Origin", origin)], &ping);
        assert_eq!(reply.status, 403, "{origin}");
        let body = reply.json();
        assert!(body.get("id").is_none());
        assert!(
            body["error"]["message"]
                .as_str()
                .unwrap()
                .contains("web pages")
        );
    }
    let own = format!("http://localhost:{}", server.address().port());
    assert_eq!(
        post(&server, &[("Origin", own.as_str())], &ping).status,
        200
    );

    // A page that reached the port under its own domain name.
    let reply = post(&server, &[("Host", "evil.example:47821")], &ping);
    assert_eq!(reply.status, 403);
    let reply = send(&server, "GET", "/health", &[("Host", "evil.example")], "");
    assert_eq!(reply.status, 403);
}

#[test]
fn get_has_no_stream_and_bad_bodies_are_refused() {
    let server = start();
    let reply = send(&server, "GET", "/mcp", &[ACCEPT], "");
    assert_eq!(reply.status, 405);
    assert_eq!(reply.header("allow"), Some("POST, DELETE"));

    let reply = send(&server, "POST", "/mcp", &[JSON], "{oops");
    assert_eq!(reply.status, 400);
    assert_eq!(reply.json()["error"]["code"], -32700);

    let small = HttpServer::start(
        McpServer::new(demo_tools()),
        HttpConfig {
            port: 0,
            max_body_bytes: 64,
        },
    )
    .unwrap();
    let reply = send(&small, "POST", "/mcp", &[JSON], &"x".repeat(200));
    assert_eq!(reply.status, 413);
}

#[test]
fn a_foreign_request_is_refused_before_its_body_is_read() {
    // The body is over the limit: a request whose body was read first
    // would be answered 413.
    let (server, inbox, _) = start_with_imports_up_to(64);
    let body = "x".repeat(200);
    for (path, headers) in [
        ("/mcp", [JSON, ("Origin", "https://evil.example")]),
        ("/mcp", [JSON, ("Host", "evil.example:47821")]),
        ("/import", [JSON, ("Origin", "https://evil.example")]),
        ("/import", [JSON, ("Host", "evil.example")]),
    ] {
        let reply = send(&server, "POST", path, &headers, &body);
        assert_eq!(reply.status, 403, "{path} {headers:?}");
    }
    assert!(inbox.try_recv().is_err());
}

#[test]
fn health_says_what_is_running() {
    let server = start();
    let reply = send(&server, "GET", "/health", &[], "");
    assert_eq!(reply.status, 200);
    let body = reply.json();
    assert_eq!(body["app"], "Moshionly");
    assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(body["mcp"], "/mcp");
    assert_eq!(send(&server, "GET", "/nothing", &[], "").status, 404);
}

#[test]
fn a_taken_port_is_reported_and_stopping_frees_it() {
    let server = start();
    let port = server.address().port();
    let config = HttpConfig {
        port,
        ..HttpConfig::default()
    };
    assert!(HttpServer::start(McpServer::new(demo_tools()), config.clone()).is_err());
    server.stop();
    let again = HttpServer::start(McpServer::new(demo_tools()), config).unwrap();
    assert_eq!(again.address().port(), port);
}

fn envelope() -> Value {
    json!({
        "format": "moshionly-import", "version": 1,
        "source": { "app": "figma", "file_name": "Launch", "file_key": "abc" },
        "nodes": [
            { "id": "1:1", "type": "artboard", "name": "Frame", "width": 400, "height": 300,
              "source_id": "figma:abc:1:1" },
            { "id": "1:2", "parent": "1:1", "type": "rect", "fill": "#0D99FF",
              "source_id": "figma:abc:1:2" }
        ],
        "notes": []
    })
}

/// A server that takes designs, its inbox, and the session it serves.
fn start_with_imports() -> (
    HttpServer,
    std::sync::mpsc::Receiver<crate::import::PendingImport>,
    crate::SessionHandle,
) {
    start_with_imports_up_to(HttpConfig::default().max_body_bytes)
}

/// Like [`start_with_imports`], taking bodies up to `max_body_bytes`.
fn start_with_imports_up_to(
    max_body_bytes: usize,
) -> (
    HttpServer,
    std::sync::mpsc::Receiver<crate::import::PendingImport>,
    crate::SessionHandle,
) {
    let session = demo_session();
    let tools = crate::Tools::new(session.clone(), crate::Host::new());
    let (sink, inbox) = ImportSink::new();
    let server = HttpServer::start_with_imports(
        McpServer::new(tools),
        HttpConfig {
            port: 0,
            max_body_bytes,
        },
        sink,
    )
    .unwrap();
    (server, inbox, session)
}

#[test]
fn the_plugin_panel_may_send_a_design_after_its_preflight() {
    let (server, inbox, session) = start_with_imports();
    let reply = send(
        &server,
        "OPTIONS",
        "/import",
        &[
            ("Origin", "null"),
            ("Access-Control-Request-Method", "POST"),
            ("Access-Control-Request-Headers", "content-type"),
            ("Access-Control-Request-Private-Network", "true"),
        ],
        "",
    );
    assert_eq!(reply.status, 204);
    assert_eq!(reply.header("access-control-allow-origin"), Some("null"));
    assert_eq!(
        reply.header("access-control-allow-methods"),
        Some("POST, OPTIONS")
    );
    assert_eq!(
        reply.header("access-control-allow-headers"),
        Some("Content-Type")
    );
    assert_eq!(
        reply.header("access-control-allow-private-network"),
        Some("true")
    );

    let reply = send(
        &server,
        "POST",
        "/import",
        &[("Origin", "null"), JSON],
        &envelope().to_string(),
    );
    assert_eq!(reply.status, 202, "{}", reply.body);
    assert_eq!(reply.header("access-control-allow-origin"), Some("null"));
    assert_eq!(reply.json()["status"], "pending");
    assert_eq!(reply.json()["layers"], 2);
    let pending = inbox.try_recv().unwrap();
    assert_eq!(pending.source.file_name.as_deref(), Some("Launch"));
    // Nothing was applied: the app decides.
    assert_eq!(session.document().artboards().len(), 2);
    assert_eq!(session.revision(), 0);
}

#[test]
fn a_bad_design_is_refused_with_a_reason_the_plugin_can_show() {
    let (server, inbox, _) = start_with_imports();
    let mut bad = envelope();
    bad["nodes"][1]["fil"] = json!("#000000");
    let reply = send(
        &server,
        "POST",
        "/import",
        &[("Origin", "null"), JSON],
        &bad.to_string(),
    );
    assert_eq!(reply.status, 400);
    assert_eq!(reply.header("access-control-allow-origin"), Some("null"));
    assert_eq!(reply.json()["ok"], false);
    let error = reply.json()["error"].as_str().unwrap().to_owned();
    assert!(error.starts_with("The design cannot be used"), "{error}");
    assert!(error.contains("Did you mean \"fill\"?"), "{error}");
    assert!(inbox.try_recv().is_err());

    let reply = send(&server, "POST", "/import", &[JSON], "[1, 2]");
    assert_eq!(reply.status, 400);
    assert!(reply.header("access-control-allow-origin").is_none());
}

#[test]
fn a_design_too_large_is_refused_in_a_way_the_plugin_can_read() {
    let (server, inbox, _) = start_with_imports_up_to(64);
    let reply = send(
        &server,
        "POST",
        "/import",
        &[("Origin", "null"), JSON],
        &envelope().to_string(),
    );
    assert_eq!(reply.status, 413);
    assert_eq!(reply.header("access-control-allow-origin"), Some("null"));
    assert_eq!(reply.json()["ok"], false);
    let error = reply.json()["error"].as_str().unwrap().to_owned();
    assert!(
        error.starts_with("The design is larger than Moshionly takes (1 KB)"),
        "{error}"
    );
    assert!(inbox.try_recv().is_err());
}

#[test]
fn designs_beyond_the_few_waiting_are_refused_until_the_app_takes_one() {
    let (server, inbox, _) = start_with_imports();
    let post_design = || {
        send(
            &server,
            "POST",
            "/import",
            &[("Origin", "null"), JSON],
            &envelope().to_string(),
        )
    };
    for _ in 0..crate::import::MAX_WAITING {
        assert_eq!(post_design().status, 202);
    }
    let reply = post_design();
    assert_eq!(reply.status, 429);
    assert_eq!(reply.header("access-control-allow-origin"), Some("null"));
    let error = reply.json()["error"].as_str().unwrap().to_owned();
    assert!(error.contains("designs waiting to be accepted"), "{error}");

    inbox.try_recv().unwrap();
    assert_eq!(post_design().status, 202);
}

#[test]
fn other_pages_and_methods_are_refused_at_import() {
    let (server, inbox, _) = start_with_imports();
    let reply = send(
        &server,
        "POST",
        "/import",
        &[("Origin", "https://evil.example"), JSON],
        &envelope().to_string(),
    );
    assert_eq!(reply.status, 403);
    assert!(reply.header("access-control-allow-origin").is_none());
    let reply = send(
        &server,
        "OPTIONS",
        "/import",
        &[("Origin", "https://evil.example")],
        "",
    );
    assert_eq!(reply.status, 403);
    assert_eq!(send(&server, "GET", "/import", &[], "").status, 405);
    assert!(inbox.try_recv().is_err());

    // A server that does not take designs says so.
    let plain_server = start();
    let reply = send(
        &plain_server,
        "POST",
        "/import",
        &[JSON],
        &envelope().to_string(),
    );
    assert_eq!(reply.status, 503);
    assert!(
        reply.json()["error"]
            .as_str()
            .unwrap()
            .contains("not taking designs")
    );
}

#[test]
fn the_plugin_can_ask_whether_the_app_is_running() {
    let server = start();
    let reply = send(&server, "GET", "/health", &[("Origin", "null")], "");
    assert_eq!(reply.status, 200);
    assert_eq!(reply.header("access-control-allow-origin"), Some("null"));
    assert_eq!(reply.json()["import"], "/import");
    let reply = send(
        &server,
        "OPTIONS",
        "/health",
        &[
            ("Origin", "null"),
            ("Access-Control-Request-Private-Network", "true"),
        ],
        "",
    );
    assert_eq!(reply.status, 204);
    assert_eq!(
        reply.header("access-control-allow-private-network"),
        Some("true")
    );
    let reply = send(
        &server,
        "GET",
        "/health",
        &[("Origin", "https://evil.example")],
        "",
    );
    assert_eq!(reply.status, 403);
}
