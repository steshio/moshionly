//! The AI chat: a panel where the person asks an AI to change the design,
//! and the settings where they choose which AI that is.
//!
//! The chat talks to the AI through `moshionly-ai` and lets it work in the
//! document through the tools of `moshionly-agent`. It is self-contained:
//! the shell owns one [`Chat`], hands it plain [`ChatInputs`] every frame
//! and reads back a [`ChatOutput`]. It knows nothing else about the editor.
//!
//! # How the shell uses it
//!
//! ```no_run
//! use moshionly_agent::{Host, SessionHandle, Tools};
//! use moshionly_app::chat::{Chat, ChatInputs, Focus};
//! use moshionly_app::dialogs::NativeDialogs;
//! use moshionly_core::{Document, NodeId};
//!
//! // Once, when the window opens: the chat loads the AI settings and keeps
//! // API keys in the system's keychain.
//! let mut chat = Chat::new();
//! let tools = Tools::new(SessionHandle::from_document(Document::new("Demo")), Host::new());
//!
//! // Every frame, inside the chat panel's rectangle:
//! # fn frame(ui: &mut egui::Ui, chat: &mut Chat, tools: &Tools, artboard: &NodeId, selection: &[NodeId]) {
//! let output = chat.show(
//!     ui,
//!     ChatInputs {
//!         tools,
//!         // The app's MCP server, while it runs, for agents such as Claude Code.
//!         mcp_url: Some("http://127.0.0.1:47821/mcp"),
//!         // What the person is looking at, for the AI's instructions.
//!         focus: Focus { artboard: Some(artboard), selection, time_ms: Some(500.0) },
//!         dialogs: Some(&NativeDialogs),
//!     },
//! );
//! if output.close {
//!     // hide the panel
//! }
//! if output.document_changed {
//!     // the AI edited the document; the session has already told the editor
//! }
//! # }
//! ```
//!
//! While the panel is hidden, call [`Chat::poll`] once a frame instead, so
//! a turn that is still running keeps reporting. When another document is
//! opened, [`Chat::new_conversation`] starts the chat over, so the AI does
//! not carry on about a file that is gone.
//!
//! The colours and sizes come from [`crate::theme`] through the `ui`, like
//! every other widget of the app.
//!
//! # The parts
//!
//! - [`conversation`]: the messages, built up from a provider's events.
//! - [`tool_summary`]: a few readable words for each tool call.
//! - [`executor`]: the agent tools as API providers call them.
//! - [`source`]: where providers come from: the registry, or in tests
//!   [`testing`]'s scripted stand-ins.
//! - [`controller`]: runs turns and provider checks in the background.
//! - [`format`]: the bit of formatting answers may use.
//! - [`help`]: the way forward after each kind of failure.
//! - The panel itself (`panel`, `messages`), the AI settings dialog
//!   (`settings_view`) and the picture cache (`images`).

pub mod controller;
pub mod conversation;
pub mod executor;
pub mod format;
pub mod help;
mod images;
mod messages;
mod panel;
mod settings_view;
pub mod source;
pub mod testing;
pub mod tool_summary;

use std::path::PathBuf;
use std::sync::Arc;

use moshionly_agent::{ToolBridge, Tools};
use moshionly_ai::{AiSettings, ImageData, ProviderRegistry};

pub use controller::{Controller, KeyNote, MCP_SERVER_NAME, ModelList, Polled, SendError};
pub use conversation::{
    AnswerState, Author, ChatError, Conversation, Message, Part, ProviderSession, ToolCall,
    ToolState, Usage,
};
pub use executor::AgentTools;
pub use images::{AttachError, MAX_ATTACHMENT_BYTES};
pub use moshionly_agent::guide::Focus;
pub use source::{ProviderSource, RegistrySource};

use crate::dialogs::Dialogs;
use panel::{PanelAction, PanelView};
use settings_view::SettingsView;

/// What the chat needs from the editor, every frame.
pub struct ChatInputs<'a> {
    /// The tools of the open document. Their session is the document the
    /// AI edits; the host behind them renders its screenshots.
    pub tools: &'a Tools,
    /// The address of the app's MCP server while it runs, such as
    /// `http://127.0.0.1:47821/mcp`. Agent CLIs reach the tools only
    /// through it; without it they can talk but not edit.
    pub mcp_url: Option<&'a str>,
    /// What the person is looking at: the artboard, the selection and the
    /// playhead. The AI is told about it with each message.
    pub focus: Focus<'a>,
    /// Asks which pictures to attach. Without it the attach button is
    /// hidden.
    pub dialogs: Option<&'a dyn Dialogs>,
}

/// What happened in the chat this frame.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ChatOutput {
    /// The person closed the panel.
    pub close: bool,
    /// The AI changed the document since the last frame. The session has
    /// already notified its subscribers; this is for anything else the
    /// shell wants to do, such as a note to the person.
    pub document_changed: bool,
    /// An answer is arriving or being stopped.
    pub busy: bool,
}

/// The chat panel and everything behind it. The shell owns one.
pub struct Chat {
    controller: Controller,
    draft: String,
    attachments: Vec<ImageData>,
    notice: Option<String>,
    images: images::ImageCache,
    settings: SettingsView,
    settings_open: bool,
    /// The settings were opened by themselves once, because no AI was
    /// ready. They are not opened by themselves again.
    offered_settings: bool,
}

impl Default for Chat {
    fn default() -> Self {
        Self::new()
    }
}

impl Chat {
    /// The chat as the app uses it: the person's AI settings from their
    /// usual file, keys in the system's keychain, and the real providers.
    ///
    /// Nothing is checked or started until the panel is first shown.
    pub fn new() -> Self {
        let (settings, file, problem) = match AiSettings::load() {
            Ok(settings) => (settings, AiSettings::default_path(), None),
            Err(error) => (
                AiSettings::default(),
                None,
                Some(format!(
                    "{} Changes made here are not saved until it is repaired or removed.",
                    controller::sentence(&error.to_string())
                )),
            ),
        };
        let registry = ProviderRegistry::with_keychain(settings);
        let mut chat = Self::with_source(registry, Arc::new(RegistrySource), file);
        chat.controller.set_settings_problem(problem);
        chat
    }

    /// A chat over `registry` (settings and key store) that checks and
    /// builds providers with `source`, saving settings changes to
    /// `settings_file` when given. Tests pass a
    /// [`ScriptedSource`](testing::ScriptedSource) and an in-memory key
    /// store.
    pub fn with_source(
        registry: ProviderRegistry,
        source: Arc<dyn ProviderSource>,
        settings_file: Option<PathBuf>,
    ) -> Self {
        Self {
            controller: Controller::new(registry, source, settings_file),
            draft: String::new(),
            attachments: Vec::new(),
            notice: None,
            images: images::ImageCache::default(),
            settings: SettingsView::default(),
            settings_open: false,
            offered_settings: false,
        }
    }

    /// The engine behind the panel: the conversation, the providers and
    /// the running turn.
    pub fn controller(&self) -> &Controller {
        &self.controller
    }

    /// The engine behind the panel, for changing it.
    pub fn controller_mut(&mut self) -> &mut Controller {
        &mut self.controller
    }

    /// The text in the composer.
    pub fn draft_mut(&mut self) -> &mut String {
        &mut self.draft
    }

    /// Starts over with an empty conversation, stopping an answer that is
    /// still arriving.
    pub fn new_conversation(&mut self) {
        self.controller.new_conversation();
        self.images.clear();
        self.attachments.clear();
        self.notice = None;
    }

    /// Attaches a picture to the next message.
    pub fn attach(&mut self, image: ImageData) {
        self.attachments.push(image);
    }

    /// Opens the AI settings dialog.
    pub fn open_settings(&mut self) {
        self.settings_open = true;
    }

    /// Closes the AI settings dialog.
    pub fn close_settings(&mut self) {
        self.settings_open = false;
        self.settings.reset();
    }

    /// Whether the AI settings dialog is open.
    pub fn is_settings_open(&self) -> bool {
        self.settings_open
    }

    /// Opens one provider's details in the AI settings, or closes them all.
    pub fn expand_in_settings(&mut self, provider: Option<&str>) {
        self.settings.expand(provider);
    }

    /// Takes in what the background produced, without drawing. Call it
    /// every frame while the panel is hidden; [`show`](Self::show) does it
    /// otherwise.
    pub fn poll(&mut self, ctx: &egui::Context) -> ChatOutput {
        self.controller.attach(ctx);
        let polled = self.controller.poll();
        ChatOutput {
            close: false,
            document_changed: polled.document_changed,
            busy: self.controller.is_busy(),
        }
    }

    /// Draws the panel into `ui`'s available rectangle, and the AI settings
    /// over the window when they are open.
    pub fn show(&mut self, ui: &mut egui::Ui, inputs: ChatInputs<'_>) -> ChatOutput {
        let mut output = self.poll(ui.ctx());
        self.controller.ensure_checked();
        if !self.offered_settings && self.controller.statuses().is_some() {
            self.offered_settings = true;
            if self.controller.selected().is_none() && self.controller.runtime_problem().is_none() {
                self.settings_open = true;
            }
        }

        let view = PanelView {
            draft: &mut self.draft,
            attachments: &self.attachments,
            can_attach: inputs.dialogs.is_some(),
            mcp_running: inputs.mcp_url.is_some(),
            has_selection: !inputs.focus.selection.is_empty(),
            notice: self.notice.as_deref(),
            dialog_open: self.settings_open,
        };
        let actions = panel::panel(ui, &self.controller, view, &mut self.images);
        for action in actions {
            self.perform(action, &inputs, &mut output);
        }

        if self.settings_open && self.settings.show(ui.ctx(), &mut self.controller) {
            self.settings_open = false;
        }
        output.busy = self.controller.is_busy();
        output
    }

    /// Sends a message as if it was typed, with the pictures attached so
    /// far. When it cannot be sent, the reason is shown above the composer
    /// and the text stays there.
    pub fn send(&mut self, text: &str, inputs: &ChatInputs<'_>) -> Result<(), SendError> {
        let images = self.attachments.clone();
        let result = self.controller.send(
            text,
            images,
            system_prompt(inputs),
            inputs.tools,
            inputs.mcp_url,
        );
        match &result {
            Ok(()) => {
                self.attachments.clear();
                self.notice = None;
            }
            Err(error) => {
                if self.draft.trim().is_empty() {
                    text.clone_into(&mut self.draft);
                }
                self.notice = Some(controller::sentence(&error.to_string()));
            }
        }
        result
    }

    fn perform(&mut self, action: PanelAction, inputs: &ChatInputs<'_>, output: &mut ChatOutput) {
        match action {
            PanelAction::Close => output.close = true,
            PanelAction::NewChat => self.new_conversation(),
            PanelAction::OpenSettings => self.open_settings(),
            PanelAction::Send(text) => {
                // A refusal is shown as the notice; the text goes back in the box.
                let _ = self.send(&text, inputs);
            }
            PanelAction::Stop => self.controller.stop(),
            PanelAction::TryAgain => {
                let result =
                    self.controller
                        .retry(system_prompt(inputs), inputs.tools, inputs.mcp_url);
                self.notice = result
                    .err()
                    .map(|error| controller::sentence(&error.to_string()));
            }
            PanelAction::SelectProvider(id) => self.controller.select_provider(&id),
            PanelAction::SelectModel(model) => self.controller.select_model(model),
            PanelAction::Attach => self.pick_attachments(inputs),
            PanelAction::RemoveAttachment(index) => {
                if index < self.attachments.len() {
                    self.attachments.remove(index);
                }
            }
        }
    }

    fn pick_attachments(&mut self, inputs: &ChatInputs<'_>) {
        let Some(paths) = inputs.dialogs.and_then(|dialogs| dialogs.import_images()) else {
            return;
        };
        self.notice = None;
        for path in paths {
            match images::read_attachment(&path) {
                Ok(image) => self.attachments.push(image),
                Err(error) => self.notice = Some(controller::sentence(&error.to_string())),
            }
        }
    }
}

/// The instructions for the AI: the agent guide and what the person is
/// looking at.
fn system_prompt(inputs: &ChatInputs<'_>) -> String {
    ToolBridge::new(inputs.tools.clone()).system_prompt(Some(&inputs.focus))
}
