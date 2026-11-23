//! Reads the event stream of a Messages API response.
//!
//! The stream is a sequence of events: `message_start` (with the input token
//! counts), then for each content block a `content_block_start`, any number
//! of `content_block_delta`s and a `content_block_stop`, then `message_delta`
//! (the stop reason and output token count) and `message_stop`. `ping`
//! events keep the connection alive and an `error` event ends it early.
//!
//! Text and thinking are passed on as they arrive. Every block is also kept
//! whole, exactly as the API sent it, because the next request of a tool
//! loop has to send the assistant's message back unchanged, thinking blocks
//! and their signatures included.

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

use crate::api::ToolCall;
use crate::error::{AiError, ErrorKind};
use crate::event::EventSender;
use crate::http::sse::SseEvent;
use crate::http::{Failure, clean};
use crate::redact::ApiKey;

/// A complete response.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct StreamedMessage {
    /// The content blocks to send back as the assistant's message.
    pub content: Vec<Value>,
    /// The tools the model asked for.
    pub tool_calls: Vec<ToolCall>,
    /// Why the model stopped.
    pub stop_reason: Option<String>,
    /// Tokens read for this request, cached ones included.
    pub input_tokens: u64,
    /// Tokens written for this request.
    pub output_tokens: u64,
}

/// Whether the stream has more to say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Progress {
    /// More events are expected.
    Continue,
    /// `message_stop` arrived.
    Done,
}

#[derive(Debug, Default)]
struct Block {
    /// The block as the API described it, filled in by its deltas.
    value: Map<String, Value>,
    /// Tool arguments arrive as pieces of JSON text.
    partial_json: String,
    /// The arguments as text, when they did not parse as a JSON object.
    invalid_input: Option<String>,
}

/// Collects one response from its events.
#[derive(Debug, Default)]
pub(super) struct MessageStream {
    blocks: BTreeMap<u64, Block>,
    stop_reason: Option<String>,
    uncached_input: u64,
    cache_creation: u64,
    cache_read: u64,
    output_tokens: u64,
    /// Something has been shown to the user, so starting over would show it
    /// twice.
    emitted: bool,
    done: bool,
}

impl MessageStream {
    /// A reader at the start of a response.
    pub fn new() -> Self {
        Self::default()
    }

    /// True once text or thinking from this response reached the user.
    pub fn has_emitted(&self) -> bool {
        self.emitted
    }

    /// Handles one event.
    pub fn on_event(
        &mut self,
        event: &SseEvent,
        events: &EventSender,
        key: Option<&ApiKey>,
    ) -> Result<Progress, Failure> {
        let Ok(data) = serde_json::from_str::<Value>(&event.data) else {
            return Err(Failure::transient(AiError::protocol(
                "the API sent an event that is not JSON",
            )));
        };
        // The `type` inside the data is authoritative; the event name
        // repeats it.
        let kind = data
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or(&event.event);
        match kind {
            "message_start" => self.read_usage(data.pointer("/message/usage")),
            "content_block_start" => self.on_block_start(&data, events),
            "content_block_delta" => self.on_block_delta(&data, events),
            "content_block_stop" => self.on_block_stop(&data),
            "message_delta" => {
                if let Some(reason) = data.pointer("/delta/stop_reason").and_then(Value::as_str) {
                    self.stop_reason = Some(reason.to_owned());
                }
                self.read_usage(data.get("usage"));
            }
            "message_stop" => {
                self.done = true;
                return Ok(Progress::Done);
            }
            "error" => return Err(stream_error(&data, key)),
            // `ping`, and whatever is added later.
            _ => {}
        }
        Ok(Progress::Continue)
    }

    /// The response, once the stream has ended.
    pub fn finish(self) -> Result<StreamedMessage, Failure> {
        if !self.done {
            return Err(Failure::transient(AiError::network(
                "the connection closed before the answer was complete",
            )));
        }
        let blocks: Vec<Block> = self.blocks.into_values().collect();
        // After a refusal fallback, only what follows the last switch is
        // the serving model's own work.
        let boundary = blocks
            .iter()
            .rposition(|block| block_type(&block.value) == "fallback");

        let mut content = Vec::new();
        let mut tool_calls = Vec::new();
        for (index, block) in blocks.into_iter().enumerate() {
            let kind = block_type(&block.value).to_owned();
            let before_boundary = boundary.is_some_and(|boundary| index < boundary);
            if before_boundary && kind != "text" && kind != "fallback" {
                // Thinking and tool calls of a model that then declined are
                // not sent back, as the API asks.
                continue;
            }
            if kind == "text" && text_of(&block.value, "text").is_empty() {
                continue; // the API rejects empty text blocks
            }
            if kind == "tool_use" {
                let text = |name: &str| text_of(&block.value, name).to_owned();
                tool_calls.push(ToolCall {
                    id: text("id"),
                    name: text("name"),
                    input: match &block.invalid_input {
                        Some(raw) => Err(raw.clone()),
                        None => Ok(block.value.get("input").cloned().unwrap_or(json!({}))),
                    },
                });
            }
            content.push(Value::Object(block.value));
        }
        Ok(StreamedMessage {
            content,
            tool_calls,
            stop_reason: self.stop_reason,
            input_tokens: self.uncached_input + self.cache_creation + self.cache_read,
            output_tokens: self.output_tokens,
        })
    }

    /// Token counts arrive in `message_start` and are updated, as running
    /// totals, in `message_delta`.
    fn read_usage(&mut self, usage: Option<&Value>) {
        let Some(usage) = usage else {
            return;
        };
        let read = |name: &str, slot: &mut u64| {
            if let Some(count) = usage.get(name).and_then(Value::as_u64) {
                *slot = count;
            }
        };
        read("input_tokens", &mut self.uncached_input);
        read("cache_creation_input_tokens", &mut self.cache_creation);
        read("cache_read_input_tokens", &mut self.cache_read);
        read("output_tokens", &mut self.output_tokens);
    }

    fn on_block_start(&mut self, data: &Value, events: &EventSender) {
        let Some(index) = data.get("index").and_then(Value::as_u64) else {
            return;
        };
        let value = data
            .get("content_block")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        // A block can start with some of its text already in it.
        match block_type(&value) {
            "text" => self.emit_text(text_of(&value, "text"), events),
            "thinking" => self.emit_thinking(text_of(&value, "thinking"), events),
            _ => {}
        }
        self.blocks.insert(
            index,
            Block {
                value,
                ..Block::default()
            },
        );
    }

    fn on_block_delta(&mut self, data: &Value, events: &EventSender) {
        let Some(delta) = data.get("delta") else {
            return;
        };
        let piece = |name: &str| delta.get(name).and_then(Value::as_str).unwrap_or("");
        let kind = delta.get("type").and_then(Value::as_str).unwrap_or("");
        // Shown before it is stored, so that the borrow of the block below
        // does not overlap.
        match kind {
            "text_delta" => self.emit_text(piece("text"), events),
            "thinking_delta" => self.emit_thinking(piece("thinking"), events),
            _ => {}
        }
        let Some(block) = data
            .get("index")
            .and_then(Value::as_u64)
            .and_then(|index| self.blocks.get_mut(&index))
        else {
            return;
        };
        match kind {
            "text_delta" => append(&mut block.value, "text", piece("text")),
            "thinking_delta" => append(&mut block.value, "thinking", piece("thinking")),
            "signature_delta" => append(&mut block.value, "signature", piece("signature")),
            "input_json_delta" => block.partial_json.push_str(piece("partial_json")),
            "citations_delta" => {
                if let Some(citation) = delta.get("citation") {
                    let citations = block
                        .value
                        .entry("citations")
                        .or_insert_with(|| Value::Array(Vec::new()));
                    if let Some(list) = citations.as_array_mut() {
                        list.push(citation.clone());
                    }
                }
            }
            _ => {}
        }
    }

    /// A finished tool call has all of its arguments. They are parsed
    /// strictly: with eager input streaming the API passes on whatever the
    /// model wrote, checked or not.
    fn on_block_stop(&mut self, data: &Value) {
        let Some(block) = data
            .get("index")
            .and_then(Value::as_u64)
            .and_then(|index| self.blocks.get_mut(&index))
        else {
            return;
        };
        if block.partial_json.trim().is_empty() {
            return;
        }
        let raw = std::mem::take(&mut block.partial_json);
        match serde_json::from_str::<Value>(&raw) {
            Ok(input @ Value::Object(_)) => {
                block.value.insert("input".into(), input);
            }
            _ => {
                // The block still has to go back to the API in a valid
                // shape; the tool result will carry the text.
                block.value.insert("input".into(), json!({}));
                block.invalid_input = Some(raw);
            }
        }
    }

    fn emit_text(&mut self, text: &str, events: &EventSender) {
        if !text.is_empty() {
            self.emitted = true;
            events.text(text);
        }
    }

    fn emit_thinking(&mut self, text: &str, events: &EventSender) {
        if !text.is_empty() {
            self.emitted = true;
            events.thinking(text);
        }
    }
}

fn block_type(block: &Map<String, Value>) -> &str {
    text_of(block, "type")
}

fn text_of<'a>(block: &'a Map<String, Value>, name: &str) -> &'a str {
    block.get(name).and_then(Value::as_str).unwrap_or("")
}

/// Appends to a string field of a block, creating it when missing.
fn append(block: &mut Map<String, Value>, name: &str, piece: &str) {
    match block.get_mut(name) {
        Some(Value::String(existing)) => existing.push_str(piece),
        _ => {
            block.insert(name.to_owned(), Value::String(piece.to_owned()));
        }
    }
}

/// The kind of failure an API error type stands for, and whether the same
/// request may work a moment later.
pub(super) fn classify_error_type(error_type: &str) -> Option<(ErrorKind, bool)> {
    Some(match error_type {
        "authentication_error" => (ErrorKind::InvalidKey, false),
        "permission_error" | "billing_error" => (ErrorKind::Other, false),
        "not_found_error" => (ErrorKind::ModelUnavailable, false),
        "invalid_request_error" | "request_too_large" => (ErrorKind::InvalidRequest, false),
        "rate_limit_error" => (ErrorKind::RateLimited, true),
        "overloaded_error" | "api_error" => (ErrorKind::Overloaded, true),
        "timeout_error" => (ErrorKind::Network, true),
        _ => return None,
    })
}

/// The failure for an `error` event in the middle of a stream.
fn stream_error(data: &Value, key: Option<&ApiKey>) -> Failure {
    let error_type = data
        .pointer("/error/type")
        .and_then(Value::as_str)
        .unwrap_or("");
    let message = data
        .pointer("/error/message")
        .and_then(Value::as_str)
        .unwrap_or("the API reported an error without a message");
    let (kind, retryable) = classify_error_type(error_type).unwrap_or((ErrorKind::Other, false));
    Failure {
        error: AiError::from_kind(kind, clean(message, key)),
        retryable,
        retry_after: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{ChatEvent, EventReceiver, event_channel};

    fn feed(lines: &[Value]) -> (Result<StreamedMessage, Failure>, Vec<ChatEvent>, bool) {
        let (sender, mut receiver) = event_channel();
        let mut stream = MessageStream::new();
        let mut failure = None;
        for data in lines {
            let event = SseEvent {
                event: data["type"].as_str().unwrap_or("").to_owned(),
                data: data.to_string(),
            };
            if let Err(error) = stream.on_event(&event, &sender, None) {
                failure = Some(error);
                break;
            }
        }
        let emitted = stream.has_emitted();
        let result = match failure {
            Some(failure) => Err(failure),
            None => stream.finish(),
        };
        (result, drain(&mut receiver), emitted)
    }

    fn drain(receiver: &mut EventReceiver) -> Vec<ChatEvent> {
        let mut events = Vec::new();
        while let Ok(event) = receiver.try_recv() {
            events.push(event);
        }
        events
    }

    fn start(usage: Value) -> Value {
        json!({"type":"message_start","message":{"id":"msg_1","type":"message","role":"assistant",
            "model":"claude-opus-5-5","content":[],"stop_reason":null,"usage":usage}})
    }

    fn block_start(index: u64, block: Value) -> Value {
        json!({"type":"content_block_start","index":index,"content_block":block})
    }

    fn delta(index: u64, delta: Value) -> Value {
        json!({"type":"content_block_delta","index":index,"delta":delta})
    }

    fn block_stop(index: u64) -> Value {
        json!({"type":"content_block_stop","index":index})
    }

    fn end(stop_reason: &str, output_tokens: u64) -> [Value; 2] {
        [
            json!({"type":"message_delta","delta":{"stop_reason":stop_reason,"stop_sequence":null},
                "usage":{"output_tokens":output_tokens}}),
            json!({"type":"message_stop"}),
        ]
    }

    #[test]
    fn text_is_streamed_and_kept_as_one_block() {
        let mut lines = vec![
            start(json!({"input_tokens":25,"cache_creation_input_tokens":100,
                "cache_read_input_tokens":2000,"output_tokens":1})),
            block_start(0, json!({"type":"text","text":""})),
            json!({"type":"ping"}),
            delta(0, json!({"type":"text_delta","text":"Hel"})),
            delta(0, json!({"type":"text_delta","text":"lo"})),
            block_stop(0),
        ];
        lines.extend(end("end_turn", 12));
        let (message, events, emitted) = feed(&lines);
        let message = message.unwrap();
        assert_eq!(message.content, [json!({"type":"text","text":"Hello"})]);
        assert_eq!(message.stop_reason.as_deref(), Some("end_turn"));
        // Cached tokens count as input.
        assert_eq!(message.input_tokens, 2125);
        assert_eq!(message.output_tokens, 12);
        assert!(message.tool_calls.is_empty());
        assert!(emitted);
        assert_eq!(
            events,
            [
                ChatEvent::TextDelta {
                    text: "Hel".to_owned()
                },
                ChatEvent::TextDelta {
                    text: "lo".to_owned()
                },
            ]
        );
    }

    #[test]
    fn thinking_is_streamed_and_kept_with_its_signature() {
        let mut lines = vec![
            start(json!({"input_tokens":10,"output_tokens":1})),
            block_start(0, json!({"type":"thinking","thinking":"","signature":""})),
            delta(0, json!({"type":"thinking_delta","thinking":"The card "})),
            delta(
                0,
                json!({"type":"thinking_delta","thinking":"needs a fade."}),
            ),
            delta(0, json!({"type":"signature_delta","signature":"EqQBCg=="})),
            block_stop(0),
            block_start(1, json!({"type":"text","text":""})),
            delta(1, json!({"type":"text_delta","text":"Done."})),
            block_stop(1),
        ];
        lines.extend(end("end_turn", 30));
        let (message, events, _) = feed(&lines);
        assert_eq!(
            message.unwrap().content[0],
            json!({"type":"thinking","thinking":"The card needs a fade.","signature":"EqQBCg=="})
        );
        assert_eq!(
            events[0],
            ChatEvent::ThinkingDelta {
                text: "The card ".to_owned()
            }
        );
    }

    #[test]
    fn tool_arguments_are_assembled_from_their_pieces() {
        let mut lines = vec![
            start(json!({"input_tokens":10,"output_tokens":1})),
            block_start(
                0,
                json!({"type":"tool_use","id":"toolu_1","name":"echo","input":{}}),
            ),
            delta(0, json!({"type":"input_json_delta","partial_json":""})),
            delta(
                0,
                json!({"type":"input_json_delta","partial_json":"{\"text\": \"pi"}),
            ),
            delta(0, json!({"type":"input_json_delta","partial_json":"ng\"}"})),
            block_stop(0),
            block_start(
                1,
                json!({"type":"tool_use","id":"toolu_2","name":"get_overview","input":{}}),
            ),
            block_stop(1),
        ];
        lines.extend(end("tool_use", 40));
        let (message, events, emitted) = feed(&lines);
        let message = message.unwrap();
        assert_eq!(
            message.tool_calls,
            [
                ToolCall {
                    id: "toolu_1".to_owned(),
                    name: "echo".to_owned(),
                    input: Ok(json!({"text":"ping"})),
                },
                // A tool called without arguments gets an empty object.
                ToolCall {
                    id: "toolu_2".to_owned(),
                    name: "get_overview".to_owned(),
                    input: Ok(json!({})),
                },
            ]
        );
        assert_eq!(
            message.content[0],
            json!({"type":"tool_use","id":"toolu_1","name":"echo","input":{"text":"ping"}})
        );
        assert!(events.is_empty());
        assert!(!emitted);
    }

    #[test]
    fn arguments_that_are_not_a_json_object_are_kept_as_text() {
        for raw in ["{\"text\": \"cut off", "[1, 2]", "\"just a string\""] {
            let mut lines = vec![
                start(json!({"input_tokens":1,"output_tokens":1})),
                block_start(
                    0,
                    json!({"type":"tool_use","id":"toolu_1","name":"echo","input":{}}),
                ),
                delta(0, json!({"type":"input_json_delta","partial_json":raw})),
                block_stop(0),
            ];
            lines.extend(end("tool_use", 5));
            let message = feed(&lines).0.unwrap();
            assert_eq!(message.tool_calls[0].input, Err(raw.to_owned()), "{raw}");
            // What goes back to the API is still a well-formed block.
            assert_eq!(message.content[0]["input"], json!({}));
        }
    }

    #[test]
    fn empty_text_blocks_are_not_sent_back() {
        let mut lines = vec![
            start(json!({"input_tokens":1,"output_tokens":1})),
            block_start(0, json!({"type":"text","text":""})),
            block_stop(0),
            block_start(
                1,
                json!({"type":"tool_use","id":"toolu_1","name":"echo","input":{}}),
            ),
            block_stop(1),
        ];
        lines.extend(end("tool_use", 5));
        let message = feed(&lines).0.unwrap();
        assert_eq!(message.content.len(), 1);
        assert_eq!(message.content[0]["type"], "tool_use");
    }

    #[test]
    fn blocks_of_unknown_types_are_kept_as_they_came() {
        let unknown = json!({"type":"redacted_thinking","data":"b3BhcXVl"});
        let mut lines = vec![
            start(json!({"input_tokens":1,"output_tokens":1})),
            block_start(0, unknown.clone()),
            block_stop(0),
            block_start(1, json!({"type":"text","text":"ok"})),
            block_stop(1),
        ];
        lines.extend(end("end_turn", 5));
        let message = feed(&lines).0.unwrap();
        assert_eq!(message.content[0], unknown);
    }

    #[test]
    fn after_a_fallback_only_the_serving_models_work_is_sent_back() {
        let fallback = json!({"type":"fallback","from":{"model":"claude-opus-5-5"},
            "to":{"model":"claude-opus-4-8"}});
        let mut lines = vec![
            start(json!({"input_tokens":1,"output_tokens":1})),
            block_start(
                0,
                json!({"type":"thinking","thinking":"declined model","signature":"a"}),
            ),
            block_stop(0),
            block_start(1, json!({"type":"text","text":"Partial "})),
            block_stop(1),
            block_start(
                2,
                json!({"type":"tool_use","id":"toolu_old","name":"echo","input":{}}),
            ),
            block_stop(2),
            block_start(3, fallback.clone()),
            block_stop(3),
            block_start(
                4,
                json!({"type":"thinking","thinking":"serving model","signature":"b"}),
            ),
            block_stop(4),
            block_start(
                5,
                json!({"type":"tool_use","id":"toolu_new","name":"echo","input":{}}),
            ),
            block_stop(5),
        ];
        lines.extend(end("tool_use", 5));
        let message = feed(&lines).0.unwrap();
        let kinds: Vec<&str> = message
            .content
            .iter()
            .map(|block| block["type"].as_str().unwrap())
            .collect();
        assert_eq!(kinds, ["text", "fallback", "thinking", "tool_use"]);
        assert_eq!(message.content[1], fallback);
        assert_eq!(message.tool_calls.len(), 1);
        assert_eq!(message.tool_calls[0].id, "toolu_new");
    }

    #[test]
    fn an_error_event_is_classified_by_its_type() {
        let cases = [
            ("overloaded_error", ErrorKind::Overloaded, true),
            ("rate_limit_error", ErrorKind::RateLimited, true),
            ("api_error", ErrorKind::Overloaded, true),
            ("invalid_request_error", ErrorKind::InvalidRequest, false),
            ("authentication_error", ErrorKind::InvalidKey, false),
            ("something_new", ErrorKind::Other, false),
        ];
        for (error_type, kind, retryable) in cases {
            let lines = [
                start(json!({"input_tokens":1,"output_tokens":1})),
                json!({"type":"error","error":{"type":error_type,"message":"Overloaded"}}),
            ];
            let failure = feed(&lines).0.unwrap_err();
            assert_eq!(failure.error.kind(), kind, "{error_type}");
            assert_eq!(failure.retryable, retryable, "{error_type}");
        }
    }

    #[test]
    fn a_stream_that_ends_early_is_a_network_failure() {
        let lines = [
            start(json!({"input_tokens":1,"output_tokens":1})),
            block_start(0, json!({"type":"text","text":""})),
            delta(0, json!({"type":"text_delta","text":"Hel"})),
        ];
        let (result, _, emitted) = feed(&lines);
        let failure = result.unwrap_err();
        assert_eq!(failure.error.kind(), ErrorKind::Network);
        assert!(emitted);
    }

    #[test]
    fn data_that_is_not_json_is_a_protocol_failure() {
        let (sender, _receiver) = event_channel();
        let mut stream = MessageStream::new();
        let event = SseEvent {
            event: "message_start".to_owned(),
            data: "<html>gateway error</html>".to_owned(),
        };
        let failure = stream.on_event(&event, &sender, None).unwrap_err();
        assert_eq!(failure.error.kind(), ErrorKind::Protocol);
    }

    #[test]
    fn a_refusal_keeps_its_stop_reason() {
        let mut lines = vec![start(json!({"input_tokens":9,"output_tokens":0}))];
        lines.push(
            json!({"type":"message_delta","delta":{"stop_reason":"refusal",
            "stop_details":{"type":"refusal","category":"cyber","explanation":null}},
            "usage":{"output_tokens":0}}),
        );
        lines.push(json!({"type":"message_stop"}));
        let message = feed(&lines).0.unwrap();
        assert_eq!(message.stop_reason.as_deref(), Some("refusal"));
        assert!(message.content.is_empty());
    }
}
