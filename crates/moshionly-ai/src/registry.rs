//! Every provider in one place, built from the user's settings.
//!
//! [`known_providers`] is the fixed list: who the providers are and what
//! they can do, enough to draw a settings screen at once. A
//! [`ProviderRegistry`] joins that list with the user's [`AiSettings`] and a
//! [`KeyStore`]: it says whether each provider can be used right now, and it
//! builds the provider to chat with.

use std::collections::HashMap;
use std::fmt;
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::api::anthropic::{ANTHROPIC_API_URL, ANTHROPIC_ID, Anthropic, AnthropicOptions};
use crate::api::openai::presets::{OPENAI_PRESETS, openai_preset};
use crate::api::openai::{OpenAiCompatible, OpenAiCompatibleOptions};
use crate::chat::ChatRequest;
use crate::cli::claude_code::{CLAUDE_CODE_ID, ClaudeCode, ClaudeCodeOptions};
use crate::cli::codex::{CODEX_ID, Codex, CodexOptions};
use crate::cli::cursor_agent::{CURSOR_AGENT_ID, CursorAgent, CursorAgentOptions};
use crate::cli::gemini::{GEMINI_CLI_ID, GeminiCli, GeminiCliOptions};
use crate::error::AiError;
use crate::event::EventSender;
use crate::keys::{KeyStore, KeyStoreError, KeychainStore};
use crate::provider::{
    Capabilities, ChatProvider, ModelInfo, ProviderInfo, ProviderKind, Readiness, ReadyState,
};
use crate::redact::{ApiKey, scrub};
use crate::settings::{AiSettings, ProviderSettings, SettingsError};

/// The subscription providers, in the order a settings screen lists them.
const SUBSCRIPTION_IDS: [&str; 4] = [CLAUDE_CODE_ID, CURSOR_AGENT_ID, CODEX_ID, GEMINI_CLI_ID];

/// Where the user can create an Anthropic API key.
const ANTHROPIC_KEY_URL: &str = "https://platform.claude.com";

/// What is known about a provider before anything is checked: enough to
/// draw its row in a settings screen straight away.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderDescriptor {
    /// Who the provider is.
    pub info: ProviderInfo,
    /// What it can do with its default options.
    pub capabilities: Capabilities,
    /// The server address used when the settings name none. `None` for
    /// subscription providers, and for `custom`, where the user has to
    /// enter one.
    pub default_base_url: Option<String>,
    /// Where the user can create an API key, when the service has such a
    /// page.
    pub key_url: Option<String>,
}

/// Every provider this crate has, in the order a settings screen should
/// list them: subscriptions first, then the API providers.
pub fn known_providers() -> Vec<ProviderDescriptor> {
    provider_ids()
        .into_iter()
        .filter_map(|id| {
            let provider = build(id, &ProviderSettings::default(), None)?;
            let preset = openai_preset(id);
            let default_base_url = match id {
                ANTHROPIC_ID => Some(ANTHROPIC_API_URL),
                _ => preset.map(|preset| preset.base_url),
            };
            let key_url = match id {
                ANTHROPIC_ID => Some(ANTHROPIC_KEY_URL),
                _ => preset.map(|preset| preset.key_url),
            };
            let stated =
                |text: Option<&str>| text.filter(|text| !text.is_empty()).map(str::to_owned);
            Some(ProviderDescriptor {
                info: provider.info(),
                capabilities: provider.capabilities(),
                default_base_url: stated(default_base_url),
                key_url: stated(key_url),
            })
        })
        .collect()
}

/// One provider as it stands right now, for a row in a settings screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderStatus {
    /// Who the provider is.
    pub info: ProviderInfo,
    /// What it can do with the user's settings applied.
    pub capabilities: Capabilities,
    /// Whether the user wants it offered in the chat.
    pub enabled: bool,
    /// Whether an API key is stored for it. Always false for subscription
    /// providers, which take none.
    pub key_stored: bool,
    /// Whether it can be used, and what to do when it cannot.
    pub readiness: Readiness,
}

impl ProviderStatus {
    /// True when the provider can be offered in the chat: the user left it
    /// enabled and it is ready.
    pub fn is_usable(&self) -> bool {
        self.enabled && self.readiness.is_ready()
    }
}

/// Picks the provider the chat should open with: the user's default when it
/// can be used, otherwise the first provider that can. `None` when nothing
/// is usable, which is when to send the user to the settings screen.
pub fn choose_provider<'a>(
    settings: &AiSettings,
    statuses: &'a [ProviderStatus],
) -> Option<&'a ProviderStatus> {
    let usable = || statuses.iter().filter(|status| status.is_usable());
    settings
        .default_provider()
        .and_then(|wanted| usable().find(|status| status.info.id == wanted))
        .or_else(|| usable().next())
}

/// Why the registry could not do what was asked.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegistryError {
    /// No provider has this id.
    #[error("there is no provider with the id `{0}`")]
    UnknownProvider(String),
    /// A key was offered to a subscription provider.
    #[error("{0} signs in through its own app and takes no API key")]
    NotAnApiProvider(String),
    /// The key store failed, so it is not known whether a key is stored.
    #[error(transparent)]
    KeyStore(#[from] KeyStoreError),
    /// The settings for the provider cannot be used as they are.
    #[error(transparent)]
    Settings(#[from] SettingsError),
}

/// The providers as the user configured them.
///
/// Cheap to clone. Its methods are `async` because reading a key can wait
/// on the operating system; they need a Tokio runtime, like the rest of
/// this crate.
#[derive(Clone)]
pub struct ProviderRegistry {
    settings: AiSettings,
    keys: Arc<dyn KeyStore>,
}

impl fmt::Debug for ProviderRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderRegistry")
            .field("settings", &self.settings)
            .finish_non_exhaustive()
    }
}

impl ProviderRegistry {
    /// A registry over the given settings and key store.
    pub fn new(settings: AiSettings, keys: Arc<dyn KeyStore>) -> Self {
        Self { settings, keys }
    }

    /// A registry that keeps keys in the operating system's keychain, which
    /// is what the app uses.
    pub fn with_keychain(settings: AiSettings) -> Self {
        Self::new(settings, Arc::new(KeychainStore::new()))
    }

    /// The settings in use.
    pub fn settings(&self) -> &AiSettings {
        &self.settings
    }

    /// The settings, for changing them. Providers built afterwards use the
    /// new values; saving them is up to the caller
    /// ([`AiSettings::save`]).
    pub fn settings_mut(&mut self) -> &mut AiSettings {
        &mut self.settings
    }

    /// The key store in use.
    pub fn keys(&self) -> &Arc<dyn KeyStore> {
        &self.keys
    }

    /// Builds the provider with the given id, ready to use: the user's
    /// options applied, their API key attached, and their chosen model used
    /// whenever a request names none.
    ///
    /// Building does not check that the provider works; a missing CLI or
    /// key shows up as an error event when a message is sent, and
    /// beforehand in [`status`](Self::status). A provider that the user
    /// disabled is still built, so that a settings screen can test it.
    pub async fn provider(&self, id: &str) -> Result<Box<dyn ChatProvider>, RegistryError> {
        let kind = kind_of(id).ok_or_else(|| RegistryError::UnknownProvider(id.to_owned()))?;
        let settings = self.settings.provider(id);
        if settings.address_has_secret() {
            return Err(SettingsError::SecretInAddress {
                provider: id.to_owned(),
            }
            .into());
        }
        let key = match kind {
            ProviderKind::Api => self.read_key(id).await?,
            // A subscription is signed in to in its own CLI. The key store
            // is not even asked.
            ProviderKind::Subscription => None,
        };
        configured(id, settings, key).ok_or_else(|| RegistryError::UnknownProvider(id.to_owned()))
    }

    /// Checks one provider: is the CLI installed and signed in, is there a
    /// key and does the service accept it, does the local server answer.
    /// Nothing is sent to a model and nothing is spent.
    ///
    /// Returns `None` when no provider has this id. Problems with the
    /// settings or the key store are not errors here; they are the reason
    /// given in [`ProviderStatus::readiness`].
    pub async fn status(&self, id: &str) -> Option<ProviderStatus> {
        let kind = kind_of(id)?;
        let mut settings = self.settings.provider(id).clone();
        let mut problem = None;
        if settings.address_has_secret() {
            // The address is not used, and not repeated in the message.
            settings.base_url = None;
            problem = Some(
                SettingsError::SecretInAddress {
                    provider: id.to_owned(),
                }
                .to_string(),
            );
        }
        let key = match kind {
            ProviderKind::Subscription => None,
            ProviderKind::Api => match self.read_key(id).await {
                Ok(key) => key,
                Err(error) => {
                    problem.get_or_insert(format!(
                        "It is not known whether a key is stored, because {error}."
                    ));
                    None
                }
            },
        };
        let key_stored = key.is_some();
        let provider = configured(id, &settings, key)?;
        let mut readiness = match problem {
            Some(problem) => Readiness::not_ready(ReadyState::Unknown, sentence(&problem)),
            None => provider.check_ready().await,
        };
        readiness.message = scrub(&readiness.message);
        Some(ProviderStatus {
            info: provider.info(),
            capabilities: provider.capabilities(),
            enabled: settings.enabled,
            key_stored,
            readiness,
        })
    }

    /// Checks every provider, all at the same time, and returns them in the
    /// order of [`known_providers`].
    ///
    /// The slowest check decides how long this takes: an agent CLI can need
    /// a second or two to report its version and sign-in state. A settings
    /// screen that wants to fill rows in as they arrive can call
    /// [`status`](Self::status) per provider instead.
    pub async fn statuses(&self) -> Vec<ProviderStatus> {
        let ids = provider_ids();
        let mut checks = JoinSet::new();
        let mut position = HashMap::new();
        for (index, id) in ids.iter().copied().enumerate() {
            let registry = self.clone();
            let handle = checks.spawn(async move { registry.status(id).await });
            position.insert(handle.id(), index);
        }
        let mut statuses: Vec<Option<ProviderStatus>> = vec![None; ids.len()];
        while let Some(joined) = checks.join_next_with_id().await {
            let (task, status) = match joined {
                Ok((task, status)) => (task, status),
                // A check that panicked must not make its row disappear.
                Err(error) => (error.id(), None),
            };
            let Some(&index) = position.get(&task) else {
                continue;
            };
            statuses[index] = status.or_else(|| self.unchecked(ids[index]));
        }
        statuses.into_iter().flatten().collect()
    }

    /// Stores the API key for a provider. An empty key removes the stored
    /// one.
    pub async fn set_key(&self, id: &str, key: ApiKey) -> Result<(), RegistryError> {
        let id = api_provider_id(id)?;
        let keys = Arc::clone(&self.keys);
        off_thread(move || keys.set(&id, &key)).await?;
        Ok(())
    }

    /// Removes the API key stored for a provider, if there is one.
    pub async fn delete_key(&self, id: &str) -> Result<(), RegistryError> {
        let id = api_provider_id(id)?;
        let keys = Arc::clone(&self.keys);
        off_thread(move || keys.delete(&id)).await?;
        Ok(())
    }

    /// Whether an API key is stored for a provider. The key itself is not
    /// handed out: a settings screen shows that one exists, never what it
    /// is.
    pub async fn has_key(&self, id: &str) -> Result<bool, RegistryError> {
        let id = api_provider_id(id)?;
        Ok(self.read_key(&id).await?.is_some())
    }

    async fn read_key(&self, id: &str) -> Result<Option<ApiKey>, KeyStoreError> {
        let keys = Arc::clone(&self.keys);
        let id = id.to_owned();
        off_thread(move || keys.get(&id)).await
    }

    /// The row for a provider whose check did not finish.
    fn unchecked(&self, id: &str) -> Option<ProviderStatus> {
        let provider = build(id, &ProviderSettings::default(), None)?;
        Some(ProviderStatus {
            info: provider.info(),
            capabilities: provider.capabilities(),
            enabled: self.settings.is_enabled(id),
            key_stored: false,
            readiness: Readiness::not_ready(
                ReadyState::Unknown,
                "The check stopped unexpectedly. Try again.",
            ),
        })
    }
}

/// The ids of all providers, in listing order.
fn provider_ids() -> Vec<&'static str> {
    SUBSCRIPTION_IDS
        .into_iter()
        .chain([ANTHROPIC_ID])
        .chain(OPENAI_PRESETS.iter().map(|preset| preset.id))
        .collect()
}

/// Whether `id` names a subscription or an API provider; `None` when it
/// names neither.
fn kind_of(id: &str) -> Option<ProviderKind> {
    if SUBSCRIPTION_IDS.contains(&id) {
        Some(ProviderKind::Subscription)
    } else if id == ANTHROPIC_ID || openai_preset(id).is_some() {
        Some(ProviderKind::Api)
    } else {
        None
    }
}

/// `id` as an owned string, after checking that it names a provider that
/// takes an API key.
fn api_provider_id(id: &str) -> Result<String, RegistryError> {
    match kind_of(id) {
        Some(ProviderKind::Api) => Ok(id.to_owned()),
        Some(ProviderKind::Subscription) => {
            let name = build(id, &ProviderSettings::default(), None)
                .map_or_else(|| id.to_owned(), |provider| provider.info().name);
            Err(RegistryError::NotAnApiProvider(name))
        }
        None => Err(RegistryError::UnknownProvider(id.to_owned())),
    }
}

/// Runs a key store call on a thread that may block. A keychain can wait
/// for the user to answer a system prompt.
async fn off_thread<T, F>(call: F) -> Result<T, KeyStoreError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, KeyStoreError> + Send + 'static,
{
    tokio::task::spawn_blocking(call).await.unwrap_or_else(|_| {
        Err(KeyStoreError::Failed(
            "the request to the key store did not finish".to_owned(),
        ))
    })
}

/// Makes `text` read as a sentence of its own.
fn sentence(text: &str) -> String {
    let mut characters = text.trim().chars();
    let mut sentence: String = match characters.next() {
        Some(first) => first.to_uppercase().chain(characters).collect(),
        None => return String::new(),
    };
    if !sentence.ends_with(['.', '!', '?']) {
        sentence.push('.');
    }
    sentence
}

/// The provider for `id` with the user's options and key, wrapped so that
/// the user's chosen model applies.
fn configured(
    id: &str,
    settings: &ProviderSettings,
    key: Option<ApiKey>,
) -> Option<Box<dyn ChatProvider>> {
    let provider = build(id, settings, key)?;
    Some(match settings.model() {
        Some(model) => Box::new(WithChosenModel {
            inner: provider,
            model: model.to_owned(),
        }),
        None => provider,
    })
}

/// The provider for `id` with the user's options and key. This is the one
/// place that knows how each provider is put together; a new provider is
/// added here and to [`provider_ids`].
fn build(
    id: &str,
    settings: &ProviderSettings,
    key: Option<ApiKey>,
) -> Option<Box<dyn ChatProvider>> {
    let binary = settings.binary().map(Path::to_path_buf);
    let provider: Box<dyn ChatProvider> = match id {
        CLAUDE_CODE_ID => {
            let defaults = ClaudeCodeOptions::default();
            Box::new(ClaudeCode::with_options(ClaudeCodeOptions {
                binary,
                isolate_settings: settings
                    .isolate_settings
                    .unwrap_or(defaults.isolate_settings),
            }))
        }
        CURSOR_AGENT_ID => {
            let defaults = CursorAgentOptions::default();
            Box::new(CursorAgent::with_options(CursorAgentOptions {
                binary,
                allow_file_reads: settings
                    .allow_file_reads
                    .unwrap_or(defaults.allow_file_reads),
            }))
        }
        CODEX_ID => {
            let defaults = CodexOptions::default();
            Box::new(Codex::with_options(CodexOptions {
                binary,
                isolate_settings: settings
                    .isolate_settings
                    .unwrap_or(defaults.isolate_settings),
            }))
        }
        GEMINI_CLI_ID => Box::new(GeminiCli::with_options(GeminiCliOptions { binary })),
        ANTHROPIC_ID => {
            let defaults = AnthropicOptions::default();
            Box::new(Anthropic::with_options(
                key,
                AnthropicOptions {
                    base_url: settings
                        .base_url()
                        .map_or_else(|| defaults.base_url.clone(), str::to_owned),
                    max_tokens: settings.max_tokens.unwrap_or(defaults.max_tokens),
                    effort: settings.effort.or(defaults.effort),
                    max_tool_rounds: settings.max_tool_rounds.unwrap_or(defaults.max_tool_rounds),
                    ..defaults
                },
            ))
        }
        other => {
            let preset = openai_preset(other)?;
            let defaults = OpenAiCompatibleOptions::default();
            Box::new(OpenAiCompatible::with_options(
                *preset,
                key,
                OpenAiCompatibleOptions {
                    base_url: settings.base_url().map(str::to_owned),
                    // The chosen model is applied by `WithChosenModel`, the
                    // same way for every provider.
                    model: None,
                    images: settings.images,
                    stream_usage: settings.stream_usage.unwrap_or(defaults.stream_usage),
                    max_tool_rounds: settings.max_tool_rounds.unwrap_or(defaults.max_tool_rounds),
                },
            ))
        }
    };
    Some(provider)
}

/// A provider together with the model the user chose for it in settings.
///
/// The model is used for every request that names none, and leads the model
/// list, so that "the first one is the default" stays true.
struct WithChosenModel {
    inner: Box<dyn ChatProvider>,
    model: String,
}

#[async_trait]
impl ChatProvider for WithChosenModel {
    fn info(&self) -> ProviderInfo {
        self.inner.info()
    }

    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>, AiError> {
        let mut models = self.inner.list_models().await?;
        let chosen = match models.iter().position(|model| model.id == self.model) {
            Some(position) => models.remove(position),
            // A model the user typed in, which the provider does not list.
            None => ModelInfo::from_id(&self.model),
        };
        models.insert(0, chosen);
        Ok(models)
    }

    async fn check_ready(&self) -> Readiness {
        self.inner.check_ready().await
    }

    async fn send(&self, mut request: ChatRequest, events: EventSender, cancel: CancellationToken) {
        let named = request
            .model
            .as_deref()
            .is_some_and(|model| !model.trim().is_empty());
        if !named {
            request.model = Some(self.model.clone());
        }
        self.inner.send(request, events, cancel).await;
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::sync::Mutex;

    use serde_json::json;

    use super::*;
    use crate::api::test_support::{answer, collect_events};
    use crate::event::{ChatEvent, StopReason};
    use crate::http::test_support::{MockResponse, MockServer, sse_event};
    use crate::keys::MemoryKeyStore;

    const KEY: &str = "sk-test-Abc123def456ghi789jkl012";

    fn registry(settings: AiSettings, keys: MemoryKeyStore) -> ProviderRegistry {
        ProviderRegistry::new(settings, Arc::new(keys))
    }

    /// A key store that cannot be read, like a locked keychain.
    struct LockedKeyStore;

    impl KeyStore for LockedKeyStore {
        fn get(&self, _: &str) -> Result<Option<ApiKey>, KeyStoreError> {
            Err(KeyStoreError::AccessDenied("it is locked".to_owned()))
        }
        fn set(&self, _: &str, _: &ApiKey) -> Result<(), KeyStoreError> {
            Err(KeyStoreError::AccessDenied("it is locked".to_owned()))
        }
        fn delete(&self, _: &str) -> Result<(), KeyStoreError> {
            Err(KeyStoreError::AccessDenied("it is locked".to_owned()))
        }
    }

    /// An address on this machine that nothing listens on. Port 1 is a
    /// privileged port no test server can be given, so the connection is
    /// refused at once.
    const CLOSED_ADDRESS: &str = "http://127.0.0.1:1/v1";

    /// Settings under which no check leaves the test process: the agent
    /// CLIs are looked for where there are none, and the local servers at
    /// an address that is closed.
    fn sealed_settings() -> AiSettings {
        let mut settings = AiSettings::default();
        for id in SUBSCRIPTION_IDS {
            settings.provider_mut(id).binary = Some(format!("/nonexistent/{id}").into());
        }
        for id in ["ollama", "lm-studio"] {
            settings.provider_mut(id).base_url = Some(CLOSED_ADDRESS.to_owned());
        }
        settings
    }

    #[test]
    fn every_provider_is_listed_once_subscriptions_first() {
        let providers = known_providers();
        let ids: Vec<&str> = providers
            .iter()
            .map(|provider| provider.info.id.as_str())
            .collect();
        assert_eq!(
            ids,
            [
                "claude-code",
                "cursor-agent",
                "codex",
                "gemini-cli",
                "anthropic",
                "openai",
                "openrouter",
                "gemini",
                "groq",
                "mistral",
                "xai",
                "ollama",
                "lm-studio",
                "custom",
            ]
        );
        assert_eq!(ids.iter().collect::<HashSet<_>>().len(), ids.len());
        for provider in &providers {
            let expected = if SUBSCRIPTION_IDS.contains(&provider.info.id.as_str()) {
                ProviderKind::Subscription
            } else {
                ProviderKind::Api
            };
            assert_eq!(provider.info.kind, expected, "{}", provider.info.id);
            assert_eq!(kind_of(&provider.info.id), Some(expected));
            assert!(!provider.info.name.is_empty());
        }
        assert_eq!(kind_of("nope"), None);
    }

    #[test]
    fn descriptors_say_where_a_provider_lives_and_where_to_get_a_key() {
        let providers = known_providers();
        let find = |id: &str| {
            providers
                .iter()
                .find(|provider| provider.info.id == id)
                .unwrap()
        };
        let claude_code = find("claude-code");
        assert_eq!(claude_code.default_base_url, None);
        assert_eq!(claude_code.key_url, None);
        assert!(!claude_code.capabilities.needs_api_key);

        let anthropic = find("anthropic");
        assert_eq!(
            anthropic.default_base_url.as_deref(),
            Some("https://api.anthropic.com")
        );
        assert!(anthropic.key_url.is_some());
        assert!(anthropic.capabilities.needs_api_key);

        let ollama = find("ollama");
        assert_eq!(
            ollama.default_base_url.as_deref(),
            Some("http://localhost:11434/v1")
        );
        assert_eq!(ollama.key_url, None);

        let custom = find("custom");
        assert_eq!(custom.default_base_url, None);
        assert_eq!(custom.key_url, None);
    }

    #[tokio::test]
    async fn an_unknown_id_is_an_error() {
        let registry = registry(AiSettings::default(), MemoryKeyStore::new());
        assert_eq!(
            registry.provider("nope").await.err(),
            Some(RegistryError::UnknownProvider("nope".to_owned()))
        );
        assert_eq!(registry.status("nope").await, None);
    }

    #[tokio::test]
    async fn an_api_provider_gets_its_key_and_address_from_the_registry() {
        let server = MockServer::start(vec![MockResponse::json(
            200,
            json!({ "data": [], "has_more": false }),
        )])
        .await;
        let mut settings = AiSettings::default();
        settings.provider_mut("anthropic").base_url = Some(server.url());
        let keys = MemoryKeyStore::new().with_key("anthropic", ApiKey::new(KEY));
        let registry = registry(settings, keys);

        let provider = registry.provider("anthropic").await.unwrap();
        assert_eq!(provider.info().id, "anthropic");
        assert!(provider.check_ready().await.is_ready());

        let requests = server.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].header("x-api-key"), KEY);
    }

    #[tokio::test]
    async fn the_chosen_model_is_used_when_a_request_names_none() {
        let reply = |text: &str| {
            MockResponse::sse(&[
                sse_event(
                    "",
                    &json!({"choices":[{"index":0,"delta":{"content":text},"finish_reason":null}]}),
                ),
                sse_event(
                    "",
                    &json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}),
                ),
                "data: [DONE]\n\n".to_owned(),
            ])
        };
        let server = MockServer::start(vec![reply("one"), reply("two")]).await;
        let mut settings = AiSettings::default();
        let groq = settings.provider_mut("groq");
        groq.base_url = Some(format!("{}/v1", server.url()));
        groq.model = Some("chosen-in-settings".to_owned());
        let keys = MemoryKeyStore::new().with_key("groq", ApiKey::new(KEY));
        let provider = registry(settings, keys).provider("groq").await.unwrap();

        let events = collect_events(provider.as_ref(), ChatRequest::from_prompt("hi")).await;
        assert_eq!(answer(&events), "one");
        assert_eq!(
            events.last(),
            Some(&ChatEvent::Finished {
                stop_reason: StopReason::EndTurn
            })
        );
        let named = ChatRequest {
            model: Some("named-in-the-request".to_owned()),
            ..ChatRequest::from_prompt("hi")
        };
        collect_events(provider.as_ref(), named).await;

        let requests = server.requests();
        assert_eq!(requests[0].body["model"], "chosen-in-settings");
        assert_eq!(requests[0].header("authorization"), format!("Bearer {KEY}"));
        assert_eq!(requests[1].body["model"], "named-in-the-request");
    }

    /// A provider that records the model of each request it is sent.
    #[derive(Default)]
    struct Recorder {
        models: Arc<Mutex<Vec<Option<String>>>>,
    }

    #[async_trait]
    impl ChatProvider for Recorder {
        fn info(&self) -> ProviderInfo {
            ClaudeCode::new().info()
        }
        fn capabilities(&self) -> Capabilities {
            ClaudeCode::new().capabilities()
        }
        async fn list_models(&self) -> Result<Vec<ModelInfo>, AiError> {
            Ok(vec![
                ModelInfo::new("default", "Default"),
                ModelInfo::new("sonnet", "Sonnet"),
            ])
        }
        async fn check_ready(&self) -> Readiness {
            Readiness::ready("Recording.")
        }
        async fn send(&self, request: ChatRequest, events: EventSender, _: CancellationToken) {
            self.models.lock().unwrap().push(request.model);
            events.finish(Ok(StopReason::EndTurn));
        }
    }

    #[tokio::test]
    async fn a_request_that_names_a_model_keeps_it() {
        let recorder = Recorder::default();
        let models = Arc::clone(&recorder.models);
        let provider = WithChosenModel {
            inner: Box::new(recorder),
            model: "sonnet".to_owned(),
        };
        for model in [None, Some("  "), Some("default"), Some("opus")] {
            let request = ChatRequest {
                model: model.map(str::to_owned),
                ..ChatRequest::from_prompt("hi")
            };
            collect_events(&provider, request).await;
        }
        assert_eq!(
            *models.lock().unwrap(),
            [
                Some("sonnet".to_owned()),
                Some("sonnet".to_owned()),
                Some("default".to_owned()),
                Some("opus".to_owned()),
            ]
        );
    }

    #[tokio::test]
    async fn the_chosen_model_leads_the_model_list() {
        let listed = WithChosenModel {
            inner: Box::new(Recorder::default()),
            model: "sonnet".to_owned(),
        };
        let ids = |models: Vec<ModelInfo>| -> Vec<String> {
            models.into_iter().map(|model| model.id).collect()
        };
        assert_eq!(
            ids(listed.list_models().await.unwrap()),
            ["sonnet", "default"]
        );

        let typed_in = WithChosenModel {
            inner: Box::new(Recorder::default()),
            model: "claude-something-new".to_owned(),
        };
        assert_eq!(
            ids(typed_in.list_models().await.unwrap()),
            ["claude-something-new", "default", "sonnet"]
        );
    }

    #[tokio::test]
    async fn a_subscription_provider_uses_its_settings_and_never_asks_for_a_key() {
        let mut settings = AiSettings::default();
        settings.provider_mut("claude-code").binary = Some("/nonexistent/claude".into());
        // A locked store would fail any read, so building proves none
        // happened.
        let registry = ProviderRegistry::new(settings, Arc::new(LockedKeyStore));

        let provider = registry.provider("claude-code").await.unwrap();
        assert_eq!(provider.check_ready().await.state, ReadyState::NotInstalled);
        let status = registry.status("claude-code").await.unwrap();
        assert_eq!(status.readiness.state, ReadyState::NotInstalled);
        assert!(!status.key_stored);
    }

    #[tokio::test]
    async fn a_key_store_that_cannot_be_read_is_reported_not_mistaken_for_no_key() {
        let registry = ProviderRegistry::new(AiSettings::default(), Arc::new(LockedKeyStore));
        assert_eq!(
            registry.provider("openai").await.err(),
            Some(RegistryError::KeyStore(KeyStoreError::AccessDenied(
                "it is locked".to_owned()
            )))
        );
        let status = registry.status("openai").await.unwrap();
        assert_eq!(status.readiness.state, ReadyState::Unknown);
        assert_eq!(
            status.readiness.message,
            "It is not known whether a key is stored, because the keychain refused access: \
             it is locked."
        );
        assert!(!status.key_stored);
        assert_eq!(status.info.name, "OpenAI");
    }

    #[tokio::test]
    async fn status_tells_a_missing_key_from_a_working_one() {
        let server = MockServer::start(vec![MockResponse::json(
            200,
            json!({ "data": [{ "id": "gpt-x" }] }),
        )])
        .await;
        let mut settings = AiSettings::default();
        settings.provider_mut("openai").base_url = Some(format!("{}/v1", server.url()));
        settings.provider_mut("openai").enabled = false;

        let without = registry(settings.clone(), MemoryKeyStore::new());
        let status = without.status("openai").await.unwrap();
        assert_eq!(status.readiness.state, ReadyState::MissingKey);
        assert!(!status.key_stored);
        assert!(
            server.requests().is_empty(),
            "nothing is asked without a key"
        );

        let keys = MemoryKeyStore::new().with_key("openai", ApiKey::new(KEY));
        let status = registry(settings, keys).status("openai").await.unwrap();
        assert!(status.readiness.is_ready(), "{:?}", status.readiness);
        assert!(status.key_stored);
        // Ready, but the user turned it off.
        assert!(!status.enabled);
        assert!(!status.is_usable());
    }

    #[tokio::test]
    async fn all_statuses_come_back_in_listing_order() {
        let mut settings = sealed_settings();
        settings.provider_mut("codex").enabled = false;
        let keys = MemoryKeyStore::new();
        let statuses = registry(settings, keys).statuses().await;

        let listed: Vec<String> = known_providers()
            .into_iter()
            .map(|provider| provider.info.id)
            .collect();
        let checked: Vec<String> = statuses
            .iter()
            .map(|status| status.info.id.clone())
            .collect();
        assert_eq!(checked, listed);

        let state = |id: &str| {
            let status = statuses.iter().find(|status| status.info.id == id).unwrap();
            status.readiness.state
        };
        assert_eq!(state("claude-code"), ReadyState::NotInstalled);
        assert_eq!(state("gemini-cli"), ReadyState::NotInstalled);
        assert_eq!(state("anthropic"), ReadyState::MissingKey);
        assert_eq!(state("openrouter"), ReadyState::MissingKey);
        assert_eq!(state("ollama"), ReadyState::Unreachable);
        assert_eq!(state("lm-studio"), ReadyState::Unreachable);
        // No address has been entered for the custom server.
        assert_eq!(state("custom"), ReadyState::Unknown);

        assert!(statuses.iter().all(|status| !status.is_usable()));
        let enabled = |id: &str| {
            statuses
                .iter()
                .find(|status| status.info.id == id)
                .unwrap()
                .enabled
        };
        assert!(!enabled("codex"));
        assert!(enabled("claude-code"));
    }

    #[tokio::test]
    async fn keys_are_stored_and_removed_through_the_registry() {
        let registry = registry(AiSettings::default(), MemoryKeyStore::new());
        assert_eq!(registry.has_key("openai").await, Ok(false));

        registry.set_key("openai", ApiKey::new(KEY)).await.unwrap();
        assert_eq!(registry.has_key("openai").await, Ok(true));
        assert_eq!(registry.has_key("groq").await, Ok(false));
        assert_eq!(
            registry.keys().get("openai").unwrap().unwrap().expose(),
            KEY
        );

        registry.delete_key("openai").await.unwrap();
        assert_eq!(registry.has_key("openai").await, Ok(false));

        // Clearing the key field forgets the key.
        registry.set_key("openai", ApiKey::new(KEY)).await.unwrap();
        registry.set_key("openai", ApiKey::new("")).await.unwrap();
        assert_eq!(registry.has_key("openai").await, Ok(false));
    }

    #[tokio::test]
    async fn only_api_providers_take_a_key() {
        let registry = registry(AiSettings::default(), MemoryKeyStore::new());
        assert_eq!(
            registry.set_key("claude-code", ApiKey::new(KEY)).await,
            Err(RegistryError::NotAnApiProvider("Claude Code".to_owned()))
        );
        assert_eq!(
            registry.delete_key("nope").await,
            Err(RegistryError::UnknownProvider("nope".to_owned()))
        );
        assert_eq!(registry.keys().get("claude-code").unwrap(), None);
        assert_eq!(
            RegistryError::NotAnApiProvider("Claude Code".to_owned()).to_string(),
            "Claude Code signs in through its own app and takes no API key"
        );
    }

    #[tokio::test]
    async fn a_stored_key_never_reaches_the_settings_file_or_debug_output() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ai-settings.json");
        let mut registry = registry(AiSettings::default(), MemoryKeyStore::new());
        registry.settings_mut().default_provider = Some("openai".to_owned());
        registry.settings_mut().provider_mut("openai").model = Some("gpt-x".to_owned());
        registry.set_key("openai", ApiKey::new(KEY)).await.unwrap();

        registry.settings().save_to(&path).unwrap();
        let file = std::fs::read_to_string(&path).unwrap();
        assert!(file.contains("gpt-x"));
        assert!(!file.contains(KEY));

        assert!(!format!("{registry:?}").contains(KEY));
        let status = registry.status("openai").await.unwrap();
        assert!(!format!("{status:?}").contains(KEY));
        assert!(!serde_json::to_string(&status).unwrap().contains(KEY));
    }

    #[tokio::test]
    async fn an_address_with_a_secret_in_it_is_not_used() {
        let secret = "AIzaSyA1234567890abcdefghijklmnop";
        let mut settings = AiSettings::default();
        settings.provider_mut("custom").base_url =
            Some(format!("https://example.com/v1?key={secret}"));
        let registry = registry(settings, MemoryKeyStore::new());

        assert_eq!(
            registry.provider("custom").await.err(),
            Some(RegistryError::Settings(SettingsError::SecretInAddress {
                provider: "custom".to_owned()
            }))
        );
        let status = registry.status("custom").await.unwrap();
        assert_eq!(status.readiness.state, ReadyState::Unknown);
        assert!(status.readiness.message.contains("key field"));
        assert!(!status.readiness.message.contains(secret));
    }

    fn status(id: &str, enabled: bool, state: ReadyState) -> ProviderStatus {
        let provider = build(id, &ProviderSettings::default(), None).unwrap();
        ProviderStatus {
            info: provider.info(),
            capabilities: provider.capabilities(),
            enabled,
            key_stored: false,
            readiness: Readiness::not_ready(state, "test"),
        }
    }

    #[test]
    fn the_chat_opens_with_the_default_when_it_can_be_used() {
        let statuses = [
            status("claude-code", true, ReadyState::Ready),
            status("anthropic", true, ReadyState::Ready),
            status("ollama", true, ReadyState::Unreachable),
        ];
        let chosen = |default: Option<&str>| {
            let settings = AiSettings {
                default_provider: default.map(str::to_owned),
                ..AiSettings::default()
            };
            choose_provider(&settings, &statuses).map(|status| status.info.id.clone())
        };
        assert_eq!(chosen(Some("anthropic")).as_deref(), Some("anthropic"));
        // Nothing set, or a default that cannot be used: the first that can.
        assert_eq!(chosen(None).as_deref(), Some("claude-code"));
        assert_eq!(chosen(Some("ollama")).as_deref(), Some("claude-code"));
        assert_eq!(chosen(Some("nope")).as_deref(), Some("claude-code"));
    }

    #[test]
    fn a_disabled_or_unready_provider_is_never_chosen() {
        let statuses = [
            status("claude-code", false, ReadyState::Ready),
            status("anthropic", true, ReadyState::MissingKey),
        ];
        let settings = AiSettings {
            default_provider: Some("claude-code".to_owned()),
            ..AiSettings::default()
        };
        assert_eq!(choose_provider(&settings, &statuses), None);
        assert_eq!(choose_provider(&settings, &[]), None);
    }

    #[test]
    fn messages_are_made_into_sentences() {
        assert_eq!(sentence("it is locked"), "It is locked.");
        assert_eq!(sentence("Already done."), "Already done.");
        assert_eq!(sentence("  "), "");
    }
}
