//! Reads the event stream of a Chat Completions response.
//!
//! Each event carries a chunk: `choices[0].delta` has a piece of text, of
//! reasoning, or of one or more tool calls, and the last chunk of a choice
//! has a `finish_reason`. Token counts come in a chunk of their own when the
//! request asked for them, and `[DONE]` ends the stream.
//!
//! The format has many implementers and they differ in details, so this
//! reader is forgiving: reasoning may be called `reasoning_content` or
//! `reasoning`, tool call pieces may lack an index or an id, arguments may
//! arrive as text or as an object, and a missing `[DONE]` is fine when the
//! model said it had finished.

use serde_json::{Map, Value, json};

use crate::api::ToolCall;
use crate::error::{AiError, ErrorKind};
use crate::event::EventSender;
use crate::http::{Failure, clean, is_retryable_status, kind_for_status};
use crate::redact::ApiKey;

/// A complete response.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct AssistantTurn {
    /// What the model said.
    pub text: String,
    /// The tools the model asked for.
    pub tool_calls: Vec<ToolCall>,
    /// The same calls in the shape the API wants to see them again.
    pub raw_tool_calls: Vec<Value>,
    /// Why the model stopped.
    pub finish_reason: Option<String>,
    /// Tokens read for this request.
    pub input_tokens: u64,
    /// Tokens written for this request.
    pub output_tokens: u64,
}

/// Whether the stream has more to say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Progress {
    /// More chunks are expected.
    Continue,
    /// `[DONE]` arrived.
    Done,
}

/// A tool call being put together from its pieces.
#[derive(Debug, Default)]
struct PartialCall {
    index: Option<u64>,
    id: String,
    name: String,
    arguments: String,
    /// Fields this crate does not know, such as the signatures some
    /// services attach to a call and expect back.
    extra: Map<String, Value>,
}

/// Collects one response from its chunks.
#[derive(Debug, Default)]
pub(super) struct ChatStream {
    text: String,
    calls: Vec<PartialCall>,
    finish_reason: Option<String>,
    input_tokens: u64,
    output_tokens: u64,
    /// Something has been shown to the user, so starting over would show it
    /// twice.
    emitted: bool,
}

impl ChatStream {
    /// A reader at the start of a response.
    pub fn new() -> Self {
        Self::default()
    }

    /// True once text or reasoning from this response reached the user.
    pub fn has_emitted(&self) -> bool {
        self.emitted
    }

    /// Handles the data of one event.
    pub fn on_data(
        &mut self,
        data: &str,
        events: &EventSender,
        key: Option<&ApiKey>,
    ) -> Result<Progress, Failure> {
        let data = data.trim();
        if data == "[DONE]" {
            return Ok(Progress::Done);
        }
        if data.is_empty() {
            return Ok(Progress::Continue);
        }
        let Ok(chunk) = serde_json::from_str::<Value>(data) else {
            return Err(Failure::transient(AiError::protocol(
                "the server sent a chunk that is not JSON",
            )));
        };
        if chunk.get("error").is_some_and(|error| !error.is_null()) {
            return Err(error_failure(None, &chunk, "", key));
        }
        if let Some(usage) = chunk.get("usage").filter(|usage| usage.is_object()) {
            let count = |name: &str| usage.get(name).and_then(Value::as_u64);
            if let Some(tokens) = count("prompt_tokens") {
                self.input_tokens = tokens;
            }
            if let Some(tokens) = count("completion_tokens") {
                self.output_tokens = tokens;
            }
        }
        let Some(choice) = chunk.pointer("/choices/0") else {
            return Ok(Progress::Continue);
        };
        if let Some(delta) = choice.get("delta") {
            let piece = |name: &str| delta.get(name).and_then(Value::as_str).unwrap_or("");
            let reasoning = match piece("reasoning_content") {
                "" => piece("reasoning"),
                text => text,
            };
            if !reasoning.is_empty() {
                self.emitted = true;
                events.thinking(reasoning);
            }
            let content = piece("content");
            if !content.is_empty() {
                self.emitted = true;
                self.text.push_str(content);
                events.text(content);
            }
            for piece in delta
                .get("tool_calls")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                self.merge_call(piece);
            }
        }
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            self.finish_reason = Some(reason.to_owned());
        }
        Ok(Progress::Continue)
    }

    /// The response, once the stream has ended. `saw_done` says whether the
    /// server sent `[DONE]`.
    pub fn finish(self, saw_done: bool) -> Result<AssistantTurn, Failure> {
        if !saw_done && self.finish_reason.is_none() {
            return Err(Failure::transient(AiError::network(
                "the connection closed before the answer was complete",
            )));
        }
        let mut tool_calls = Vec::new();
        let mut raw_tool_calls = Vec::new();
        for (position, call) in self.calls.into_iter().enumerate() {
            if call.name.is_empty() {
                continue; // a call without a name cannot be run or answered
            }
            let id = if call.id.is_empty() {
                format!("call_{position}")
            } else {
                call.id
            };
            let input = parse_arguments(&call.arguments);
            let mut raw = call.extra;
            raw.insert("id".into(), json!(id));
            raw.insert("type".into(), json!("function"));
            raw.insert(
                "function".into(),
                json!({
                    "name": call.name,
                    // Arguments that were not JSON go back as an empty
                    // object; the tool result carries the original text.
                    "arguments": if input.is_ok() && !call.arguments.trim().is_empty() {
                        call.arguments.as_str()
                    } else {
                        "{}"
                    },
                }),
            );
            raw_tool_calls.push(Value::Object(raw));
            tool_calls.push(ToolCall {
                id,
                name: call.name,
                input,
            });
        }
        Ok(AssistantTurn {
            text: self.text,
            tool_calls,
            raw_tool_calls,
            finish_reason: self.finish_reason,
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
        })
    }

    /// Adds one piece of a tool call to the call it belongs to.
    fn merge_call(&mut self, piece: &Value) {
        let index = piece.get("index").and_then(Value::as_u64);
        let id = piece.get("id").and_then(Value::as_str).unwrap_or("");
        let position = match index {
            Some(index) => self.calls.iter().position(|call| call.index == Some(index)),
            // No index: a piece with a new id starts a call, anything else
            // continues the latest one.
            None if !id.is_empty() => self.calls.iter().position(|call| call.id == id),
            None => self.calls.len().checked_sub(1),
        };
        let call = match position {
            Some(position) => &mut self.calls[position],
            None => {
                self.calls.push(PartialCall {
                    index,
                    ..PartialCall::default()
                });
                self.calls.last_mut().expect("a call was just added")
            }
        };
        if call.id.is_empty() {
            call.id = id.to_owned();
        }
        if let Some(function) = piece.get("function") {
            // The name comes once; servers that repeat it mean the same one.
            if call.name.is_empty()
                && let Some(name) = function.get("name").and_then(Value::as_str)
            {
                call.name = name.to_owned();
            }
            match function.get("arguments") {
                Some(Value::String(text)) => call.arguments.push_str(text),
                Some(Value::Null) | None => {}
                // Some servers send the arguments already parsed.
                Some(object) => call.arguments = object.to_string(),
            }
        }
        if let Some(fields) = piece.as_object() {
            for (name, value) in fields {
                if !matches!(name.as_str(), "index" | "id" | "type" | "function") {
                    call.extra.insert(name.clone(), value.clone());
                }
            }
        }
    }
}

/// Tool arguments must be a JSON object. No arguments at all means an empty
/// one.
fn parse_arguments(arguments: &str) -> Result<Value, String> {
    if arguments.trim().is_empty() {
        return Ok(json!({}));
    }
    match serde_json::from_str::<Value>(arguments) {
        Ok(input @ Value::Object(_)) => Ok(input),
        _ => Err(arguments.to_owned()),
    }
}

/// What a service said went wrong: its message, and its own code for the
/// error when it gave one. Understands the shapes in common use:
/// `{"error": {"message", "code" | "type" | "status"}}`, the same wrapped in
/// a list, `{"error": "text"}`, and `{"message" | "detail": "text"}`.
pub(super) fn describe_error(body: &Value) -> Option<(String, Option<String>)> {
    let body = match body {
        Value::Array(items) => items.first()?,
        other => other,
    };
    let text = |value: &Value, name: &str| -> Option<String> {
        match value.get(name)? {
            Value::String(text) if !text.trim().is_empty() => Some(text.clone()),
            Value::Number(number) => Some(number.to_string()),
            _ => None,
        }
    };
    match body.get("error") {
        Some(Value::String(message)) => Some((message.clone(), None)),
        Some(error @ Value::Object(_)) => {
            let message = text(error, "message")?;
            let code = ["code", "type", "status"]
                .iter()
                .filter_map(|name| text(error, name))
                // A bare HTTP status says nothing the status did not.
                .find(|code| code.parse::<u16>().is_err());
            Some((message, code))
        }
        _ => text(body, "message")
            .or_else(|| text(body, "detail"))
            .map(|message| (message, None)),
    }
}

/// The failure for an error the service reported, in an HTTP error
/// response (`status` given) or in the middle of a stream (`None`).
pub(super) fn error_failure(
    status: Option<u16>,
    body: &Value,
    raw_body: &str,
    key: Option<&ApiKey>,
) -> Failure {
    let (message, code) =
        describe_error(body).unwrap_or_else(|| (crate::tool::summarize_text(raw_body), None));
    let code = code.unwrap_or_default().to_ascii_lowercase();
    let lowered = message.to_ascii_lowercase();
    let by_code = match code.as_str() {
        "invalid_api_key"
        | "invalid_authentication"
        | "authentication_error"
        | "unauthenticated" => Some(ErrorKind::InvalidKey),
        "model_not_found" | "model_not_available" => Some(ErrorKind::ModelUnavailable),
        "insufficient_quota"
        | "rate_limit_exceeded"
        | "rate_limit_error"
        | "resource_exhausted" => Some(ErrorKind::RateLimited),
        "context_length_exceeded" | "invalid_request_error" | "invalid_argument" => {
            Some(ErrorKind::InvalidRequest)
        }
        "overloaded_error" | "server_error" | "unavailable" => Some(ErrorKind::Overloaded),
        _ => None,
    };
    // Some services answer a bad key with a 400 or 403 and say so in words.
    let key_problem = matches!(status, Some(400 | 401 | 403))
        && (lowered.contains("api key") || lowered.contains("api_key"));
    let kind = if key_problem {
        ErrorKind::InvalidKey
    } else {
        by_code
            .or_else(|| status.map(kind_for_status))
            .unwrap_or(ErrorKind::Other)
    };
    let mut message = clean(message.trim(), key);
    if message.is_empty() {
        message = match status {
            Some(status) => format!("the server answered with HTTP status {status}"),
            None => "the server reported an error without a message".to_owned(),
        };
    }
    Failure {
        error: AiError::from_kind(kind, message),
        retryable: match status {
            Some(status) => is_retryable_status(status),
            None => matches!(kind, ErrorKind::RateLimited | ErrorKind::Overloaded),
        },
        retry_after: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{ChatEvent, event_channel};

    fn feed(chunks: &[&str]) -> (Result<AssistantTurn, Failure>, Vec<ChatEvent>) {
        let (sender, mut receiver) = event_channel();
        let mut stream = ChatStream::new();
        let mut done = false;
        let mut failure = None;
        for chunk in chunks {
            match stream.on_data(chunk, &sender, None) {
                Ok(Progress::Continue) => {}
                Ok(Progress::Done) => {
                    done = true;
                    break;
                }
                Err(error) => {
                    failure = Some(error);
                    break;
                }
            }
        }
        let result = match failure {
            Some(failure) => Err(failure),
            None => stream.finish(done),
        };
        let mut events = Vec::new();
        while let Ok(event) = receiver.try_recv() {
            events.push(event);
        }
        (result, events)
    }

    fn delta(delta: Value) -> String {
        json!({"id":"chatcmpl-1","object":"chat.completion.chunk",
            "choices":[{"index":0,"delta":delta,"finish_reason":null}]})
        .to_string()
    }

    fn finish(reason: &str) -> String {
        json!({"choices":[{"index":0,"delta":{},"finish_reason":reason}]}).to_string()
    }

    #[test]
    fn text_and_usage_are_collected() {
        let chunks = [
            delta(json!({"role":"assistant","content":""})),
            delta(json!({"content":"Hel"})),
            delta(json!({"content":"lo"})),
            finish("stop"),
            json!({"choices":[],"usage":{"prompt_tokens":12,"completion_tokens":3,
                "total_tokens":15}})
            .to_string(),
            "[DONE]".to_owned(),
        ];
        let chunks: Vec<&str> = chunks.iter().map(String::as_str).collect();
        let (turn, events) = feed(&chunks);
        let turn = turn.unwrap();
        assert_eq!(turn.text, "Hello");
        assert_eq!(turn.finish_reason.as_deref(), Some("stop"));
        assert_eq!((turn.input_tokens, turn.output_tokens), (12, 3));
        assert!(turn.tool_calls.is_empty());
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
    fn reasoning_is_passed_on_under_either_name() {
        let chunks = [
            delta(json!({"reasoning_content":"First, "})),
            delta(json!({"reasoning":"then."})),
            delta(json!({"content":"Done"})),
            finish("stop"),
        ];
        let chunks: Vec<&str> = chunks.iter().map(String::as_str).collect();
        let (turn, events) = feed(&chunks);
        // No `[DONE]`, but the model said it had finished.
        assert_eq!(turn.unwrap().text, "Done");
        assert_eq!(
            events[..2],
            [
                ChatEvent::ThinkingDelta {
                    text: "First, ".to_owned()
                },
                ChatEvent::ThinkingDelta {
                    text: "then.".to_owned()
                },
            ]
        );
    }

    #[test]
    fn parallel_tool_calls_are_assembled_by_index() {
        let chunks = [
            delta(
                json!({"tool_calls":[{"index":0,"id":"call_a","type":"function",
                "function":{"name":"echo","arguments":""}}]}),
            ),
            delta(
                json!({"tool_calls":[{"index":1,"id":"call_b","type":"function",
                "function":{"name":"get_overview","arguments":""}}]}),
            ),
            delta(json!({"tool_calls":[{"index":0,"function":{"arguments":"{\"text\":"}}]})),
            delta(json!({"tool_calls":[{"index":0,"function":{"arguments":"\"ping\"}"}}]})),
            finish("tool_calls"),
            "[DONE]".to_owned(),
        ];
        let chunks: Vec<&str> = chunks.iter().map(String::as_str).collect();
        let turn = feed(&chunks).0.unwrap();
        assert_eq!(
            turn.tool_calls,
            [
                ToolCall {
                    id: "call_a".to_owned(),
                    name: "echo".to_owned(),
                    input: Ok(json!({"text":"ping"})),
                },
                ToolCall {
                    id: "call_b".to_owned(),
                    name: "get_overview".to_owned(),
                    input: Ok(json!({})),
                },
            ]
        );
        assert_eq!(
            turn.raw_tool_calls[0],
            json!({"id":"call_a","type":"function",
                "function":{"name":"echo","arguments":"{\"text\":\"ping\"}"}})
        );
        assert_eq!(turn.raw_tool_calls[1]["function"]["arguments"], "{}");
    }

    #[test]
    fn whole_calls_without_index_or_id_are_understood() {
        // The shape some local servers send: one piece, arguments already
        // an object, no id.
        let chunks = [
            delta(json!({"tool_calls":[
                {"function":{"name":"echo","arguments":{"text":"ping"}}}]})),
            finish("stop"),
            "[DONE]".to_owned(),
        ];
        let chunks: Vec<&str> = chunks.iter().map(String::as_str).collect();
        let turn = feed(&chunks).0.unwrap();
        assert_eq!(turn.tool_calls.len(), 1);
        assert_eq!(turn.tool_calls[0].id, "call_0");
        assert_eq!(turn.tool_calls[0].input, Ok(json!({"text":"ping"})));
        assert_eq!(turn.raw_tool_calls[0]["id"], "call_0");
    }

    #[test]
    fn fields_a_service_attaches_to_a_call_are_sent_back() {
        let chunks = [
            delta(
                json!({"tool_calls":[{"index":0,"id":"call_a","type":"function",
                "function":{"name":"echo","arguments":"{}"},
                "extra_content":{"google":{"thought_signature":"c2ln"}}}]}),
            ),
            finish("tool_calls"),
            "[DONE]".to_owned(),
        ];
        let chunks: Vec<&str> = chunks.iter().map(String::as_str).collect();
        let turn = feed(&chunks).0.unwrap();
        assert_eq!(
            turn.raw_tool_calls[0]["extra_content"]["google"]["thought_signature"],
            "c2ln"
        );
    }

    #[test]
    fn arguments_that_are_not_a_json_object_are_kept_as_text() {
        let chunks = [
            delta(
                json!({"tool_calls":[{"index":0,"id":"call_a","type":"function",
                "function":{"name":"echo","arguments":"{\"text\": \"cut"}}]}),
            ),
            finish("tool_calls"),
            "[DONE]".to_owned(),
        ];
        let chunks: Vec<&str> = chunks.iter().map(String::as_str).collect();
        let turn = feed(&chunks).0.unwrap();
        assert_eq!(turn.tool_calls[0].input, Err("{\"text\": \"cut".to_owned()));
        assert_eq!(turn.raw_tool_calls[0]["function"]["arguments"], "{}");
    }

    #[test]
    fn an_error_in_the_stream_is_a_failure() {
        let chunk = json!({"error":{"message":"Provider returned error","code":502}});
        let failure = feed(&[&chunk.to_string()]).0.unwrap_err();
        assert!(
            failure
                .error
                .to_string()
                .contains("Provider returned error")
        );

        let chunk = json!({"error":{"message":"Rate limit reached",
            "code":"rate_limit_exceeded"}});
        let failure = feed(&[&chunk.to_string()]).0.unwrap_err();
        assert_eq!(failure.error.kind(), ErrorKind::RateLimited);
        assert!(failure.retryable);
    }

    #[test]
    fn a_stream_that_ends_without_a_finish_is_a_network_failure() {
        let chunk = delta(json!({"content":"Hel"}));
        let failure = feed(&[&chunk]).0.unwrap_err();
        assert_eq!(failure.error.kind(), ErrorKind::Network);
    }

    #[test]
    fn error_bodies_of_every_common_shape_are_understood() {
        let cases = [
            (
                json!({"error":{"message":"Incorrect API key provided","type":"invalid_request_error",
                    "code":"invalid_api_key"}}),
                "Incorrect API key provided",
                Some("invalid_api_key"),
            ),
            (
                json!([{"error":{"code":400,"message":"API key not valid.","status":"INVALID_ARGUMENT"}}]),
                "API key not valid.",
                Some("INVALID_ARGUMENT"),
            ),
            (
                json!({"error":"model \"llama9\" not found, try pulling it first"}),
                "model \"llama9\" not found, try pulling it first",
                None,
            ),
            (json!({"message":"Unauthorized"}), "Unauthorized", None),
            (json!({"detail":"Not Found"}), "Not Found", None),
        ];
        for (body, message, code) in cases {
            let described = describe_error(&body).unwrap();
            assert_eq!(described.0, message);
            assert_eq!(described.1.as_deref(), code);
        }
        assert_eq!(describe_error(&json!({"ok":true})), None);
    }

    #[test]
    fn errors_are_classified_by_code_status_and_wording() {
        let classify =
            |status: u16, body: Value| error_failure(Some(status), &body, "", None).error.kind();
        assert_eq!(
            classify(
                401,
                json!({"error":{"message":"bad","code":"invalid_api_key"}})
            ),
            ErrorKind::InvalidKey
        );
        // A 400 that is really about the key.
        assert_eq!(
            classify(
                400,
                json!([{"error":{"code":400,"message":"API key not valid. Please pass a valid API key.","status":"INVALID_ARGUMENT"}}])
            ),
            ErrorKind::InvalidKey
        );
        assert_eq!(
            classify(
                404,
                json!({"error":{"message":"The model `x` does not exist","code":"model_not_found"}})
            ),
            ErrorKind::ModelUnavailable
        );
        assert_eq!(
            classify(404, json!({"error":"model \"llama9\" not found"})),
            ErrorKind::ModelUnavailable
        );
        assert_eq!(
            classify(
                429,
                json!({"error":{"message":"You exceeded your current quota","code":"insufficient_quota"}})
            ),
            ErrorKind::RateLimited
        );
        assert_eq!(
            classify(
                400,
                json!({"error":{"message":"too long","code":"context_length_exceeded"}})
            ),
            ErrorKind::InvalidRequest
        );
        assert_eq!(classify(503, json!({})), ErrorKind::Overloaded);
        // A body that is not JSON at all still gives a message.
        let failure = error_failure(Some(502), &Value::Null, "<html>Bad gateway</html>", None);
        assert!(failure.error.to_string().contains("Bad gateway"));
        assert!(failure.retryable);
    }
}
