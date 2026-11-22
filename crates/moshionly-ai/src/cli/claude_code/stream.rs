//! Reads the event stream of `claude -p --output-format stream-json`.
//!
//! Each line is one JSON object. The ones that matter here:
//!
//! - `system` / `init`: the session id and the state of attached MCP servers.
//! - `stream_event`: raw model stream events (text and thinking deltas),
//!   present because the CLI runs with `--include-partial-messages`.
//! - `assistant`: a finished content block of the model's message. Tool calls
//!   are taken from here because the input is complete. Error replies that
//!   the CLI writes itself also arrive as `assistant` lines, marked with
//!   `is_api_error_message`.
//! - `user`: tool results going back to the model.
//! - `result`: the end of the turn, with usage, cost and any error.
//!
//! Lines are read as loose JSON rather than strict structs: the CLI adds
//! fields and event types often, and unknown ones must be ignored.

use std::collections::HashSet;
use std::time::Duration;

use serde_json::Value;

use crate::cli::process::{LineAction, ProcessExit};
use crate::cli::{classify_failure_text, summarize_tool_content};
use crate::error::{AiError, ErrorKind};
use crate::event::{ChatEvent, EventSender, StopReason};
use crate::redact::scrub;
use crate::tool::summarize_text;

/// What to tell someone whose CLI is not signed in.
const SIGN_IN_HINT: &str = "open a terminal, run `claude` and sign in, then try again";

/// Turns stream-json lines into [`ChatEvent`]s.
pub(crate) struct ClaudeStream {
    /// The prefix the CLI puts in front of our tool names, `mcp__<server>__`.
    tool_prefix: Option<String>,
    /// The MCP server that must be connected for the chat to be useful.
    mcp_server: Option<String>,
    /// Model messages whose text arrived as deltas, so the finished message
    /// must not be emitted a second time.
    streamed_messages: HashSet<String>,
    /// Tool calls already reported.
    started_tools: HashSet<String>,
    session_reported: bool,
    /// An error reply the CLI wrote in place of a model message.
    reported_error: Option<AiError>,
    /// When the usage limit resets, if the CLI said the limit was hit.
    limit_resets_in: Option<Duration>,
    /// Set once the `result` line has been read.
    outcome: Option<Result<StopReason, AiError>>,
    /// A reason to stop early.
    fatal: Option<AiError>,
    /// Output that was not JSON, kept to explain a failure.
    noise: Vec<String>,
}

impl ClaudeStream {
    /// A parser for a run with the given MCP server attached, if any.
    pub fn new(mcp_server: Option<&str>) -> Self {
        Self {
            tool_prefix: mcp_server.map(|name| format!("mcp__{name}__")),
            mcp_server: mcp_server.map(str::to_owned),
            streamed_messages: HashSet::new(),
            started_tools: HashSet::new(),
            session_reported: false,
            reported_error: None,
            limit_resets_in: None,
            outcome: None,
            fatal: None,
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
        match value.get("type").and_then(Value::as_str) {
            Some("system") => self.on_system(&value, events),
            Some("stream_event") => self.on_stream_event(&value, events),
            Some("assistant") => self.on_assistant(&value, events),
            Some("user") => self.on_user(&value, events),
            Some("rate_limit_event") => self.on_rate_limit(&value),
            Some("result") => return self.on_result(&value, events),
            _ => {}
        }
        if self.fatal.is_some() {
            LineAction::Abort
        } else {
            LineAction::Continue
        }
    }

    /// The outcome of the run, once the process has ended.
    pub fn finish(self, exit: ProcessExit) -> Result<StopReason, AiError> {
        if let Some(error) = self.fatal {
            return Err(error);
        }
        if let Some(outcome) = self.outcome {
            return outcome;
        }
        match exit {
            ProcessExit::Stopped => Ok(StopReason::Cancelled),
            ProcessExit::Exited { code, stderr } => {
                if let Some(error) = self.reported_error {
                    return Err(error);
                }
                // The CLI ended without a result line: it failed before or
                // outside the conversation. Explain with what it printed.
                let mut detail = stderr;
                if detail.is_empty() {
                    detail = self.noise.join("\n");
                }
                let kind = classify_failure_text(&detail);
                let detail = summarize_text(&detail);
                let message = match (kind, code) {
                    (ErrorKind::NotSignedIn, _) => SIGN_IN_HINT.to_owned(),
                    (_, Some(code)) if !detail.is_empty() => {
                        format!("Claude Code exited with code {code}: {detail}")
                    }
                    (_, Some(code)) => format!("Claude Code exited with code {code}"),
                    (_, None) => "Claude Code was stopped before it answered".to_owned(),
                };
                Err(AiError::from_kind(kind, message))
            }
        }
    }

    fn on_system(&mut self, value: &Value, events: &EventSender) {
        if value.get("subtype").and_then(Value::as_str) != Some("init") {
            return;
        }
        if let Some(session_id) = value.get("session_id").and_then(Value::as_str)
            && !self.session_reported
        {
            self.session_reported = true;
            events.emit(ChatEvent::SessionStarted {
                session_id: session_id.to_owned(),
            });
        }
        let Some(wanted) = &self.mcp_server else {
            return;
        };
        let status = value
            .get("mcp_servers")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|server| server.get("name").and_then(Value::as_str) == Some(wanted))
            .and_then(|server| server.get("status").and_then(Value::as_str));
        // "pending" can still turn into "connected"; only a definite failure
        // is worth stopping for. Without the tools the model can only talk.
        if matches!(status, Some("failed" | "needs-auth" | "disabled")) {
            self.fatal = Some(AiError::other(format!(
                "Claude Code could not connect to the {wanted} tools (status: {})",
                status.unwrap_or("unknown")
            )));
        }
    }

    fn on_stream_event(&mut self, value: &Value, events: &EventSender) {
        if is_from_subagent(value) {
            return;
        }
        let Some(event) = value.get("event") else {
            return;
        };
        match event.get("type").and_then(Value::as_str) {
            Some("message_start") => {
                if let Some(id) = event.pointer("/message/id").and_then(Value::as_str) {
                    self.streamed_messages.insert(id.to_owned());
                }
            }
            Some("content_block_delta") => {
                let delta = event.get("delta");
                let field = |name: &str| {
                    delta
                        .and_then(|delta| delta.get(name))
                        .and_then(Value::as_str)
                        .unwrap_or("")
                };
                match delta.and_then(|d| d.get("type")).and_then(Value::as_str) {
                    Some("text_delta") => events.text(field("text")),
                    Some("thinking_delta") => events.thinking(field("thinking")),
                    // Tool input is taken whole from the `assistant` line.
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn on_assistant(&mut self, value: &Value, events: &EventSender) {
        if is_from_subagent(value) {
            return;
        }
        let message = value.get("message");
        let blocks = message
            .and_then(|message| message.get("content"))
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();

        let is_error_reply = value
            .get("is_api_error_message")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            || value.get("error").is_some_and(|error| !error.is_null());
        if is_error_reply {
            let text: Vec<&str> = blocks
                .iter()
                .filter_map(|block| block.get("text").and_then(Value::as_str))
                .collect();
            self.reported_error = Some(self.error_from(
                value.get("error").and_then(Value::as_str),
                value.get("api_error_status").and_then(Value::as_u64),
                value.get("api_error_type").and_then(Value::as_str),
                &text.join("\n"),
            ));
            return;
        }

        let already_streamed = message
            .and_then(|message| message.get("id"))
            .and_then(Value::as_str)
            .is_some_and(|id| self.streamed_messages.contains(id));
        for block in blocks {
            match block.get("type").and_then(Value::as_str) {
                Some("text") if !already_streamed => {
                    events.text(block.get("text").and_then(Value::as_str).unwrap_or(""));
                }
                Some("thinking") if !already_streamed => {
                    events.thinking(block.get("thinking").and_then(Value::as_str).unwrap_or(""));
                }
                Some("tool_use") => {
                    let id = block.get("id").and_then(Value::as_str).unwrap_or("");
                    if !self.started_tools.insert(id.to_owned()) {
                        continue;
                    }
                    let name = block.get("name").and_then(Value::as_str).unwrap_or("");
                    events.emit(ChatEvent::ToolCallStarted {
                        id: id.to_owned(),
                        name: self.plain_tool_name(name).to_owned(),
                        input: block.get("input").cloned().unwrap_or(Value::Null),
                    });
                }
                _ => {}
            }
        }
    }

    fn on_user(&mut self, value: &Value, events: &EventSender) {
        if is_from_subagent(value) {
            return;
        }
        let blocks = value
            .pointer("/message/content")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        for block in blocks {
            if block.get("type").and_then(Value::as_str) != Some("tool_result") {
                continue;
            }
            let id = block
                .get("tool_use_id")
                .and_then(Value::as_str)
                .unwrap_or("");
            let failed = block
                .get("is_error")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            events.emit(ChatEvent::ToolCallFinished {
                id: id.to_owned(),
                ok: !failed,
                summary: summarize_tool_content(block.get("content"), failed),
            });
        }
    }

    fn on_rate_limit(&mut self, value: &Value) {
        let Some(info) = value.get("rate_limit_info") else {
            return;
        };
        let status = info.get("status").and_then(Value::as_str).unwrap_or("");
        if status == "allowed" || status == "allowed_warning" {
            return;
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or(0);
        self.limit_resets_in = info
            .get("resetsAt")
            .and_then(Value::as_u64)
            .filter(|resets_at| *resets_at > now)
            .map(|resets_at| Duration::from_secs(resets_at - now));
    }

    fn on_result(&mut self, value: &Value, events: &EventSender) -> LineAction {
        if let Some(usage) = value.get("usage") {
            let count = |name: &str| usage.get(name).and_then(Value::as_u64).unwrap_or(0);
            events.emit(ChatEvent::Usage {
                input_tokens: count("input_tokens")
                    + count("cache_creation_input_tokens")
                    + count("cache_read_input_tokens"),
                output_tokens: count("output_tokens"),
                cost_usd: value.get("total_cost_usd").and_then(Value::as_f64),
            });
        }

        let subtype = value.get("subtype").and_then(Value::as_str).unwrap_or("");
        let failed = value
            .get("is_error")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let outcome = if subtype == "error_max_turns" {
            Ok(StopReason::ToolLoopLimit)
        } else if failed || subtype.starts_with("error") {
            Err(self.reported_error.take().unwrap_or_else(|| {
                self.error_from(
                    None,
                    value.get("api_error_status").and_then(Value::as_u64),
                    value.get("api_error_type").and_then(Value::as_str),
                    &result_message(value, subtype),
                )
            }))
        } else {
            Ok(match value.get("stop_reason").and_then(Value::as_str) {
                Some("max_tokens" | "model_context_window_exceeded") => StopReason::MaxTokens,
                Some("refusal") => StopReason::Refusal,
                _ => StopReason::EndTurn,
            })
        };
        self.outcome = Some(outcome);
        LineAction::Finish
    }

    /// Builds the error for a failure the CLI described.
    ///
    /// `code` is the CLI's own label (`authentication_failed`,
    /// `model_not_found`, `rate_limit`, ...), `status` and `error_type` come
    /// from the model API when the request got that far.
    fn error_from(
        &self,
        code: Option<&str>,
        status: Option<u64>,
        error_type: Option<&str>,
        message: &str,
    ) -> AiError {
        let kind = match (code, status, error_type) {
            (Some("authentication_failed"), _, _)
            | (_, Some(401), _)
            | (_, _, Some("authentication_error")) => ErrorKind::NotSignedIn,
            (Some("rate_limit"), _, _) | (_, Some(429), _) | (_, _, Some("rate_limit_error")) => {
                ErrorKind::RateLimited
            }
            (Some("overloaded"), _, _) | (_, Some(529), _) | (_, _, Some("overloaded_error")) => {
                ErrorKind::Overloaded
            }
            (Some("model_not_found"), _, _)
            | (_, Some(404), _)
            | (_, _, Some("not_found_error")) => ErrorKind::ModelUnavailable,
            (Some("invalid_request"), _, _)
            | (_, Some(400 | 413), _)
            | (_, _, Some("invalid_request_error")) => ErrorKind::InvalidRequest,
            _ => classify_failure_text(message),
        };
        let message = scrub(message.trim());
        match kind {
            ErrorKind::NotSignedIn => AiError::NotSignedIn(SIGN_IN_HINT.to_owned()),
            ErrorKind::RateLimited => AiError::RateLimited {
                message,
                retry_after: self.limit_resets_in,
            },
            kind if message.is_empty() => {
                AiError::from_kind(kind, "Claude Code reported an error without a message")
            }
            kind => AiError::from_kind(kind, message),
        }
    }

    /// Removes the `mcp__<server>__` prefix the CLI adds to our tool names.
    fn plain_tool_name<'a>(&self, name: &'a str) -> &'a str {
        self.tool_prefix
            .as_deref()
            .and_then(|prefix| name.strip_prefix(prefix))
            .unwrap_or(name)
    }
}

/// True for lines produced inside a sub-agent. They are not part of the
/// conversation the user sees.
fn is_from_subagent(value: &Value) -> bool {
    value
        .get("parent_tool_use_id")
        .is_some_and(|parent| !parent.is_null())
}

/// The message of a failed `result` line. Error subtypes carry a list of
/// `errors` in place of the `result` text.
fn result_message(value: &Value, subtype: &str) -> String {
    if let Some(text) = value.get("result").and_then(Value::as_str)
        && !text.trim().is_empty()
    {
        return text.to_owned();
    }
    let errors: Vec<&str> = value
        .get("errors")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    if errors.is_empty() {
        format!("Claude Code stopped ({subtype})")
    } else {
        errors.join("; ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::event_channel;
    use serde_json::json;

    const TEXT_REPLY: &str = include_str!("../../../tests/fixtures/claude-code/text_reply.jsonl");
    const TOOL_CALL: &str = include_str!("../../../tests/fixtures/claude-code/tool_call.jsonl");
    const MODEL_NOT_FOUND: &str =
        include_str!("../../../tests/fixtures/claude-code/model_not_found.jsonl");
    const NOT_SIGNED_IN: &str =
        include_str!("../../../tests/fixtures/claude-code/not_signed_in.jsonl");

    /// Feeds recorded output through the parser, as the provider does.
    fn replay(stream: &str, mcp: Option<&str>, exit_code: i32) -> Vec<ChatEvent> {
        let (sender, mut receiver) = event_channel();
        let mut parser = ClaudeStream::new(mcp);
        let mut finished = false;
        for line in stream.lines() {
            match parser.push_line(line, &sender) {
                LineAction::Continue => {}
                LineAction::Finish | LineAction::Abort => {
                    finished = true;
                    break;
                }
            }
        }
        assert!(finished, "the stream should end with a result line");
        sender.finish(parser.finish(ProcessExit::Exited {
            code: Some(exit_code),
            stderr: String::new(),
        }));
        let mut events = Vec::new();
        while let Ok(event) = receiver.try_recv() {
            events.push(event);
        }
        events
    }

    fn exited(code: i32, stderr: &str) -> ProcessExit {
        ProcessExit::Exited {
            code: Some(code),
            stderr: stderr.to_owned(),
        }
    }

    #[test]
    fn a_recorded_text_reply_becomes_session_text_usage_finished() {
        let events = replay(TEXT_REPLY, Some("moshionly"), 0);
        assert_eq!(
            events,
            vec![
                ChatEvent::SessionStarted {
                    session_id: "00000000-0000-4000-8000-000000000001".to_owned()
                },
                ChatEvent::TextDelta {
                    text: "ready".to_owned()
                },
                ChatEvent::Usage {
                    input_tokens: 4073,
                    output_tokens: 4,
                    cost_usd: Some(0.0327941),
                },
                ChatEvent::Finished {
                    stop_reason: StopReason::EndTurn
                },
            ]
        );
    }

    #[test]
    fn a_recorded_tool_call_reports_start_and_finish_with_plain_names() {
        let events = replay(TOOL_CALL, Some("moshionly"), 0);
        let kinds: Vec<&str> = events
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
            .collect();
        assert_eq!(
            kinds,
            [
                "session",
                "tool_started",
                "tool_finished",
                "text",
                "usage",
                "finished"
            ]
        );
        assert_eq!(
            events[1],
            ChatEvent::ToolCallStarted {
                id: "toolu_01".to_owned(),
                name: "echo".to_owned(),
                input: json!({ "text": "ping" }),
            }
        );
        assert_eq!(
            events[2],
            ChatEvent::ToolCallFinished {
                id: "toolu_01".to_owned(),
                ok: true,
                summary: "echo: ping".to_owned(),
            }
        );
        assert_eq!(
            events[3],
            ChatEvent::TextDelta {
                text: "done".to_owned()
            }
        );
    }

    #[test]
    fn a_recorded_unknown_model_is_a_model_unavailable_error() {
        let events = replay(MODEL_NOT_FOUND, Some("moshionly"), 1);
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, ChatEvent::TextDelta { .. })),
            "the CLI's error text must not be shown as an answer"
        );
        let error = events
            .iter()
            .find_map(|event| match event {
                ChatEvent::Error { message, kind } => Some((message.as_str(), *kind)),
                _ => None,
            })
            .expect("an error event");
        assert_eq!(error.1, ErrorKind::ModelUnavailable);
        assert!(error.0.contains("moshionly-no-such-model"));
        assert_eq!(
            events.last(),
            Some(&ChatEvent::Finished {
                stop_reason: StopReason::Error
            })
        );
    }

    #[test]
    fn a_recorded_signed_out_run_is_a_not_signed_in_error() {
        let events = replay(NOT_SIGNED_IN, None, 1);
        let error = events
            .iter()
            .find_map(|event| match event {
                ChatEvent::Error { message, kind } => Some((message.as_str(), *kind)),
                _ => None,
            })
            .expect("an error event");
        assert_eq!(error.1, ErrorKind::NotSignedIn);
        assert!(error.0.contains("sign in"));
    }

    #[test]
    fn text_is_taken_from_the_finished_message_when_there_were_no_deltas() {
        // Output of a CLI that does not stream partial messages.
        let stream = [
            json!({"type":"system","subtype":"init","session_id":"s1","mcp_servers":[]}),
            json!({"type":"assistant","parent_tool_use_id":null,"message":{"id":"msg_1","content":[
                {"type":"thinking","thinking":"hm"},
                {"type":"text","text":"Hello"}]}}),
            json!({"type":"result","subtype":"success","is_error":false,"stop_reason":"end_turn",
                "usage":{"input_tokens":1,"output_tokens":2}}),
        ]
        .map(|line| line.to_string())
        .join("\n");
        let events = replay(&stream, None, 0);
        assert_eq!(
            events[1..3],
            [
                ChatEvent::ThinkingDelta {
                    text: "hm".to_owned()
                },
                ChatEvent::TextDelta {
                    text: "Hello".to_owned()
                },
            ]
        );
        assert_eq!(
            events[3],
            ChatEvent::Usage {
                input_tokens: 1,
                output_tokens: 2,
                cost_usd: None
            }
        );
    }

    #[test]
    fn thinking_deltas_are_forwarded() {
        let (sender, mut receiver) = event_channel();
        let mut parser = ClaudeStream::new(None);
        let line = json!({"type":"stream_event","parent_tool_use_id":null,"event":{
            "type":"content_block_delta","index":0,
            "delta":{"type":"thinking_delta","thinking":"Considering"}}});
        parser.push_line(&line.to_string(), &sender);
        assert_eq!(
            receiver.try_recv().unwrap(),
            ChatEvent::ThinkingDelta {
                text: "Considering".to_owned()
            }
        );
    }

    #[test]
    fn output_from_sub_agents_is_not_part_of_the_answer() {
        let (sender, mut receiver) = event_channel();
        let mut parser = ClaudeStream::new(None);
        let line = json!({"type":"assistant","parent_tool_use_id":"toolu_9","message":{
            "id":"msg_2","content":[{"type":"text","text":"inner"}]}});
        parser.push_line(&line.to_string(), &sender);
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn a_failed_tool_result_is_reported_as_not_ok() {
        let (sender, mut receiver) = event_channel();
        let mut parser = ClaudeStream::new(Some("moshionly"));
        let line = json!({"type":"user","parent_tool_use_id":null,"message":{"role":"user","content":[
            {"type":"tool_result","tool_use_id":"toolu_1","is_error":true,
             "content":"node cd34 does not exist\nfix the id"}]}});
        parser.push_line(&line.to_string(), &sender);
        assert_eq!(
            receiver.try_recv().unwrap(),
            ChatEvent::ToolCallFinished {
                id: "toolu_1".to_owned(),
                ok: false,
                summary: "node cd34 does not exist".to_owned(),
            }
        );
    }

    #[test]
    fn a_failed_mcp_connection_aborts_the_run() {
        let (sender, _receiver) = event_channel();
        let mut parser = ClaudeStream::new(Some("moshionly"));
        let line = json!({"type":"system","subtype":"init","session_id":"s1",
            "mcp_servers":[{"name":"moshionly","status":"failed"}]});
        assert_eq!(
            parser.push_line(&line.to_string(), &sender),
            LineAction::Abort
        );
        let error = parser.finish(ProcessExit::Stopped).unwrap_err();
        assert!(error.to_string().contains("moshionly tools"));
    }

    #[test]
    fn a_pending_mcp_connection_is_given_time() {
        let (sender, _receiver) = event_channel();
        let mut parser = ClaudeStream::new(Some("moshionly"));
        let line = json!({"type":"system","subtype":"init","session_id":"s1",
            "mcp_servers":[{"name":"moshionly","status":"pending"}]});
        assert_eq!(
            parser.push_line(&line.to_string(), &sender),
            LineAction::Continue
        );
    }

    #[test]
    fn a_rate_limit_carries_the_reset_time() {
        let (sender, _receiver) = event_channel();
        let mut parser = ClaudeStream::new(None);
        let resets_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 3600;
        let lines = [
            json!({"type":"rate_limit_event","rate_limit_info":{"status":"rejected","resetsAt":resets_at}}),
            json!({"type":"assistant","parent_tool_use_id":null,"error":"rate_limit",
                "is_api_error_message":true,"api_error_status":429,
                "message":{"id":"x","content":[{"type":"text","text":"You've hit your limit"}]}}),
            json!({"type":"result","subtype":"success","is_error":true,"api_error_status":429,
                "result":"You've hit your limit"}),
        ];
        for line in lines {
            parser.push_line(&line.to_string(), &sender);
        }
        match parser.finish(exited(1, "")).unwrap_err() {
            AiError::RateLimited {
                message,
                retry_after,
            } => {
                assert!(message.contains("limit"));
                let wait = retry_after.expect("a reset time").as_secs();
                assert!((3500..=3600).contains(&wait));
            }
            other => panic!("expected a rate limit, got {other:?}"),
        }
    }

    #[test]
    fn api_statuses_map_to_error_kinds() {
        let parser = ClaudeStream::new(None);
        let cases = [
            (Some(401), None, ErrorKind::NotSignedIn),
            (Some(429), None, ErrorKind::RateLimited),
            (Some(529), None, ErrorKind::Overloaded),
            (None, Some("overloaded_error"), ErrorKind::Overloaded),
            (
                Some(404),
                Some("not_found_error"),
                ErrorKind::ModelUnavailable,
            ),
            (
                Some(400),
                Some("invalid_request_error"),
                ErrorKind::InvalidRequest,
            ),
            (Some(500), Some("api_error"), ErrorKind::Other),
        ];
        for (status, error_type, expected) in cases {
            let error = parser.error_from(None, status, error_type, "something went wrong");
            assert_eq!(error.kind(), expected, "{status:?} {error_type:?}");
        }
    }

    #[test]
    fn hitting_the_turn_limit_is_a_stop_reason_not_an_error() {
        let (sender, _receiver) = event_channel();
        let mut parser = ClaudeStream::new(None);
        let line = json!({"type":"result","subtype":"error_max_turns","is_error":true});
        assert_eq!(
            parser.push_line(&line.to_string(), &sender),
            LineAction::Finish
        );
        assert_eq!(parser.finish(exited(1, "")), Ok(StopReason::ToolLoopLimit));
    }

    #[test]
    fn an_execution_error_uses_the_errors_list() {
        let (sender, _receiver) = event_channel();
        let mut parser = ClaudeStream::new(None);
        let line = json!({"type":"result","subtype":"error_during_execution","is_error":true,
            "errors":["first problem","second problem"]});
        parser.push_line(&line.to_string(), &sender);
        let error = parser.finish(exited(1, "")).unwrap_err();
        assert!(error.to_string().contains("first problem; second problem"));
    }

    #[test]
    fn a_crash_without_a_result_reports_what_the_cli_printed() {
        let (sender, _receiver) = event_channel();
        let mut parser = ClaudeStream::new(None);
        parser.push_line("error: unknown option '--frobnicate'", &sender);
        let error = parser.finish(exited(1, "")).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Other);
        assert!(error.to_string().contains("exited with code 1"));
        assert!(error.to_string().contains("--frobnicate"));
    }

    #[test]
    fn a_stopped_run_is_cancelled() {
        let parser = ClaudeStream::new(None);
        assert_eq!(
            parser.finish(ProcessExit::Stopped),
            Ok(StopReason::Cancelled)
        );
    }

    #[test]
    fn a_max_tokens_stop_is_reported() {
        let (sender, _receiver) = event_channel();
        let mut parser = ClaudeStream::new(None);
        let line = json!({"type":"result","subtype":"success","is_error":false,
            "stop_reason":"max_tokens"});
        parser.push_line(&line.to_string(), &sender);
        assert_eq!(parser.finish(exited(0, "")), Ok(StopReason::MaxTokens));
    }
}
