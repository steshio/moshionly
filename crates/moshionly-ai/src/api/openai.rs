//! The OpenAI-compatible provider: any service that speaks the Chat
//! Completions format, from OpenAI itself to a model running on the user's
//! own machine.
//!
//! Which service is a matter of data, see [`presets`]: an address, whether a
//! key is needed, and what the service's models accept. One turn is the same
//! loop as for the Anthropic provider: send the conversation with the app's
//! tools as functions, stream the answer, run the tools the model asked for,
//! send their results, repeat.
//!
//! Not verified against live services: no keys were available when this was
//! written. It is tested against a local server that plays the format.

pub(crate) mod presets;
mod request;
mod stream;

use async_trait::async_trait;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use self::presets::OpenAiPreset;
use self::request::RequestShape;
use self::stream::{AssistantTurn, ChatStream, Progress, error_failure};
use super::{UsageTotals, chosen_model, conversation, run_tools};
use crate::chat::ChatRequest;
use crate::error::{AiError, ErrorKind};
use crate::event::{EventSender, StopReason};
use crate::http::sse::SseParser;
use crate::http::{self, AttemptError, Failure, Interrupt, RetryPolicy, join_url, with_retries};
use crate::provider::{
    Capabilities, ChatProvider, ModelInfo, ProviderInfo, ProviderKind, Readiness, ReadyState,
    ToolTransport,
};
use crate::redact::ApiKey;

/// Words in a model id that mark a model as not for chat. Services list
/// these next to their chat models.
const NOT_CHAT_MODELS: &[&str] = &[
    "embedding",
    "embed-",
    "whisper",
    "tts",
    "dall-e",
    "moderation",
    "transcribe",
    "realtime",
    "rerank",
    "imagine",
    "-image",
    "image-",
    "veo-",
    "imagen",
    "lyria",
    "aqa",
    "prompt-guard",
];

/// Settings for the OpenAI-compatible provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenAiCompatibleOptions {
    /// The address to use in place of the preset's, up to and including
    /// the version segment (`https://host/v1`). Required for the custom
    /// preset.
    pub base_url: Option<String>,
    /// The model to use when a request names none.
    pub model: Option<String>,
    /// Whether the model can be sent images. `None` takes the preset's
    /// answer.
    pub images: Option<bool>,
    /// Ask for token counts in the stream. A few older servers reject the
    /// request field for it.
    pub stream_usage: bool,
    /// How many rounds of tool calls one turn may take before it is
    /// stopped.
    pub max_tool_rounds: u32,
}

impl Default for OpenAiCompatibleOptions {
    fn default() -> Self {
        Self {
            base_url: None,
            model: None,
            images: None,
            stream_usage: true,
            max_tool_rounds: 30,
        }
    }
}

/// Chat with a service that speaks the OpenAI Chat Completions format.
#[derive(Debug, Clone)]
pub struct OpenAiCompatible {
    preset: OpenAiPreset,
    key: Option<ApiKey>,
    options: OpenAiCompatibleOptions,
    retry: RetryPolicy,
}

impl OpenAiCompatible {
    /// A provider for `preset` with default options. `key` is the user's
    /// API key, when one is stored.
    pub fn new(preset: OpenAiPreset, key: Option<ApiKey>) -> Self {
        Self::with_options(preset, key, OpenAiCompatibleOptions::default())
    }

    /// A provider for `preset` with the given options.
    pub fn with_options(
        preset: OpenAiPreset,
        key: Option<ApiKey>,
        options: OpenAiCompatibleOptions,
    ) -> Self {
        Self {
            preset,
            key: key.filter(|key| !key.is_empty()),
            options,
            retry: RetryPolicy::default(),
        }
    }

    /// The preset this provider was made from.
    pub fn preset(&self) -> &OpenAiPreset {
        &self.preset
    }

    /// Replaces the retry timing, so tests do not wait.
    #[cfg(test)]
    fn with_retry_policy(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    fn base_url(&self) -> Result<&str, AiError> {
        let chosen = self
            .options
            .base_url
            .as_deref()
            .map(str::trim)
            .filter(|url| !url.is_empty())
            .unwrap_or(self.preset.base_url);
        if chosen.is_empty() {
            return Err(AiError::from_kind(
                ErrorKind::InvalidRequest,
                format!("no server address is set for {}", self.preset.name),
            ));
        }
        Ok(chosen)
    }

    /// The key to send, or an error when the service needs one and none is
    /// stored. A stored key is sent even where none is required: a custom
    /// server, or a local one behind a proxy, may ask for it.
    fn key(&self) -> Result<Option<&ApiKey>, AiError> {
        match &self.key {
            Some(key) => Ok(Some(key)),
            None if self.preset.needs_api_key => Err(AiError::MissingKey {
                provider: self.preset.name.to_owned(),
            }),
            None => Ok(None),
        }
    }

    fn images(&self) -> bool {
        self.options.images.unwrap_or(self.preset.images)
    }

    fn authorized(
        &self,
        request: reqwest::RequestBuilder,
        key: Option<&ApiKey>,
    ) -> reqwest::RequestBuilder {
        match key {
            Some(key) => request.bearer_auth(key.expose()),
            None => request,
        }
    }

    /// The model for a request: its own choice, then the one from settings,
    /// then the preset's first suggestion, then the first model the server
    /// lists.
    async fn model_for(&self, request: &ChatRequest) -> Result<String, AiError> {
        let configured = self
            .options
            .model
            .as_deref()
            .map(str::trim)
            .filter(|model| !model.is_empty());
        let fallback = configured.or(self.preset.suggested_models.first().copied());
        let chosen = chosen_model(request, fallback.unwrap_or(""));
        if !chosen.is_empty() {
            return Ok(chosen.to_owned());
        }
        let models = self.list_models().await?;
        models
            .into_iter()
            .next()
            .map(|model| model.id)
            .ok_or_else(|| {
                AiError::from_kind(
                    ErrorKind::ModelUnavailable,
                    format!(
                        "{} has no models to choose from; install or select one first",
                        self.preset.name
                    ),
                )
            })
    }

    async fn run(
        &self,
        request: &ChatRequest,
        events: &EventSender,
        cancel: &CancellationToken,
    ) -> Result<StopReason, Interrupt> {
        let base_url = self.base_url()?;
        let key = self.key()?;
        let images = self.images();
        let mut messages = request::initial_messages(
            request.system_prompt.as_deref(),
            conversation(request)?,
            images,
        );
        if !messages.iter().any(|message| message["role"] == "user") {
            return Err(AiError::from_kind(
                ErrorKind::InvalidRequest,
                "the message to answer is empty",
            )
            .into());
        }
        let model = self.model_for(request).await?;
        let executor = request.tools.clone();
        let tools = executor
            .as_ref()
            .map(|executor| executor.tools())
            .unwrap_or_default();
        let shape = RequestShape {
            model: &model,
            tools: &tools,
            stream_usage: self.options.stream_usage,
        };
        let client = http::client()?;
        let url = join_url(base_url, "chat/completions");
        let mut usage = UsageTotals::default();
        let mut rounds = 0;

        loop {
            let body = request::body(&shape, &messages);
            let turn = with_retries(&self.retry, cancel, events, || {
                self.stream_once(&client, &url, key, &body, events, cancel)
            })
            .await?;
            usage.add(turn.input_tokens, turn.output_tokens, events);

            let finish = turn.finish_reason.as_deref();
            if finish == Some("length") {
                // Cut off at the output limit; a tool call in this message
                // may be half written, so none is run.
                return Ok(StopReason::MaxTokens);
            }
            if finish == Some("content_filter") {
                return Ok(StopReason::Refusal);
            }
            if turn.tool_calls.is_empty() {
                return Ok(StopReason::EndTurn);
            }
            if rounds >= self.options.max_tool_rounds {
                return Ok(StopReason::ToolLoopLimit);
            }
            rounds += 1;
            let Some(outputs) =
                run_tools(executor.as_ref(), &turn.tool_calls, events, cancel).await
            else {
                return Err(Interrupt::Cancelled);
            };
            messages.push(request::assistant_message(&turn));
            messages.extend(request::tool_result_messages(
                &turn.tool_calls,
                &outputs,
                images,
            ));
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
        url: &str,
        key: Option<&ApiKey>,
        body: &Value,
        events: &EventSender,
        cancel: &CancellationToken,
    ) -> Result<AssistantTurn, AttemptError> {
        let request = self
            .authorized(client.post(url), key)
            .header("content-type", "application/json")
            .header("accept", "text/event-stream")
            .body(body.to_string());
        let mut response = http::send(request, key, cancel, events).await?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let headers = response.headers().clone();
            let text = http::read_error_body(response).await;
            return Err(error_response(status, &headers, &text, key).into());
        }

        let mut parser = SseParser::new();
        let mut stream = ChatStream::new();
        let only_before_output = |mut failure: Failure, stream: &ChatStream| {
            failure.retryable &= !stream.has_emitted();
            AttemptError::Failed(failure)
        };
        let mut saw_done = false;
        'read: loop {
            let chunk = match http::next_chunk(&mut response, key, cancel, events).await {
                Ok(Some(chunk)) => chunk,
                Ok(None) => break,
                Err(AttemptError::Failed(failure)) => {
                    return Err(only_before_output(failure, &stream));
                }
                Err(cancelled) => return Err(cancelled),
            };
            for event in parser.push(&chunk) {
                match stream.on_data(&event.data, events, key) {
                    Ok(Progress::Continue) => {}
                    Ok(Progress::Done) => {
                        saw_done = true;
                        break 'read;
                    }
                    Err(failure) => return Err(only_before_output(failure, &stream)),
                }
            }
        }
        if !saw_done && let Some(event) = parser.finish() {
            match stream.on_data(&event.data, events, key) {
                Ok(Progress::Done) => saw_done = true,
                Ok(Progress::Continue) => {}
                Err(failure) => return Err(only_before_output(failure, &stream)),
            }
        }
        let emitted = stream.has_emitted();
        stream.finish(saw_done).map_err(|mut failure| {
            failure.retryable &= !emitted;
            AttemptError::Failed(failure)
        })
    }

    async fn fetch_models(&self) -> Result<Vec<ModelInfo>, AiError> {
        let base_url = self.base_url()?;
        let key = self.key()?;
        let client = http::client()?;
        let request = self.authorized(client.get(join_url(base_url, "models")), key);
        let fetched = http::fetch(request, key)
            .await
            .map_err(|failure| failure.error)?;
        if fetched.status != 200 {
            let failure = error_response(fetched.status, &fetched.headers, &fetched.body, key);
            // Here a 404 is about the address, not about a model.
            return Err(match failure.error {
                AiError::ModelUnavailable(_) => AiError::other(format!(
                    "{} has no model list at this address; check the server address",
                    self.preset.name
                )),
                other => other,
            });
        }
        let list: Value = serde_json::from_str(&fetched.body)
            .map_err(|_| AiError::protocol("the model list is not JSON"))?;
        // `{"data": [...]}` everywhere; a bare list from a few servers.
        let entries = list.get("data").or(Some(&list)).and_then(Value::as_array);
        Ok(entries
            .into_iter()
            .flatten()
            .filter_map(|model| {
                let id = model.get("id").and_then(Value::as_str)?;
                let name = model
                    .get("name")
                    .or_else(|| model.get("display_name"))
                    .and_then(Value::as_str)
                    .unwrap_or_else(|| id.strip_prefix("models/").unwrap_or(id));
                Some(ModelInfo::new(id, name))
            })
            .collect())
    }
}

#[async_trait]
impl ChatProvider for OpenAiCompatible {
    fn info(&self) -> ProviderInfo {
        ProviderInfo {
            id: self.preset.id.to_owned(),
            name: self.preset.name.to_owned(),
            kind: ProviderKind::Api,
        }
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            tools: ToolTransport::InProcess,
            images: self.images(),
            resume: false,
            // Shown when a service sends it; many do not.
            thinking: true,
            live_model_list: true,
            needs_api_key: self.preset.needs_api_key,
            // Tested against a local server only; no keys were available.
            verified: false,
        }
    }

    /// The service's chat models, with the default one in front: the model
    /// from settings, or the preset's first suggestion that the service
    /// offers.
    async fn list_models(&self) -> Result<Vec<ModelInfo>, AiError> {
        let mut models = self.fetch_models().await?;
        models.retain(|model| {
            let id = model.id.to_ascii_lowercase();
            !NOT_CHAT_MODELS.iter().any(|word| id.contains(word))
        });
        let same = |id: &str, wanted: &str| {
            id == wanted
                || id
                    .strip_prefix("models/")
                    .is_some_and(|rest| rest == wanted)
        };
        let preferred = self
            .options
            .model
            .as_deref()
            .into_iter()
            .chain(self.preset.suggested_models.iter().copied())
            .find_map(|wanted| models.iter().position(|model| same(&model.id, wanted)));
        if let Some(position) = preferred {
            let default = models.remove(position);
            models.insert(0, default);
        }
        Ok(models)
    }

    /// Checks the address and the key by asking for the model list, which
    /// costs nothing.
    async fn check_ready(&self) -> Readiness {
        let name = self.preset.name;
        let Ok(base_url) = self.base_url() else {
            return Readiness::not_ready(
                ReadyState::Unknown,
                format!("Enter the address of the server to use for {name}."),
            );
        };
        if self.key().is_err() {
            let hint = if self.preset.key_url.is_empty() {
                String::new()
            } else {
                format!(" You can create one at {}.", self.preset.key_url)
            };
            return Readiness::not_ready(
                ReadyState::MissingKey,
                format!("Add an API key for {name}.{hint}"),
            );
        }
        match self.list_models().await {
            Ok(models) if self.preset.local && models.is_empty() => Readiness::not_ready(
                ReadyState::Unknown,
                format!("{name} is running but has no models yet. Download one in {name} first."),
            ),
            Ok(models) => Readiness::ready(format!(
                "{name} answered with {} model{}.",
                models.len(),
                if models.len() == 1 { "" } else { "s" }
            )),
            Err(error) if error.kind() == ErrorKind::Network && self.preset.local => {
                Readiness::not_ready(
                    ReadyState::Unreachable,
                    format!("{name} is not running at {base_url}. Start it and try again."),
                )
            }
            Err(error) => Readiness::from_error(&error),
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

/// The failure for an error response.
fn error_response(
    status: u16,
    headers: &reqwest::header::HeaderMap,
    body: &str,
    key: Option<&ApiKey>,
) -> Failure {
    let parsed: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let mut failure = error_failure(Some(status), &parsed, body, key);
    failure.retry_after = http::retry_after(headers);
    if let AiError::RateLimited { retry_after, .. } = &mut failure.error {
        *retry_after = failure.retry_after;
    }
    failure
}

#[cfg(test)]
mod tests;
