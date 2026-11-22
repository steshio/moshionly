//! The error type shared by every provider.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::redact::scrub;

/// A coarse classification of a failure, carried in
/// [`ChatEvent::Error`](crate::ChatEvent::Error) so the app can offer the
/// right next step (open settings, show sign-in instructions, try again).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// The agent CLI is not installed.
    NotInstalled,
    /// The agent CLI is installed but the user has not signed in to it.
    NotSignedIn,
    /// No API key has been stored for the provider.
    MissingKey,
    /// The API rejected the key.
    InvalidKey,
    /// Too many requests, or the plan's usage limit was reached.
    RateLimited,
    /// The service is temporarily over capacity.
    Overloaded,
    /// The model does not exist or the account may not use it.
    ModelUnavailable,
    /// The request was malformed or too large.
    InvalidRequest,
    /// The service could not be reached.
    Network,
    /// The provider answered with something this crate does not understand.
    Protocol,
    /// Anything else.
    Other,
}

/// Why a provider call failed.
///
/// Messages are safe to show and to log: they pass through
/// [`scrub`](crate::redact::scrub) when the error is built.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum AiError {
    /// The agent CLI is not installed.
    #[error("{program} is not installed")]
    NotInstalled {
        /// The command that was looked for, for example `claude`.
        program: String,
    },
    /// The agent CLI is installed but not signed in.
    #[error("not signed in: {0}")]
    NotSignedIn(String),
    /// No API key has been stored for the provider.
    #[error("no API key is set for {provider}")]
    MissingKey {
        /// The provider's display name.
        provider: String,
    },
    /// The API rejected the key.
    #[error("the API key was rejected: {0}")]
    InvalidKey(String),
    /// Too many requests, or a usage limit was reached.
    #[error("rate limited: {message}")]
    RateLimited {
        /// What the service said.
        message: String,
        /// How long the service asked the caller to wait, when it said.
        retry_after: Option<Duration>,
    },
    /// The service is temporarily over capacity.
    #[error("the service is overloaded: {0}")]
    Overloaded(String),
    /// The model does not exist or the account may not use it.
    #[error("model unavailable: {0}")]
    ModelUnavailable(String),
    /// The request was malformed or too large.
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    /// The service could not be reached.
    #[error("network error: {0}")]
    Network(String),
    /// The provider answered with something this crate does not understand.
    #[error("unexpected response: {0}")]
    Protocol(String),
    /// Anything else.
    #[error("{0}")]
    Other(String),
}

impl AiError {
    /// The classification the app sees.
    pub fn kind(&self) -> ErrorKind {
        match self {
            Self::NotInstalled { .. } => ErrorKind::NotInstalled,
            Self::NotSignedIn(_) => ErrorKind::NotSignedIn,
            Self::MissingKey { .. } => ErrorKind::MissingKey,
            Self::InvalidKey(_) => ErrorKind::InvalidKey,
            Self::RateLimited { .. } => ErrorKind::RateLimited,
            Self::Overloaded(_) => ErrorKind::Overloaded,
            Self::ModelUnavailable(_) => ErrorKind::ModelUnavailable,
            Self::InvalidRequest(_) => ErrorKind::InvalidRequest,
            Self::Network(_) => ErrorKind::Network,
            Self::Protocol(_) => ErrorKind::Protocol,
            Self::Other(_) => ErrorKind::Other,
        }
    }

    /// True when the same request may succeed if it is sent again shortly.
    pub fn is_transient(&self) -> bool {
        matches!(
            self,
            Self::RateLimited { .. } | Self::Overloaded(_) | Self::Network(_)
        )
    }

    /// Builds the error for `kind` from a message that came from outside.
    /// The message is scrubbed of anything that looks like a key.
    pub fn from_kind(kind: ErrorKind, message: impl AsRef<str>) -> Self {
        let message = scrub(message.as_ref().trim());
        match kind {
            ErrorKind::NotInstalled => Self::NotInstalled { program: message },
            ErrorKind::NotSignedIn => Self::NotSignedIn(message),
            ErrorKind::MissingKey => Self::MissingKey { provider: message },
            ErrorKind::InvalidKey => Self::InvalidKey(message),
            ErrorKind::RateLimited => Self::RateLimited {
                message,
                retry_after: None,
            },
            ErrorKind::Overloaded => Self::Overloaded(message),
            ErrorKind::ModelUnavailable => Self::ModelUnavailable(message),
            ErrorKind::InvalidRequest => Self::InvalidRequest(message),
            ErrorKind::Network => Self::Network(message),
            ErrorKind::Protocol => Self::Protocol(message),
            ErrorKind::Other => Self::Other(message),
        }
    }

    /// A scrubbed [`AiError::Other`].
    pub fn other(message: impl AsRef<str>) -> Self {
        Self::from_kind(ErrorKind::Other, message)
    }

    /// A scrubbed [`AiError::Protocol`].
    pub fn protocol(message: impl AsRef<str>) -> Self {
        Self::from_kind(ErrorKind::Protocol, message)
    }

    /// A scrubbed [`AiError::Network`].
    pub fn network(message: impl AsRef<str>) -> Self {
        Self::from_kind(ErrorKind::Network, message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_match_variants() {
        let cases = [
            ErrorKind::NotInstalled,
            ErrorKind::NotSignedIn,
            ErrorKind::MissingKey,
            ErrorKind::InvalidKey,
            ErrorKind::RateLimited,
            ErrorKind::Overloaded,
            ErrorKind::ModelUnavailable,
            ErrorKind::InvalidRequest,
            ErrorKind::Network,
            ErrorKind::Protocol,
            ErrorKind::Other,
        ];
        for kind in cases {
            assert_eq!(AiError::from_kind(kind, "x").kind(), kind);
        }
    }

    #[test]
    fn only_capacity_and_network_failures_are_transient() {
        assert!(AiError::from_kind(ErrorKind::RateLimited, "slow down").is_transient());
        assert!(AiError::from_kind(ErrorKind::Overloaded, "busy").is_transient());
        assert!(AiError::network("reset").is_transient());
        assert!(!AiError::from_kind(ErrorKind::InvalidKey, "no").is_transient());
        assert!(!AiError::from_kind(ErrorKind::InvalidRequest, "no").is_transient());
    }

    #[test]
    fn messages_from_outside_are_scrubbed() {
        let error = AiError::from_kind(
            ErrorKind::InvalidKey,
            "Incorrect API key provided: sk-proj-Abc123def456ghi789jkl012",
        );
        assert!(!error.to_string().contains("sk-proj"));
        assert!(error.to_string().contains("[redacted]"));
    }

    #[test]
    fn kind_serialises_in_snake_case() {
        let json = serde_json::to_string(&ErrorKind::ModelUnavailable).unwrap();
        assert_eq!(json, "\"model_unavailable\"");
    }
}
