//! The one interface every provider implements.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::chat::ChatRequest;
use crate::error::{AiError, ErrorKind};
use crate::event::EventSender;

/// How a provider is paid for and reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    /// An agent CLI the user installed and signed in to. Moshionly only runs
    /// the binary.
    Subscription,
    /// A model API called directly, with the user's key or a local server.
    Api,
}

/// Who a provider is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderInfo {
    /// A stable id used in settings and on the command line, such as
    /// `claude-code` or `openrouter`.
    pub id: String,
    /// The name shown to the user.
    pub name: String,
    /// Subscription or API.
    pub kind: ProviderKind,
}

/// How a provider reaches the Moshionly tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolTransport {
    /// Through the MCP server in [`ChatRequest::mcp`].
    Mcp,
    /// Through the executor in [`ChatRequest::tools`].
    InProcess,
}

/// What a provider can do, so the app can adapt its interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    /// How tools are reached.
    pub tools: ToolTransport,
    /// Images attached to user messages are passed to the model.
    pub images: bool,
    /// The provider keeps the conversation and can continue it from a session
    /// id. When false, the app sends the whole conversation every time.
    pub resume: bool,
    /// The model's reasoning may arrive as
    /// [`ChatEvent::ThinkingDelta`](crate::ChatEvent::ThinkingDelta).
    pub thinking: bool,
    /// [`ChatProvider::list_models`] asks the service. When false the list is
    /// a fixed set of suggestions and any model id may be typed in.
    pub live_model_list: bool,
    /// An API key is needed before the provider can be used.
    pub needs_api_key: bool,
    /// The provider has been run against the real service or CLI by the
    /// maintainers. False marks a provider written from documentation only.
    pub verified: bool,
}

/// A model the user can pick.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelInfo {
    /// The id to put in [`ChatRequest::model`].
    pub id: String,
    /// The name shown to the user.
    pub name: String,
}

impl ModelInfo {
    /// A model with separate id and display name.
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
        }
    }

    /// A model whose id doubles as its display name.
    pub fn from_id(id: impl Into<String>) -> Self {
        let id = id.into();
        Self {
            name: id.clone(),
            id,
        }
    }
}

/// Whether a provider can be used right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadyState {
    /// Good to go.
    Ready,
    /// The agent CLI was not found.
    NotInstalled,
    /// The agent CLI is installed but not signed in.
    NotSignedIn,
    /// No API key has been stored.
    MissingKey,
    /// The API rejected the stored key.
    InvalidKey,
    /// The service or local server did not answer.
    Unreachable,
    /// The check itself failed for another reason.
    Unknown,
}

/// The answer to "can I use this provider?", for a settings screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Readiness {
    /// The verdict.
    pub state: ReadyState,
    /// One sentence for the user: what was found, or what to do next.
    pub message: String,
    /// The CLI version, when the provider is a CLI and it answered.
    pub version: Option<String>,
}

impl Readiness {
    /// Ready, with a sentence describing what was found.
    pub fn ready(message: impl Into<String>) -> Self {
        Self {
            state: ReadyState::Ready,
            message: message.into(),
            version: None,
        }
    }

    /// Not ready, with a sentence saying what to do.
    pub fn not_ready(state: ReadyState, message: impl Into<String>) -> Self {
        Self {
            state,
            message: message.into(),
            version: None,
        }
    }

    /// Records the CLI version.
    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    /// True when the provider can be used.
    pub fn is_ready(&self) -> bool {
        self.state == ReadyState::Ready
    }

    /// Describes a failed readiness probe in the same terms as a failed chat.
    pub fn from_error(error: &AiError) -> Self {
        let state = match error.kind() {
            ErrorKind::NotInstalled => ReadyState::NotInstalled,
            ErrorKind::NotSignedIn => ReadyState::NotSignedIn,
            ErrorKind::MissingKey => ReadyState::MissingKey,
            ErrorKind::InvalidKey => ReadyState::InvalidKey,
            ErrorKind::Network => ReadyState::Unreachable,
            _ => ReadyState::Unknown,
        };
        Self::not_ready(state, error.to_string())
    }
}

/// A chat provider: one way of getting an answer from a model.
///
/// Implementations are cheap to construct and hold no conversation state.
/// See the crate README for how to add one.
#[async_trait]
pub trait ChatProvider: Send + Sync {
    /// Who this provider is.
    fn info(&self) -> ProviderInfo;

    /// What it can do.
    fn capabilities(&self) -> Capabilities;

    /// The models the user can choose from. The first one is the default.
    async fn list_models(&self) -> Result<Vec<ModelInfo>, AiError>;

    /// Checks whether the provider can be used: is the CLI installed and
    /// signed in, is there a key, does the local server answer. This never
    /// sends a prompt and never costs anything.
    async fn check_ready(&self) -> Readiness;

    /// Answers `request`, reporting progress through `events`.
    ///
    /// The call returns when the turn is over. It always ends the stream with
    /// one [`ChatEvent::Finished`](crate::ChatEvent::Finished); failures are
    /// reported as a [`ChatEvent::Error`](crate::ChatEvent::Error) just
    /// before it. Cancelling `cancel`, or dropping the receiver, stops the
    /// work promptly: a CLI child process is terminated together with any
    /// processes it started, and an HTTP stream is closed.
    async fn send(&self, request: ChatRequest, events: EventSender, cancel: CancellationToken);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readiness_from_error_maps_the_actionable_kinds() {
        let cases = [
            (ErrorKind::NotInstalled, ReadyState::NotInstalled),
            (ErrorKind::NotSignedIn, ReadyState::NotSignedIn),
            (ErrorKind::MissingKey, ReadyState::MissingKey),
            (ErrorKind::InvalidKey, ReadyState::InvalidKey),
            (ErrorKind::Network, ReadyState::Unreachable),
            (ErrorKind::Overloaded, ReadyState::Unknown),
        ];
        for (kind, state) in cases {
            let readiness = Readiness::from_error(&AiError::from_kind(kind, "x"));
            assert_eq!(readiness.state, state);
            assert!(!readiness.is_ready());
        }
    }

    #[test]
    fn model_from_id_uses_the_id_as_name() {
        let model = ModelInfo::from_id("llama3.2");
        assert_eq!(model.id, "llama3.2");
        assert_eq!(model.name, "llama3.2");
    }

    #[test]
    fn provider_trait_is_object_safe() {
        fn assert_object_safe(_: Option<&dyn ChatProvider>) {}
        assert_object_safe(None);
    }
}
