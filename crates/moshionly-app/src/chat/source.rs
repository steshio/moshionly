//! Where the chat gets its providers from.
//!
//! The chat keeps the person's choices and keys in a [`ProviderRegistry`],
//! but the two things that reach outside the app (checking whether a
//! provider can be used, and building one to talk to) go through a
//! [`ProviderSource`]. The app uses [`RegistrySource`], which asks the
//! registry; tests use [`ScriptedSource`](super::testing::ScriptedSource),
//! which never starts a CLI or opens a connection.

use async_trait::async_trait;
use moshionly_ai::{ChatProvider, ProviderRegistry, ProviderStatus, RegistryError};

/// Checks and builds providers for the chat.
///
/// Every method gets the registry as it is at the moment of the call, with
/// the person's current settings and key store.
#[async_trait]
pub trait ProviderSource: Send + Sync {
    /// Every provider and whether it can be used now, in settings order.
    async fn statuses(&self, registry: &ProviderRegistry) -> Vec<ProviderStatus>;

    /// One provider and whether it can be used now; `None` for an unknown
    /// id.
    async fn status(&self, registry: &ProviderRegistry, id: &str) -> Option<ProviderStatus>;

    /// The provider with this id, ready to answer.
    async fn provider(
        &self,
        registry: &ProviderRegistry,
        id: &str,
    ) -> Result<Box<dyn ChatProvider>, RegistryError>;
}

/// The real providers, straight from the registry.
#[derive(Debug, Clone, Copy, Default)]
pub struct RegistrySource;

#[async_trait]
impl ProviderSource for RegistrySource {
    async fn statuses(&self, registry: &ProviderRegistry) -> Vec<ProviderStatus> {
        registry.statuses().await
    }

    async fn status(&self, registry: &ProviderRegistry, id: &str) -> Option<ProviderStatus> {
        registry.status(id).await
    }

    async fn provider(
        &self,
        registry: &ProviderRegistry,
        id: &str,
    ) -> Result<Box<dyn ChatProvider>, RegistryError> {
        registry.provider(id).await
    }
}
