//! The chat's engine room: runs turns and checks providers in the
//! background, and hands the results to the panel between frames.
//!
//! [`Controller`] owns a small Tokio runtime. Everything slow happens
//! there: checking which providers are ready, listing models, saving keys
//! and, above all, a turn, which builds the provider, sends the request and
//! forwards each [`ChatEvent`] as it arrives. Results come back through a
//! channel that [`Controller::poll`] drains on the UI thread, and every
//! result wakes the window so it is drawn at once.
//!
//! One turn runs at a time. [`Controller::stop`] ends the answer in the
//! panel straight away and cancels the provider, which stops its CLI and
//! everything that CLI started; the next message can be sent once that
//! clean-up is over, which takes at most a moment.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use moshionly_agent::{Origin, Subscription, ToolBridge, Tools};
use moshionly_ai::{
    AiSettings, ApiKey, CancellationToken, ChatEvent, ChatRequest, ErrorKind, ImageData,
    McpEndpoint, ModelInfo, ProviderRegistry, ProviderStatus, RegistryError, StopReason,
    choose_provider, event_channel, redact::scrub,
};
use serde_json::Value;
use tokio::runtime::Runtime;

use super::conversation::Conversation;
use super::executor::AgentTools;
use super::source::ProviderSource;

/// The name the Moshionly MCP server is given in an agent CLI. Tool names
/// the agent sees are derived from it.
pub const MCP_SERVER_NAME: &str = "moshionly";

/// How long closing the chat waits for a running turn to clean up.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(3);

/// Wakes the window from a background thread.
#[derive(Clone, Default)]
pub(crate) struct Waker(Arc<Mutex<Option<egui::Context>>>);

impl Waker {
    /// Remembers the window to wake.
    pub fn attach(&self, ctx: &egui::Context) {
        let mut slot = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if slot.is_none() {
            *slot = Some(ctx.clone());
        }
    }

    /// Asks the window to draw a new frame.
    pub fn wake(&self) {
        let slot = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(ctx) = slot.as_ref() {
            ctx.request_repaint();
        }
    }
}

/// The models of one provider, as far as they are known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelList {
    /// The provider is being asked.
    Loading,
    /// The models it offers; the first is its default.
    Loaded(Vec<ModelInfo>),
    /// It could not say, for this reason.
    Failed(String),
}

/// The outcome of the latest key change for a provider, for the settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyNote {
    /// The keychain is being asked.
    Working,
    /// The key was stored.
    Saved,
    /// The key was removed.
    Removed,
    /// The keychain refused, for this reason.
    Failed(String),
}

/// Why a message could not be sent.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SendError {
    /// An answer is still arriving.
    #[error("wait for the answer to finish, or stop it")]
    Busy,
    /// No provider is chosen, usually because none is ready.
    #[error("no AI is set up yet")]
    NoProvider,
    /// The message is empty.
    #[error("the message is empty")]
    Empty,
    /// The chat's background threads could not be started.
    #[error("the chat could not start its background work: {0}")]
    NoRuntime(String),
}

/// What the controller noticed while catching up.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Polled {
    /// The assistant changed the document since the last poll.
    pub document_changed: bool,
    /// Something new arrived, so the panel should be drawn again.
    pub updated: bool,
}

/// A result from the background, for the UI thread.
enum Update {
    Statuses {
        generation: u64,
        statuses: Vec<ProviderStatus>,
    },
    Status(ProviderStatus),
    Models {
        provider: String,
        result: Result<Vec<ModelInfo>, String>,
    },
    Event {
        turn: u64,
        event: ChatEvent,
    },
    ToolImages {
        turn: u64,
        name: String,
        input: Value,
        images: Vec<ImageData>,
    },
    TurnEnded {
        turn: u64,
    },
    Key {
        provider: String,
        note: KeyNote,
    },
}

/// The turn that is running.
struct Turn {
    id: u64,
    cancel: CancellationToken,
    /// Stop was pressed; the provider is cleaning up.
    stopping: bool,
    /// Notes changes the assistant makes to the document.
    _watch: Subscription,
}

/// Sends `TurnEnded` when a turn's task ends, however it ends, so that the
/// panel never waits for a turn that is gone.
struct EndGuard {
    turn: u64,
    updates: mpsc::Sender<Update>,
    waker: Waker,
}

impl Drop for EndGuard {
    fn drop(&mut self) {
        let _ = self.updates.send(Update::TurnEnded { turn: self.turn });
        self.waker.wake();
    }
}

/// Runs turns and provider checks in the background; see the module
/// documentation.
pub struct Controller {
    runtime: Result<Runtime, String>,
    registry: ProviderRegistry,
    source: Arc<dyn ProviderSource>,
    /// Where settings are saved; `None` leaves the file alone.
    settings_file: Option<PathBuf>,
    /// A problem with the settings file, for the settings screen.
    settings_problem: Option<String>,
    sender: mpsc::Sender<Update>,
    receiver: mpsc::Receiver<Update>,
    waker: Waker,
    statuses: Option<Vec<ProviderStatus>>,
    checking: bool,
    generation: u64,
    models: HashMap<String, ModelList>,
    keys: HashMap<String, KeyNote>,
    selected: Option<String>,
    conversation: Conversation,
    turn: Option<Turn>,
    next_turn: u64,
    agent_changed: Arc<AtomicBool>,
}

impl Controller {
    /// A controller over `registry`, checking and building providers with
    /// `source`. Settings changes are saved to `settings_file` when given.
    pub fn new(
        registry: ProviderRegistry,
        source: Arc<dyn ProviderSource>,
        settings_file: Option<PathBuf>,
    ) -> Self {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("moshionly-chat")
            .enable_all()
            .build()
            .map_err(|error| error.to_string());
        let (sender, receiver) = mpsc::channel();
        Self {
            runtime,
            registry,
            source,
            settings_file,
            settings_problem: None,
            sender,
            receiver,
            waker: Waker::default(),
            statuses: None,
            checking: false,
            generation: 0,
            models: HashMap::new(),
            keys: HashMap::new(),
            selected: None,
            conversation: Conversation::new(),
            turn: None,
            next_turn: 0,
            agent_changed: Arc::default(),
        }
    }

    /// Records a problem with the settings file, shown in the settings.
    pub fn set_settings_problem(&mut self, problem: Option<String>) {
        self.settings_problem = problem;
    }

    /// Wakes `ctx` whenever something arrives from the background, so the
    /// window draws it at once.
    pub fn attach(&self, ctx: &egui::Context) {
        self.waker.attach(ctx);
    }

    /// The conversation.
    pub fn conversation(&self) -> &Conversation {
        &self.conversation
    }

    /// The settings as they are now.
    pub fn settings(&self) -> &AiSettings {
        self.registry.settings()
    }

    /// A problem with the settings file, when there is one.
    pub fn settings_problem(&self) -> Option<&str> {
        self.settings_problem.as_deref()
    }

    /// Every provider's status, once the first check is done.
    pub fn statuses(&self) -> Option<&[ProviderStatus]> {
        self.statuses.as_deref()
    }

    /// The status of one provider, when known.
    pub fn status(&self, id: &str) -> Option<&ProviderStatus> {
        self.statuses
            .as_deref()?
            .iter()
            .find(|status| status.info.id == id)
    }

    /// True while providers are being checked.
    pub fn is_checking(&self) -> bool {
        self.checking
    }

    /// The provider the next message goes to.
    pub fn selected(&self) -> Option<&ProviderStatus> {
        self.status(self.selected.as_deref()?)
    }

    /// The models of a provider, when they were asked for.
    pub fn models(&self, id: &str) -> Option<&ModelList> {
        self.models.get(id)
    }

    /// The outcome of the latest key change for a provider.
    pub fn key_note(&self, id: &str) -> Option<&KeyNote> {
        self.keys.get(id)
    }

    /// True while a turn runs or cleans up.
    pub fn is_busy(&self) -> bool {
        self.turn.is_some()
    }

    /// True after Stop, while the provider cleans up.
    pub fn is_stopping(&self) -> bool {
        self.turn.as_ref().is_some_and(|turn| turn.stopping)
    }

    /// Why background work cannot run, when it cannot.
    pub fn runtime_problem(&self) -> Option<&str> {
        self.runtime.as_ref().err().map(String::as_str)
    }

    /// Checks every provider, unless that was done or is under way.
    pub fn ensure_checked(&mut self) {
        if self.statuses.is_none() && !self.checking {
            self.refresh_statuses();
        }
    }

    /// Checks every provider again.
    pub fn refresh_statuses(&mut self) {
        self.generation += 1;
        self.checking = true;
        let (generation, registry, source) = (
            self.generation,
            self.registry.clone(),
            Arc::clone(&self.source),
        );
        self.spawn(async move {
            let statuses = source.statuses(&registry).await;
            Some(Update::Statuses {
                generation,
                statuses,
            })
        });
    }

    /// Checks one provider again.
    pub fn refresh_status(&mut self, id: &str) {
        let (registry, source, id) = (
            self.registry.clone(),
            Arc::clone(&self.source),
            id.to_owned(),
        );
        self.spawn(async move { source.status(&registry, &id).await.map(Update::Status) });
    }

    /// Asks a provider for its models, unless that was done.
    pub fn ensure_models(&mut self, id: &str) {
        if self.models.contains_key(id) {
            return;
        }
        self.models.insert(id.to_owned(), ModelList::Loading);
        let (registry, source, id) = (
            self.registry.clone(),
            Arc::clone(&self.source),
            id.to_owned(),
        );
        self.spawn(async move {
            let result = match source.provider(&registry, &id).await {
                Ok(provider) => provider
                    .list_models()
                    .await
                    .map_err(|error| error.to_string()),
                Err(error) => Err(scrub(&error.to_string())),
            };
            Some(Update::Models {
                provider: id,
                result,
            })
        });
    }

    /// Chooses the provider for the next message, and makes it the default
    /// in the settings.
    pub fn select_provider(&mut self, id: &str) {
        self.selected = Some(id.to_owned());
        self.change_settings(|settings| settings.default_provider = Some(id.to_owned()));
        self.ensure_models(id);
    }

    /// Chooses the model of the selected provider (`None` for its default),
    /// and keeps the choice in the settings.
    pub fn select_model(&mut self, model: Option<String>) {
        let Some(id) = self.selected.clone() else {
            return;
        };
        self.change_settings(|settings| settings.provider_mut(&id).model = model);
    }

    /// Changes the settings and saves them. A change that would put a
    /// secret in the settings file is refused and reported.
    pub fn change_settings(&mut self, change: impl FnOnce(&mut AiSettings)) -> bool {
        let mut settings = self.registry.settings().clone();
        change(&mut settings);
        if let Err(error) = settings.validate() {
            self.settings_problem = Some(sentence(&error.to_string()));
            return false;
        }
        if settings == *self.registry.settings() {
            return true;
        }
        // Models depend on the address and key, so changed providers are asked again.
        let before = self.registry.settings().clone();
        self.models
            .retain(|id, _| before.provider(id) == settings.provider(id));
        *self.registry.settings_mut() = settings;
        if let Some(path) = &self.settings_file {
            match self.registry.settings().save_to(path) {
                Ok(()) => self.settings_problem = None,
                Err(error) => self.settings_problem = Some(sentence(&error.to_string())),
            }
        }
        self.reselect();
        true
    }

    /// Stores an API key in the key store, then checks the provider again.
    /// The key is not kept anywhere else.
    pub fn save_key(&mut self, id: &str, key: ApiKey) {
        self.key_task(id, KeyNote::Saved, move |registry, id| async move {
            registry.set_key(&id, key).await
        });
    }

    /// Removes the stored API key of a provider.
    pub fn forget_key(&mut self, id: &str) {
        self.key_task(id, KeyNote::Removed, |registry, id| async move {
            registry.delete_key(&id).await
        });
    }

    fn key_task<F, Fut>(&mut self, id: &str, done: KeyNote, work: F)
    where
        F: FnOnce(ProviderRegistry, String) -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), RegistryError>> + Send,
    {
        self.keys.insert(id.to_owned(), KeyNote::Working);
        self.models.remove(id);
        let (registry, provider) = (self.registry.clone(), id.to_owned());
        self.spawn(async move {
            let note = match work(registry, provider.clone()).await {
                Ok(()) => done,
                Err(error) => KeyNote::Failed(sentence(&error.to_string())),
            };
            Some(Update::Key { provider, note })
        });
    }

    /// Starts over with an empty conversation. A running turn is stopped.
    pub fn new_conversation(&mut self) {
        self.stop();
        self.conversation.clear();
    }

    /// Sends a message to the selected provider and starts streaming the
    /// answer into the conversation.
    ///
    /// `system_prompt` is the guide with what the person is looking at,
    /// `tools` the document's tools for API providers, and `mcp_url` the
    /// app's MCP server for agent CLIs, when it is running.
    pub fn send(
        &mut self,
        text: &str,
        images: Vec<ImageData>,
        system_prompt: String,
        tools: &Tools,
        mcp_url: Option<&str>,
    ) -> Result<(), SendError> {
        if self.turn.is_some() {
            return Err(SendError::Busy);
        }
        let text = text.trim();
        if text.is_empty() && images.is_empty() {
            return Err(SendError::Empty);
        }
        let Some(status) = self.selected().cloned() else {
            return Err(SendError::NoProvider);
        };
        let runtime = match &self.runtime {
            Ok(runtime) => runtime,
            Err(problem) => return Err(SendError::NoRuntime(problem.clone())),
        };

        let provider_id = status.info.id.clone();
        let session_id = self
            .conversation
            .session_for(&provider_id)
            .map(str::to_owned);
        self.conversation
            .begin_turn(text, images, &provider_id, &status.info.name);

        let turn = self.next_turn;
        self.next_turn += 1;
        let (sender, waker) = (self.sender.clone(), self.waker.clone());
        let sink_sender = sender.clone();
        let sink_waker = waker.clone();
        let executor = AgentTools::new(ToolBridge::new(tools.clone())).with_image_sink(Arc::new(
            move |name: &str, input: &Value, images: Vec<ImageData>| {
                let _ = sink_sender.send(Update::ToolImages {
                    turn,
                    name: name.to_owned(),
                    input: input.clone(),
                    images,
                });
                sink_waker.wake();
            },
        ));
        let request = ChatRequest {
            messages: self.conversation.history(),
            system_prompt: Some(system_prompt),
            model: None,
            session_id,
            mcp: mcp_url.map(|url| McpEndpoint::new(MCP_SERVER_NAME, url)),
            tools: Some(Arc::new(executor)),
            working_dir: None,
        };

        let changed = Arc::clone(&self.agent_changed);
        let watch = tools.session().subscribe(move |change| {
            if change.origin == Origin::Agent {
                changed.store(true, Ordering::Release);
            }
        });
        let cancel = CancellationToken::new();
        let (registry, source, task_cancel) = (
            self.registry.clone(),
            Arc::clone(&self.source),
            cancel.clone(),
        );
        runtime.spawn(async move {
            let _guard = EndGuard {
                turn,
                updates: sender.clone(),
                waker: waker.clone(),
            };
            let forward = |event: ChatEvent| {
                let _ = sender.send(Update::Event { turn, event });
                waker.wake();
            };
            let provider = match source.provider(&registry, &provider_id).await {
                Ok(provider) => provider,
                Err(error) => {
                    forward(ChatEvent::Error {
                        message: sentence(&scrub(&error.to_string())),
                        kind: registry_error_kind(&error),
                    });
                    forward(ChatEvent::Finished {
                        stop_reason: StopReason::Error,
                    });
                    return;
                }
            };
            let (events, mut receiver) = event_channel();
            let relay = async {
                while let Some(event) = receiver.recv().await {
                    forward(event);
                }
            };
            tokio::join!(provider.send(request, events, task_cancel), relay);
        });
        self.turn = Some(Turn {
            id: turn,
            cancel,
            stopping: false,
            _watch: watch,
        });
        Ok(())
    }

    /// Stops the running turn: the answer ends now, and the provider is
    /// told to stop its work.
    pub fn stop(&mut self) {
        if let Some(turn) = &mut self.turn {
            turn.cancel.cancel();
            turn.stopping = true;
            if self.conversation.is_running() {
                self.conversation.finish(StopReason::Cancelled);
            }
        }
    }

    /// Takes back the last message and sends it again.
    pub fn retry(
        &mut self,
        system_prompt: String,
        tools: &Tools,
        mcp_url: Option<&str>,
    ) -> Result<(), SendError> {
        if self.turn.is_some() {
            return Err(SendError::Busy);
        }
        if self.selected.is_none() {
            return Err(SendError::NoProvider);
        }
        let Some((text, images)) = self.conversation.take_last_request() else {
            return Err(SendError::Empty);
        };
        self.send(&text, images, system_prompt, tools, mcp_url)
    }

    /// Takes in everything the background produced since the last call.
    pub fn poll(&mut self) -> Polled {
        let mut polled = Polled::default();
        while let Ok(update) = self.receiver.try_recv() {
            polled.updated = true;
            self.take(update);
        }
        polled.document_changed = self.agent_changed.swap(false, Ordering::AcqRel);
        polled.updated |= polled.document_changed;
        polled
    }

    /// Waits until no turn runs and nothing is being checked or loaded, or
    /// until `timeout`. For tests and scripts; the window never waits.
    /// Returns true when everything settled.
    pub fn settle(&mut self, timeout: Duration) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            self.poll();
            let loading = self
                .models
                .values()
                .any(|models| *models == ModelList::Loading)
                || self.keys.values().any(|note| *note == KeyNote::Working);
            if !self.is_busy() && !self.checking && !loading {
                return true;
            }
            if std::time::Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn take(&mut self, update: Update) {
        match update {
            Update::Statuses {
                generation,
                statuses,
            } => {
                if generation == self.generation {
                    self.statuses = Some(statuses);
                    self.checking = false;
                    self.reselect();
                }
            }
            Update::Status(status) => {
                if let Some(statuses) = &mut self.statuses
                    && let Some(slot) = statuses
                        .iter_mut()
                        .find(|slot| slot.info.id == status.info.id)
                {
                    *slot = status;
                }
                self.reselect();
            }
            Update::Models { provider, result } => {
                let list = match result {
                    Ok(models) => ModelList::Loaded(models),
                    Err(problem) => ModelList::Failed(problem),
                };
                self.models.insert(provider, list);
            }
            Update::Event { turn, event } => {
                if self.is_current(turn) {
                    self.conversation.apply(&event);
                }
            }
            Update::ToolImages {
                turn,
                name,
                input,
                images,
            } => {
                if self.is_current(turn) {
                    self.conversation.attach_tool_images(&name, &input, images);
                }
            }
            Update::TurnEnded { turn } => {
                if self.is_current(turn) {
                    if self.conversation.is_running() {
                        self.conversation.fail(
                            ErrorKind::Other,
                            "The AI stopped without finishing its answer.",
                        );
                    }
                    self.turn = None;
                }
            }
            Update::Key { provider, note } => {
                self.refresh_status(&provider);
                self.keys.insert(provider, note);
            }
        }
    }

    fn is_current(&self, turn: u64) -> bool {
        self.turn.as_ref().is_some_and(|running| running.id == turn)
    }

    /// Keeps the selected provider if it can still be used, otherwise picks
    /// the default or the first that is ready.
    fn reselect(&mut self) {
        let Some(statuses) = &self.statuses else {
            return;
        };
        let keep = self.selected.as_deref().is_some_and(|id| {
            statuses
                .iter()
                .any(|status| status.info.id == id && status.is_usable())
        });
        if !keep {
            self.selected = choose_provider(self.registry.settings(), statuses)
                .map(|status| status.info.id.clone());
        }
        if let Some(id) = self.selected.clone() {
            self.ensure_models(&id);
        }
    }

    fn spawn(&self, work: impl Future<Output = Option<Update>> + Send + 'static) {
        let Ok(runtime) = &self.runtime else {
            return;
        };
        let (sender, waker) = (self.sender.clone(), self.waker.clone());
        runtime.spawn(async move {
            if let Some(update) = work.await {
                let _ = sender.send(update);
                waker.wake();
            }
        });
    }
}

impl Drop for Controller {
    fn drop(&mut self) {
        if let Some(turn) = &self.turn {
            // Let a cancelled agent CLI stop politely, with everything it
            // started, before the runtime goes. Should it take too long,
            // dropping the turn's work kills the CLI's process group.
            turn.cancel.cancel();
            let (current, deadline) = (turn.id, std::time::Instant::now() + SHUTDOWN_GRACE);
            while let Some(left) = deadline.checked_duration_since(std::time::Instant::now()) {
                match self.receiver.recv_timeout(left) {
                    Ok(Update::TurnEnded { turn }) if turn == current => break,
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
        }
        let runtime = std::mem::replace(&mut self.runtime, Err(String::new()));
        if let Ok(runtime) = runtime {
            runtime.shutdown_timeout(Duration::from_millis(100));
        }
    }
}

/// The kind of failure a registry error is, for the way forward offered.
fn registry_error_kind(error: &RegistryError) -> ErrorKind {
    match error {
        RegistryError::Settings(_) => ErrorKind::InvalidRequest,
        RegistryError::UnknownProvider(_)
        | RegistryError::NotAnApiProvider(_)
        | RegistryError::KeyStore(_) => ErrorKind::Other,
    }
}

/// `text` as a sentence: a capital first letter and a full stop.
pub(crate) fn sentence(text: &str) -> String {
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

#[cfg(test)]
mod tests {
    use moshionly_agent::{Host, SessionHandle};
    use moshionly_ai::{MemoryKeyStore, ReadyState, Role};
    use moshionly_core::{Document, NodeType, build};
    use serde_json::json;

    use super::*;
    use crate::chat::conversation::{AnswerState, Part, ToolState};
    use crate::chat::testing::{ScriptedProvider, ScriptedSource, Step, status, statuses};

    const WAIT: Duration = Duration::from_secs(10);

    fn controller(source: ScriptedSource) -> Controller {
        let registry =
            ProviderRegistry::new(AiSettings::default(), Arc::new(MemoryKeyStore::new()));
        let mut controller = Controller::new(registry, Arc::new(source), None);
        controller.ensure_checked();
        assert!(controller.settle(WAIT));
        controller
    }

    fn session() -> (SessionHandle, String) {
        let mut document = Document::new("Test");
        let (artboard, edits) = build::add_artboard(&document, "Launch", 800.0, 600.0).unwrap();
        document.apply(&edits).unwrap();
        let rect = build::default_layer(NodeType::Rect).unwrap();
        let (layer, edits) = build::add_layer(&document, &artboard, rect).unwrap();
        document.apply(&edits).unwrap();
        (SessionHandle::from_document(document), layer.to_string())
    }

    fn tools(session: &SessionHandle) -> Tools {
        Tools::new(session.clone(), Host::new())
    }

    fn send(controller: &mut Controller, session: &SessionHandle, text: &str) {
        controller
            .send(
                text,
                Vec::new(),
                "guide".into(),
                &tools(session),
                Some("http://127.0.0.1:1/mcp"),
            )
            .unwrap();
    }

    /// Polls until `done` holds.
    fn wait_until(controller: &mut Controller, done: impl Fn(&Controller) -> bool) {
        let deadline = std::time::Instant::now() + WAIT;
        while !done(controller) {
            controller.poll();
            assert!(std::time::Instant::now() < deadline, "timed out");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn picks_the_first_ready_provider_and_lists_its_models() {
        let source = ScriptedSource::new(statuses(&["anthropic"]))
            .with_provider(ScriptedProvider::new("anthropic", Vec::new()));
        let controller = controller(source);
        assert_eq!(controller.selected().unwrap().info.id, "anthropic");
        assert_eq!(
            controller.models("anthropic"),
            Some(&ModelList::Loaded(vec![ModelInfo::new(
                "scripted",
                "Scripted model"
            )]))
        );
    }

    #[test]
    fn nothing_is_selected_when_nothing_is_ready() {
        let controller = controller(ScriptedSource::new(statuses(&[])));
        assert!(controller.statuses().is_some());
        assert!(controller.selected().is_none());
        let mut controller = controller;
        let (session, _) = session();
        assert_eq!(
            controller.send("hi", Vec::new(), String::new(), &tools(&session), None),
            Err(SendError::NoProvider)
        );
    }

    #[test]
    fn a_turn_streams_into_the_conversation_and_builds_the_request() {
        let provider = ScriptedProvider::new(
            "claude-code",
            vec![
                Step::Event(ChatEvent::SessionStarted {
                    session_id: "s-1".into(),
                }),
                Step::thinking("Looking."),
                Step::text("Hello"),
                Step::text(" there."),
                Step::Event(ChatEvent::Usage {
                    input_tokens: 5,
                    output_tokens: 2,
                    cost_usd: Some(0.01),
                }),
            ],
        );
        let source =
            ScriptedSource::new(statuses(&["claude-code"])).with_provider(provider.clone());
        let mut controller = controller(source);
        let (session, _) = session();
        send(&mut controller, &session, "  Say hello  ");
        assert!(controller.is_busy());
        assert!(controller.settle(WAIT));

        let answer = controller.conversation().messages().last().unwrap();
        assert_eq!(answer.state, AnswerState::Finished(StopReason::EndTurn));
        assert_eq!(answer.text(), "Hello there.");
        assert_eq!(answer.parts[0], Part::Thinking("Looking.".into()));
        assert_eq!(answer.usage.unwrap().cost_usd, Some(0.01));
        assert_eq!(
            controller.conversation().session_for("claude-code"),
            Some("s-1")
        );

        let request = &provider.requests()[0];
        assert_eq!(request.messages.len(), 1);
        assert_eq!(request.messages[0].text, "Say hello");
        assert_eq!(request.system_prompt.as_deref(), Some("guide"));
        assert_eq!(request.session_id, None);
        assert_eq!(request.model, None);
        assert_eq!(
            request.mcp,
            Some(McpEndpoint::new(MCP_SERVER_NAME, "http://127.0.0.1:1/mcp"))
        );
        assert!(request.tools.is_some());

        // The next turn continues the session and carries the history.
        send(&mut controller, &session, "Again");
        assert!(controller.settle(WAIT));
        let request = &provider.requests()[1];
        assert_eq!(request.session_id.as_deref(), Some("s-1"));
        let roles: Vec<Role> = request.messages.iter().map(|m| m.role).collect();
        assert_eq!(roles, vec![Role::User, Role::Assistant, Role::User]);
    }

    #[test]
    fn one_turn_at_a_time() {
        let provider = ScriptedProvider::new("anthropic", vec![Step::WaitForStop]);
        let source = ScriptedSource::new(statuses(&["anthropic"])).with_provider(provider);
        let mut controller = controller(source);
        let (session, _) = session();
        send(&mut controller, &session, "first");
        assert_eq!(
            controller.send("second", Vec::new(), String::new(), &tools(&session), None),
            Err(SendError::Busy)
        );
        controller.stop();
        assert!(controller.settle(WAIT));
    }

    #[test]
    fn stop_ends_the_answer_at_once_and_cancels_the_provider() {
        let provider = ScriptedProvider::new(
            "anthropic",
            vec![
                Step::text("Working"),
                Step::Event(ChatEvent::ToolCallStarted {
                    id: "t".into(),
                    name: "screenshot".into(),
                    input: json!({}),
                }),
                Step::WaitForStop,
            ],
        );
        let source = ScriptedSource::new(statuses(&["anthropic"])).with_provider(provider.clone());
        let mut controller = controller(source);
        let (session, _) = session();
        send(&mut controller, &session, "go");
        wait_until(&mut controller, |c| {
            c.conversation()
                .messages()
                .last()
                .unwrap()
                .tool_calls()
                .count()
                == 1
        });

        controller.stop();
        let answer = controller.conversation().messages().last().unwrap();
        assert_eq!(answer.state, AnswerState::Finished(StopReason::Cancelled));
        assert_eq!(
            answer.tool_calls().next().unwrap().state,
            ToolState::Interrupted
        );
        assert!(controller.is_stopping());
        assert!(controller.settle(WAIT));
        assert!(provider.was_stopped());
        assert!(!controller.is_busy());
        // The late Finished event did not reopen or change the answer.
        let answer = controller.conversation().messages().last().unwrap();
        assert_eq!(answer.text(), "Working");
    }

    #[test]
    fn dropping_the_controller_cancels_the_turn() {
        let provider = ScriptedProvider::new("anthropic", vec![Step::WaitForStop]);
        let source = ScriptedSource::new(statuses(&["anthropic"])).with_provider(provider.clone());
        let mut controller = controller(source);
        let (session, _) = session();
        send(&mut controller, &session, "go");
        wait_until(&mut controller, |_| !provider.requests().is_empty());
        drop(controller);
        assert!(provider.was_stopped());
    }

    #[test]
    fn an_api_tool_call_changes_the_document_and_is_reported() {
        let (session, layer) = session();
        let provider = ScriptedProvider::new(
            "anthropic",
            vec![
                Step::call(
                    "c1",
                    "apply_edits",
                    json!({ "operations": [{ "op": "update", "id": layer, "set": { "x": 77 } }] }),
                ),
                Step::text("Moved it."),
            ],
        );
        let source = ScriptedSource::new(statuses(&["anthropic"])).with_provider(provider);
        let mut controller = controller(source);
        let revision = session.revision();
        send(&mut controller, &session, "move it");
        let mut changed = false;
        let deadline = std::time::Instant::now() + WAIT;
        while controller.is_busy() || !changed {
            changed |= controller.poll().document_changed;
            assert!(std::time::Instant::now() < deadline, "timed out");
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(session.revision() > revision);
        let answer = controller.conversation().messages().last().unwrap();
        let call = answer.tool_calls().next().unwrap();
        assert_eq!(call.state, ToolState::Succeeded);
        assert_eq!(answer.text(), "Moved it.");
        // Nothing more changed since.
        assert!(!controller.poll().document_changed);
    }

    #[test]
    fn errors_from_the_provider_and_the_registry_reach_the_conversation() {
        let provider = ScriptedProvider::new(
            "claude-code",
            vec![Step::Event(ChatEvent::Error {
                message: "not signed in: run claude".into(),
                kind: ErrorKind::NotSignedIn,
            })],
        );
        let source =
            ScriptedSource::new(statuses(&["claude-code", "anthropic"])).with_provider(provider);
        let mut controller = controller(source);
        let (session, _) = session();
        send(&mut controller, &session, "hi");
        assert!(controller.settle(WAIT));
        let answer = controller.conversation().messages().last().unwrap();
        assert_eq!(answer.errors().next().unwrap().kind, ErrorKind::NotSignedIn);
        assert_eq!(answer.state, AnswerState::Finished(StopReason::Error));

        // Anthropic is ready but has no provider to build: the registry's error shows.
        controller.select_provider("anthropic");
        send(&mut controller, &session, "hi");
        assert!(controller.settle(WAIT));
        let answer = controller.conversation().messages().last().unwrap();
        let error = answer.errors().next().unwrap();
        assert_eq!(error.kind, ErrorKind::Other);
        assert!(error.message.contains("anthropic"), "{}", error.message);
    }

    #[test]
    fn retry_sends_the_last_message_again() {
        let provider = ScriptedProvider::new("anthropic", vec![Step::text("ok")]);
        let source = ScriptedSource::new(statuses(&["anthropic"])).with_provider(provider.clone());
        let mut controller = controller(source);
        let (session, _) = session();
        send(&mut controller, &session, "once");
        assert!(controller.settle(WAIT));
        controller
            .retry(String::new(), &tools(&session), None)
            .unwrap();
        assert!(controller.settle(WAIT));
        assert_eq!(controller.conversation().messages().len(), 2);
        assert_eq!(provider.requests().len(), 2);
        assert_eq!(provider.requests()[1].messages.len(), 1);
    }

    #[test]
    fn new_conversation_forgets_everything() {
        let provider = ScriptedProvider::new("anthropic", vec![Step::text("ok")]);
        let source = ScriptedSource::new(statuses(&["anthropic"])).with_provider(provider);
        let mut controller = controller(source);
        let (session, _) = session();
        send(&mut controller, &session, "once");
        assert!(controller.settle(WAIT));
        controller.new_conversation();
        assert!(controller.conversation().is_empty());
    }

    #[test]
    fn choosing_a_provider_and_model_updates_and_saves_the_settings() {
        let dir = std::env::temp_dir().join(format!("moshionly-chat-{}", std::process::id()));
        let file = dir.join("ai-settings.json");
        let source = ScriptedSource::new(statuses(&["claude-code", "anthropic"]))
            .with_provider(ScriptedProvider::new("claude-code", Vec::new()))
            .with_provider(ScriptedProvider::new("anthropic", Vec::new()));
        let registry =
            ProviderRegistry::new(AiSettings::default(), Arc::new(MemoryKeyStore::new()));
        let mut controller = Controller::new(registry, Arc::new(source), Some(file.clone()));
        controller.ensure_checked();
        assert!(controller.settle(WAIT));
        assert_eq!(controller.selected().unwrap().info.id, "claude-code");

        controller.select_provider("anthropic");
        controller.select_model(Some("claude-sonnet-5-5".into()));
        assert_eq!(controller.selected().unwrap().info.id, "anthropic");
        let saved = AiSettings::load_from(&file).unwrap();
        assert_eq!(saved.default_provider(), Some("anthropic"));
        assert_eq!(
            saved.provider("anthropic").model(),
            Some("claude-sonnet-5-5")
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_secret_in_an_address_is_refused() {
        let mut controller = controller(ScriptedSource::new(statuses(&[])));
        let accepted = controller.change_settings(|settings| {
            settings.provider_mut("custom").base_url =
                Some("https://user:hunter2@example.com/v1".into());
        });
        assert!(!accepted);
        assert_eq!(controller.settings().provider("custom").base_url, None);
        assert!(controller.settings_problem().unwrap().contains("password"));
    }

    #[test]
    fn keys_are_stored_and_removed_without_coming_back() {
        let keys = Arc::new(MemoryKeyStore::new());
        let registry = ProviderRegistry::new(AiSettings::default(), keys.clone());
        let source = ScriptedSource::new(vec![
            status("anthropic", ReadyState::MissingKey, "Add a key.").unwrap(),
        ]);
        let mut controller = Controller::new(registry, Arc::new(source), None);
        controller.ensure_checked();
        assert!(controller.settle(WAIT));
        assert!(!controller.status("anthropic").unwrap().key_stored);

        controller.save_key("anthropic", ApiKey::new("sk-ant-test-0123456789abcdef"));
        assert!(controller.settle(WAIT));
        assert_eq!(controller.key_note("anthropic"), Some(&KeyNote::Saved));
        use moshionly_ai::KeyStore as _;
        assert_eq!(
            keys.get("anthropic").unwrap().unwrap().expose(),
            "sk-ant-test-0123456789abcdef"
        );
        wait_until(&mut controller, |c| {
            c.status("anthropic").unwrap().key_stored
        });

        controller.forget_key("anthropic");
        assert!(controller.settle(WAIT));
        assert_eq!(controller.key_note("anthropic"), Some(&KeyNote::Removed));
        assert!(keys.get("anthropic").unwrap().is_none());

        controller.save_key("claude-code", ApiKey::new("sk-nope-0123456789abcdef"));
        assert!(controller.settle(WAIT));
        assert!(matches!(
            controller.key_note("claude-code"),
            Some(KeyNote::Failed(_))
        ));
    }

    #[test]
    fn sentences_start_with_a_capital_and_end_with_a_stop() {
        assert_eq!(sentence(" no key is set "), "No key is set.");
        assert_eq!(sentence("Done!"), "Done!");
        assert_eq!(sentence(""), "");
    }
}
