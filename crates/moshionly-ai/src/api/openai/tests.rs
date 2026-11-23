//! The OpenAI-compatible provider from request to events, against a local
//! server that plays the Chat Completions format.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::presets::{CUSTOM, GROQ, OLLAMA, OPENAI};
use super::*;
use crate::api::test_support::{TestTools, answer, collect_events};
use crate::chat::{ChatMessage, ImageData};
use crate::event::{ChatEvent, event_channel};
use crate::http::test_support::{MockResponse, MockServer, sse_event};
use crate::tool::ToolExecutor;

const KEY: &str = "sk-proj-TestKey0123456789abcdefghij";

fn provider(server: &MockServer, preset: OpenAiPreset) -> OpenAiCompatible {
    provider_with(server, preset, OpenAiCompatibleOptions::default())
}

fn provider_with(
    server: &MockServer,
    preset: OpenAiPreset,
    options: OpenAiCompatibleOptions,
) -> OpenAiCompatible {
    let key = preset.needs_api_key.then(|| ApiKey::new(KEY));
    OpenAiCompatible::with_options(
        preset,
        key,
        OpenAiCompatibleOptions {
            base_url: Some(format!("{}/v1", server.url())),
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

fn chunk(delta: Value, finish_reason: Option<&str>) -> String {
    sse_event(
        "",
        &json!({"id":"chatcmpl-1","object":"chat.completion.chunk","model":"test",
            "choices":[{"index":0,"delta":delta,"finish_reason":finish_reason}]}),
    )
}

fn usage(prompt: u64, completion: u64) -> String {
    sse_event(
        "",
        &json!({"id":"chatcmpl-1","choices":[],
            "usage":{"prompt_tokens":prompt,"completion_tokens":completion,
                "total_tokens":prompt + completion}}),
    )
}

const DONE: &str = "data: [DONE]\n\n";

/// A response that says `pieces`, one chunk each.
fn text_response(pieces: &[&str]) -> MockResponse {
    let mut events = vec![chunk(json!({"role":"assistant","content":""}), None)];
    events.extend(
        pieces
            .iter()
            .map(|piece| chunk(json!({"content":piece}), None)),
    );
    events.push(chunk(json!({}), Some("stop")));
    events.push(usage(100, 20));
    events.push(DONE.to_owned());
    MockResponse::sse(&events)
}

/// A response that calls the given tools in one step. Each call is
/// `(id, name, arguments as JSON text)`.
fn tool_response(calls: &[(&str, &str, &str)]) -> MockResponse {
    let mut events = vec![chunk(json!({"role":"assistant","content":null}), None)];
    for (index, (id, name, arguments)) in calls.iter().enumerate() {
        events.push(chunk(
            json!({"tool_calls":[{"index":index,"id":id,"type":"function",
                "function":{"name":name,"arguments":""}}]}),
            None,
        ));
        let middle = arguments.len() / 2;
        for part in [&arguments[..middle], &arguments[middle..]] {
            events.push(chunk(
                json!({"tool_calls":[{"index":index,"function":{"arguments":part}}]}),
                None,
            ));
        }
    }
    events.push(chunk(json!({}), Some("tool_calls")));
    events.push(usage(200, 50));
    events.push(DONE.to_owned());
    MockResponse::sse(&events)
}

fn api_error(status: u16, code: &str, message: &str) -> MockResponse {
    MockResponse::json(
        status,
        json!({"error":{"message":message,"type":"invalid_request_error","code":code}}),
    )
}

fn models(ids: &[&str]) -> MockResponse {
    let data: Vec<Value> = ids
        .iter()
        .map(|id| json!({"id":id,"object":"model"}))
        .collect();
    MockResponse::json(200, json!({"object":"list","data":data}))
}

fn finished(stop_reason: StopReason) -> ChatEvent {
    ChatEvent::Finished { stop_reason }
}

#[tokio::test]
async fn text_streams_as_it_arrives() {
    let server = MockServer::start(vec![text_response(&["Fading ", "the card in."])]).await;
    let request = ChatRequest {
        system_prompt: Some("You edit motion designs.".to_owned()),
        ..ChatRequest::from_prompt("Fade the card in")
    };
    let events = collect_events(&provider(&server, OPENAI), request).await;
    assert_eq!(
        events,
        vec![
            ChatEvent::TextDelta {
                text: "Fading ".to_owned()
            },
            ChatEvent::TextDelta {
                text: "the card in.".to_owned()
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
    assert_eq!(sent.path, "/v1/chat/completions");
    assert_eq!(sent.header("authorization"), format!("Bearer {KEY}"));
    // The preset's first suggestion is the model when none was chosen.
    assert_eq!(sent.body["model"], "gpt-6.2-sol");
    assert_eq!(sent.body["stream"], true);
    assert_eq!(
        sent.body["stream_options"],
        json!({ "include_usage": true })
    );
    assert_eq!(
        sent.body["messages"],
        json!([
            {"role":"system","content":"You edit motion designs."},
            {"role":"user","content":"Fade the card in"}
        ])
    );
}

#[tokio::test]
async fn a_tool_round_trip_answers_every_call_and_sends_images_after() {
    let server = MockServer::start(vec![
        tool_response(&[
            ("call_1", "screenshot", "{}"),
            ("call_2", "echo", r#"{"text": "ping"}"#),
        ]),
        text_response(&["Done."]),
    ])
    .await;
    let tools = TestTools::shared();
    let events = collect_events(&provider(&server, OPENAI), request_with_tools(&tools)).await;

    assert_eq!(
        events[1],
        ChatEvent::ToolCallStarted {
            id: "call_1".to_owned(),
            name: "screenshot".to_owned(),
            input: json!({}),
        }
    );
    assert!(events.contains(&ChatEvent::ToolCallFinished {
        id: "call_2".to_owned(),
        ok: true,
        summary: "echo: ping".to_owned(),
    }));
    assert_eq!(answer(&events), "Done.");
    // Token use adds up over the two requests.
    assert!(events.contains(&ChatEvent::Usage {
        input_tokens: 300,
        output_tokens: 70,
        cost_usd: None
    }));
    assert_eq!(events.last(), Some(&finished(StopReason::EndTurn)));
    assert_eq!(tools.calls().len(), 2);

    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].body["tools"][0]["type"], "function");
    assert_eq!(requests[0].body["tools"][0]["function"]["name"], "echo");
    let messages = requests[1].body["messages"].as_array().unwrap();
    // system, user, assistant with the calls, two tool results, the image.
    assert_eq!(messages.len(), 6);
    assert_eq!(messages[2]["role"], "assistant");
    assert_eq!(messages[2]["content"], Value::Null);
    assert_eq!(
        messages[2]["tool_calls"][1],
        json!({"id":"call_2","type":"function",
            "function":{"name":"echo","arguments":"{\"text\": \"ping\"}"}})
    );
    assert_eq!(messages[3]["role"], "tool");
    assert_eq!(messages[3]["tool_call_id"], "call_1");
    assert_eq!(
        messages[4],
        json!({"role":"tool","tool_call_id":"call_2","content":"echo: ping"})
    );
    assert_eq!(messages[5]["role"], "user");
    assert_eq!(
        messages[5]["content"][1]["image_url"]["url"],
        "data:image/png;base64,AQID"
    );
}

#[tokio::test]
async fn tool_images_degrade_to_text_where_the_model_cannot_see() {
    let server = MockServer::start(vec![
        tool_response(&[("call_1", "screenshot", "{}")]),
        text_response(&["Done."]),
    ])
    .await;
    let tools = TestTools::shared();
    // Groq's preset says its usual models take no images.
    let provider = provider(&server, GROQ);
    assert!(!provider.capabilities().images);
    collect_events(&provider, request_with_tools(&tools)).await;
    let messages = server.requests()[1].body["messages"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(messages.len(), 4);
    assert_eq!(
        messages[3]["content"],
        "frame at 0 ms\n\n[1 image was left out: this model cannot be sent images.]"
    );
    assert!(!messages.iter().any(|message| message["content"].is_array()));
}

#[tokio::test]
async fn images_from_the_user_follow_the_same_rule() {
    let message = ChatMessage::user("match this").with_image(ImageData::png(vec![1, 2, 3]));
    let request = || ChatRequest {
        messages: vec![message.clone()],
        ..ChatRequest::default()
    };

    let server = MockServer::start(vec![text_response(&["ok"])]).await;
    collect_events(&provider(&server, OPENAI), request()).await;
    assert_eq!(
        server.requests()[0].body["messages"][0]["content"][1]["type"],
        "image_url"
    );

    let server = MockServer::start(vec![text_response(&["ok"])]).await;
    let options = OpenAiCompatibleOptions {
        images: Some(false),
        ..OpenAiCompatibleOptions::default()
    };
    collect_events(&provider_with(&server, OPENAI, options), request()).await;
    assert!(server.requests()[0].body["messages"][0]["content"].is_string());
}

#[tokio::test]
async fn a_rejected_key_is_reported_once_and_never_shown() {
    let server = MockServer::start(vec![api_error(
        401,
        "invalid_api_key",
        &format!("Incorrect API key provided: {KEY}."),
    )])
    .await;
    let events = collect_events(&provider(&server, OPENAI), ChatRequest::from_prompt("hi")).await;
    match &events[0] {
        ChatEvent::Error { message, kind } => {
            assert_eq!(*kind, ErrorKind::InvalidKey);
            assert!(!message.contains(KEY), "{message}");
            assert!(message.contains("[redacted]"), "{message}");
        }
        other => panic!("expected an error, got {other:?}"),
    }
    assert_eq!(events[1], finished(StopReason::Error));
    assert_eq!(server.requests().len(), 1);
}

#[tokio::test]
async fn a_rate_limit_is_retried() {
    let server = MockServer::start(vec![
        api_error(429, "rate_limit_exceeded", "Rate limit reached").with_header("retry-after", "0"),
        text_response(&["ok"]),
    ])
    .await;
    let events = collect_events(&provider(&server, OPENAI), ChatRequest::from_prompt("hi")).await;
    assert_eq!(answer(&events), "ok");
    assert_eq!(server.requests().len(), 2);
}

#[tokio::test]
async fn a_model_the_server_does_not_have_is_a_model_error() {
    let server = MockServer::start(vec![MockResponse::json(
        404,
        json!({"error":"model \"llama9\" not found, try pulling it first"}),
    )])
    .await;
    let request = ChatRequest {
        model: Some("llama9".to_owned()),
        ..ChatRequest::from_prompt("hi")
    };
    let events = collect_events(&provider(&server, OLLAMA), request).await;
    assert!(matches!(
        &events[0],
        ChatEvent::Error { kind: ErrorKind::ModelUnavailable, message } if message.contains("llama9")
    ));
}

#[tokio::test]
async fn an_error_in_the_stream_after_text_is_reported_not_retried() {
    let server = MockServer::start(vec![
        MockResponse::sse(&[
            chunk(json!({"content":"Half an "}), None),
            sse_event(
                "",
                &json!({"error":{"message":"Provider returned error","code":502}}),
            ),
        ]),
        text_response(&["never sent"]),
    ])
    .await;
    let events = collect_events(&provider(&server, OPENAI), ChatRequest::from_prompt("hi")).await;
    assert!(matches!(events[0], ChatEvent::TextDelta { .. }));
    assert!(
        matches!(&events[1], ChatEvent::Error { message, .. } if message.contains("Provider returned error"))
    );
    assert_eq!(server.requests().len(), 1);
}

#[tokio::test]
async fn cancelling_closes_the_stream_and_reports_cancelled() {
    let server = MockServer::start(vec![MockResponse::sse_then_hang(&[chunk(
        json!({"content":"Thinking about it"}),
        None,
    )])])
    .await;
    let provider = provider(&server, OPENAI);
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
async fn a_model_that_keeps_calling_tools_is_stopped_at_the_limit() {
    let call = || tool_response(&[("call_1", "echo", r#"{"text":"again"}"#)]);
    let server = MockServer::start(vec![call(), call(), call()]).await;
    let options = OpenAiCompatibleOptions {
        max_tool_rounds: 1,
        ..OpenAiCompatibleOptions::default()
    };
    let tools = TestTools::shared();
    let events = collect_events(
        &provider_with(&server, OPENAI, options),
        request_with_tools(&tools),
    )
    .await;
    assert_eq!(events.last(), Some(&finished(StopReason::ToolLoopLimit)));
    assert_eq!(tools.calls().len(), 1);
    assert_eq!(server.requests().len(), 2);
}

#[tokio::test]
async fn cut_off_and_filtered_answers_are_stop_reasons_and_run_no_tools() {
    for (finish_reason, expected) in [
        ("length", StopReason::MaxTokens),
        ("content_filter", StopReason::Refusal),
    ] {
        let server = MockServer::start(vec![MockResponse::sse(&[
            chunk(
                json!({"tool_calls":[{"index":0,"id":"call_1","type":"function",
                    "function":{"name":"echo","arguments":"{\"text\": \"par"}}]}),
                None,
            ),
            chunk(json!({}), Some(finish_reason)),
            DONE.to_owned(),
        ])])
        .await;
        let tools = TestTools::shared();
        let events = collect_events(&provider(&server, OPENAI), request_with_tools(&tools)).await;
        assert_eq!(events.last(), Some(&finished(expected)), "{finish_reason}");
        assert!(tools.calls().is_empty());
    }
}

#[tokio::test]
async fn a_hosted_service_without_a_key_sends_nothing() {
    let server = MockServer::start(vec![]).await;
    let provider = OpenAiCompatible::with_options(
        OPENAI,
        None,
        OpenAiCompatibleOptions {
            base_url: Some(format!("{}/v1", server.url())),
            ..OpenAiCompatibleOptions::default()
        },
    );
    let events = collect_events(&provider, ChatRequest::from_prompt("hi")).await;
    assert_eq!(
        events[0],
        ChatEvent::Error {
            message: "no API key is set for OpenAI".to_owned(),
            kind: ErrorKind::MissingKey,
        }
    );
    assert!(server.requests().is_empty());
    let readiness = provider.check_ready().await;
    assert_eq!(readiness.state, ReadyState::MissingKey);
    assert!(readiness.message.contains("platform.openai.com"));
}

#[tokio::test]
async fn a_local_server_gets_no_key_and_its_first_model() {
    let server = MockServer::start(vec![
        models(&["llama3.2:latest", "nomic-embed-text:latest"]),
        text_response(&["ok"]),
    ])
    .await;
    let events = collect_events(&provider(&server, OLLAMA), ChatRequest::from_prompt("hi")).await;
    assert_eq!(answer(&events), "ok");
    let requests = server.requests();
    // No model was chosen, so the server was asked which ones it has.
    assert_eq!(requests[0].path, "/v1/models");
    assert_eq!(requests[0].header("authorization"), "");
    assert_eq!(requests[1].body["model"], "llama3.2:latest");
    assert_eq!(requests[1].header("authorization"), "");
}

#[tokio::test]
async fn the_model_comes_from_the_request_then_the_settings() {
    let server = MockServer::start(vec![text_response(&["ok"]), text_response(&["ok"])]).await;
    let options = OpenAiCompatibleOptions {
        model: Some("qwen3:14b".to_owned()),
        ..OpenAiCompatibleOptions::default()
    };
    let provider = provider_with(&server, OLLAMA, options);
    collect_events(&provider, ChatRequest::from_prompt("hi")).await;
    let request = ChatRequest {
        model: Some("llama3.2".to_owned()),
        ..ChatRequest::from_prompt("hi")
    };
    collect_events(&provider, request).await;
    let requests = server.requests();
    assert_eq!(requests[0].body["model"], "qwen3:14b");
    assert_eq!(requests[1].body["model"], "llama3.2");
}

#[tokio::test]
async fn the_custom_preset_needs_an_address() {
    let provider = OpenAiCompatible::new(CUSTOM, None);
    let events = collect_events(&provider, ChatRequest::from_prompt("hi")).await;
    assert!(matches!(
        &events[0],
        ChatEvent::Error { kind: ErrorKind::InvalidRequest, message } if message.contains("server address")
    ));
    let readiness = provider.check_ready().await;
    assert_eq!(readiness.state, ReadyState::Unknown);
    assert!(readiness.message.contains("address"));
}

#[tokio::test]
async fn models_are_listed_without_the_ones_that_cannot_chat() {
    let server = MockServer::start(vec![models(&[
        "text-embedding-3-large",
        "gpt-6-luna",
        "whisper-1",
        "gpt-6.2-astra",
        "gpt-6.2-sol",
        "dall-e-3",
        "tts-1-hd",
    ])])
    .await;
    let models = provider(&server, OPENAI).list_models().await.unwrap();
    let ids: Vec<&str> = models.iter().map(|model| model.id.as_str()).collect();
    // The preset's first suggestion leads; the rest keep the server's order.
    assert_eq!(ids, ["gpt-6.2-sol", "gpt-6-luna", "gpt-6.2-astra"]);
    assert_eq!(
        server.requests()[0].header("authorization"),
        format!("Bearer {KEY}")
    );
}

#[tokio::test]
async fn model_names_come_from_the_service_when_it_gives_them() {
    let server = MockServer::start(vec![MockResponse::json(
        200,
        json!({"data":[
            {"id":"openrouter/auto","name":"Auto Router"},
            {"id":"models/gemini-flash-latest"}]}),
    )])
    .await;
    let models = provider(&server, presets::GEMINI)
        .list_models()
        .await
        .unwrap();
    assert_eq!(
        models,
        vec![
            // Listed with a `models/` prefix, still recognised as the default.
            ModelInfo::new("models/gemini-flash-latest", "gemini-flash-latest"),
            ModelInfo::new("openrouter/auto", "Auto Router"),
        ]
    );
}

#[tokio::test]
async fn readiness_describes_what_was_found() {
    let server = MockServer::start(vec![
        models(&["llama3.2"]),
        models(&[]),
        api_error(401, "invalid_api_key", "Incorrect API key provided"),
        MockResponse::text(404, "404 page not found"),
    ])
    .await;
    let ollama = provider(&server, OLLAMA);
    let ready = ollama.check_ready().await;
    assert_eq!(ready.state, ReadyState::Ready);
    assert_eq!(ready.message, "Ollama answered with 1 model.");
    let empty = ollama.check_ready().await;
    assert_eq!(empty.state, ReadyState::Unknown);
    assert!(empty.message.contains("no models yet"));

    let openai = provider(&server, OPENAI);
    assert_eq!(openai.check_ready().await.state, ReadyState::InvalidKey);
    let wrong_address = openai.check_ready().await;
    assert_eq!(wrong_address.state, ReadyState::Unknown);
    assert!(wrong_address.message.contains("server address"));
}

#[tokio::test]
async fn a_local_server_that_is_not_running_says_so() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let provider = OpenAiCompatible::with_options(
        OLLAMA,
        None,
        OpenAiCompatibleOptions {
            base_url: Some(format!("http://{address}/v1")),
            ..OpenAiCompatibleOptions::default()
        },
    );
    let readiness = provider.check_ready().await;
    assert_eq!(readiness.state, ReadyState::Unreachable);
    assert!(readiness.message.contains("Ollama is not running"));
}

#[test]
fn the_provider_takes_its_identity_from_the_preset() {
    let provider = OpenAiCompatible::new(OLLAMA, Some(ApiKey::new(KEY)));
    assert_eq!(provider.info().id, "ollama");
    assert_eq!(provider.info().name, "Ollama");
    assert_eq!(provider.info().kind, ProviderKind::Api);
    assert!(!provider.capabilities().needs_api_key);
    assert_eq!(provider.preset(), &OLLAMA);
    assert_eq!(provider.base_url().unwrap(), "http://localhost:11434/v1");
    let debug = format!("{provider:?}");
    assert!(!debug.contains(KEY));
}
