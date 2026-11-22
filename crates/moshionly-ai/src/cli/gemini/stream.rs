//! Reads the event stream of `gemini --output-format stream-json`.
//!
//! UNVERIFIED: the Gemini CLI was not installed when this was written. The
//! shapes follow the stream event types and the headless mode reference that
//! Google publishes for the CLI.
//!
//! The events, one JSON object per line:
//!
//! - `init`: the session id and model.
//! - `message`: a piece of the conversation. Only `assistant` messages are
//!   new text; `user` messages repeat the prompt.
//! - `tool_use` and `tool_result`, correlated by `tool_id`.
//! - `error`: a warning or an error that did not end the run.
//! - `result`: the end of the turn, with token counts in `stats`.
//!
//! Exit code 53 means the CLI's own turn limit was reached.

use serde_json::Value;

use crate::cli::process::{LineAction, ProcessExit};
use crate::cli::{classify_failure_text, exit_failure};
use crate::error::{AiError, ErrorKind};
use crate::event::{ChatEvent, EventSender, StopReason};
use crate::redact::scrub;
use crate::tool::summarize_text;

/// What to tell someone whose CLI is not signed in.
pub(super) const SIGN_IN_HINT: &str = "open a terminal, run `gemini` and sign in, then try again";

/// The exit code the CLI uses when it stops at its turn limit.
const EXIT_TURN_LIMIT: i32 = 53;

/// Turns stream-json lines into [`ChatEvent`]s.
pub(crate) struct GeminiStream {
    /// The MCP server whose name is removed from tool names.
    mcp_server: Option<String>,
    session_reported: bool,
    /// The most recent error the stream mentioned, kept to explain a failure.
    last_error: Option<String>,
    outcome: Option<Result<StopReason, AiError>>,
    noise: Vec<String>,
}

impl GeminiStream {
    /// A parser for a run with the given MCP server attached, if any.
    pub fn new(mcp_server: Option<&str>) -> Self {
        Self {
            mcp_server: mcp_server.map(str::to_owned),
            session_reported: false,
            last_error: None,
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
        let text = |name: &str| value.get(name).and_then(Value::as_str).unwrap_or("");
        match value.get("type").and_then(Value::as_str) {
            Some("init") => {
                let session_id = text("session_id");
                if !session_id.is_empty() && !self.session_reported {
                    self.session_reported = true;
                    events.emit(ChatEvent::SessionStarted {
                        session_id: session_id.to_owned(),
                    });
                }
            }
            Some("message") if text("role") == "assistant" => events.text(text("content")),
            Some("tool_use") => {
                events.emit(ChatEvent::ToolCallStarted {
                    id: text("tool_id").to_owned(),
                    name: self.plain_tool_name(text("tool_name")).to_owned(),
                    input: value.get("parameters").cloned().unwrap_or(Value::Null),
                });
            }
            Some("tool_result") => {
                let ok = text("status") == "success";
                let detail = value
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .filter(|_| !ok)
                    .unwrap_or_else(|| text("output"));
                let mut summary = summarize_text(detail);
                if summary.is_empty() {
                    summary = if ok { "ok" } else { "failed" }.to_owned();
                }
                events.emit(ChatEvent::ToolCallFinished {
                    id: text("tool_id").to_owned(),
                    ok,
                    summary,
                });
            }
            Some("error") if text("severity") == "error" => {
                self.last_error = Some(text("message").to_owned());
            }
            Some("result") => {
                self.on_result(&value, events);
                return LineAction::Finish;
            }
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
            ProcessExit::Exited {
                code: Some(EXIT_TURN_LIMIT),
                ..
            } => Ok(StopReason::ToolLoopLimit),
            ProcessExit::Exited { code, stderr } => {
                let detail = match self.last_error {
                    Some(message) => message,
                    None if !stderr.is_empty() => stderr,
                    None => self.noise.join("\n"),
                };
                Err(exit_failure("Gemini CLI", SIGN_IN_HINT, code, &detail))
            }
        }
    }

    fn on_result(&mut self, value: &Value, events: &EventSender) {
        if let Some(stats) = value.get("stats") {
            let count = |name: &str| stats.get(name).and_then(Value::as_u64).unwrap_or(0);
            events.emit(ChatEvent::Usage {
                // Cached tokens are already part of `input_tokens`.
                input_tokens: count("input_tokens"),
                output_tokens: count("output_tokens"),
                cost_usd: None,
            });
        }
        let succeeded = value.get("status").and_then(Value::as_str) == Some("success");
        self.outcome = Some(if succeeded {
            Ok(StopReason::EndTurn)
        } else {
            let message = value
                .pointer("/error/message")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| self.last_error.clone())
                .unwrap_or_default();
            Err(turn_error(&message))
        });
    }

    /// Removes the server name the CLI puts in front of an MCP tool name:
    /// `mcp_<server>_<tool>`, or `<server>__<tool>` in older releases.
    fn plain_tool_name<'a>(&self, name: &'a str) -> &'a str {
        let Some(server) = &self.mcp_server else {
            return name;
        };
        let qualified = name
            .strip_prefix("mcp_")
            .and_then(|rest| rest.strip_prefix(server.as_str()))
            .and_then(|rest| rest.strip_prefix('_'));
        let legacy = name
            .strip_prefix(server.as_str())
            .and_then(|rest| rest.strip_prefix("__"));
        qualified
            .or(legacy)
            .filter(|rest| !rest.is_empty())
            .unwrap_or(name)
    }
}

/// The error for a turn the CLI reported as failed.
fn turn_error(message: &str) -> AiError {
    match classify_failure_text(message) {
        ErrorKind::NotSignedIn => AiError::NotSignedIn(SIGN_IN_HINT.to_owned()),
        kind if message.trim().is_empty() => {
            AiError::from_kind(kind, "Gemini CLI reported a failure without a message")
        }
        kind => AiError::from_kind(kind, summarize_text(&scrub(message))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::event_channel;
    use serde_json::json;

    const DOCUMENTED_TOOL_CALL: &str =
        include_str!("../../../tests/fixtures/gemini/documented_tool_call.jsonl");

    fn replay(stream: &str, mcp: Option<&str>, exit_code: i32, stderr: &str) -> Vec<ChatEvent> {
        let (sender, mut receiver) = event_channel();
        let mut parser = GeminiStream::new(mcp);
        for line in stream.lines() {
            if parser.push_line(line, &sender) == LineAction::Finish {
                break;
            }
        }
        sender.finish(parser.finish(ProcessExit::Exited {
            code: Some(exit_code),
            stderr: stderr.to_owned(),
        }));
        let mut events = Vec::new();
        while let Ok(event) = receiver.try_recv() {
            events.push(event);
        }
        events
    }

    #[test]
    fn a_documented_turn_with_an_mcp_call_maps_to_events() {
        let events = replay(DOCUMENTED_TOOL_CALL, Some("moshionly"), 0, "");
        assert_eq!(
            events,
            vec![
                ChatEvent::SessionStarted {
                    session_id: "00000000-0000-4000-8000-000000000001".to_owned()
                },
                ChatEvent::ToolCallStarted {
                    id: "mcp_moshionly_echo-1795400000000-0".to_owned(),
                    name: "echo".to_owned(),
                    input: json!({ "text": "ping" }),
                },
                ChatEvent::ToolCallFinished {
                    id: "mcp_moshionly_echo-1795400000000-0".to_owned(),
                    ok: true,
                    summary: "echo: ping".to_owned(),
                },
                ChatEvent::TextDelta {
                    text: "do".to_owned()
                },
                ChatEvent::TextDelta {
                    text: "ne".to_owned()
                },
                ChatEvent::Usage {
                    input_tokens: 8200,
                    output_tokens: 100,
                    cost_usd: None,
                },
                ChatEvent::Finished {
                    stop_reason: StopReason::EndTurn
                },
            ]
        );
    }

    #[test]
    fn server_prefixes_are_removed_in_both_spellings() {
        let parser = GeminiStream::new(Some("moshionly"));
        assert_eq!(
            parser.plain_tool_name("mcp_moshionly_apply_edits"),
            "apply_edits"
        );
        assert_eq!(
            parser.plain_tool_name("moshionly__apply_edits"),
            "apply_edits"
        );
        assert_eq!(parser.plain_tool_name("apply_edits"), "apply_edits");
        assert_eq!(parser.plain_tool_name("mcp_other_echo"), "mcp_other_echo");
        assert_eq!(parser.plain_tool_name("mcp_moshionly_"), "mcp_moshionly_");
        let unscoped = GeminiStream::new(None);
        assert_eq!(
            unscoped.plain_tool_name("mcp_moshionly_echo"),
            "mcp_moshionly_echo"
        );
    }

    #[test]
    fn a_failed_tool_result_is_not_ok_and_says_why() {
        let (sender, mut receiver) = event_channel();
        let mut parser = GeminiStream::new(None);
        let line = json!({"type":"tool_result","timestamp":"t","tool_id":"x1","status":"error",
            "output":"","error":{"type":"MCP_TOOL_ERROR","message":"node cd34 does not exist"}});
        parser.push_line(&line.to_string(), &sender);
        assert_eq!(
            receiver.try_recv().unwrap(),
            ChatEvent::ToolCallFinished {
                id: "x1".to_owned(),
                ok: false,
                summary: "node cd34 does not exist".to_owned(),
            }
        );
    }

    #[test]
    fn a_failed_result_is_classified_from_its_message() {
        let cases = [
            (
                "Please set an Auth method in your settings.json",
                ErrorKind::NotSignedIn,
            ),
            (
                "Quota exceeded for quota metric 'requests'",
                ErrorKind::RateLimited,
            ),
            ("models/nope is not found", ErrorKind::ModelUnavailable),
            ("something else broke", ErrorKind::Other),
        ];
        for (message, expected) in cases {
            let line = json!({"type":"result","timestamp":"t","status":"error",
                "error":{"type":"Error","message":message}});
            let events = replay(&line.to_string(), None, 1, "");
            assert!(
                matches!(&events[0], ChatEvent::Error { kind, .. } if *kind == expected),
                "{message}: {:?}",
                events[0]
            );
        }
    }

    #[test]
    fn warnings_are_ignored_and_errors_explain_a_later_failure() {
        let stream = [
            json!({"type":"error","timestamp":"t","severity":"warning","message":"slow network"}),
            json!({"type":"error","timestamp":"t","severity":"error","message":"API returned 503"}),
        ]
        .map(|line| line.to_string())
        .join("\n");
        let events = replay(&stream, None, 1, "unrelated log line");
        assert!(matches!(&events[0], ChatEvent::Error { message, .. } if message.contains("503")));
    }

    #[test]
    fn the_turn_limit_exit_code_is_a_stop_reason() {
        let events = replay("", None, EXIT_TURN_LIMIT, "");
        assert_eq!(
            events,
            vec![ChatEvent::Finished {
                stop_reason: StopReason::ToolLoopLimit
            }]
        );
    }

    #[test]
    fn an_exit_without_a_result_reports_stderr() {
        let events = replay("", None, 42, "Error: unknown argument: frobnicate");
        assert!(
            matches!(&events[0], ChatEvent::Error { message, .. } if message.contains("frobnicate"))
        );
    }

    #[test]
    fn user_messages_are_not_echoed_as_the_answer() {
        let (sender, mut receiver) = event_channel();
        let mut parser = GeminiStream::new(None);
        let line = json!({"type":"message","timestamp":"t","role":"user","content":"hi"});
        parser.push_line(&line.to_string(), &sender);
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn a_stopped_run_is_cancelled() {
        assert_eq!(
            GeminiStream::new(None).finish(ProcessExit::Stopped),
            Ok(StopReason::Cancelled)
        );
    }
}
