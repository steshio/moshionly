//! The conversation as the panel shows it: what the person wrote, and the
//! assistant's answers built up from a provider's stream of events.
//!
//! [`Conversation`] is plain data with no window and no threads. A turn
//! starts with [`Conversation::begin_turn`], grows with every
//! [`ChatEvent`] passed to [`Conversation::apply`], and ends with the
//! provider's `Finished` event. Everything the panel draws is read from
//! here, so every rule of the stream is tested without a window.

use moshionly_ai::{ChatEvent, ChatMessage, ErrorKind, ImageData, Role, StopReason};
use serde_json::Value;

use super::tool_summary;

/// Who wrote a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Author {
    /// The person using the app.
    Person,
    /// The AI.
    Assistant,
}

/// Where an assistant's answer stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnswerState {
    /// The provider is still sending it.
    Running,
    /// The provider finished, for this reason.
    Finished(StopReason),
}

/// Where one tool call stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolState {
    /// The tool is working.
    Running,
    /// The tool returned a result.
    Succeeded,
    /// The tool reported an error; the AI usually reads it and tries again.
    Failed,
    /// The turn ended before the tool reported back, because it was stopped
    /// or failed.
    Interrupted,
}

/// One call the assistant made to a Moshionly tool.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    /// The provider's id for the call.
    pub id: String,
    /// The tool's name, as the model called it, for example `apply_edits`.
    pub name: String,
    /// The tool's name for people, for example "Edit".
    pub title: String,
    /// A few words saying what the call was asked to do.
    pub summary: String,
    /// The arguments the model passed.
    pub input: Value,
    /// Where the call stands.
    pub state: ToolState,
    /// One line about the result, once there is one.
    pub result: Option<String>,
    /// Pictures the tool returned, such as screenshots. Only tools that run
    /// in the app (API providers) hand them over.
    pub images: Vec<ImageData>,
}

/// Token use and cost of one answer, as the provider reported it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Usage {
    /// Tokens the model read.
    pub input_tokens: u64,
    /// Tokens the model wrote.
    pub output_tokens: u64,
    /// What the answer cost in US dollars, when the provider says.
    pub cost_usd: Option<f64>,
}

/// A failure shown inside the conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatError {
    /// What kind of failure it was, which decides the way forward offered.
    pub kind: ErrorKind,
    /// What went wrong, safe to show.
    pub message: String,
}

/// One piece of a message, in the order it arrived.
#[derive(Debug, Clone, PartialEq)]
pub enum Part {
    /// Text of the message.
    Text(String),
    /// The model's reasoning, shown collapsed.
    Thinking(String),
    /// A tool call.
    Tool(ToolCall),
    /// An image the person attached.
    Image(ImageData),
    /// Something went wrong.
    Error(ChatError),
}

/// One message of the conversation.
#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    /// A number that stays with the message, for the panel to keep state by.
    pub id: u64,
    /// Who wrote it.
    pub author: Author,
    /// What it holds, in order.
    pub parts: Vec<Part>,
    /// For an answer, whether it is still arriving. A person's message is
    /// always finished.
    pub state: AnswerState,
    /// Token use, for an answer whose provider reported it.
    pub usage: Option<Usage>,
    /// The name of the provider that answered, for an answer.
    pub provider: Option<String>,
}

impl Message {
    /// All text parts joined, without thinking, tools or errors.
    pub fn text(&self) -> String {
        let mut text = String::new();
        for part in &self.parts {
            if let Part::Text(piece) = part {
                text.push_str(piece);
            }
        }
        text
    }

    /// The images the person attached.
    pub fn images(&self) -> impl Iterator<Item = &ImageData> {
        self.parts.iter().filter_map(|part| match part {
            Part::Image(image) => Some(image),
            _ => None,
        })
    }

    /// The tool calls, in order.
    pub fn tool_calls(&self) -> impl Iterator<Item = &ToolCall> {
        self.parts.iter().filter_map(|part| match part {
            Part::Tool(call) => Some(call),
            _ => None,
        })
    }

    /// The errors, in order.
    pub fn errors(&self) -> impl Iterator<Item = &ChatError> {
        self.parts.iter().filter_map(|part| match part {
            Part::Error(error) => Some(error),
            _ => None,
        })
    }

    /// True while the provider is still sending this answer.
    pub fn is_running(&self) -> bool {
        self.state == AnswerState::Running
    }
}

/// The provider's own record of the conversation, which agent CLIs use to
/// continue it on the next turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderSession {
    /// The provider that holds it.
    pub provider: String,
    /// Its id for the conversation.
    pub session_id: String,
}

/// The whole conversation.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Conversation {
    messages: Vec<Message>,
    session: Option<ProviderSession>,
    /// The provider answering the running turn.
    turn_provider: Option<String>,
    next_id: u64,
}

impl Conversation {
    /// An empty conversation.
    pub fn new() -> Self {
        Self::default()
    }

    /// Every message, oldest first.
    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    /// True when nothing has been said yet.
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    /// True while an answer is arriving.
    pub fn is_running(&self) -> bool {
        self.messages.last().is_some_and(Message::is_running)
    }

    /// The provider's session, when one was opened.
    pub fn session(&self) -> Option<&ProviderSession> {
        self.session.as_ref()
    }

    /// The session id to continue with when `provider` answers next. A
    /// session belongs to one provider: after switching provider the
    /// conversation is sent in full instead.
    pub fn session_for(&self, provider: &str) -> Option<&str> {
        self.session
            .as_ref()
            .filter(|session| session.provider == provider)
            .map(|session| session.session_id.as_str())
    }

    /// Starts over: forgets every message and the provider's session.
    pub fn clear(&mut self) {
        self.messages.clear();
        self.session = None;
        self.turn_provider = None;
    }

    /// Starts a turn: adds the person's message and an empty answer from
    /// `provider` (its id and its name for people) that the provider's
    /// events then fill in.
    pub fn begin_turn(
        &mut self,
        text: &str,
        images: Vec<ImageData>,
        provider_id: &str,
        provider_name: &str,
    ) {
        let mut parts = Vec::new();
        if !text.is_empty() {
            parts.push(Part::Text(text.to_owned()));
        }
        parts.extend(images.into_iter().map(Part::Image));
        let id = self.take_id();
        self.messages.push(Message {
            id,
            author: Author::Person,
            parts,
            state: AnswerState::Finished(StopReason::EndTurn),
            usage: None,
            provider: None,
        });
        let id = self.take_id();
        self.messages.push(Message {
            id,
            author: Author::Assistant,
            parts: Vec::new(),
            state: AnswerState::Running,
            usage: None,
            provider: Some(provider_name.to_owned()),
        });
        self.turn_provider = Some(provider_id.to_owned());
    }

    /// The conversation as a provider takes it: every finished message's
    /// text and images, oldest first, ending with the person's newest
    /// message. Thinking, tool calls and errors stay in the panel. Answers
    /// without text are left out, because some services refuse empty
    /// messages.
    pub fn history(&self) -> Vec<ChatMessage> {
        self.messages
            .iter()
            .filter(|message| !message.is_running())
            .filter_map(|message| {
                let text = message.text();
                match message.author {
                    Author::Person => Some(ChatMessage {
                        role: Role::User,
                        text,
                        images: message.images().cloned().collect(),
                    }),
                    Author::Assistant if text.trim().is_empty() => None,
                    Author::Assistant => Some(ChatMessage::assistant(text)),
                }
            })
            .collect()
    }

    /// Adds one event of the running answer. Events that arrive when no
    /// answer is running are ignored: they belong to a turn that is over.
    pub fn apply(&mut self, event: &ChatEvent) {
        let provider = self.turn_provider.clone();
        let Some(answer) = self.running_answer() else {
            return;
        };
        match event {
            ChatEvent::SessionStarted { session_id } => {
                if let Some(provider) = provider {
                    self.session = Some(ProviderSession {
                        provider,
                        session_id: session_id.clone(),
                    });
                }
            }
            ChatEvent::TextDelta { text } => match answer.parts.last_mut() {
                Some(Part::Text(existing)) => existing.push_str(text),
                _ => answer.parts.push(Part::Text(text.clone())),
            },
            ChatEvent::ThinkingDelta { text } => match answer.parts.last_mut() {
                Some(Part::Thinking(existing)) => existing.push_str(text),
                _ => answer.parts.push(Part::Thinking(text.clone())),
            },
            ChatEvent::ToolCallStarted { id, name, input } => {
                answer.parts.push(Part::Tool(ToolCall {
                    id: id.clone(),
                    name: name.clone(),
                    title: tool_summary::title(name),
                    summary: tool_summary::describe(name, input),
                    input: input.clone(),
                    state: ToolState::Running,
                    result: None,
                    images: Vec::new(),
                }));
            }
            ChatEvent::ToolCallFinished { id, ok, summary } => {
                if let Some(call) = tool_call_mut(answer, id) {
                    call.state = if *ok {
                        ToolState::Succeeded
                    } else {
                        ToolState::Failed
                    };
                    let summary = summary.trim();
                    call.result = (!summary.is_empty()).then(|| summary.to_owned());
                }
            }
            ChatEvent::Usage {
                input_tokens,
                output_tokens,
                cost_usd,
            } => {
                answer.usage = Some(Usage {
                    input_tokens: *input_tokens,
                    output_tokens: *output_tokens,
                    cost_usd: *cost_usd,
                });
            }
            ChatEvent::Error { message, kind } => {
                answer.parts.push(Part::Error(ChatError {
                    kind: *kind,
                    message: message.clone(),
                }));
            }
            ChatEvent::Finished { stop_reason } => self.finish(*stop_reason),
        }
    }

    /// Ends the running answer for `reason`. Tool calls still running then
    /// will never report back, so they are marked interrupted.
    pub fn finish(&mut self, reason: StopReason) {
        if let Some(answer) = self.running_answer() {
            for part in &mut answer.parts {
                if let Part::Tool(call) = part
                    && call.state == ToolState::Running
                {
                    call.state = ToolState::Interrupted;
                }
            }
            answer.state = AnswerState::Finished(reason);
        }
        self.turn_provider = None;
    }

    /// Ends the running answer with an error that did not come from the
    /// provider's stream, for example a provider that could not be built
    /// or that stopped without saying it had finished.
    pub fn fail(&mut self, kind: ErrorKind, message: impl Into<String>) {
        if let Some(answer) = self.running_answer() {
            answer.parts.push(Part::Error(ChatError {
                kind,
                message: message.into(),
            }));
        }
        self.finish(StopReason::Error);
    }

    /// Adds images a tool returned to the running call with this name and
    /// input, newest first. API providers report a call's start before the
    /// tool runs, so the call is already listed.
    pub fn attach_tool_images(&mut self, name: &str, input: &Value, images: Vec<ImageData>) {
        let Some(answer) = self.running_answer() else {
            return;
        };
        let call = answer.parts.iter_mut().rev().find_map(|part| match part {
            Part::Tool(call)
                if call.name == name && call.input == *input && call.images.is_empty() =>
            {
                Some(call)
            }
            _ => None,
        });
        if let Some(call) = call {
            call.images = images;
        }
    }

    /// Takes back the person's last message and the answer to it, so that
    /// it can be sent again. Returns its text and images; `None` while an
    /// answer is running or when there is nothing to retry.
    pub fn take_last_request(&mut self) -> Option<(String, Vec<ImageData>)> {
        if self.is_running() {
            return None;
        }
        let position = self
            .messages
            .iter()
            .rposition(|message| message.author == Author::Person)?;
        let person = self.messages.remove(position);
        self.messages.truncate(position);
        let images = person.images().cloned().collect();
        Some((person.text(), images))
    }

    fn running_answer(&mut self) -> Option<&mut Message> {
        self.messages
            .last_mut()
            .filter(|message| message.author == Author::Assistant && message.is_running())
    }

    fn take_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
}

fn tool_call_mut<'a>(answer: &'a mut Message, id: &str) -> Option<&'a mut ToolCall> {
    answer.parts.iter_mut().find_map(|part| match part {
        Part::Tool(call) if call.id == id => Some(call),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn text(text: &str) -> ChatEvent {
        ChatEvent::TextDelta {
            text: text.to_owned(),
        }
    }

    fn thinking(text: &str) -> ChatEvent {
        ChatEvent::ThinkingDelta {
            text: text.to_owned(),
        }
    }

    fn finished(stop_reason: StopReason) -> ChatEvent {
        ChatEvent::Finished { stop_reason }
    }

    fn started(id: &str, name: &str, input: Value) -> ChatEvent {
        ChatEvent::ToolCallStarted {
            id: id.to_owned(),
            name: name.to_owned(),
            input,
        }
    }

    fn turn(events: &[ChatEvent]) -> Conversation {
        let mut conversation = Conversation::new();
        conversation.begin_turn(
            "Fade the title in",
            Vec::new(),
            "claude-code",
            "Claude Code",
        );
        for event in events {
            conversation.apply(event);
        }
        conversation
    }

    fn answer(conversation: &Conversation) -> &Message {
        conversation.messages().last().unwrap()
    }

    #[test]
    fn a_turn_adds_the_message_and_a_running_answer() {
        let conversation = turn(&[]);
        let [person, assistant] = conversation.messages() else {
            panic!("two messages expected");
        };
        assert_eq!(person.author, Author::Person);
        assert_eq!(person.text(), "Fade the title in");
        assert_eq!(assistant.author, Author::Assistant);
        assert!(assistant.is_running());
        assert_eq!(assistant.provider.as_deref(), Some("Claude Code"));
        assert!(conversation.is_running());
        assert_ne!(person.id, assistant.id);
    }

    #[test]
    fn streamed_text_joins_into_one_part() {
        let conversation = turn(&[text("Fading "), text("the title"), text(" in.")]);
        assert_eq!(
            answer(&conversation).parts,
            vec![Part::Text("Fading the title in.".into())]
        );
    }

    #[test]
    fn thinking_is_kept_apart_from_the_answer() {
        let conversation = turn(&[
            thinking("The title is "),
            thinking("layer t1."),
            text("Done."),
            thinking("Check it."),
        ]);
        assert_eq!(
            answer(&conversation).parts,
            vec![
                Part::Thinking("The title is layer t1.".into()),
                Part::Text("Done.".into()),
                Part::Thinking("Check it.".into()),
            ]
        );
        assert_eq!(answer(&conversation).text(), "Done.");
    }

    #[test]
    fn tool_calls_run_and_finish_by_id() {
        let input = json!({ "label": "Fade in the title", "operations": [{ "op": "add" }] });
        let conversation = turn(&[
            text("Adding a fade."),
            started("call_1", "apply_edits", input.clone()),
            started(
                "call_2",
                "screenshot",
                json!({ "artboard": "scene", "time": 500 }),
            ),
            ChatEvent::ToolCallFinished {
                id: "call_2".into(),
                ok: false,
                summary: "no artboard \"scene\"\nmore".into(),
            },
            ChatEvent::ToolCallFinished {
                id: "call_1".into(),
                ok: true,
                summary: "applied 1 operation".into(),
            },
            text("Done."),
        ]);
        let calls: Vec<&ToolCall> = answer(&conversation).tool_calls().collect();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "apply_edits");
        assert_eq!(calls[0].title, "Edit");
        assert_eq!(calls[0].summary, "Fade in the title");
        assert_eq!(calls[0].input, input);
        assert_eq!(calls[0].state, ToolState::Succeeded);
        assert_eq!(calls[0].result.as_deref(), Some("applied 1 operation"));
        assert_eq!(calls[1].state, ToolState::Failed);
        assert_eq!(
            calls[1].result.as_deref(),
            Some("no artboard \"scene\"\nmore")
        );
        assert_eq!(answer(&conversation).text(), "Adding a fade.Done.");
        assert!(matches!(answer(&conversation).parts[3], Part::Text(_)));
    }

    #[test]
    fn a_finish_for_an_unknown_call_changes_nothing() {
        let conversation = turn(&[ChatEvent::ToolCallFinished {
            id: "nope".into(),
            ok: true,
            summary: "ok".into(),
        }]);
        assert!(answer(&conversation).parts.is_empty());
    }

    #[test]
    fn later_usage_replaces_earlier_usage() {
        let conversation = turn(&[
            ChatEvent::Usage {
                input_tokens: 10,
                output_tokens: 2,
                cost_usd: None,
            },
            ChatEvent::Usage {
                input_tokens: 1200,
                output_tokens: 340,
                cost_usd: Some(0.012),
            },
            finished(StopReason::EndTurn),
        ]);
        assert_eq!(
            answer(&conversation).usage,
            Some(Usage {
                input_tokens: 1200,
                output_tokens: 340,
                cost_usd: Some(0.012),
            })
        );
    }

    #[test]
    fn an_error_is_shown_with_its_kind_and_ends_the_turn() {
        let conversation = turn(&[
            ChatEvent::Error {
                message: "not signed in: run claude".into(),
                kind: ErrorKind::NotSignedIn,
            },
            finished(StopReason::Error),
        ]);
        let answer = answer(&conversation);
        assert_eq!(
            answer.errors().collect::<Vec<_>>(),
            vec![&ChatError {
                kind: ErrorKind::NotSignedIn,
                message: "not signed in: run claude".into(),
            }]
        );
        assert_eq!(answer.state, AnswerState::Finished(StopReason::Error));
        assert!(!conversation.is_running());
    }

    #[test]
    fn cancelling_interrupts_tools_that_are_still_running() {
        let conversation = turn(&[
            started("a", "screenshot", json!({})),
            started("b", "get_overview", json!({})),
            ChatEvent::ToolCallFinished {
                id: "b".into(),
                ok: true,
                summary: "ok".into(),
            },
            finished(StopReason::Cancelled),
        ]);
        let states: Vec<ToolState> = answer(&conversation)
            .tool_calls()
            .map(|call| call.state)
            .collect();
        assert_eq!(states, vec![ToolState::Interrupted, ToolState::Succeeded]);
        assert_eq!(
            answer(&conversation).state,
            AnswerState::Finished(StopReason::Cancelled)
        );
    }

    #[test]
    fn events_after_the_finish_are_ignored() {
        let conversation = turn(&[
            text("Done."),
            finished(StopReason::EndTurn),
            text(" Late."),
            started("x", "export", json!({})),
        ]);
        assert_eq!(
            answer(&conversation).parts,
            vec![Part::Text("Done.".into())]
        );
    }

    #[test]
    fn the_session_belongs_to_the_provider_that_opened_it() {
        let mut conversation = turn(&[
            ChatEvent::SessionStarted {
                session_id: "s-1".into(),
            },
            finished(StopReason::EndTurn),
        ]);
        assert_eq!(conversation.session_for("claude-code"), Some("s-1"));
        assert_eq!(conversation.session_for("cursor-agent"), None);

        // A turn with another provider that opens no session keeps the old one.
        conversation.begin_turn("Again", Vec::new(), "anthropic", "Anthropic");
        conversation.apply(&finished(StopReason::EndTurn));
        assert_eq!(conversation.session_for("claude-code"), Some("s-1"));

        conversation.clear();
        assert!(conversation.is_empty());
        assert_eq!(conversation.session(), None);
    }

    #[test]
    fn history_holds_finished_text_and_images_only() {
        let image = ImageData::png(vec![1, 2, 3]);
        let mut conversation = Conversation::new();
        conversation.begin_turn("First", vec![image.clone()], "anthropic", "Anthropic");
        for event in [
            thinking("hmm"),
            text("Answer"),
            started("t", "get_overview", json!({})),
            finished(StopReason::EndTurn),
        ] {
            conversation.apply(&event);
        }
        conversation.begin_turn("Second", Vec::new(), "anthropic", "Anthropic");
        conversation.apply(&ChatEvent::Error {
            message: "overloaded".into(),
            kind: ErrorKind::Overloaded,
        });
        conversation.apply(&finished(StopReason::Error));
        conversation.begin_turn("Third", Vec::new(), "anthropic", "Anthropic");

        let history = conversation.history();
        let summary: Vec<(Role, &str, usize)> = history
            .iter()
            .map(|m| (m.role, m.text.as_str(), m.images.len()))
            .collect();
        assert_eq!(
            summary,
            vec![
                (Role::User, "First", 1),
                (Role::Assistant, "Answer", 0),
                (Role::User, "Second", 0),
                (Role::User, "Third", 0),
            ]
        );
    }

    #[test]
    fn failing_outside_the_stream_ends_the_turn_with_an_error() {
        let mut conversation = turn(&[started("a", "export", json!({}))]);
        conversation.fail(ErrorKind::Other, "the provider stopped");
        let answer = answer(&conversation);
        assert_eq!(answer.state, AnswerState::Finished(StopReason::Error));
        assert_eq!(answer.errors().count(), 1);
        assert_eq!(
            answer.tool_calls().next().unwrap().state,
            ToolState::Interrupted
        );
    }

    #[test]
    fn tool_images_go_to_the_matching_call() {
        let shot = json!({ "artboard": "scene" });
        let mut conversation = turn(&[
            started("a", "screenshot", shot.clone()),
            started("b", "screenshot", json!({ "artboard": "other" })),
        ]);
        conversation.attach_tool_images("screenshot", &shot, vec![ImageData::png(vec![9])]);
        conversation.attach_tool_images("export", &shot, vec![ImageData::png(vec![8])]);
        let calls: Vec<&ToolCall> = answer(&conversation).tool_calls().collect();
        assert_eq!(calls[0].images.len(), 1);
        assert!(calls[1].images.is_empty());
    }

    #[test]
    fn the_last_request_can_be_taken_back_to_retry() {
        let mut conversation = turn(&[]);
        assert_eq!(conversation.take_last_request(), None, "still running");
        conversation.apply(&ChatEvent::Error {
            message: "network error".into(),
            kind: ErrorKind::Network,
        });
        conversation.apply(&finished(StopReason::Error));
        let (text, images) = conversation.take_last_request().unwrap();
        assert_eq!(text, "Fade the title in");
        assert!(images.is_empty());
        assert!(conversation.is_empty());
    }
}
