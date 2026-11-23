//! Chat providers for Moshionly.
//!
//! This crate gives the app one chat interface over many AI providers, so a
//! user can work with whichever AI they already pay for. It has no user
//! interface and does not depend on the other Moshionly crates.
//!
//! There are two kinds of provider behind the [`ChatProvider`] trait:
//!
//! - **Subscription providers** run an agent CLI that the user has installed
//!   and signed in to. Moshionly starts it non-interactively, attaches the
//!   Moshionly MCP server and streams its output. It never reads, copies or
//!   stores the CLI's credentials; it only runs the binary.
//! - **API providers** call a model API with the user's key and run the tool
//!   loop in-process against a [`ToolExecutor`] that the app supplies.
//!
//! A turn is one call to [`ChatProvider::send`] with a [`ChatRequest`]. The
//! answer arrives as a stream of [`ChatEvent`]s that always ends with
//! [`ChatEvent::Finished`].
//!
//! The app does not construct providers one by one. A [`ProviderRegistry`]
//! joins the list of providers with the user's [`AiSettings`] and their API
//! keys, which live in the OS keychain behind the [`KeyStore`] trait:
//!
//! ```no_run
//! # use std::sync::Arc;
//! use moshionly_ai::{
//!     AiSettings, CancellationToken, ChatEvent, ChatRequest, McpEndpoint, ProviderRegistry,
//!     choose_provider, event_channel,
//! };
//!
//! # async fn demo(
//! #     tools: Arc<dyn moshionly_ai::ToolExecutor>,
//! # ) -> Result<(), Box<dyn std::error::Error>> {
//! // The user's choices, and their keys in the OS keychain.
//! let registry = ProviderRegistry::with_keychain(AiSettings::load().unwrap_or_default());
//!
//! // For a settings screen: every provider and whether it can be used now.
//! let statuses = registry.statuses().await;
//!
//! // For the chat: the user's default, or else the first provider that is ready.
//! let Some(chosen) = choose_provider(registry.settings(), &statuses) else {
//!     return Ok(()); // nothing is ready; send the user to the settings screen
//! };
//! let provider = registry.provider(&chosen.info.id).await?;
//!
//! let request = ChatRequest {
//!     system_prompt: Some("You edit motion designs.".to_owned()),
//!     // Subscription providers reach the tools through the app's MCP server...
//!     mcp: Some(McpEndpoint::new("moshionly", "http://127.0.0.1:47821/mcp")),
//!     // ...and API providers call them in-process.
//!     tools: Some(tools),
//!     ..ChatRequest::from_prompt("Fade the title in over half a second")
//! };
//!
//! let (events, mut receiver) = event_channel();
//! let cancel = CancellationToken::new(); // cancel() stops the turn
//! tokio::spawn(async move { provider.send(request, events, cancel).await });
//!
//! while let Some(event) = receiver.recv().await {
//!     match event {
//!         ChatEvent::TextDelta { text } => print!("{text}"),
//!         ChatEvent::Error { message, .. } => eprintln!("{message}"),
//!         ChatEvent::Finished { .. } => break,
//!         _ => {}
//!     }
//! }
//! # Ok(())
//! # }
//! ```
//!
//! The crate README describes each provider, what has been verified against
//! the real services, and how to add a provider.

#![warn(missing_docs)]

mod api;
mod chat;
mod cli;
mod error;
mod event;
mod http;
mod keys;
mod paths;
mod provider;
pub mod redact;
mod registry;
mod settings;
mod tool;

pub use api::anthropic::{
    ANTHROPIC_API_URL, ANTHROPIC_DEFAULT_MODEL, ANTHROPIC_ID, Anthropic, AnthropicOptions, Effort,
};
pub use api::openai::presets::{OPENAI_PRESETS, OpenAiPreset, openai_preset};
pub use api::openai::{OpenAiCompatible, OpenAiCompatibleOptions};
pub use chat::{ChatMessage, ChatRequest, ImageData, McpEndpoint, Role};
pub use cli::claude_code::{CLAUDE_CODE_ID, ClaudeCode, ClaudeCodeOptions};
pub use cli::codex::{CODEX_ID, Codex, CodexOptions};
pub use cli::cursor_agent::{CURSOR_AGENT_ID, CursorAgent, CursorAgentOptions};
pub use cli::gemini::{GEMINI_CLI_ID, GeminiCli, GeminiCliOptions};
pub use error::{AiError, ErrorKind};
pub use event::{ChatEvent, EventReceiver, EventSender, StopReason, event_channel};
pub use keys::{KEYCHAIN_SERVICE, KeyStore, KeyStoreError, KeychainStore, MemoryKeyStore};
pub use paths::{config_dir, default_working_dir};
pub use provider::{
    Capabilities, ChatProvider, ModelInfo, ProviderInfo, ProviderKind, Readiness, ReadyState,
    ToolTransport,
};
pub use redact::ApiKey;
pub use registry::{
    ProviderDescriptor, ProviderRegistry, ProviderStatus, RegistryError, choose_provider,
    known_providers,
};
pub use settings::{AiSettings, ProviderSettings, SETTINGS_FILE, SettingsError};
pub use tool::{ToolContent, ToolExecutor, ToolOutput, ToolSpec};

/// The token used to cancel a request, re-exported so that callers do not
/// need their own dependency on `tokio-util`.
pub use tokio_util::sync::CancellationToken;
