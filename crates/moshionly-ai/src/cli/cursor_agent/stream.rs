//! Reads the event stream of `cursor-agent --print --output-format
//! stream-json --stream-partial-output`.
//!
//! UNVERIFIED against a signed-in CLI: no Cursor account was available when
//! this was written. The shapes below follow Cursor's published output format
//! reference and the field names used by the installed CLI (2026.08.11). The
//! failure path (signed out) is real.
//!
//! The events, one JSON object per line:
//!
//! - `system` / `init`: the session id.
//! - `thinking` with subtype `delta`: the model's reasoning, in `text`.
//! - `assistant`: text. With partial output on, the same text arrives up to
//!   three times; only lines that have `timestamp_ms` and no `model_call_id`
//!   are new text, the others repeat it.
//! - `tool_call` with subtype `started` or `completed`, correlated by
//!   `call_id`. The call itself sits under a key named after the tool kind.
//! - `result`: the end of a successful turn, with token counts in `usage`.
//!
//! A failed run has no `result` line: the CLI exits non-zero and explains
//! itself on stderr.

use std::collections::HashSet;

use serde_json::Value;

use crate::cli::classify_failure_text;
use crate::cli::process::{LineAction, ProcessExit};
use crate::error::{AiError, ErrorKind};
use crate::event::{ChatEvent, EventSender, StopReason};
use crate::tool::summarize_text;

/// What to tell someone whose CLI is not signed in.
pub(super) const SIGN_IN_HINT: &str =
    "open a terminal, run `cursor-agent login` and sign in, then try again";

/// Turns stream-json lines into [`ChatEvent`]s.
pub(crate) struct CursorStream {
    /// The MCP server whose name is removed from tool names.
    mcp_server: Option<String>,
    session_reported: bool,
    /// True once a streaming delta was seen, which makes every other
    /// `assistant` line a repeat.
    saw_deltas: bool,
    started_tools: HashSet<String>,
    outcome: Option<Result<StopReason, AiError>>,
    noise: Vec<String>,
}

impl CursorStream {
    /// A parser for a run with the given MCP server attached, if any.
    pub fn new(mcp_server: Option<&str>) -> Self {
        Self {
            mcp_server: mcp_server.map(str::to_owned),
            session_reported: false,
            saw_deltas: false,
            started_tools: HashSet::new(),
            outcome: None,
            noise: Vec::new(),
        }
    }

    /// Handles one line of output.
    pub fn push_line(&mut self, line: &str, events: &EventSender) -> LineAction {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            if self.noise.len() < 20 {
                self.noise.push(line.to_owned());
            }
            return LineAction::Continue;
        };
        self.report_session(&value, events);
        match value.get("type").and_then(Value::as_str) {
            Some("assistant") => self.on_assistant(&value, events),
            Some("thinking") => on_thinking(&value, events),
            Some("tool_call") => self.on_tool_call(&value, events),
            Some("result") => return self.on_result(&value, events),
            _ => {}
        }
        LineAction::Continue
    }

    /// The outcome of the run, once the process has ended.
    pub fn finish(self, exit: ProcessExit) -> Result<StopReason, AiError> {
        if let Some(outcome) = self.outcome {
            return outcome;
        }
        match exit {
            ProcessExit::Stopped => Ok(StopReason::Cancelled),
            ProcessExit::Exited { code, stderr } => {
                let mut detail = stderr;
                if detail.is_empty() {
                    detail = self.noise.join("\n");
                }
                Err(failure(code, &detail))
            }
        }
    }

    fn report_session(&mut self, value: &Value, events: &EventSender) {
        if self.session_reported {
            return;
        }
        if let Some(session_id) = value.get("session_id").and_then(Value::as_str) {
            self.session_reported = true;
            events.emit(ChatEvent::SessionStarted {
                session_id: session_id.to_owned(),
            });
        }
    }

    fn on_assistant(&mut self, value: &Value, events: &EventSender) {
        let is_delta = value.get("timestamp_ms").is_some() && value.get("model_call_id").is_none();
        if is_delta {
            self.saw_deltas = true;
        } else if self.saw_deltas {
            // A flush that repeats text already delivered as deltas.
            return;
        }
        let blocks = value
            .pointer("/message/content")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        for block in blocks {
            if let Some(text) = block.get("text").and_then(Value::as_str) {
                events.text(text);
            }
        }
    }

    fn on_tool_call(&mut self, value: &Value, events: &EventSender) {
        let id = value
            .get("call_id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let Some((kind, call)) = value
            .get("tool_call")
            .and_then(Value::as_object)
            .and_then(|object| object.iter().next())
        else {
            return;
        };
        match value.get("subtype").and_then(Value::as_str) {
            Some("started") => {
                if self.started_tools.insert(id.clone()) {
                    let (name, input) = self.describe_call(kind, call);
                    events.emit(ChatEvent::ToolCallStarted { id, name, input });
                }
            }
            Some("completed") => {
                // A completion without a start still has to be a pair.
                if self.started_tools.insert(id.clone()) {
                    let (name, input) = self.describe_call(kind, call);
                    events.emit(ChatEvent::ToolCallStarted {
                        id: id.clone(),
                        name,
                        input,
                    });
                }
                let (ok, summary) = describe_result(call.get("result"));
                events.emit(ChatEvent::ToolCallFinished { id, ok, summary });
            }
            _ => {}
        }
    }

    fn on_result(&mut self, value: &Value, events: &EventSender) -> LineAction {
        if let Some(usage) = value.get("usage") {
            let count = |names: &[&str]| {
                names
                    .iter()
                    .find_map(|name| usage.get(*name).and_then(Value::as_u64))
                    .unwrap_or(0)
            };
            events.emit(ChatEvent::Usage {
                input_tokens: count(&["inputTokens", "input_tokens"])
                    + count(&["cacheReadTokens", "cache_read_tokens"])
                    + count(&["cacheWriteTokens", "cache_write_tokens"]),
                output_tokens: count(&["outputTokens", "output_tokens"]),
                cost_usd: None,
            });
        }
        let failed = value
            .get("is_error")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        self.outcome = Some(if failed {
            let message = value.get("result").and_then(Value::as_str).unwrap_or("");
            Err(failure(None, message))
        } else {
            Ok(StopReason::EndTurn)
        });
        LineAction::Finish
    }

    /// The tool name and arguments of a call.
    ///
    /// `kind` is the key the CLI files the call under: `mcpToolCall` for MCP
    /// tools, `function` for generic ones, otherwise the built-in tool's name
    /// with a `ToolCall` suffix (`readToolCall`, `shellToolCall`, ...).
    fn describe_call(&self, kind: &str, call: &Value) -> (String, Value) {
        let args = call.get("args").cloned().unwrap_or(Value::Null);
        match kind {
            "mcpToolCall" => {
                let text = |name: &str| args.get(name).and_then(Value::as_str);
                let name = text("toolName")
                    .or_else(|| text("name"))
                    .unwrap_or("mcp_tool");
                let input = args.get("args").cloned().unwrap_or(Value::Null);
                (self.plain_tool_name(name).to_owned(), input)
            }
            "function" => {
                let name = call
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("function");
                let input = match call.get("arguments") {
                    Some(Value::String(json)) => serde_json::from_str(json)
                        .unwrap_or_else(|_| Value::String(json.to_owned())),
                    Some(other) => other.clone(),
                    None => Value::Null,
                };
                (self.plain_tool_name(name).to_owned(), input)
            }
            other => (
                other.strip_suffix("ToolCall").unwrap_or(other).to_owned(),
                args,
            ),
        }
    }

    /// Removes the server name the CLI may put in front of an MCP tool name
    /// (`moshionly-echo`, `moshionly__echo`, `mcp__moshionly__echo`).
    fn plain_tool_name<'a>(&self, name: &'a str) -> &'a str {
        let Some(server) = &self.mcp_server else {
            return name;
        };
        let name = name.strip_prefix("mcp__").unwrap_or(name);
        name.strip_prefix(server.as_str())
            .and_then(|rest| {
                rest.strip_prefix("__")
                    .or_else(|| rest.strip_prefix('-'))
                    .or_else(|| rest.strip_prefix('_'))
                    .or_else(|| rest.strip_prefix(':'))
            })
            .filter(|rest| !rest.is_empty())
            .unwrap_or(name)
    }
}

/// Forwards a reasoning delta. The `completed` subtype carries no text.
fn on_thinking(value: &Value, events: &EventSender) {
    if value.get("subtype").and_then(Value::as_str) == Some("delta") {
        events.thinking(value.get("text").and_then(Value::as_str).unwrap_or(""));
    }
}

/// The error for a run that ended without a successful result.
fn failure(code: Option<i32>, detail: &str) -> AiError {
    let kind = classify_failure_text(detail);
    let detail = summarize_text(detail);
    let detail = detail.strip_prefix("Error: ").unwrap_or(&detail);
    let message = match (kind, code) {
        (ErrorKind::NotSignedIn, _) => SIGN_IN_HINT.to_owned(),
        (_, _) if !detail.is_empty() => detail.to_owned(),
        (_, Some(code)) => format!("Cursor Agent exited with code {code}"),
        (_, None) => "Cursor Agent stopped without an answer".to_owned(),
    };
    AiError::from_kind(kind, message)
}

/// Whether a finished tool call succeeded, and one line about its result.
///
/// The result is an object with one key naming the outcome: `success`, or
/// something else (`error`, `rejected`, `failure`) when it did not work.
fn describe_result(result: Option<&Value>) -> (bool, String) {
    let Some((outcome, body)) = result
        .and_then(Value::as_object)
        .and_then(|object| object.iter().next())
    else {
        return (true, "ok".to_owned());
    };
    let tool_said_error = body
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let ok = outcome == "success" && !tool_said_error;
    let mut summary = summarize_text(&first_text(body).unwrap_or_default());
    if summary.is_empty() {
        summary = if ok {
            "ok".to_owned()
        } else {
            outcome.to_owned()
        };
    }
    (ok, summary)
}

/// The first piece of human-readable text inside a tool result, wherever the
/// CLI put it.
fn first_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) if !text.trim().is_empty() => Some(text.clone()),
        Value::Array(items) => items.iter().find_map(first_text),
        Value::Object(object) => ["text", "content", "message", "reason", "error", "output"]
            .iter()
            .find_map(|key| object.get(*key).and_then(first_text)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::event_channel;
    use serde_json::json;

    const DOCUMENTED_TOOL_CALL: &str =
        include_str!("../../../tests/fixtures/cursor-agent/documented_tool_call.jsonl");
    const SIGNED_OUT_STDERR: &str =
        include_str!("../../../tests/fixtures/cursor-agent/not_signed_in.stderr.txt");

    fn replay(stream: &str, mcp: Option<&str>) -> Vec<ChatEvent> {
        let (sender, mut receiver) = event_channel();
        let mut parser = CursorStream::new(mcp);
        for line in stream.lines() {
            if parser.push_line(line, &sender) == LineAction::Finish {
                break;
            }
        }
        sender.finish(parser.finish(ProcessExit::Exited {
            code: Some(0),
            stderr: String::new(),
        }));
        let mut events = Vec::new();
        while let Ok(event) = receiver.try_recv() {
            events.push(event);
        }
        events
    }

    #[test]
    fn a_documented_turn_with_an_mcp_call_maps_to_events() {
        let events = replay(DOCUMENTED_TOOL_CALL, Some("moshionly"));
        assert_eq!(
            events,
            vec![
                ChatEvent::SessionStarted {
                    session_id: "00000000-0000-4000-8000-000000000001".to_owned()
                },
                ChatEvent::ThinkingDelta {
                    text: "The user wants an echo.".to_owned()
                },
                ChatEvent::TextDelta {
                    text: "Calling ".to_owned()
                },
                ChatEvent::TextDelta {
                    text: "the tool.".to_owned()
                },
                ChatEvent::ToolCallStarted {
                    id: "call_01".to_owned(),
                    name: "echo".to_owned(),
                    input: json!({ "text": "ping" }),
                },
                ChatEvent::ToolCallFinished {
                    id: "call_01".to_owned(),
                    ok: true,
                    summary: "echo: ping".to_owned(),
                },
                ChatEvent::TextDelta {
                    text: "done".to_owned()
                },
                ChatEvent::Usage {
                    input_tokens: 1500,
                    output_tokens: 25,
                    cost_usd: None,
                },
                ChatEvent::Finished {
                    stop_reason: StopReason::EndTurn
                },
            ]
        );
    }

    #[test]
    fn whole_messages_are_used_when_the_cli_does_not_stream_deltas() {
        let stream = [
            json!({"type":"system","subtype":"init","session_id":"s1"}),
            json!({"type":"assistant","session_id":"s1","message":{"role":"assistant",
                "content":[{"type":"text","text":"Hello there"}]}}),
            json!({"type":"result","subtype":"success","is_error":false,"result":"Hello there",
                "session_id":"s1"}),
        ]
        .map(|line| line.to_string())
        .join("\n");
        let events = replay(&stream, None);
        assert_eq!(
            events[1],
            ChatEvent::TextDelta {
                text: "Hello there".to_owned()
            }
        );
        assert_eq!(events.len(), 3);
    }

    #[test]
    fn the_real_signed_out_message_is_a_not_signed_in_error() {
        let parser = CursorStream::new(None);
        let error = parser
            .finish(ProcessExit::Exited {
                code: Some(1),
                stderr: SIGNED_OUT_STDERR.trim().to_owned(),
            })
            .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::NotSignedIn);
        assert!(error.to_string().contains("cursor-agent login"));
    }

    #[test]
    fn other_failures_report_stderr_without_the_error_prefix() {
        let parser = CursorStream::new(None);
        let error = parser
            .finish(ProcessExit::Exited {
                code: Some(1),
                stderr: "Error: Named models unavailable on the free plan".to_owned(),
            })
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "Named models unavailable on the free plan"
        );
    }

    #[test]
    fn a_silent_failure_reports_the_exit_code() {
        let parser = CursorStream::new(None);
        let error = parser
            .finish(ProcessExit::Exited {
                code: Some(7),
                stderr: String::new(),
            })
            .unwrap_err();
        assert!(error.to_string().contains("code 7"));
    }

    #[test]
    fn a_stopped_run_is_cancelled() {
        let parser = CursorStream::new(None);
        assert_eq!(
            parser.finish(ProcessExit::Stopped),
            Ok(StopReason::Cancelled)
        );
    }

    #[test]
    fn built_in_and_function_calls_are_named_sensibly() {
        let parser = CursorStream::new(Some("moshionly"));
        assert_eq!(
            parser.describe_call("readToolCall", &json!({"args":{"path":"a.txt"}})),
            ("read".to_owned(), json!({"path":"a.txt"}))
        );
        assert_eq!(
            parser.describe_call(
                "function",
                &json!({"name":"moshionly-apply_edits","arguments":"{\"edits\":[]}"})
            ),
            ("apply_edits".to_owned(), json!({"edits":[]}))
        );
        assert_eq!(
            parser.describe_call(
                "mcpToolCall",
                &json!({"args":{"name":"moshionly-echo","toolName":"echo","args":{"text":"x"}}})
            ),
            ("echo".to_owned(), json!({"text":"x"}))
        );
    }

    #[test]
    fn server_prefixes_are_removed_in_every_spelling() {
        let parser = CursorStream::new(Some("moshionly"));
        for name in [
            "echo",
            "moshionly-echo",
            "moshionly__echo",
            "moshionly_echo",
            "mcp__moshionly__echo",
        ] {
            assert_eq!(parser.plain_tool_name(name), "echo", "{name}");
        }
        assert_eq!(parser.plain_tool_name("moshionly"), "moshionly");
        assert_eq!(parser.plain_tool_name("other-echo"), "other-echo");
    }

    #[test]
    fn rejected_and_failed_tool_calls_are_not_ok() {
        let rejected = json!({"rejected":{"reason":"blocked by permissions"}});
        assert_eq!(
            describe_result(Some(&rejected)),
            (false, "blocked by permissions".to_owned())
        );
        let tool_error = json!({"success":{"isError":true,
            "content":[{"text":{"text":"node cd34 does not exist"}}]}});
        assert_eq!(
            describe_result(Some(&tool_error)),
            (false, "node cd34 does not exist".to_owned())
        );
        assert_eq!(
            describe_result(Some(&json!({"error":{}}))),
            (false, "error".to_owned())
        );
        assert_eq!(describe_result(None), (true, "ok".to_owned()));
    }

    #[test]
    fn a_completion_without_a_start_still_produces_a_pair() {
        let (sender, mut receiver) = event_channel();
        let mut parser = CursorStream::new(None);
        let line = json!({"type":"tool_call","subtype":"completed","call_id":"c9",
            "tool_call":{"readToolCall":{"args":{"path":"a"},"result":{"success":{"content":"x"}}}}});
        parser.push_line(&line.to_string(), &sender);
        assert!(matches!(
            receiver.try_recv().unwrap(),
            ChatEvent::ToolCallStarted { .. }
        ));
        assert!(matches!(
            receiver.try_recv().unwrap(),
            ChatEvent::ToolCallFinished { ok: true, .. }
        ));
    }

    #[test]
    fn usage_is_reported_when_the_result_carries_it() {
        let (sender, mut receiver) = event_channel();
        let mut parser = CursorStream::new(None);
        let line = json!({"type":"result","subtype":"success","is_error":false,"result":"x",
            "usage":{"inputTokens":10,"outputTokens":3,"cacheReadTokens":5}});
        parser.push_line(&line.to_string(), &sender);
        assert_eq!(
            receiver.try_recv().unwrap(),
            ChatEvent::Usage {
                input_tokens: 15,
                output_tokens: 3,
                cost_usd: None
            }
        );
    }
}
