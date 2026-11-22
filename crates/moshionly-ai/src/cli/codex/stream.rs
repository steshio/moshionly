//! Reads the event stream of `codex exec --json`.
//!
//! UNVERIFIED: the Codex CLI was not installed when this was written. The
//! shapes follow the event definitions and non-interactive mode guide that
//! OpenAI publishes for the CLI.
//!
//! The events, one JSON object per line:
//!
//! - `thread.started`: the thread id, which is what `codex exec resume`
//!   takes.
//! - `item.started`, `item.updated`, `item.completed`: an item of the turn.
//!   `agent_message` and `reasoning` carry text; `mcp_tool_call` is a call to
//!   one of our tools; `command_execution`, `file_change` and `web_search`
//!   are the CLI's own tools, which the provider turns off but which are
//!   still reported if they appear.
//! - `turn.completed`: the end of a successful turn, with token counts.
//! - `turn.failed`: the end of a failed turn, with a message.
//! - `error`: a problem with the stream. The CLI also uses it to announce
//!   that it is retrying, so it does not end the turn by itself.

use std::collections::{HashMap, HashSet};

use serde_json::{Value, json};

use crate::cli::process::{LineAction, ProcessExit};
use crate::cli::{classify_failure_text, exit_failure, summarize_tool_content};
use crate::error::{AiError, ErrorKind};
use crate::event::{ChatEvent, EventSender, StopReason};
use crate::redact::scrub;
use crate::tool::summarize_text;

/// What to tell someone whose CLI is not signed in.
pub(super) const SIGN_IN_HINT: &str =
    "open a terminal, run `codex login` and sign in, then try again";

/// Which stream a piece of text belongs to.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TextKind {
    Answer,
    Reasoning,
}

/// Turns `codex exec --json` lines into [`ChatEvent`]s.
#[derive(Default)]
pub(crate) struct CodexStream {
    session_reported: bool,
    /// How many bytes of each text item have been passed on, by item id.
    /// Items normally arrive whole, but an updated item repeats its text.
    emitted: HashMap<String, usize>,
    answered: bool,
    reasoned: bool,
    started_tools: HashSet<String>,
    /// The most recent error the stream mentioned, kept to explain a failure.
    last_error: Option<String>,
    outcome: Option<Result<StopReason, AiError>>,
    noise: Vec<String>,
}

impl CodexStream {
    /// A parser for one run.
    pub fn new() -> Self {
        Self::default()
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
            Some("thread.started") => self.on_thread_started(&value, events),
            Some("item.started" | "item.updated") => {
                self.on_item(value.get("item"), false, events);
            }
            Some("item.completed") => self.on_item(value.get("item"), true, events),
            Some("turn.completed") => {
                self.on_turn_completed(&value, events);
                return LineAction::Finish;
            }
            Some("turn.failed") => {
                let message = value
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .or_else(|| self.last_error.clone())
                    .unwrap_or_default();
                self.outcome = Some(Err(turn_error(&message)));
                return LineAction::Finish;
            }
            Some("error") => {
                if let Some(message) = value.get("message").and_then(Value::as_str) {
                    self.last_error = Some(message.to_owned());
                }
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
            ProcessExit::Exited { code, stderr } => {
                let detail = match self.last_error {
                    Some(message) => message,
                    None if !stderr.is_empty() => stderr,
                    None => self.noise.join("\n"),
                };
                Err(exit_failure("Codex", SIGN_IN_HINT, code, &detail))
            }
        }
    }

    fn on_thread_started(&mut self, value: &Value, events: &EventSender) {
        if let Some(thread_id) = value.get("thread_id").and_then(Value::as_str)
            && !self.session_reported
        {
            self.session_reported = true;
            events.emit(ChatEvent::SessionStarted {
                session_id: thread_id.to_owned(),
            });
        }
    }

    fn on_item(&mut self, item: Option<&Value>, completed: bool, events: &EventSender) {
        let Some(item) = item else {
            return;
        };
        let id = item.get("id").and_then(Value::as_str).unwrap_or("");
        let text = |name: &str| item.get(name).and_then(Value::as_str).unwrap_or("");
        let status = text("status");
        match item.get("type").and_then(Value::as_str) {
            Some("agent_message") => self.emit_text(id, text("text"), TextKind::Answer, events),
            Some("reasoning") => self.emit_text(id, text("text"), TextKind::Reasoning, events),
            Some("mcp_tool_call") => {
                let input = item.get("arguments").cloned().unwrap_or(Value::Null);
                self.start_tool(id, text("tool"), input, events);
                if completed {
                    let ok = status == "completed" && item.get("error").is_none_or(Value::is_null);
                    let summary = match item.pointer("/error/message").and_then(Value::as_str) {
                        Some(message) => summarize_text(message),
                        None => summarize_tool_content(item.pointer("/result/content"), !ok),
                    };
                    finish_tool(id, ok, summary, events);
                }
            }
            Some("command_execution") => {
                self.start_tool(id, "shell", json!({ "command": text("command") }), events);
                if completed {
                    let ok = status == "completed"
                        && item.get("exit_code").and_then(Value::as_i64) == Some(0);
                    let mut summary = summarize_text(text("aggregated_output"));
                    if summary.is_empty() {
                        summary = if ok { "ok" } else { status }.to_owned();
                    }
                    finish_tool(id, ok, summary, events);
                }
            }
            Some("file_change") => {
                let input =
                    json!({ "changes": item.get("changes").cloned().unwrap_or(Value::Null) });
                self.start_tool(id, "apply_patch", input, events);
                if completed {
                    let ok = status == "completed";
                    finish_tool(id, ok, if ok { "ok" } else { status }.to_owned(), events);
                }
            }
            Some("web_search") => {
                self.start_tool(id, "web_search", json!({ "query": text("query") }), events);
                if completed {
                    finish_tool(id, true, "ok".to_owned(), events);
                }
            }
            Some("error") => self.last_error = Some(text("message").to_owned()),
            _ => {}
        }
    }

    /// Passes on the part of an item's text that has not been sent yet.
    fn emit_text(&mut self, id: &str, text: &str, kind: TextKind, events: &EventSender) {
        let sent = self.emitted.get(id).copied().unwrap_or(0);
        let Some(fresh) = text.get(sent..).filter(|fresh| !fresh.is_empty()) else {
            return;
        };
        let seen_before = match kind {
            TextKind::Answer => &mut self.answered,
            TextKind::Reasoning => &mut self.reasoned,
        };
        // Each item is a message of its own; keep them apart as paragraphs.
        let separator = if sent == 0 && *seen_before {
            "\n\n"
        } else {
            ""
        };
        *seen_before = true;
        let delta = format!("{separator}{fresh}");
        match kind {
            TextKind::Answer => events.text(delta),
            TextKind::Reasoning => events.thinking(delta),
        }
        self.emitted.insert(id.to_owned(), text.len());
    }

    fn start_tool(&mut self, id: &str, name: &str, input: Value, events: &EventSender) {
        if self.started_tools.insert(id.to_owned()) {
            events.emit(ChatEvent::ToolCallStarted {
                id: id.to_owned(),
                name: name.to_owned(),
                input,
            });
        }
    }

    fn on_turn_completed(&mut self, value: &Value, events: &EventSender) {
        if let Some(usage) = value.get("usage") {
            let count = |name: &str| usage.get(name).and_then(Value::as_u64).unwrap_or(0);
            events.emit(ChatEvent::Usage {
                // Cached tokens are already part of `input_tokens`.
                input_tokens: count("input_tokens"),
                output_tokens: count("output_tokens"),
                cost_usd: None,
            });
        }
        self.outcome = Some(Ok(StopReason::EndTurn));
    }
}

fn finish_tool(id: &str, ok: bool, summary: String, events: &EventSender) {
    events.emit(ChatEvent::ToolCallFinished {
        id: id.to_owned(),
        ok,
        summary,
    });
}

/// The error for a turn the CLI reported as failed.
fn turn_error(message: &str) -> AiError {
    match classify_failure_text(message) {
        ErrorKind::NotSignedIn => AiError::NotSignedIn(SIGN_IN_HINT.to_owned()),
        kind if message.trim().is_empty() => {
            AiError::from_kind(kind, "Codex reported a failure without a message")
        }
        kind => AiError::from_kind(kind, summarize_text(&scrub(message))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::event_channel;

    const DOCUMENTED_TOOL_CALL: &str =
        include_str!("../../../tests/fixtures/codex/documented_tool_call.jsonl");

    fn replay(stream: &str, exit_code: i32, stderr: &str) -> Vec<ChatEvent> {
        let (sender, mut receiver) = event_channel();
        let mut parser = CodexStream::new();
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

    fn lines(values: &[Value]) -> String {
        values
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn a_documented_turn_with_an_mcp_call_maps_to_events() {
        let events = replay(DOCUMENTED_TOOL_CALL, 0, "");
        assert_eq!(
            events,
            vec![
                ChatEvent::SessionStarted {
                    session_id: "00000000-0000-7000-8000-000000000001".to_owned()
                },
                ChatEvent::ThinkingDelta {
                    text: "Calling the echo tool".to_owned()
                },
                ChatEvent::ToolCallStarted {
                    id: "item_1".to_owned(),
                    name: "echo".to_owned(),
                    input: json!({ "text": "ping" }),
                },
                ChatEvent::ToolCallFinished {
                    id: "item_1".to_owned(),
                    ok: true,
                    summary: "echo: ping".to_owned(),
                },
                ChatEvent::TextDelta {
                    text: "done".to_owned()
                },
                ChatEvent::Usage {
                    input_tokens: 24763,
                    output_tokens: 122,
                    cost_usd: None,
                },
                ChatEvent::Finished {
                    stop_reason: StopReason::EndTurn
                },
            ]
        );
    }

    #[test]
    fn an_updated_message_only_adds_its_new_text() {
        let stream = lines(&[
            json!({"type":"item.started","item":{"id":"m1","type":"agent_message","text":"Hel"}}),
            json!({"type":"item.updated","item":{"id":"m1","type":"agent_message","text":"Hello"}}),
            json!({"type":"item.completed","item":{"id":"m1","type":"agent_message","text":"Hello"}}),
            json!({"type":"item.completed","item":{"id":"m2","type":"agent_message","text":"Again"}}),
            json!({"type":"turn.completed","usage":{"input_tokens":1,"output_tokens":1}}),
        ]);
        let text: Vec<String> = replay(&stream, 0, "")
            .into_iter()
            .filter_map(|event| match event {
                ChatEvent::TextDelta { text } => Some(text),
                _ => None,
            })
            .collect();
        assert_eq!(text, ["Hel", "lo", "\n\nAgain"]);
    }

    #[test]
    fn a_failed_tool_call_is_not_ok_and_says_why() {
        let stream = lines(&[
            json!({"type":"item.started","item":{"id":"t1","type":"mcp_tool_call","server":"moshionly",
                "tool":"apply_edits","arguments":{"edits":[]},"result":null,"error":null,"status":"in_progress"}}),
            json!({"type":"item.completed","item":{"id":"t1","type":"mcp_tool_call","server":"moshionly",
                "tool":"apply_edits","arguments":{"edits":[]},"result":null,
                "error":{"message":"node cd34 does not exist"},"status":"failed"}}),
        ]);
        let events = replay(&stream, 1, "");
        assert_eq!(
            events[1],
            ChatEvent::ToolCallFinished {
                id: "t1".to_owned(),
                ok: false,
                summary: "node cd34 does not exist".to_owned(),
            }
        );
    }

    #[test]
    fn the_clis_own_tools_are_reported_if_they_appear() {
        let stream = lines(&[
            json!({"type":"item.started","item":{"id":"c1","type":"command_execution",
                "command":"bash -lc ls","aggregated_output":"","exit_code":null,"status":"in_progress"}}),
            json!({"type":"item.completed","item":{"id":"c1","type":"command_execution",
                "command":"bash -lc ls","aggregated_output":"","exit_code":null,"status":"declined"}}),
            json!({"type":"item.completed","item":{"id":"f1","type":"file_change",
                "changes":[{"path":"a.txt","kind":"add"}],"status":"failed"}}),
        ]);
        let events = replay(&stream, 1, "");
        assert_eq!(
            events[0],
            ChatEvent::ToolCallStarted {
                id: "c1".to_owned(),
                name: "shell".to_owned(),
                input: json!({ "command": "bash -lc ls" }),
            }
        );
        assert_eq!(
            events[1],
            ChatEvent::ToolCallFinished {
                id: "c1".to_owned(),
                ok: false,
                summary: "declined".to_owned(),
            }
        );
        assert!(
            matches!(&events[2], ChatEvent::ToolCallStarted { name, .. } if name == "apply_patch")
        );
        assert!(matches!(
            events[3],
            ChatEvent::ToolCallFinished { ok: false, .. }
        ));
    }

    #[test]
    fn a_failed_turn_is_classified_from_its_message() {
        let cases = [
            ("unexpected status 401 Unauthorized", ErrorKind::NotSignedIn),
            (
                "You've hit your usage limit. Try again later.",
                ErrorKind::RateLimited,
            ),
            (
                "The 'nope' model is not supported when using Codex with a ChatGPT account.",
                ErrorKind::ModelUnavailable,
            ),
            ("stream disconnected before completion", ErrorKind::Other),
        ];
        for (message, expected) in cases {
            let stream = lines(&[
                json!({"type":"thread.started","thread_id":"t"}),
                json!({"type":"turn.failed","error":{"message":message}}),
            ]);
            let events = replay(&stream, 1, "");
            assert!(
                matches!(&events[1], ChatEvent::Error { kind, .. } if *kind == expected),
                "{message}: {:?}",
                events[1]
            );
            assert_eq!(
                events.last(),
                Some(&ChatEvent::Finished {
                    stop_reason: StopReason::Error
                })
            );
        }
    }

    #[test]
    fn a_retry_notice_does_not_end_the_turn() {
        let stream = lines(&[
            json!({"type":"error","message":"Reconnecting... 1/5"}),
            json!({"type":"item.completed","item":{"id":"m1","type":"agent_message","text":"ok"}}),
            json!({"type":"turn.completed","usage":{"input_tokens":3,"output_tokens":1}}),
        ]);
        let events = replay(&stream, 0, "");
        assert_eq!(
            events.last(),
            Some(&ChatEvent::Finished {
                stop_reason: StopReason::EndTurn
            })
        );
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, ChatEvent::Error { .. }))
        );
    }

    #[test]
    fn an_exit_without_a_turn_end_is_explained() {
        // The last error the stream mentioned wins over stderr.
        let stream = lines(&[json!({"type":"error","message":"unexpected status 503"})]);
        let events = replay(&stream, 1, "some log line");
        assert!(matches!(&events[0], ChatEvent::Error { message, .. } if message.contains("503")));

        let events = replay("", 1, "Error: Not logged in");
        assert!(matches!(
            events[0],
            ChatEvent::Error {
                kind: ErrorKind::NotSignedIn,
                ..
            }
        ));

        let events = replay("", 2, "");
        assert!(
            matches!(&events[0], ChatEvent::Error { message, .. } if message.contains("code 2"))
        );
    }

    #[test]
    fn a_stopped_run_is_cancelled() {
        assert_eq!(
            CodexStream::new().finish(ProcessExit::Stopped),
            Ok(StopReason::Cancelled)
        );
    }
}
