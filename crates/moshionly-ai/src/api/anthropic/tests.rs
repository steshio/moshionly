//! The Anthropic provider from request to events, against a local server
//! that plays the documented event stream.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::*;
use crate::api::test_support::{TestTools, answer, collect_events};
use crate::chat::{ChatMessage, ImageData};
use crate::event::{ChatEvent, event_channel};
use crate::http::test_support::{MockResponse, MockServer, sse_event};
use crate::tool::ToolExecutor;

const KEY: &str = "sk-ant-api03-test-key-0123456789abcdef";

fn provider(server: &MockServer) -> Anthropic {
    provider_with(server, AnthropicOptions::default())
}

fn provider_with(server: &MockServer, options: AnthropicOptions) -> Anthropic {
    Anthropic::with_options(
        Some(ApiKey::new(KEY)),
        AnthropicOptions {
            base_url: server.url(),
            ..options
        },
    )
    .with_retry_policy(RetryPolicy {
        max_retries: 2,
        base_delay: Duration::from_millis(1),
        max_delay: Duration::from_millis(100),
    })
}

fn request_with_tools(tools: &Arc<TestTools>) -> ChatRequest {
    let executor: Arc<dyn ToolExecutor> = tools.clone();
    ChatRequest {
        system_prompt: Some("You edit motion designs.".to_owned()),
        tools: Some(executor),
        ..ChatRequest::from_prompt("Fade the card in")
    }
}

fn event(data: Value) -> String {
    sse_event(data["type"].as_str().unwrap(), &data)
}

fn message_start(input_tokens: u64) -> String {
    event(
        json!({"type":"message_start","message":{"id":"msg_1","type":"message",
        "role":"assistant","model":"claude-opus-5-5","content":[],"stop_reason":null,
        "usage":{"input_tokens":input_tokens,"cache_creation_input_tokens":0,
            "cache_read_input_tokens":0,"output_tokens":1}}}),
    )
}

fn message_end(stop_reason: &str, output_tokens: u64) -> Vec<String> {
    vec![
        event(
            json!({"type":"message_delta","delta":{"stop_reason":stop_reason,
            "stop_sequence":null},"usage":{"output_tokens":output_tokens}}),
        ),
        event(json!({"type":"message_stop"})),
    ]
}

/// A response that says `pieces`, one delta each.
fn text_response(pieces: &[&str]) -> MockResponse {
    let mut events = vec![
        message_start(100),
        event(json!({"type":"content_block_start","index":0,
            "content_block":{"type":"text","text":""}})),
        event(json!({"type":"ping"})),
    ];
    for piece in pieces {
        events.push(event(json!({"type":"content_block_delta","index":0,
            "delta":{"type":"text_delta","text":piece}})));
    }
    events.push(event(json!({"type":"content_block_stop","index":0})));
    events.extend(message_end("end_turn", 20));
    MockResponse::sse(&events)
}

/// A response that thinks, then calls the given tools in one step. Each
/// call is `(id, name, arguments as JSON text)`.
fn tool_response(calls: &[(&str, &str, &str)]) -> MockResponse {
    let mut events = vec![
        message_start(200),
        event(json!({"type":"content_block_start","index":0,
            "content_block":{"type":"thinking","thinking":"","signature":""}})),
        event(json!({"type":"content_block_delta","index":0,
            "delta":{"type":"thinking_delta","thinking":"I need the tools."}})),
        event(json!({"type":"content_block_delta","index":0,
            "delta":{"type":"signature_delta","signature":"c2lnbmF0dXJl"}})),
        event(json!({"type":"content_block_stop","index":0})),
    ];
    for (offset, (id, name, arguments)) in calls.iter().enumerate() {
        let index = offset + 1;
        events.push(event(json!({"type":"content_block_start","index":index,
            "content_block":{"type":"tool_use","id":id,"name":name,"input":{}}})));
        // The arguments arrive split in two, as they do from the API.
        let middle = arguments.len() / 2;
        for part in [&arguments[..middle], &arguments[middle..]] {
            events.push(event(json!({"type":"content_block_delta","index":index,
                "delta":{"type":"input_json_delta","partial_json":part}})));
        }
        events.push(event(json!({"type":"content_block_stop","index":index})));
    }
    events.extend(message_end("tool_use", 50));
    MockResponse::sse(&events)
}

fn api_error(status: u16, error_type: &str, message: &str) -> MockResponse {
    MockResponse::json(
        status,
        json!({"type":"error","error":{"type":error_type,"message":message},
            "request_id":"req_test"}),
    )
}

fn kinds(events: &[ChatEvent]) -> Vec<&'static str> {
    events
        .iter()
        .map(|event| match event {
            ChatEvent::SessionStarted { .. } => "session",
            ChatEvent::TextDelta { .. } => "text",
            ChatEvent::ThinkingDelta { .. } => "thinking",
            ChatEvent::ToolCallStarted { .. } => "tool_started",
            ChatEvent::ToolCallFinished { .. } => "tool_finished",
            ChatEvent::Usage { .. } => "usage",
            ChatEvent::Error { .. } => "error",
            ChatEvent::Finished { .. } => "finished",
        })
        .collect()
}

fn finished(stop_reason: StopReason) -> ChatEvent {
    ChatEvent::Finished { stop_reason }
}

#[tokio::test]
async fn text_streams_as_it_arrives() {
    let server = MockServer::start(vec![text_response(&["Fading ", "the card ", "in."])]).await;
    let request = ChatRequest {
        system_prompt: Some("You edit motion designs.".to_owned()),
        ..ChatRequest::from_prompt("Fade the card in")
    };
    let events = collect_events(&provider(&server), request).await;
    assert_eq!(
        events,
        vec![
            ChatEvent::TextDelta {
                text: "Fading ".to_owned()
            },
            ChatEvent::TextDelta {
                text: "the card ".to_owned()
            },
            ChatEvent::TextDelta {
                text: "in.".to_owned()
            },
            ChatEvent::Usage {
                input_tokens: 100,
                output_tokens: 20,
                cost_usd: None
            },
            finished(StopReason::EndTurn),
        ]
    );

    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    let sent = &requests[0];
    assert_eq!(sent.method, "POST");
    assert_eq!(sent.path, "/v1/messages");
    assert_eq!(sent.header("x-api-key"), KEY);
    assert_eq!(sent.header("anthropic-version"), "2026-10-21");
    assert_eq!(sent.header("content-type"), "application/json");
    assert_eq!(sent.body["model"], "claude-opus-5-5");
    assert_eq!(sent.body["stream"], true);
    assert_eq!(sent.body["max_tokens"], 64000);
    assert_eq!(sent.body["system"][0]["text"], "You edit motion designs.");
    assert_eq!(
        sent.body["messages"][0]["content"][0]["text"],
        "Fade the card in"
    );
    // A local address is not Anthropic's API, so nothing beyond the basic
    // request is sent.
    assert_eq!(sent.header("anthropic-beta"), "");
    assert!(sent.body.get("fallbacks").is_none());
}

#[tokio::test]
async fn a_tool_round_trip_sends_every_result_back_in_one_message() {
    let server = MockServer::start(vec![
        tool_response(&[
            ("toolu_1", "screenshot", "{}"),
            ("toolu_2", "echo", r#"{"text": "ping"}"#),
        ]),
        text_response(&["Done."]),
    ])
    .await;
    let tools = TestTools::shared();
    let events = collect_events(&provider(&server), request_with_tools(&tools)).await;

    assert_eq!(
        kinds(&events),
        [
            "thinking",
            "usage",
            "tool_started",
            "tool_started",
            "tool_finished",
            "tool_finished",
            "text",
            "usage",
            "finished"
        ]
    );
    assert_eq!(
        events[2],
        ChatEvent::ToolCallStarted {
            id: "toolu_1".to_owned(),
            name: "screenshot".to_owned(),
            input: json!({}),
        }
    );
    assert_eq!(
        events[3],
        ChatEvent::ToolCallStarted {
            id: "toolu_2".to_owned(),
            name: "echo".to_owned(),
            input: json!({ "text": "ping" }),
        }
    );
    assert!(events.contains(&ChatEvent::ToolCallFinished {
        id: "toolu_1".to_owned(),
        ok: true,
        summary: "frame at 0 ms, 1 image".to_owned(),
    }));
    // Token use adds up over the two requests.
    assert_eq!(
        events[7],
        ChatEvent::Usage {
            input_tokens: 300,
            output_tokens: 70,
            cost_usd: None
        }
    );
    assert_eq!(events[8], finished(StopReason::EndTurn));
    assert_eq!(tools.calls().len(), 2);

    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    // The tools were offered.
    assert_eq!(requests[0].body["tools"][0]["name"], "echo");
    let messages = requests[1].body["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 3);
    // The assistant's message goes back whole, thinking and signature
    // included.
    assert_eq!(messages[1]["role"], "assistant");
    assert_eq!(
        messages[1]["content"][0],
        json!({"type":"thinking","thinking":"I need the tools.","signature":"c2lnbmF0dXJl"})
    );
    assert_eq!(
        messages[1]["content"][2],
        json!({"type":"tool_use","id":"toolu_2","name":"echo","input":{"text":"ping"}})
    );
    // One user message answers both calls, in their order, image included.
    assert_eq!(messages[2]["role"], "user");
    let results = messages[2]["content"].as_array().unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0]["tool_use_id"], "toolu_1");
    assert_eq!(results[0]["content"][1]["type"], "image");
    assert_eq!(
        results[0]["content"][1]["source"]["media_type"],
        "image/png"
    );
    assert_eq!(results[1]["tool_use_id"], "toolu_2");
    assert_eq!(results[1]["content"][0]["text"], "echo: ping");
    // Everything before the new messages is byte for byte what was sent
    // first, which is what keeps the cache and the thinking blocks valid.
    assert_eq!(requests[0].body["system"], requests[1].body["system"]);
    assert_eq!(requests[0].body["tools"], requests[1].body["tools"]);
    assert_eq!(
        requests[0].body["messages"][0]["content"][0]["text"],
        messages[0]["content"][0]["text"]
    );
}

#[tokio::test]
async fn a_failed_tool_is_reported_to_the_model_as_an_error() {
    let server = MockServer::start(vec![
        tool_response(&[("toolu_1", "fail", "{}")]),
        text_response(&["I could not find that layer."]),
    ])
    .await;
    let tools = TestTools::shared();
    let events = collect_events(&provider(&server), request_with_tools(&tools)).await;
    assert!(events.contains(&ChatEvent::ToolCallFinished {
        id: "toolu_1".to_owned(),
        ok: false,
        summary: "node cd34 does not exist".to_owned(),
    }));
    let result = &server.requests()[1].body["messages"][2]["content"][0];
    assert_eq!(result["is_error"], true);
    assert_eq!(result["content"][0]["text"], "node cd34 does not exist");
}

#[tokio::test]
async fn arguments_that_are_not_json_go_back_as_an_error_without_running_the_tool() {
    let server = MockServer::start(vec![
        tool_response(&[("toolu_1", "echo", r#"{"text": "cut of"#)]),
        text_response(&["Let me try again."]),
    ])
    .await;
    let tools = TestTools::shared();
    let events = collect_events(&provider(&server), request_with_tools(&tools)).await;
    assert!(tools.calls().is_empty());
    assert!(matches!(
        events[3],
        ChatEvent::ToolCallFinished { ok: false, .. }
    ));
    let messages = &server.requests()[1].body["messages"];
    // The call is echoed in a valid shape and answered with its own text.
    assert_eq!(messages[1]["content"][1]["input"], json!({}));
    let result = &messages[2]["content"][0];
    assert_eq!(result["is_error"], true);
    let reply: Value =
        serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(reply["INVALID_JSON"], r#"{"text": "cut of"#);
}

#[tokio::test]
async fn images_in_the_conversation_are_sent_to_the_model() {
    let server = MockServer::start(vec![text_response(&["It is red."])]).await;
    let request = ChatRequest {
        messages: vec![
            ChatMessage::user("What colour is this?").with_image(ImageData::png(vec![1, 2, 3])),
        ],
        ..ChatRequest::default()
    };
    collect_events(&provider(&server), request).await;
    let content = &server.requests()[0].body["messages"][0]["content"];
    assert_eq!(content[0]["type"], "image");
    assert_eq!(content[0]["source"]["data"], "AQID");
    assert_eq!(content[1]["text"], "What colour is this?");
}

#[tokio::test]
async fn the_requested_model_is_used() {
    let server = MockServer::start(vec![text_response(&["ok"])]).await;
    let request = ChatRequest {
        model: Some("claude-haiku-5-5".to_owned()),
        ..ChatRequest::from_prompt("hi")
    };
    collect_events(&provider(&server), request).await;
    assert_eq!(server.requests()[0].body["model"], "claude-haiku-5-5");
}

#[tokio::test]
async fn anthropics_own_api_gets_fallbacks_and_their_beta_header() {
    let server = MockServer::start(vec![text_response(&["ok"])]).await;
    let options = AnthropicOptions {
        first_party_features: Some(true),
        effort: Some(Effort::High),
        ..AnthropicOptions::default()
    };
    let tools = TestTools::shared();
    collect_events(&provider_with(&server, options), request_with_tools(&tools)).await;
    let sent = &server.requests()[0];
    assert_eq!(
        sent.header("anthropic-beta"),
        "server-side-fallback-2026-07-01"
    );
    assert_eq!(sent.body["fallbacks"], "default");
    assert_eq!(sent.body["tools"][0]["eager_input_streaming"], true);
    assert_eq!(sent.body["cache_control"], json!({ "type": "ephemeral" }));
    assert_eq!(sent.body["output_config"], json!({ "effort": "high" }));
}

#[tokio::test]
async fn a_rejected_key_is_reported_once_and_never_shown() {
    let server = MockServer::start(vec![api_error(
        401,
        "authentication_error",
        &format!("invalid x-api-key: {KEY}"),
    )])
    .await;
    let events = collect_events(&provider(&server), ChatRequest::from_prompt("hi")).await;
    assert_eq!(kinds(&events), ["error", "finished"]);
    match &events[0] {
        ChatEvent::Error { message, kind } => {
            assert_eq!(*kind, ErrorKind::InvalidKey);
            assert!(!message.contains(KEY), "{message}");
            assert!(message.contains("[redacted]"), "{message}");
        }
        other => panic!("expected an error, got {other:?}"),
    }
    assert_eq!(events[1], finished(StopReason::Error));
    // A bad key does not get better by trying again.
    assert_eq!(server.requests().len(), 1);
}

#[tokio::test]
async fn a_rate_limit_is_retried_after_the_wait_the_api_asks_for() {
    let server = MockServer::start(vec![
        api_error(429, "rate_limit_error", "slow down").with_header("retry-after", "0"),
        text_response(&["ok"]),
    ])
    .await;
    let events = collect_events(&provider(&server), ChatRequest::from_prompt("hi")).await;
    assert_eq!(answer(&events), "ok");
    assert_eq!(events.last(), Some(&finished(StopReason::EndTurn)));
    assert_eq!(server.requests().len(), 2);
}

#[tokio::test]
async fn an_overloaded_api_is_retried_and_then_reported() {
    let overloaded = || api_error(529, "overloaded_error", "Overloaded");
    let server = MockServer::start(vec![overloaded(), overloaded(), overloaded()]).await;
    let events = collect_events(&provider(&server), ChatRequest::from_prompt("hi")).await;
    assert_eq!(
        events,
        vec![
            ChatEvent::Error {
                message: "the service is overloaded: Overloaded".to_owned(),
                kind: ErrorKind::Overloaded,
            },
            finished(StopReason::Error),
        ]
    );
    assert_eq!(server.requests().len(), 3);
}

#[tokio::test]
async fn an_unknown_model_is_a_model_unavailable_error() {
    let server = MockServer::start(vec![api_error(
        404,
        "not_found_error",
        "model: claude-nope-9",
    )])
    .await;
    let events = collect_events(&provider(&server), ChatRequest::from_prompt("hi")).await;
    assert!(matches!(
        &events[0],
        ChatEvent::Error { kind: ErrorKind::ModelUnavailable, message } if message.contains("claude-nope-9")
    ));
}

#[tokio::test]
async fn an_error_before_any_text_is_retried() {
    let server = MockServer::start(vec![
        MockResponse::sse(&[
            message_start(10),
            event(json!({"type":"error","error":{"type":"overloaded_error",
                "message":"Overloaded"}})),
        ]),
        text_response(&["ok"]),
    ])
    .await;
    let events = collect_events(&provider(&server), ChatRequest::from_prompt("hi")).await;
    assert_eq!(answer(&events), "ok");
    assert_eq!(server.requests().len(), 2);
}

#[tokio::test]
async fn an_error_after_text_is_reported_instead_of_repeating_the_text() {
    let server = MockServer::start(vec![
        MockResponse::sse(&[
            message_start(10),
            event(json!({"type":"content_block_start","index":0,
                "content_block":{"type":"text","text":""}})),
            event(json!({"type":"content_block_delta","index":0,
                "delta":{"type":"text_delta","text":"Half an "}})),
            event(json!({"type":"error","error":{"type":"overloaded_error",
                "message":"Overloaded"}})),
        ]),
        text_response(&["never sent"]),
    ])
    .await;
    let events = collect_events(&provider(&server), ChatRequest::from_prompt("hi")).await;
    assert_eq!(kinds(&events), ["text", "error", "finished"]);
    assert_eq!(server.requests().len(), 1);
}

#[tokio::test]
async fn a_connection_that_closes_mid_answer_is_a_network_error() {
    let server = MockServer::start(vec![MockResponse::sse(&[
        message_start(10),
        event(json!({"type":"content_block_start","index":0,
            "content_block":{"type":"text","text":""}})),
        event(json!({"type":"content_block_delta","index":0,
            "delta":{"type":"text_delta","text":"Half"}})),
    ])])
    .await;
    let events = collect_events(&provider(&server), ChatRequest::from_prompt("hi")).await;
    assert!(matches!(
        events[1],
        ChatEvent::Error {
            kind: ErrorKind::Network,
            ..
        }
    ));
}

#[tokio::test]
async fn cancelling_closes_the_stream_and_reports_cancelled() {
    let server = MockServer::start(vec![MockResponse::sse_then_hang(&[
        message_start(10),
        event(json!({"type":"content_block_start","index":0,
            "content_block":{"type":"text","text":""}})),
        event(json!({"type":"content_block_delta","index":0,
            "delta":{"type":"text_delta","text":"Thinking about it"}})),
    ])])
    .await;
    let provider = provider(&server);
    let cancel = CancellationToken::new();
    let (events, mut receiver) = event_channel();
    let task = tokio::spawn({
        let cancel = cancel.clone();
        async move {
            provider
                .send(ChatRequest::from_prompt("hi"), events, cancel)
                .await;
        }
    });
    assert_eq!(
        receiver.recv().await,
        Some(ChatEvent::TextDelta {
            text: "Thinking about it".to_owned()
        })
    );
    let started = std::time::Instant::now();
    cancel.cancel();
    assert_eq!(receiver.recv().await, Some(finished(StopReason::Cancelled)));
    task.await.unwrap();
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn cancelling_while_a_tool_runs_stops_the_turn() {
    let server = MockServer::start(vec![tool_response(&[("toolu_1", "slow", "{}")])]).await;
    let tools = TestTools::shared();
    let provider = provider(&server);
    let cancel = CancellationToken::new();
    let (events, mut receiver) = event_channel();
    let task = tokio::spawn({
        let cancel = cancel.clone();
        let request = request_with_tools(&tools);
        async move { provider.send(request, events, cancel).await }
    });
    loop {
        match receiver.recv().await {
            Some(ChatEvent::ToolCallStarted { .. }) => break,
            Some(_) => {}
            None => panic!("the turn ended before the tool started"),
        }
    }
    cancel.cancel();
    assert_eq!(receiver.recv().await, Some(finished(StopReason::Cancelled)));
    task.await.unwrap();
    // No second request was made.
    assert_eq!(server.requests().len(), 1);
}

#[tokio::test]
async fn dropping_the_receiver_stops_the_request() {
    let server = MockServer::start(vec![MockResponse::sse_then_hang(&[message_start(10)])]).await;
    let (events, receiver) = event_channel();
    drop(receiver);
    let started = std::time::Instant::now();
    provider(&server)
        .send(
            ChatRequest::from_prompt("hi"),
            events,
            CancellationToken::new(),
        )
        .await;
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn a_model_that_keeps_calling_tools_is_stopped_at_the_limit() {
    let call = || tool_response(&[("toolu_1", "echo", r#"{"text":"again"}"#)]);
    let server = MockServer::start(vec![call(), call(), call(), call()]).await;
    let options = AnthropicOptions {
        max_tool_rounds: 2,
        ..AnthropicOptions::default()
    };
    let tools = TestTools::shared();
    let events = collect_events(&provider_with(&server, options), request_with_tools(&tools)).await;
    assert_eq!(events.last(), Some(&finished(StopReason::ToolLoopLimit)));
    // Two rounds ran; the third request's calls were not carried out.
    assert_eq!(tools.calls().len(), 2);
    assert_eq!(server.requests().len(), 3);
}

#[tokio::test]
async fn refusals_and_cut_off_answers_are_stop_reasons_not_errors() {
    for (stop_reason, expected) in [
        ("refusal", StopReason::Refusal),
        ("max_tokens", StopReason::MaxTokens),
        ("model_context_window_exceeded", StopReason::MaxTokens),
        ("end_turn", StopReason::EndTurn),
        ("stop_sequence", StopReason::EndTurn),
    ] {
        let mut events = vec![message_start(10)];
        events.extend(message_end(stop_reason, 0));
        let server = MockServer::start(vec![MockResponse::sse(&events)]).await;
        let events = collect_events(&provider(&server), ChatRequest::from_prompt("hi")).await;
        assert_eq!(events.last(), Some(&finished(expected)), "{stop_reason}");
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, ChatEvent::Error { .. }))
        );
    }
}

#[tokio::test]
async fn tools_of_a_refused_or_cut_off_message_are_never_run() {
    // A tool call was streamed, but the message ended at the token limit.
    let mut events = vec![
        message_start(10),
        event(json!({"type":"content_block_start","index":0,
            "content_block":{"type":"tool_use","id":"toolu_1","name":"echo","input":{}}})),
        event(json!({"type":"content_block_delta","index":0,
            "delta":{"type":"input_json_delta","partial_json":"{\"text\": \"par"}})),
        event(json!({"type":"content_block_stop","index":0})),
    ];
    events.extend(message_end("max_tokens", 64000));
    let server = MockServer::start(vec![MockResponse::sse(&events)]).await;
    let tools = TestTools::shared();
    let events = collect_events(&provider(&server), request_with_tools(&tools)).await;
    assert_eq!(events.last(), Some(&finished(StopReason::MaxTokens)));
    assert!(tools.calls().is_empty());
    assert_eq!(server.requests().len(), 1);
}

#[tokio::test]
async fn without_a_key_nothing_is_sent() {
    let server = MockServer::start(vec![]).await;
    let provider = Anthropic::with_options(
        None,
        AnthropicOptions {
            base_url: server.url(),
            ..AnthropicOptions::default()
        },
    );
    let events = collect_events(&provider, ChatRequest::from_prompt("hi")).await;
    assert_eq!(
        events,
        vec![
            ChatEvent::Error {
                message: "no API key is set for Anthropic".to_owned(),
                kind: ErrorKind::MissingKey,
            },
            finished(StopReason::Error),
        ]
    );
    assert!(server.requests().is_empty());
    assert_eq!(provider.check_ready().await.state, ReadyState::MissingKey);
    assert_eq!(
        provider.list_models().await.unwrap_err().kind(),
        ErrorKind::MissingKey
    );
    // An empty key is no key.
    let blank = Anthropic::new(Some(ApiKey::new("  ")));
    assert_eq!(blank.check_ready().await.state, ReadyState::MissingKey);
}

#[tokio::test]
async fn an_unreachable_server_is_a_network_error_after_retries() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let provider = Anthropic::with_options(
        Some(ApiKey::new(KEY)),
        AnthropicOptions {
            base_url: format!("http://{address}"),
            ..AnthropicOptions::default()
        },
    )
    .with_retry_policy(RetryPolicy {
        max_retries: 1,
        base_delay: Duration::from_millis(1),
        max_delay: Duration::from_millis(10),
    });
    let events = collect_events(&provider, ChatRequest::from_prompt("hi")).await;
    assert!(matches!(
        events[0],
        ChatEvent::Error {
            kind: ErrorKind::Network,
            ..
        }
    ));
    assert_eq!(provider.check_ready().await.state, ReadyState::Unreachable);
}

#[tokio::test]
async fn models_are_listed_across_pages_with_the_default_first() {
    let server = MockServer::start(vec![
        MockResponse::json(
            200,
            json!({"data":[
                {"type":"model","id":"claude-fable-5-2","display_name":"Claude Fable 5.2"},
                {"type":"model","id":"claude-haiku-5-5","display_name":"Claude Haiku 5.5"}],
                "has_more":true,"first_id":"claude-fable-5-2","last_id":"claude-haiku-5-5"}),
        ),
        MockResponse::json(
            200,
            json!({"data":[
                {"type":"model","id":"claude-opus-5-5","display_name":"Claude Opus 5.5"},
                {"type":"model","id":"claude-custom"}],
                "has_more":false,"first_id":"claude-opus-5-5","last_id":"claude-custom"}),
        ),
    ])
    .await;
    let models = provider(&server).list_models().await.unwrap();
    assert_eq!(
        models,
        vec![
            ModelInfo::new("claude-opus-5-5", "Claude Opus 5.5"),
            ModelInfo::new("claude-fable-5-2", "Claude Fable 5.2"),
            ModelInfo::new("claude-haiku-5-5", "Claude Haiku 5.5"),
            ModelInfo::new("claude-custom", "claude-custom"),
        ]
    );
    let requests = server.requests();
    assert_eq!(requests[0].method, "GET");
    assert_eq!(requests[0].path, "/v1/models?limit=1000");
    assert_eq!(
        requests[1].path,
        "/v1/models?limit=1000&after_id=claude-haiku-5-5"
    );
    assert_eq!(requests[0].header("x-api-key"), KEY);
}

#[tokio::test]
async fn readiness_reflects_what_the_api_says_about_the_key() {
    let server = MockServer::start(vec![
        MockResponse::json(200, json!({"data":[],"has_more":false})),
        api_error(401, "authentication_error", "invalid x-api-key"),
        MockResponse::text(502, "<html>Bad gateway</html>"),
    ])
    .await;
    let provider = provider(&server);
    let ready = provider.check_ready().await;
    assert_eq!(ready.state, ReadyState::Ready);
    assert_eq!(provider.check_ready().await.state, ReadyState::InvalidKey);
    let gateway = provider.check_ready().await;
    assert_eq!(gateway.state, ReadyState::Unknown);
    assert!(
        gateway.message.contains("Bad gateway"),
        "{}",
        gateway.message
    );
    assert_eq!(server.requests()[0].path, "/v1/models?limit=1");
}

#[test]
fn first_party_features_follow_the_address_unless_set() {
    let at = |base_url: &str, set: Option<bool>| {
        Anthropic::with_options(
            None,
            AnthropicOptions {
                base_url: base_url.to_owned(),
                first_party_features: set,
                ..AnthropicOptions::default()
            },
        )
        .first_party()
    };
    assert!(at("https://api.anthropic.com", None));
    assert!(at("https://api.anthropic.com/", None));
    assert!(!at("https://gateway.example.com/anthropic", None));
    assert!(at("https://gateway.example.com/anthropic", Some(true)));
    assert!(!at("https://api.anthropic.com", Some(false)));
}

#[test]
fn the_provider_describes_itself() {
    let provider = Anthropic::new(None);
    assert_eq!(provider.info().id, "anthropic");
    assert_eq!(provider.info().kind, ProviderKind::Api);
    let capabilities = provider.capabilities();
    assert_eq!(capabilities.tools, ToolTransport::InProcess);
    assert!(capabilities.needs_api_key && capabilities.images && !capabilities.resume);
    // Keys never show up in debug output.
    let debug = format!("{:?}", Anthropic::new(Some(ApiKey::new(KEY))));
    assert!(!debug.contains(KEY));
}

#[test]
fn effort_levels_serialise_as_the_api_spells_them() {
    for (effort, text) in [
        (Effort::Low, "low"),
        (Effort::Medium, "medium"),
        (Effort::High, "high"),
        (Effort::XHigh, "xhigh"),
        (Effort::Max, "max"),
    ] {
        assert_eq!(effort.as_str(), text);
        assert_eq!(serde_json::to_value(effort).unwrap(), json!(text));
        assert_eq!(
            serde_json::from_value::<Effort>(json!(text)).unwrap(),
            effort
        );
    }
}
