use std::sync::Arc;

use serde_json::{Value, json};

use super::*;
use crate::Host;
use crate::tools::testing::{StubFrames, demo_session, demo_tools};

fn protocol() -> Protocol {
    Protocol::new(demo_tools(), "Be kind to the file.")
}

fn request(method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": 7, "method": method, "params": params })
}

/// The `_meta` of a modern request.
fn meta() -> Value {
    json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientCapabilities": {},
        "io.modelcontextprotocol/clientInfo": { "name": "test", "version": "1" }
    })
}

/// A modern request, with `params` and the `_meta` merged.
fn modern(method: &str, mut params: Value) -> Value {
    params["_meta"] = meta();
    request(method, params)
}

/// The headers a modern HTTP request carries.
fn headers(method: &str, name: Option<&str>) -> Headers {
    Headers {
        protocol_version: Some("2026-07-28".into()),
        method: Some(method.into()),
        name: name.map(Into::into),
    }
}

fn reply(outcome: &Outcome) -> (&Value, Status) {
    match outcome {
        Outcome::Reply { response, status } => (response, *status),
        other => panic!("expected a reply, got {other:?}"),
    }
}

fn error_code(outcome: &Outcome) -> (i64, Status) {
    let (response, status) = reply(outcome);
    let code = response["error"]["code"]
        .as_i64()
        .unwrap_or_else(|| panic!("not an error: {response}"));
    (code, status)
}

#[test]
fn initialize_agrees_on_a_legacy_version() {
    let protocol = protocol();
    for (asked, agreed) in [
        ("2025-06-18", "2025-06-18"),
        ("2025-11-25", "2025-11-25"),
        ("2024-11-05", "2025-11-25"),
        ("2026-07-28", "2025-11-25"),
    ] {
        let outcome = protocol.handle(
            &request(
                "initialize",
                json!({ "protocolVersion": asked, "capabilities": {},
                        "clientInfo": { "name": "t", "version": "1" } }),
            ),
            None,
        );
        let Outcome::Initialized { response, version } = outcome else {
            panic!("initialize failed for {asked}");
        };
        assert_eq!(version, agreed);
        let result = &response["result"];
        assert_eq!(result["protocolVersion"], agreed);
        assert_eq!(
            result["capabilities"],
            json!({ "tools": { "listChanged": false } })
        );
        assert_eq!(result["serverInfo"]["name"], "moshionly");
        assert_eq!(result["instructions"], "Be kind to the file.");
        assert_eq!(response["id"], 7);
    }
    let outcome = protocol.handle(&request("initialize", json!({})), None);
    assert_eq!(
        error_code(&outcome),
        (code::INVALID_PARAMS, Status::BadRequest)
    );
}

#[test]
fn notifications_and_client_responses_need_no_answer() {
    let protocol = protocol();
    for message in [
        json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
        json!({ "jsonrpc": "2.0", "method": "notifications/cancelled",
                "params": { "requestId": 3 } }),
        json!({ "jsonrpc": "2.0", "id": 4, "result": {} }),
    ] {
        assert_eq!(
            protocol.handle(&message, None),
            Outcome::Accepted,
            "{message}"
        );
    }
}

#[test]
fn legacy_requests_ping_list_and_call() {
    let protocol = protocol();
    let outcome = protocol.handle(&request("ping", json!({})), None);
    let (response, status) = reply(&outcome);
    assert_eq!(
        (response["result"].clone(), status),
        (json!({}), Status::Ok)
    );

    let outcome = protocol.handle(&request("tools/list", json!({})), None);
    let (response, _) = reply(&outcome);
    let tools = response["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), protocol.tools().list().len());
    assert!(response["result"].get("resultType").is_none());
    let apply = tools
        .iter()
        .find(|tool| tool["name"] == "apply_edits")
        .unwrap();
    assert_eq!(apply["inputSchema"]["type"], "object");
    assert_eq!(
        apply["annotations"],
        json!({ "title": "Edit the file", "readOnlyHint": false, "destructiveHint": true,
                "openWorldHint": false })
    );
    let overview = &tools[0];
    assert_eq!(overview["annotations"]["readOnlyHint"], true);
    assert!(overview["annotations"].get("destructiveHint").is_none());

    let outcome = protocol.handle(
        &request(
            "tools/call",
            json!({ "name": "get_node", "arguments": { "id": "dot" } }),
        ),
        None,
    );
    let (response, _) = reply(&outcome);
    let result = &response["result"];
    assert_eq!(result["isError"], false);
    assert_eq!(result["content"][0]["type"], "text");
    let node: Value = serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(node["node"]["name"], "Dot");
}

#[test]
fn a_tool_error_is_a_result_and_an_unknown_tool_a_protocol_error() {
    let protocol = protocol();
    let outcome = protocol.handle(
        &request("tools/call", json!({ "name": "get_node", "arguments": {} })),
        None,
    );
    let (response, _) = reply(&outcome);
    assert_eq!(response["result"]["isError"], true);
    assert!(
        response["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("\"id\" is required")
    );

    let outcome = protocol.handle(&request("tools/call", json!({ "name": "fly" })), None);
    let (response, _) = reply(&outcome);
    assert_eq!(response["error"]["code"], code::INVALID_PARAMS);
    assert_eq!(response["error"]["message"], "Unknown tool: fly");

    let outcome = protocol.handle(
        &request(
            "tools/call",
            json!({ "name": "get_node", "arguments": [1] }),
        ),
        None,
    );
    assert_eq!(error_code(&outcome).0, code::INVALID_PARAMS);
    // Missing arguments are an empty object.
    let outcome = protocol.handle(
        &request("tools/call", json!({ "name": "get_overview" })),
        None,
    );
    assert_eq!(reply(&outcome).0["result"]["isError"], false);
}

#[test]
fn images_are_sent_as_base64() {
    let protocol = Protocol::new(
        crate::Tools::new(
            demo_session(),
            Host::new().with_frames(Arc::new(StubFrames::default())),
        ),
        "",
    );
    let outcome = protocol.handle(
        &request(
            "tools/call",
            json!({ "name": "screenshot", "arguments": { "artboard": "scene" } }),
        ),
        None,
    );
    let (response, _) = reply(&outcome);
    let image = &response["result"]["content"][1];
    assert_eq!(image["type"], "image");
    assert_eq!(image["mimeType"], "image/png");
    let bytes = BASE64.decode(image["data"].as_str().unwrap()).unwrap();
    assert_eq!(bytes, b"\x89PNG stand-in");

    // Empty instructions are left out.
    let outcome = protocol.handle(
        &request("initialize", json!({ "protocolVersion": "2025-11-25" })),
        None,
    );
    assert!(
        outcome.response().unwrap()["result"]
            .get("instructions")
            .is_none()
    );
}

#[test]
fn modern_requests_carry_their_version_and_say_what_they_are() {
    let protocol = protocol();
    let outcome = protocol.handle(&modern("tools/list", json!({})), None);
    let (response, status) = reply(&outcome);
    assert_eq!(status, Status::Ok);
    let result = &response["result"];
    assert_eq!(result["resultType"], "complete");
    assert_eq!(result["cacheScope"], "public");
    assert!(result["ttlMs"].as_u64().unwrap() > 0);
    assert_eq!(
        result["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
        "moshionly"
    );
    assert!(!result["tools"].as_array().unwrap().is_empty());

    let outcome = protocol.handle(
        &modern("tools/call", json!({ "name": "get_overview" })),
        None,
    );
    let (response, _) = reply(&outcome);
    assert_eq!(response["result"]["resultType"], "complete");
    assert_eq!(response["result"]["isError"], false);

    let outcome = protocol.handle(&modern("server/discover", json!({})), None);
    let (response, _) = reply(&outcome);
    let result = &response["result"];
    assert_eq!(
        result["supportedVersions"],
        json!(["2026-07-28", "2025-11-25", "2025-06-18", "2025-03-26"])
    );
    assert_eq!(result["capabilities"], json!({ "tools": {} }));
    assert_eq!(result["instructions"], "Be kind to the file.");
    assert_eq!(result["resultType"], "complete");
}

#[test]
fn modern_errors_use_their_codes_and_statuses() {
    let protocol = protocol();
    let mut old = modern("tools/list", json!({}));
    old["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"] = json!("1900-01-01");
    let outcome = protocol.handle(&old, None);
    let (response, status) = reply(&outcome);
    assert_eq!(status, Status::BadRequest);
    assert_eq!(
        response["error"]["code"],
        code::UNSUPPORTED_PROTOCOL_VERSION
    );
    assert_eq!(response["error"]["data"]["requested"], "1900-01-01");
    assert_eq!(response["error"]["data"]["supported"][0], "2026-07-28");

    let mut bare = modern("tools/list", json!({}));
    bare["params"]["_meta"]
        .as_object_mut()
        .unwrap()
        .remove("io.modelcontextprotocol/clientCapabilities");
    assert_eq!(
        error_code(&protocol.handle(&bare, None)),
        (code::INVALID_PARAMS, Status::BadRequest)
    );

    // ping is gone from the modern protocol.
    assert_eq!(
        error_code(&protocol.handle(&modern("ping", json!({})), None)),
        (code::METHOD_NOT_FOUND, Status::NotFound)
    );
    assert_eq!(
        error_code(&protocol.handle(&modern("tools/call", json!({ "name": "fly" })), None)),
        (code::INVALID_PARAMS, Status::BadRequest)
    );
    // A discovery probe without the modern fields is malformed.
    assert_eq!(
        error_code(&protocol.handle(&request("server/discover", json!({})), None)),
        (code::INVALID_PARAMS, Status::BadRequest)
    );
}

#[test]
fn modern_http_requests_must_mirror_their_body_in_headers() {
    let protocol = protocol();
    let call = modern("tools/call", json!({ "name": "get_overview" }));
    let ok = protocol.handle(&call, Some(&headers("tools/call", Some("get_overview"))));
    assert_eq!(reply(&ok).1, Status::Ok);

    // The name may come Base64 encoded.
    let encoded = format!("=?base64?{}?=", BASE64.encode("get_overview"));
    let ok = protocol.handle(&call, Some(&headers("tools/call", Some(&encoded))));
    assert_eq!(reply(&ok).0["result"]["isError"], false);

    for (bad, why) in [
        (headers("tools/call", None), "missing name"),
        (headers("tools/call", Some("get_node")), "other name"),
        (headers("tools/list", Some("get_overview")), "other method"),
        (
            Headers {
                protocol_version: None,
                ..headers("tools/call", Some("get_overview"))
            },
            "missing version",
        ),
        (
            Headers {
                protocol_version: Some("2025-11-25".into()),
                ..headers("tools/call", Some("get_overview"))
            },
            "other version",
        ),
    ] {
        let outcome = protocol.handle(&call, Some(&bad));
        assert_eq!(
            error_code(&outcome),
            (code::HEADER_MISMATCH, Status::BadRequest),
            "{why}"
        );
    }
}

#[test]
fn legacy_http_requests_check_the_version_header() {
    let protocol = protocol();
    let with_version = |version: &str| Headers {
        protocol_version: Some(version.into()),
        ..Headers::default()
    };
    let ok = protocol.handle(
        &request("ping", json!({})),
        Some(&with_version("2025-06-18")),
    );
    assert_eq!(reply(&ok).1, Status::Ok);
    let ok = protocol.handle(&request("ping", json!({})), Some(&Headers::default()));
    assert_eq!(reply(&ok).1, Status::Ok);

    let outcome = protocol.handle(
        &request("ping", json!({})),
        Some(&with_version("1999-01-01")),
    );
    assert_eq!(
        error_code(&outcome),
        (code::INVALID_REQUEST, Status::BadRequest)
    );
    let outcome = protocol.handle(
        &request("tools/list", json!({})),
        Some(&with_version("2026-07-28")),
    );
    let (response, _) = reply(&outcome);
    assert!(
        response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("must carry")
    );
    let outcome = protocol.handle(&request("resources/list", json!({})), None);
    assert_eq!(error_code(&outcome), (code::METHOD_NOT_FOUND, Status::Ok));
}

#[test]
fn malformed_messages_are_refused() {
    let protocol = protocol();
    for (message, expected) in [
        (json!([request("ping", json!({}))]), code::INVALID_REQUEST),
        (json!("ping"), code::INVALID_REQUEST),
        (
            json!({ "jsonrpc": "1.0", "id": 1, "method": "ping" }),
            code::INVALID_REQUEST,
        ),
        (json!({ "jsonrpc": "2.0", "id": 1 }), code::INVALID_REQUEST),
        (
            json!({ "jsonrpc": "2.0", "id": 1, "method": 5 }),
            code::INVALID_REQUEST,
        ),
        (
            json!({ "jsonrpc": "2.0", "id": null, "method": "ping" }),
            code::INVALID_REQUEST,
        ),
        (
            json!({ "jsonrpc": "2.0", "id": 1, "method": "ping", "params": [1] }),
            code::INVALID_PARAMS,
        ),
    ] {
        assert_eq!(
            error_code(&protocol.handle(&message, None)).0,
            expected,
            "{message}"
        );
    }
}

#[test]
fn header_values_may_be_base64() {
    assert_eq!(decode_header_value("plain").as_deref(), Some("plain"));
    assert_eq!(
        decode_header_value("=?base64?SGVsbG8sIOS4lueVjA==?=").as_deref(),
        Some("Hello, 世界")
    );
    assert_eq!(decode_header_value("=?base64?***?="), None);
}

#[test]
fn text_messages_are_parsed_and_answered_on_one_line() {
    let server = crate::mcp::McpServer::with_instructions(demo_tools(), "");
    let answer = server
        .handle_text(r#"{"jsonrpc":"2.0","id":"a","method":"ping"}"#)
        .unwrap();
    assert_eq!(answer, r#"{"jsonrpc":"2.0","id":"a","result":{}}"#);
    let answer = server.handle_text("{not json").unwrap();
    let value: Value = serde_json::from_str(&answer).unwrap();
    assert_eq!(value["error"]["code"], code::PARSE_ERROR);
    assert_eq!(value["id"], Value::Null);
    assert_eq!(
        server.handle_text(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#),
        None
    );
}
