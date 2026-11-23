//! The Anthropic provider: Claude through the Messages API, with the user's
//! own API key.
//!
//! One turn is a loop. The conversation is sent with the app's tools; the
//! answer streams back; if the model asked for tools they are run, all of a
//! step at the same time, and their results are sent in the next request.
//! This repeats until the model answers without asking for a tool.
//!
//! Request settings follow Anthropic's current guidance: API version
//! `2026-10-21`, adaptive thinking with summaries, no sampling parameters,
//! the system prompt and tools cached, and a `refusal` stop reason handled
//! like any other. Requests to Anthropic's own address also ask for tool
//! arguments to be streamed as they are written and for a request that a
//! safety classifier declined to be re-run on another model.
//!
//! Not verified against the live API: no key was available when this was
//! written. It is tested against a local server that plays the documented
//! event stream.

mod request;
mod stream;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use self::request::{FALLBACK_BETA, RequestShape};
use self::stream::{MessageStream, Progress, StreamedMessage, classify_error_type};
use super::{UsageTotals, chosen_model, conversation, run_tools};
use crate::chat::ChatRequest;
use crate::error::{AiError, ErrorKind};
use crate::event::{EventSender, StopReason};
use crate::http::sse::SseParser;
use crate::http::{
    self, AttemptError, Failure, Interrupt, RetryPolicy, join_url, status_failure, with_retries,
};
use crate::provider::{
    Capabilities, ChatProvider, ModelInfo, ProviderInfo, ProviderKind, Readiness, ReadyState,
    ToolTransport,
};
use crate::redact::ApiKey;

/// The provider id used in settings and on the command line.
pub const ANTHROPIC_ID: &str = "anthropic";

/// Where Anthropic's own API lives.
pub const ANTHROPIC_API_URL: &str = "https://api.anthropic.com";

/// The model used when a request names none.
pub const ANTHROPIC_DEFAULT_MODEL: &str = "claude-opus-5-5";

/// The API version every request is written for.
const API_VERSION: &str = "2026-10-21";

/// How much thinking and tool use the model puts into an answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Effort {
    /// Fast and cheap, for simple requests.
    Low,
    /// A balance of speed and care.
    Medium,
    /// More thorough.
    High,
    /// Between high and max.
    #[serde(rename = "xhigh")]
    XHigh,
    /// As thorough as the model can be, at the highest cost.
    Max,
}

impl Effort {
    fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::XHigh => "xhigh",
            Self::Max => "max",
        }
    }
}

/// Settings for the Anthropic provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnthropicOptions {
    /// The address of the API, without `/v1`. Change it to go through a
    /// gateway or proxy.
    pub base_url: String,
    /// The most the model may write in one request. Nothing is charged for
    /// room that is not used.
    pub max_tokens: u32,
    /// How thorough the model is. `None` leaves it at the model's default.
    pub effort: Option<Effort>,
    /// How many rounds of tool calls one turn may take before it is
    /// stopped.
    pub max_tool_rounds: u32,
    /// Ask the API to re-run a request that a safety classifier declined on
    /// another model, where the model supports it.
    pub refusal_fallback: bool,
    /// Whether to send the fields that only Anthropic's own API is known to
    /// accept: streamed tool arguments, automatic cache placement and
    /// refusal fallbacks. `None` decides from [`base_url`](Self::base_url).
    pub first_party_features: Option<bool>,
}

impl Default for AnthropicOptions {
    fn default() -> Self {
        Self {
            base_url: ANTHROPIC_API_URL.to_owned(),
            max_tokens: 64_000,
            effort: None,
            max_tool_rounds: 30,
            refusal_fallback: true,
            first_party_features: None,
        }
    }
}

/// Chat with Claude through the Anthropic API.
#[derive(Debug, Clone)]
pub struct Anthropic {
    key: Option<ApiKey>,
    options: AnthropicOptions,
    retry: RetryPolicy,
}

impl Anthropic {
    /// A provider with default options. `key` is the user's API key, when
    /// one is stored.
    pub fn new(key: Option<ApiKey>) -> Self {
        Self::with_options(key, AnthropicOptions::default())
    }

    /// A provider with the given options.
    pub fn with_options(key: Option<ApiKey>, options: AnthropicOptions) -> Self {
        Self {
            key: key.filter(|key| !key.is_empty()),
            options,
            retry: RetryPolicy::default(),
        }
    }

    /// Replaces the retry timing, so tests do not wait.
    #[cfg(test)]
    fn with_retry_policy(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    fn key(&self) -> Result<&ApiKey, AiError> {
        self.key.as_ref().ok_or_else(|| AiError::MissingKey {
            provider: "Anthropic".to_owned(),
        })
    }

    fn first_party(&self) -> bool {
        self.options.first_party_features.unwrap_or_else(|| {
            self.options
                .base_url
                .trim()
                .trim_end_matches('/')
                .eq_ignore_ascii_case(ANTHROPIC_API_URL)
        })
    }

    fn url(&self, path: &str) -> String {
        join_url(&self.options.base_url, path)
    }

    fn get(&self, client: &reqwest::Client, key: &ApiKey, path: &str) -> reqwest::RequestBuilder {
        client
            .get(self.url(path))
            .header("x-api-key", key.expose())
            .header("anthropic-version", API_VERSION)
    }

    async fn run(
        &self,
        request: &ChatRequest,
        events: &EventSender,
        cancel: &CancellationToken,
    ) -> Result<StopReason, Interrupt> {
        let key = self.key()?;
        let mut messages = request::initial_messages(conversation(request)?);
        if messages.is_empty() {
            return Err(AiError::from_kind(
                ErrorKind::InvalidRequest,
                "the message to answer is empty",
            )
            .into());
        }
        let executor = request.tools.clone();
        let tools = executor
            .as_ref()
            .map(|executor| executor.tools())
            .unwrap_or_default();
        let shape = RequestShape {
            model: chosen_model(request, ANTHROPIC_DEFAULT_MODEL),
            max_tokens: self.options.max_tokens,
            effort: self.options.effort.map(Effort::as_str),
            system: request.system_prompt.as_deref(),
            tools: &tools,
            first_party: self.first_party(),
            refusal_fallback: self.options.refusal_fallback,
        };
        let client = http::client()?;
        let mut usage = UsageTotals::default();
        let mut rounds = 0;

        loop {
            let body = request::body(&shape, &messages);
            let message = with_retries(&self.retry, cancel, events, || {
                self.stream_once(&client, key, &body, shape.uses_fallbacks(), events, cancel)
            })
            .await?;
            usage.add(message.input_tokens, message.output_tokens, events);

            match message.stop_reason.as_deref() {
                Some("tool_use") if !message.tool_calls.is_empty() => {
                    if rounds >= self.options.max_tool_rounds {
                        return Ok(StopReason::ToolLoopLimit);
                    }
                    rounds += 1;
                    let Some(outputs) =
                        run_tools(executor.as_ref(), &message.tool_calls, events, cancel).await
                    else {
                        return Err(Interrupt::Cancelled);
                    };
                    messages.push(json!({ "role": "assistant", "content": message.content }));
                    messages.push(request::tool_results_message(&message.tool_calls, &outputs));
                }
                // The API paused a long turn; sending the message back as
                // it is makes it carry on.
                Some("pause_turn") if !message.content.is_empty() => {
                    if rounds >= self.options.max_tool_rounds {
                        return Ok(StopReason::ToolLoopLimit);
                    }
                    rounds += 1;
                    messages.push(json!({ "role": "assistant", "content": message.content }));
                }
                Some("max_tokens" | "model_context_window_exceeded") => {
                    return Ok(StopReason::MaxTokens);
                }
                // A classifier or the model declined. Tools from this
                // message are never run.
                Some("refusal") => return Ok(StopReason::Refusal),
                _ => return Ok(StopReason::EndTurn),
            }
        }
    }

    /// Sends one request and reads its stream to the end.
    ///
    /// A failure counts as retryable only while nothing from this response
    /// has been shown to the user: starting over after that would repeat
    /// text on screen.
    async fn stream_once(
        &self,
        client: &reqwest::Client,
        key: &ApiKey,
        body: &Value,
        with_fallbacks: bool,
        events: &EventSender,
        cancel: &CancellationToken,
    ) -> Result<StreamedMessage, AttemptError> {
        let mut request = client
            .post(self.url("v1/messages"))
            .header("x-api-key", key.expose())
            .header("anthropic-version", API_VERSION)
            .header("content-type", "application/json")
            .header("accept", "text/event-stream")
            .body(body.to_string());
        if with_fallbacks {
            request = request.header("anthropic-beta", FALLBACK_BETA);
        }
        let mut response = http::send(request, Some(key), cancel, events).await?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let headers = response.headers().clone();
            let text = http::read_error_body(response).await;
            return Err(error_response(status, &headers, &text, key).into());
        }

        let mut parser = SseParser::new();
        let mut stream = MessageStream::new();
        let only_before_output = |mut failure: Failure, stream: &MessageStream| {
            failure.retryable &= !stream.has_emitted();
            AttemptError::Failed(failure)
        };
        loop {
            let chunk = match http::next_chunk(&mut response, Some(key), cancel, events).await {
                Ok(Some(chunk)) => chunk,
                Ok(None) => break,
                Err(AttemptError::Failed(failure)) => {
                    return Err(only_before_output(failure, &stream));
                }
                Err(cancelled) => return Err(cancelled),
            };
            for event in parser.push(&chunk) {
                match stream.on_event(&event, events, Some(key)) {
                    Ok(Progress::Continue) => {}
                    Ok(Progress::Done) => {
                        return stream.finish().map_err(AttemptError::Failed);
                    }
                    Err(failure) => return Err(only_before_output(failure, &stream)),
                }
            }
        }
        if let Some(event) = parser.finish()
            && let Err(failure) = stream.on_event(&event, events, Some(key))
        {
            return Err(only_before_output(failure, &stream));
        }
        let emitted = stream.has_emitted();
        stream.finish().map_err(|mut failure| {
            failure.retryable &= !emitted;
            AttemptError::Failed(failure)
        })
    }
}

#[async_trait]
impl ChatProvider for Anthropic {
    fn info(&self) -> ProviderInfo {
        ProviderInfo {
            id: ANTHROPIC_ID.to_owned(),
            name: "Anthropic".to_owned(),
            kind: ProviderKind::Api,
        }
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            tools: ToolTransport::InProcess,
            images: true,
            resume: false,
            thinking: true,
            live_model_list: true,
            needs_api_key: true,
            // Tested against a local server only; no key was available.
            verified: false,
        }
    }

    /// The models the key can use, as the API lists them, with the default
    /// model moved to the front.
    async fn list_models(&self) -> Result<Vec<ModelInfo>, AiError> {
        let key = self.key()?;
        let client = http::client()?;
        let mut models = Vec::new();
        let mut after: Option<String> = None;
        // Far more pages than the API has models; a guard against a server
        // that never says it is done.
        for _ in 0..20 {
            let mut request = self
                .get(&client, key, "v1/models")
                .query(&[("limit", "1000")]);
            if let Some(after) = &after {
                request = request.query(&[("after_id", after.as_str())]);
            }
            let fetched = http::fetch(request, Some(key))
                .await
                .map_err(|failure| failure.error)?;
            if fetched.status != 200 {
                return Err(
                    error_response(fetched.status, &fetched.headers, &fetched.body, key).error,
                );
            }
            let page: Value = serde_json::from_str(&fetched.body)
                .map_err(|_| AiError::protocol("the model list is not JSON"))?;
            for model in page
                .get("data")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let Some(id) = model.get("id").and_then(Value::as_str) else {
                    continue;
                };
                let name = model
                    .get("display_name")
                    .and_then(Value::as_str)
                    .unwrap_or(id);
                models.push(ModelInfo::new(id, name));
            }
            after = page
                .get("last_id")
                .and_then(Value::as_str)
                .map(str::to_owned);
            let more = page
                .get("has_more")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if !more || after.is_none() {
                break;
            }
        }
        if let Some(position) = models
            .iter()
            .position(|model| model.id == ANTHROPIC_DEFAULT_MODEL)
        {
            let default = models.remove(position);
            models.insert(0, default);
        }
        Ok(models)
    }

    /// Checks the key by listing a single model, which costs nothing.
    async fn check_ready(&self) -> Readiness {
        let Ok(key) = self.key() else {
            return Readiness::not_ready(
                ReadyState::MissingKey,
                "Add an Anthropic API key. You can create one at https://platform.claude.com.",
            );
        };
        let client = match http::client() {
            Ok(client) => client,
            Err(error) => return Readiness::from_error(&error),
        };
        let request = self.get(&client, key, "v1/models").query(&[("limit", "1")]);
        match http::fetch(request, Some(key)).await {
            Ok(fetched) if fetched.status == 200 => {
                Readiness::ready("The Anthropic API key works.")
            }
            Ok(fetched) => Readiness::from_error(
                &error_response(fetched.status, &fetched.headers, &fetched.body, key).error,
            ),
            Err(failure) => Readiness::from_error(&failure.error),
        }
    }

    async fn send(&self, request: ChatRequest, events: EventSender, cancel: CancellationToken) {
        let outcome = match self.run(&request, &events, &cancel).await {
            Ok(stop_reason) => Ok(stop_reason),
            Err(Interrupt::Cancelled) => Ok(StopReason::Cancelled),
            Err(Interrupt::Failed(error)) => Err(error),
        };
        events.finish(outcome);
    }
}

/// The failure for an error response. The body names the kind of error
/// more precisely than the status does.
fn error_response(
    status: u16,
    headers: &reqwest::header::HeaderMap,
    body: &str,
    key: &ApiKey,
) -> Failure {
    let parsed: Option<Value> = serde_json::from_str(body).ok();
    let field = |name: &str| {
        parsed
            .as_ref()
            .and_then(|value| value.pointer(name))
            .and_then(Value::as_str)
    };
    let kind = field("/error/type")
        .and_then(classify_error_type)
        .map(|(kind, _)| kind);
    let message = match field("/error/message") {
        Some(message) => message.to_owned(),
        // Not the API's own error format: a gateway or proxy answered.
        None => crate::tool::summarize_text(body),
    };
    status_failure(status, kind, &message, headers, Some(key))
}

#[cfg(test)]
mod tests;
