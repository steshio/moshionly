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
pub use tool::{ToolContent, ToolExecutor, ToolOutput, ToolSpec};

/// The token used to cancel a request, re-exported so that callers do not
/// need their own dependency on `tokio-util`.
pub use tokio_util::sync::CancellationToken;
