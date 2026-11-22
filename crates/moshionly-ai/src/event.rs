//! What a provider sends back while it works: a stream of [`ChatEvent`]s.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc;

use crate::error::{AiError, ErrorKind};
use crate::redact::scrub;

/// Why a turn ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// The model finished its answer.
    EndTurn,
    /// The answer was cut off at the output limit.
    MaxTokens,
    /// The model kept calling tools until the provider's step limit.
    ToolLoopLimit,
    /// The model declined to answer.
    Refusal,
    /// The caller cancelled the request.
    Cancelled,
    /// The turn failed. A [`ChatEvent::Error`] was sent just before.
    Error,
}

/// One step of a provider's answer.
///
/// Every call to [`ChatProvider::send`](crate::ChatProvider::send) ends with
/// exactly one [`Finished`](Self::Finished) event, whatever happened.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChatEvent {
    /// The provider opened or resumed a session. Keep the id and pass it as
    /// [`ChatRequest::session_id`](crate::ChatRequest::session_id) to
    /// continue the conversation.
    SessionStarted {
        /// The provider's own id for the conversation.
        session_id: String,
    },
    /// More of the answer.
    TextDelta {
        /// Text to append to the answer.
        text: String,
    },
    /// More of the model's reasoning, when the provider shares it.
    ThinkingDelta {
        /// Text to append to the reasoning.
        text: String,
    },
    /// The model called a tool.
    ToolCallStarted {
        /// Identifies the call until its [`ToolCallFinished`](Self::ToolCallFinished).
        id: String,
        /// The tool name. For subscription providers this is the name as the
        /// agent reports it, with any MCP server prefix removed.
        name: String,
        /// The arguments the model passed.
        input: Value,
    },
    /// A tool call returned.
    ToolCallFinished {
        /// The id from [`ToolCallStarted`](Self::ToolCallStarted).
        id: String,
        /// False when the tool reported an error.
        ok: bool,
        /// One line for the chat panel.
        summary: String,
    },
    /// Token use for the turn so far. Sent at least once per successful turn
    /// when the provider reports it; later events replace earlier ones.
    Usage {
        /// Tokens read by the model, including cached ones.
        input_tokens: u64,
        /// Tokens written by the model.
        output_tokens: u64,
        /// The cost in US dollars, when the provider states it.
        cost_usd: Option<f64>,
    },
    /// Something went wrong. [`Finished`](Self::Finished) with
    /// [`StopReason::Error`] follows.
    Error {
        /// A message that is safe to show and to log.
        message: String,
        /// What kind of failure it was.
        kind: ErrorKind,
    },
    /// The turn is over. Always the last event.
    Finished {
        /// Why it ended.
        stop_reason: StopReason,
    },
}

/// The receiving half of an event channel.
pub type EventReceiver = mpsc::UnboundedReceiver<ChatEvent>;

/// Creates the channel a provider reports through.
///
/// The channel is unbounded so that a provider never blocks on a slow
/// reader; events are small and a turn produces at most a few thousand.
pub fn event_channel() -> (EventSender, EventReceiver) {
    let (sender, receiver) = mpsc::unbounded_channel();
    (EventSender { sender }, receiver)
}

/// The sending half of an event channel, handed to
/// [`ChatProvider::send`](crate::ChatProvider::send).
#[derive(Debug, Clone)]
pub struct EventSender {
    sender: mpsc::UnboundedSender<ChatEvent>,
}

impl EventSender {
    /// Sends one event. Returns false when nobody is listening any more, in
    /// which case the provider should stop working.
    pub fn emit(&self, event: ChatEvent) -> bool {
        self.sender.send(event).is_ok()
    }

    /// Sends a [`ChatEvent::TextDelta`], skipping empty text.
    pub fn text(&self, text: impl Into<String>) {
        let text = text.into();
        if !text.is_empty() {
            self.emit(ChatEvent::TextDelta { text });
        }
    }

    /// Sends a [`ChatEvent::ThinkingDelta`], skipping empty text.
    pub fn thinking(&self, text: impl Into<String>) {
        let text = text.into();
        if !text.is_empty() {
            self.emit(ChatEvent::ThinkingDelta { text });
        }
    }

    /// Sends a [`ChatEvent::Error`] built from `error`.
    pub fn error(&self, error: &AiError) {
        self.emit(ChatEvent::Error {
            message: scrub(&error.to_string()),
            kind: error.kind(),
        });
    }

    /// Ends the stream: an [`Error`](ChatEvent::Error) event if the turn
    /// failed, then the single [`Finished`](ChatEvent::Finished) event.
    ///
    /// Provider implementations run their work in a function that returns
    /// `Result<StopReason, AiError>` and pass the result here, which is what
    /// keeps the "exactly one `Finished`" promise in one place.
    pub fn finish(&self, outcome: Result<StopReason, AiError>) {
        let stop_reason = match outcome {
            Ok(stop_reason) => stop_reason,
            Err(error) => {
                self.error(&error);
                StopReason::Error
            }
        };
        self.emit(ChatEvent::Finished { stop_reason });
    }

    /// True when the receiver has been dropped.
    pub fn is_closed(&self) -> bool {
        self.sender.is_closed()
    }

    /// Completes when the receiver is dropped. Providers treat this like a
    /// cancellation.
    pub async fn closed(&self) {
        self.sender.closed().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drain(mut receiver: EventReceiver) -> Vec<ChatEvent> {
        let mut events = Vec::new();
        while let Ok(event) = receiver.try_recv() {
            events.push(event);
        }
        events
    }

    #[test]
    fn finish_with_an_error_sends_error_then_finished() {
        let (sender, receiver) = event_channel();
        sender.finish(Err(AiError::from_kind(ErrorKind::RateLimited, "slow down")));
        assert_eq!(
            drain(receiver),
            vec![
                ChatEvent::Error {
                    message: "rate limited: slow down".to_owned(),
                    kind: ErrorKind::RateLimited,
                },
                ChatEvent::Finished {
                    stop_reason: StopReason::Error
                },
            ]
        );
    }

    #[test]
    fn finish_with_a_stop_reason_sends_only_finished() {
        let (sender, receiver) = event_channel();
        sender.finish(Ok(StopReason::EndTurn));
        assert_eq!(
            drain(receiver),
            vec![ChatEvent::Finished {
                stop_reason: StopReason::EndTurn
            }]
        );
    }

    #[test]
    fn empty_deltas_are_dropped() {
        let (sender, receiver) = event_channel();
        sender.text("");
        sender.thinking("");
        sender.text("hi");
        assert_eq!(
            drain(receiver),
            vec![ChatEvent::TextDelta {
                text: "hi".to_owned()
            }]
        );
    }

    #[test]
    fn emit_reports_a_dropped_receiver() {
        let (sender, receiver) = event_channel();
        drop(receiver);
        assert!(sender.is_closed());
        assert!(!sender.emit(ChatEvent::Finished {
            stop_reason: StopReason::EndTurn
        }));
    }

    #[test]
    fn events_serialise_with_a_type_tag() {
        let event = ChatEvent::ToolCallFinished {
            id: "call_1".to_owned(),
            ok: true,
            summary: "applied 2 edits".to_owned(),
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "tool_call_finished");
        assert_eq!(json["ok"], true);
        let back: ChatEvent = serde_json::from_value(json).unwrap();
        assert_eq!(back, event);
    }
}
