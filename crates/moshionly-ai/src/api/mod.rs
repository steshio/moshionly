//! API providers: model APIs called directly with the user's key (or a
//! local server), with the tool loop run here against the app's
//! [`ToolExecutor`].
//!
//! This module holds what the providers share: picking the part of the
//! conversation to answer, running the tools a model asked for, and adding
//! up token use over the requests of one turn.

pub(crate) mod anthropic;
pub(crate) mod openai;

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{Value, json};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::chat::{ChatMessage, ChatRequest, Role};
use crate::error::{AiError, ErrorKind};
use crate::event::{ChatEvent, EventSender, stopped};
use crate::tool::{ToolExecutor, ToolOutput};

/// The model id that means "the provider's default".
pub(crate) const DEFAULT_MODEL: &str = "default";

/// The messages to send: everything up to and including the newest user
/// message. Assistant text after it would be read by a model as the start
/// of its own answer, which current APIs reject.
pub(crate) fn conversation(request: &ChatRequest) -> Result<&[ChatMessage], AiError> {
    let last_user = request
        .messages
        .iter()
        .rposition(|message| message.role == Role::User)
        .ok_or_else(|| {
            AiError::from_kind(
                ErrorKind::InvalidRequest,
                "the conversation has no user message to answer",
            )
        })?;
    Ok(&request.messages[..=last_user])
}

/// The model to ask for: the request's choice, or `fallback` when it names
/// none.
pub(crate) fn chosen_model<'a>(request: &'a ChatRequest, fallback: &'a str) -> &'a str {
    match request.model.as_deref().map(str::trim) {
        Some(model) if !model.is_empty() && model != DEFAULT_MODEL => model,
        _ => fallback,
    }
}

/// A tool call the model asked for.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ToolCall {
    /// The model's id for the call; the result is sent back under it.
    pub id: String,
    /// The tool to run.
    pub name: String,
    /// The arguments, or the raw text when the model did not send a JSON
    /// object.
    pub input: Result<Value, String>,
}

/// Runs the tools of one step, all at the same time, and returns their
/// results in the order of `calls`. Returns `None` when the turn was
/// cancelled, in which case calls still running are abandoned.
///
/// A call whose arguments were not valid JSON is not run. The model gets
/// its own text back as an error so that it can try again, which is the
/// reply Anthropic documents for input streamed without server-side
/// validation.
pub(crate) async fn run_tools(
    executor: Option<&Arc<dyn ToolExecutor>>,
    calls: &[ToolCall],
    events: &EventSender,
    cancel: &CancellationToken,
) -> Option<Vec<ToolOutput>> {
    let mut outputs: Vec<Option<ToolOutput>> = vec![None; calls.len()];
    let mut running = JoinSet::new();
    let mut task_index = HashMap::new();

    for (index, call) in calls.iter().enumerate() {
        events.emit(ChatEvent::ToolCallStarted {
            id: call.id.clone(),
            name: call.name.clone(),
            input: match &call.input {
                Ok(input) => input.clone(),
                Err(raw) => Value::String(raw.clone()),
            },
        });
        match (&call.input, executor) {
            (Err(raw), _) => {
                let output = ToolOutput::error(json!({ "INVALID_JSON": raw }).to_string());
                finish(call, &output, events);
                outputs[index] = Some(output);
            }
            (Ok(_), None) => {
                let output = ToolOutput::error("No tools are available in this chat.");
                finish(call, &output, events);
                outputs[index] = Some(output);
            }
            (Ok(input), Some(executor)) => {
                let executor = Arc::clone(executor);
                let name = call.name.clone();
                let input = input.clone();
                let handle = running.spawn(async move { executor.call(&name, input).await });
                task_index.insert(handle.id(), index);
            }
        }
    }

    loop {
        let joined = tokio::select! {
            () = stopped(cancel, events) => {
                running.abort_all();
                return None;
            }
            joined = running.join_next_with_id() => joined,
        };
        let Some(joined) = joined else {
            break;
        };
        let (task, output) = match joined {
            Ok((task, output)) => (task, output),
            // A tool that panicked must not take the whole turn with it.
            Err(error) => (
                error.id(),
                ToolOutput::error("The tool stopped unexpectedly. Try a different approach."),
            ),
        };
        if let Some(&index) = task_index.get(&task) {
            finish(&calls[index], &output, events);
            outputs[index] = Some(output);
        }
    }
    Some(
        outputs
            .into_iter()
            .map(|output| output.unwrap_or_else(|| ToolOutput::error("The tool did not run.")))
            .collect(),
    )
}

fn finish(call: &ToolCall, output: &ToolOutput, events: &EventSender) {
    events.emit(ChatEvent::ToolCallFinished {
        id: call.id.clone(),
        ok: !output.is_error,
        summary: output.summary(),
    });
}

/// Token use added up over the requests of one turn. A turn with tool calls
/// makes several requests, and each one reads the conversation again.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct UsageTotals {
    input_tokens: u64,
    output_tokens: u64,
}

impl UsageTotals {
    /// Adds one request's use and reports the totals so far.
    pub fn add(&mut self, input_tokens: u64, output_tokens: u64, events: &EventSender) {
        if input_tokens == 0 && output_tokens == 0 {
            return;
        }
        self.input_tokens += input_tokens;
        self.output_tokens += output_tokens;
        events.emit(ChatEvent::Usage {
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            cost_usd: None,
        });
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    //! A small tool set for provider tests.

    use std::sync::Mutex;

    use async_trait::async_trait;

    use super::*;
    use crate::tool::ToolSpec;

    /// Tools that record how they were called.
    ///
    /// - `echo` returns its `text`.
    /// - `screenshot` returns a caption and a PNG.
    /// - `fail` reports an error.
    /// - `slow` never returns.
    /// - `panic` panics.
    #[derive(Default)]
    pub(crate) struct TestTools {
        calls: Mutex<Vec<(String, Value)>>,
    }

    impl TestTools {
        /// The tool set, ready to put in a request.
        pub fn shared() -> Arc<Self> {
            Arc::new(Self::default())
        }

        /// Every call so far, in the order the calls started.
        pub fn calls(&self) -> Vec<(String, Value)> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl ToolExecutor for TestTools {
        fn tools(&self) -> Vec<ToolSpec> {
            let schema = json!({
                "type": "object",
                "properties": { "text": { "type": "string" } },
                "additionalProperties": false
            });
            ["echo", "screenshot", "fail", "slow", "panic"]
                .into_iter()
                .map(|name| ToolSpec::new(name, format!("The {name} test tool."), schema.clone()))
                .collect()
        }

        async fn call(&self, name: &str, input: Value) -> ToolOutput {
            self.calls
                .lock()
                .unwrap()
                .push((name.to_owned(), input.clone()));
            let text = input.get("text").and_then(Value::as_str).unwrap_or("");
            match name {
                "echo" => ToolOutput::text(format!("echo: {text}")),
                "screenshot" => ToolOutput::text("frame at 0 ms").with_png(vec![1, 2, 3]),
                "fail" => ToolOutput::error("node cd34 does not exist"),
                "slow" => std::future::pending().await,
                "panic" => panic!("the test tool was asked to panic"),
                other => ToolOutput::error(format!("unknown tool: {other}")),
            }
        }
    }

    /// Runs one turn and returns every event it produced.
    pub(crate) async fn collect_events(
        provider: &dyn crate::provider::ChatProvider,
        request: ChatRequest,
    ) -> Vec<ChatEvent> {
        let (events, mut receiver) = crate::event::event_channel();
        provider
            .send(request, events, CancellationToken::new())
            .await;
        let mut collected = Vec::new();
        while let Some(event) = receiver.recv().await {
            collected.push(event);
        }
        collected
    }

    /// The text of the answer in `events`.
    pub(crate) fn answer(events: &[ChatEvent]) -> String {
        events
            .iter()
            .filter_map(|event| match event {
                ChatEvent::TextDelta { text } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::TestTools;
    use super::*;
    use crate::event::event_channel;

    fn call(id: &str, name: &str, input: Value) -> ToolCall {
        ToolCall {
            id: id.to_owned(),
            name: name.to_owned(),
            input: Ok(input),
        }
    }

    fn drain(receiver: &mut crate::event::EventReceiver) -> Vec<ChatEvent> {
        let mut events = Vec::new();
        while let Ok(event) = receiver.try_recv() {
            events.push(event);
        }
        events
    }

    #[test]
    fn the_conversation_ends_at_the_newest_user_message() {
        let request = ChatRequest {
            messages: vec![
                ChatMessage::user("first"),
                ChatMessage::assistant("reply"),
                ChatMessage::user("second"),
                ChatMessage::assistant("partial"),
            ],
            ..ChatRequest::default()
        };
        let messages = conversation(&request).unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[2].text, "second");

        let only_assistant = ChatRequest {
            messages: vec![ChatMessage::assistant("hello")],
            ..ChatRequest::default()
        };
        assert_eq!(
            conversation(&only_assistant).unwrap_err().kind(),
            ErrorKind::InvalidRequest
        );
    }

    #[test]
    fn the_default_model_is_used_when_none_is_named() {
        let mut request = ChatRequest::from_prompt("hi");
        assert_eq!(chosen_model(&request, "fallback-model"), "fallback-model");
        request.model = Some("default".to_owned());
        assert_eq!(chosen_model(&request, "fallback-model"), "fallback-model");
        request.model = Some("  ".to_owned());
        assert_eq!(chosen_model(&request, "fallback-model"), "fallback-model");
        request.model = Some("llama3.2".to_owned());
        assert_eq!(chosen_model(&request, "fallback-model"), "llama3.2");
    }

    #[tokio::test]
    async fn results_come_back_in_call_order_whatever_finishes_first() {
        let tools = TestTools::shared();
        let executor: Arc<dyn ToolExecutor> = tools.clone();
        let (events, mut receiver) = event_channel();
        let calls = [
            call("a", "screenshot", json!({})),
            call("b", "echo", json!({ "text": "ping" })),
            call("c", "fail", json!({})),
        ];
        let outputs = run_tools(Some(&executor), &calls, &events, &CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(outputs[0].images().count(), 1);
        assert_eq!(outputs[1].joined_text(), "echo: ping");
        assert!(outputs[2].is_error);
        assert_eq!(tools.calls().len(), 3);

        let events = drain(&mut receiver);
        // All three are announced before any result.
        assert!(
            events[..3]
                .iter()
                .all(|event| matches!(event, ChatEvent::ToolCallStarted { .. }))
        );
        let failed: Vec<&str> = events
            .iter()
            .filter_map(|event| match event {
                ChatEvent::ToolCallFinished { id, ok: false, .. } => Some(id.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(failed, ["c"]);
        assert_eq!(events.len(), 6);
    }

    #[tokio::test]
    async fn arguments_that_were_not_json_are_returned_without_running_the_tool() {
        let tools = TestTools::shared();
        let executor: Arc<dyn ToolExecutor> = tools.clone();
        let (events, mut receiver) = event_channel();
        let calls = [ToolCall {
            id: "a".to_owned(),
            name: "echo".to_owned(),
            input: Err("{\"text\": \"unfinished".to_owned()),
        }];
        let outputs = run_tools(Some(&executor), &calls, &events, &CancellationToken::new())
            .await
            .unwrap();
        assert!(outputs[0].is_error);
        let reply: Value = serde_json::from_str(&outputs[0].joined_text()).unwrap();
        assert_eq!(reply["INVALID_JSON"], "{\"text\": \"unfinished");
        assert!(tools.calls().is_empty());
        assert!(matches!(
            drain(&mut receiver)[1],
            ChatEvent::ToolCallFinished { ok: false, .. }
        ));
    }

    #[tokio::test]
    async fn a_request_without_tools_answers_calls_with_an_error() {
        let (events, _receiver) = event_channel();
        let calls = [call("a", "echo", json!({}))];
        let outputs = run_tools(None, &calls, &events, &CancellationToken::new())
            .await
            .unwrap();
        assert!(outputs[0].is_error);
    }

    #[tokio::test]
    async fn a_tool_that_panics_becomes_an_error_result() {
        let executor: Arc<dyn ToolExecutor> = TestTools::shared();
        let (events, _receiver) = event_channel();
        let calls = [
            call("a", "panic", json!({})),
            call("b", "echo", json!({ "text": "still here" })),
        ];
        let outputs = run_tools(Some(&executor), &calls, &events, &CancellationToken::new())
            .await
            .unwrap();
        assert!(outputs[0].is_error);
        assert_eq!(outputs[1].joined_text(), "echo: still here");
    }

    #[tokio::test]
    async fn cancelling_abandons_tools_that_are_still_running() {
        let executor: Arc<dyn ToolExecutor> = TestTools::shared();
        let (events, _receiver) = event_channel();
        let cancel = CancellationToken::new();
        let calls = [call("a", "slow", json!({}))];
        let trigger = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            trigger.cancel();
        });
        let started = std::time::Instant::now();
        assert!(
            run_tools(Some(&executor), &calls, &events, &cancel)
                .await
                .is_none()
        );
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
    }

    #[test]
    fn usage_adds_up_over_requests() {
        let (events, mut receiver) = event_channel();
        let mut totals = UsageTotals::default();
        totals.add(100, 10, &events);
        totals.add(0, 0, &events);
        totals.add(150, 5, &events);
        assert_eq!(
            drain(&mut receiver),
            [
                ChatEvent::Usage {
                    input_tokens: 100,
                    output_tokens: 10,
                    cost_usd: None
                },
                ChatEvent::Usage {
                    input_tokens: 250,
                    output_tokens: 15,
                    cost_usd: None
                },
            ]
        );
    }
}
