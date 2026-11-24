//! Stand-ins for tests and previews of the chat. Nothing here starts a CLI,
//! opens a connection or reads the keychain, so nothing is ever spent.
//!
//! - [`ScriptedProvider`] answers every message by playing a script of
//!   [`Step`]s: events to send, tools to call, a pause until it is stopped.
//! - [`ScriptedSource`] hands the chat fixed provider statuses and scripted
//!   providers in place of the real ones.
//! - [`status`] makes a [`ProviderStatus`] for any known provider id.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use moshionly_ai::{
    AiError, CancellationToken, Capabilities, ChatEvent, ChatProvider, ChatRequest, EventSender,
    ModelInfo, ProviderInfo, ProviderKind, ProviderRegistry, ProviderStatus, Readiness, ReadyState,
    RegistryError, StopReason, ToolTransport, known_providers,
};
use serde_json::Value;

use super::source::ProviderSource;

/// One step of a [`ScriptedProvider`]'s answer.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// Send this event.
    Event(ChatEvent),
    /// Call a tool through the request's tool executor, reporting the call
    /// as it starts and finishes, the way API providers do.
    CallTool {
        /// The call's id.
        id: String,
        /// The tool.
        name: String,
        /// Its arguments.
        input: Value,
    },
    /// Wait this long, or until the turn is stopped.
    Pause(Duration),
    /// Wait until the turn is stopped, then finish as cancelled.
    WaitForStop,
}

impl Step {
    /// A [`Step::Event`] with more of the answer's text.
    pub fn text(text: &str) -> Self {
        Self::Event(ChatEvent::TextDelta {
            text: text.to_owned(),
        })
    }

    /// A [`Step::Event`] with more of the model's reasoning.
    pub fn thinking(text: &str) -> Self {
        Self::Event(ChatEvent::ThinkingDelta {
            text: text.to_owned(),
        })
    }

    /// A [`Step::CallTool`].
    pub fn call(id: &str, name: &str, input: Value) -> Self {
        Self::CallTool {
            id: id.to_owned(),
            name: name.to_owned(),
            input,
        }
    }
}

/// What a [`ScriptedProvider`] saw, shared by all its clones.
#[derive(Debug, Default)]
struct Seen {
    requests: Mutex<Vec<ChatRequest>>,
    stopped: AtomicBool,
    turns: AtomicUsize,
    /// Model listings wait while this is set.
    models_held: AtomicBool,
    /// How many times the models were asked for.
    listings: AtomicUsize,
}

/// Marks a turn as stopped when its work is dropped before it finished,
/// which is what happens to a turn whose runtime shuts down.
struct DroppedEarly<'a> {
    seen: &'a Seen,
    finished: bool,
}

impl Drop for DroppedEarly<'_> {
    fn drop(&mut self) {
        if !self.finished {
            self.seen.stopped.store(true, Ordering::Release);
        }
    }
}

/// A provider that answers by playing a script.
///
/// It finishes with `EndTurn` after the last step unless the script sent a
/// `Finished` event itself. Clones share what they saw.
#[derive(Debug, Clone)]
pub struct ScriptedProvider {
    info: ProviderInfo,
    capabilities: Capabilities,
    models: Vec<ModelInfo>,
    /// One script per turn; the last one plays again for later turns.
    scripts: Vec<Vec<Step>>,
    seen: Arc<Seen>,
}

impl ScriptedProvider {
    /// A provider that takes the name and abilities of the known provider
    /// `id` (any name for an unknown one) and plays `script` every turn.
    pub fn new(id: &str, script: Vec<Step>) -> Self {
        Self::with_scripts(id, vec![script])
    }

    /// A provider that plays the first script on the first turn, the
    /// second on the second, and the last one on every turn after.
    pub fn with_scripts(id: &str, scripts: Vec<Vec<Step>>) -> Self {
        let known = known_providers()
            .into_iter()
            .find(|descriptor| descriptor.info.id == id);
        let (info, capabilities) = match known {
            Some(descriptor) => (descriptor.info, descriptor.capabilities),
            None => (
                ProviderInfo {
                    id: id.to_owned(),
                    name: id.to_owned(),
                    kind: ProviderKind::Api,
                },
                Capabilities {
                    tools: ToolTransport::InProcess,
                    images: true,
                    resume: false,
                    thinking: true,
                    live_model_list: false,
                    needs_api_key: false,
                    verified: true,
                },
            ),
        };
        Self {
            info,
            capabilities,
            models: vec![ModelInfo::new("scripted", "Scripted model")],
            scripts,
            seen: Arc::default(),
        }
    }

    /// Offers these models.
    pub fn with_models(mut self, models: Vec<ModelInfo>) -> Self {
        self.models = models;
        self
    }

    /// Keeps every listing of its models waiting until
    /// [`release_models`](Self::release_models), like a server that is
    /// slow to answer.
    pub fn hold_models(&self) {
        self.seen.models_held.store(true, Ordering::Release);
    }

    /// Lets the held listings of its models finish.
    pub fn release_models(&self) {
        self.seen.models_held.store(false, Ordering::Release);
    }

    /// How many times its models were asked for, including listings still
    /// held.
    pub fn model_listings(&self) -> usize {
        self.seen.listings.load(Ordering::Acquire)
    }

    /// Every request it was sent, oldest first.
    pub fn requests(&self) -> Vec<ChatRequest> {
        lock(&self.seen.requests).clone()
    }

    /// True once a turn was stopped before its script ended: cancelled
    /// while it waited, or dropped unfinished.
    pub fn was_stopped(&self) -> bool {
        self.seen.stopped.load(Ordering::Acquire)
    }

    async fn play(
        &self,
        request: &ChatRequest,
        events: &EventSender,
        cancel: &CancellationToken,
    ) -> Result<StopReason, AiError> {
        let turn = self.seen.turns.fetch_add(1, Ordering::AcqRel);
        let script = match self.scripts.get(turn).or_else(|| self.scripts.last()) {
            Some(script) => script.as_slice(),
            None => &[],
        };
        for step in script {
            if cancel.is_cancelled() {
                return Ok(self.stopped());
            }
            match step {
                Step::Event(ChatEvent::Finished { stop_reason }) => return Ok(*stop_reason),
                Step::Event(error @ ChatEvent::Error { .. }) => {
                    events.emit(error.clone());
                    return Ok(StopReason::Error);
                }
                Step::Event(event) => {
                    events.emit(event.clone());
                }
                Step::CallTool { id, name, input } => {
                    events.emit(ChatEvent::ToolCallStarted {
                        id: id.clone(),
                        name: name.clone(),
                        input: input.clone(),
                    });
                    let (ok, summary) = match &request.tools {
                        Some(tools) => {
                            let output = tools.call(name, input.clone()).await;
                            (!output.is_error, output.summary())
                        }
                        None => (false, "no tools were given".to_owned()),
                    };
                    events.emit(ChatEvent::ToolCallFinished {
                        id: id.clone(),
                        ok,
                        summary,
                    });
                }
                Step::Pause(duration) => {
                    tokio::select! {
                        () = cancel.cancelled() => return Ok(self.stopped()),
                        () = tokio::time::sleep(*duration) => {}
                    }
                }
                Step::WaitForStop => {
                    cancel.cancelled().await;
                    return Ok(self.stopped());
                }
            }
        }
        Ok(StopReason::EndTurn)
    }

    fn stopped(&self) -> StopReason {
        self.seen.stopped.store(true, Ordering::Release);
        StopReason::Cancelled
    }
}

#[async_trait]
impl ChatProvider for ScriptedProvider {
    fn info(&self) -> ProviderInfo {
        self.info.clone()
    }

    fn capabilities(&self) -> Capabilities {
        self.capabilities
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>, AiError> {
        self.seen.listings.fetch_add(1, Ordering::AcqRel);
        while self.seen.models_held.load(Ordering::Acquire) {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        Ok(self.models.clone())
    }

    async fn check_ready(&self) -> Readiness {
        Readiness::ready("Scripted, always ready.")
    }

    async fn send(&self, request: ChatRequest, events: EventSender, cancel: CancellationToken) {
        lock(&self.seen.requests).push(request.clone());
        let mut guard = DroppedEarly {
            seen: &self.seen,
            finished: false,
        };
        let outcome = self.play(&request, &events, &cancel).await;
        guard.finished = true;
        events.finish(outcome);
    }
}

/// Fixed statuses and scripted providers in place of the real ones.
///
/// Clones share their statuses, providers and holds.
#[derive(Debug, Clone, Default)]
pub struct ScriptedSource {
    statuses: Arc<Mutex<Vec<ProviderStatus>>>,
    providers: Arc<Mutex<HashMap<String, ScriptedProvider>>>,
    checks_held: Arc<AtomicBool>,
}

impl ScriptedSource {
    /// A source reporting `statuses` and answering with no provider yet.
    pub fn new(statuses: Vec<ProviderStatus>) -> Self {
        Self {
            statuses: Arc::new(Mutex::new(statuses)),
            ..Self::default()
        }
    }

    /// Keeps every check of all providers waiting until
    /// [`release_checks`](Self::release_checks), like a slow probe. Each
    /// still reports the settings it started with. Checks of one provider
    /// are not held.
    pub fn hold_checks(&self) {
        self.checks_held.store(true, Ordering::Release);
    }

    /// Lets the held checks of all providers finish.
    pub fn release_checks(&self) {
        self.checks_held.store(false, Ordering::Release);
    }

    /// Answers with `provider` for its id.
    pub fn with_provider(self, provider: ScriptedProvider) -> Self {
        self.set_provider(provider);
        self
    }

    /// Answers with `provider` for its id from now on.
    pub fn set_provider(&self, provider: ScriptedProvider) {
        lock(&self.providers).insert(provider.info.id.clone(), provider);
    }

    /// Reports `statuses` from now on.
    pub fn set_statuses(&self, statuses: Vec<ProviderStatus>) {
        *lock(&self.statuses) = statuses;
    }
}

#[async_trait]
impl ProviderSource for ScriptedSource {
    async fn statuses(&self, registry: &ProviderRegistry) -> Vec<ProviderStatus> {
        while self.checks_held.load(Ordering::Acquire) {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        let mut statuses = lock(&self.statuses).clone();
        for status in &mut statuses {
            status.enabled = registry.settings().is_enabled(&status.info.id);
        }
        statuses
    }

    async fn status(&self, registry: &ProviderRegistry, id: &str) -> Option<ProviderStatus> {
        let mut status = lock(&self.statuses)
            .iter()
            .find(|status| status.info.id == id)
            .cloned()?;
        status.enabled = registry.settings().is_enabled(id);
        if status.capabilities.needs_api_key {
            status.key_stored = registry.has_key(id).await.unwrap_or(false);
        }
        Some(status)
    }

    async fn provider(
        &self,
        _registry: &ProviderRegistry,
        id: &str,
    ) -> Result<Box<dyn ChatProvider>, RegistryError> {
        lock(&self.providers)
            .get(id)
            .cloned()
            .map(|provider| Box::new(provider) as Box<dyn ChatProvider>)
            .ok_or_else(|| RegistryError::UnknownProvider(id.to_owned()))
    }
}

/// The status of the known provider `id` in `state`, with `message` as its
/// one sentence for the person. `None` for an id no provider has.
pub fn status(id: &str, state: ReadyState, message: &str) -> Option<ProviderStatus> {
    let descriptor = known_providers()
        .into_iter()
        .find(|descriptor| descriptor.info.id == id)?;
    Some(ProviderStatus {
        info: descriptor.info,
        capabilities: descriptor.capabilities,
        enabled: true,
        key_stored: false,
        readiness: if state == ReadyState::Ready {
            Readiness::ready(message)
        } else {
            Readiness::not_ready(state, message)
        },
    })
}

/// The status of every known provider, each one not ready for the reason a
/// fresh machine would give, except those listed in `ready`.
pub fn statuses(ready: &[&str]) -> Vec<ProviderStatus> {
    known_providers()
        .into_iter()
        .filter_map(|descriptor| {
            let id = descriptor.info.id.as_str();
            let (state, message) = if ready.contains(&id) {
                (ReadyState::Ready, "Ready.")
            } else if descriptor.info.kind == ProviderKind::Subscription {
                (ReadyState::NotInstalled, "Not installed.")
            } else if descriptor.capabilities.needs_api_key {
                (ReadyState::MissingKey, "Add an API key to use it.")
            } else {
                (ReadyState::Unreachable, "Nothing answered at its address.")
            };
            status(id, state, message)
        })
        .collect()
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
