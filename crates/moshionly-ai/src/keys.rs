//! Where API keys are kept.
//!
//! A key lives in the operating system's keychain and nowhere else: never in
//! the settings file, never in a log. The [`KeyStore`] trait is the only way
//! this crate reaches a key, so tests, and systems without a keychain, can
//! put the in-memory [`MemoryKeyStore`] in its place.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use crate::redact::{ApiKey, scrub};

/// The name Moshionly's keys are filed under in the OS keychain. Each key is
/// one entry with this as its service and the provider id as its account, so
/// in a keychain viewer a key shows up as "Moshionly AI", account `openai`.
pub const KEYCHAIN_SERVICE: &str = "Moshionly AI";

/// Why a key could not be read or written.
///
/// Messages never contain a key: they are built from the keychain's own
/// description of the failure and scrubbed on the way.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KeyStoreError {
    /// This system has no keychain that Moshionly can use, for example a
    /// Linux session without a secret service running.
    #[error("no keychain is available: {0}")]
    Unavailable(String),
    /// The keychain is locked, or access to it was refused.
    #[error("the keychain refused access: {0}")]
    AccessDenied(String),
    /// What is stored under the provider's name cannot be read as a key.
    #[error("the stored key cannot be used: {0}")]
    Corrupt(String),
    /// Anything else the keychain reported.
    #[error("the keychain reported an error: {0}")]
    Failed(String),
}

/// A place that holds one API key per provider.
///
/// The calls block, and an OS keychain may stop to ask the user for
/// permission before it answers, so they do not belong on a user interface
/// thread. [`ProviderRegistry`](crate::ProviderRegistry) runs them on a
/// background thread for you.
pub trait KeyStore: Send + Sync {
    /// The key stored for the provider, or `None` when there is none.
    fn get(&self, provider_id: &str) -> Result<Option<ApiKey>, KeyStoreError>;

    /// Stores `key` for the provider, replacing any earlier one. An empty
    /// key removes the stored one, so that clearing a key field in a
    /// settings screen forgets the key.
    fn set(&self, provider_id: &str, key: &ApiKey) -> Result<(), KeyStoreError>;

    /// Removes the provider's key. Removing a key that is not there is not
    /// an error.
    fn delete(&self, provider_id: &str) -> Result<(), KeyStoreError>;
}

/// The operating system's keychain: Keychain Services on macOS, the
/// Credential Manager on Windows and the Secret Service on Linux.
///
/// On macOS the system asks the user for permission when an app reads an
/// entry that a different app, or a differently signed build of the same
/// app, created. Development builds are re-signed on every build, so expect
/// that prompt once per rebuild while developing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeychainStore {
    service: String,
}

/// Keeps keychain calls one at a time. The platform stores do not promise
/// to handle one entry being used from two threads at once.
static KEYCHAIN_CALLS: Mutex<()> = Mutex::new(());

impl KeychainStore {
    /// The store the app uses, filed under [`KEYCHAIN_SERVICE`].
    pub fn new() -> Self {
        Self::with_service(KEYCHAIN_SERVICE)
    }

    /// A store filed under another service name, which keeps the entries of
    /// a test or a development build apart from the real ones.
    pub fn with_service(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    /// The service name this store files keys under.
    pub fn service(&self) -> &str {
        &self.service
    }

    fn entry(&self, provider_id: &str) -> Result<keyring::Entry, KeyStoreError> {
        keyring::Entry::new(&self.service, provider_id).map_err(|error| describe(&error))
    }
}

impl Default for KeychainStore {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyStore for KeychainStore {
    fn get(&self, provider_id: &str) -> Result<Option<ApiKey>, KeyStoreError> {
        let _one_at_a_time = lock(&KEYCHAIN_CALLS);
        match self.entry(provider_id)?.get_password() {
            Ok(key) => Ok(Some(ApiKey::new(key)).filter(|key| !key.is_empty())),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(describe(&error)),
        }
    }

    fn set(&self, provider_id: &str, key: &ApiKey) -> Result<(), KeyStoreError> {
        if key.is_empty() {
            return self.delete(provider_id);
        }
        let _one_at_a_time = lock(&KEYCHAIN_CALLS);
        self.entry(provider_id)?
            .set_password(key.expose())
            .map_err(|error| describe(&error))
    }

    fn delete(&self, provider_id: &str) -> Result<(), KeyStoreError> {
        let _one_at_a_time = lock(&KEYCHAIN_CALLS);
        match self.entry(provider_id)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(describe(&error)),
        }
    }
}

/// Turns a keychain error into ours.
///
/// Some of the keychain's errors carry the stored bytes, and their debug
/// output prints them. Nothing here formats an error that way; those
/// variants are described in words instead.
fn describe(error: &keyring::Error) -> KeyStoreError {
    use keyring::Error;

    match error {
        Error::NoStorageAccess(cause) => KeyStoreError::AccessDenied(scrub(&cause.to_string())),
        Error::NoDefaultStore => {
            KeyStoreError::Unavailable("the system's credential store could not be opened".into())
        }
        Error::Invalid(what, why) if what == "platform" => KeyStoreError::Unavailable(scrub(why)),
        Error::BadEncoding(_) => KeyStoreError::Corrupt("it is not text".into()),
        Error::BadDataFormat(..) => {
            KeyStoreError::Corrupt("it is not in the form the keychain expects".into())
        }
        Error::Ambiguous(found) => KeyStoreError::Failed(format!(
            "{} entries match where one was expected",
            found.len()
        )),
        Error::PlatformFailure(cause) => KeyStoreError::Failed(scrub(&cause.to_string())),
        // The rest describe names and limits, not secrets.
        other => KeyStoreError::Failed(scrub(&other.to_string())),
    }
}

/// A key store that lives in memory and is gone when the process ends.
///
/// For tests, for trying a provider with a key from the environment, and
/// for systems without a keychain.
#[derive(Debug, Default)]
pub struct MemoryKeyStore {
    keys: Mutex<HashMap<String, ApiKey>>,
}

impl MemoryKeyStore {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a key while building the store.
    pub fn with_key(self, provider_id: impl Into<String>, key: ApiKey) -> Self {
        if !key.is_empty() {
            lock(&self.keys).insert(provider_id.into(), key);
        }
        self
    }
}

impl KeyStore for MemoryKeyStore {
    fn get(&self, provider_id: &str) -> Result<Option<ApiKey>, KeyStoreError> {
        Ok(lock(&self.keys).get(provider_id).cloned())
    }

    fn set(&self, provider_id: &str, key: &ApiKey) -> Result<(), KeyStoreError> {
        if key.is_empty() {
            return self.delete(provider_id);
        }
        lock(&self.keys).insert(provider_id.to_owned(), key.clone());
        Ok(())
    }

    fn delete(&self, provider_id: &str) -> Result<(), KeyStoreError> {
        lock(&self.keys).remove(provider_id);
        Ok(())
    }
}

/// Locks a mutex whether or not a thread panicked while holding it. Nothing
/// guarded here can be left half changed.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    const KEY: &str = "sk-test-Abc123def456ghi789jkl012";

    #[test]
    fn a_key_can_be_stored_read_replaced_and_removed() {
        let store = MemoryKeyStore::new();
        assert_eq!(store.get("openai").unwrap(), None);

        store.set("openai", &ApiKey::new(KEY)).unwrap();
        assert_eq!(store.get("openai").unwrap().unwrap().expose(), KEY);
        assert_eq!(store.get("groq").unwrap(), None);

        store.set("openai", &ApiKey::new("another-key")).unwrap();
        assert_eq!(
            store.get("openai").unwrap().unwrap().expose(),
            "another-key"
        );

        store.delete("openai").unwrap();
        assert_eq!(store.get("openai").unwrap(), None);
    }

    #[test]
    fn removing_a_key_that_is_not_there_is_fine() {
        assert_eq!(MemoryKeyStore::new().delete("openai"), Ok(()));
    }

    #[test]
    fn storing_an_empty_key_removes_the_stored_one() {
        let store = MemoryKeyStore::new().with_key("openai", ApiKey::new(KEY));
        store.set("openai", &ApiKey::new("  \n")).unwrap();
        assert_eq!(store.get("openai").unwrap(), None);
        // Nor does building a store with an empty key keep one.
        let store = MemoryKeyStore::new().with_key("openai", ApiKey::new(""));
        assert_eq!(store.get("openai").unwrap(), None);
    }

    #[test]
    fn debug_output_of_a_store_hides_its_keys() {
        let store = MemoryKeyStore::new().with_key("openai", ApiKey::new(KEY));
        let debug = format!("{store:?}");
        assert!(debug.contains("openai"));
        assert!(!debug.contains(KEY), "{debug}");
    }

    #[test]
    fn a_store_can_be_shared_behind_the_trait() {
        let store: Arc<dyn KeyStore> = Arc::new(MemoryKeyStore::new());
        let writer = Arc::clone(&store);
        std::thread::spawn(move || writer.set("groq", &ApiKey::new(KEY)).unwrap())
            .join()
            .unwrap();
        assert!(store.get("groq").unwrap().is_some());
    }

    #[test]
    fn the_keychain_store_files_keys_under_the_app_service() {
        assert_eq!(KeychainStore::new().service(), KEYCHAIN_SERVICE);
        assert_eq!(KeychainStore::default(), KeychainStore::new());
        assert_eq!(
            KeychainStore::with_service("Moshionly AI (test)").service(),
            "Moshionly AI (test)"
        );
    }

    #[test]
    fn keychain_errors_are_sorted_into_kinds() {
        let denied = keyring::Error::NoStorageAccess("the keychain is locked".into());
        assert_eq!(
            describe(&denied),
            KeyStoreError::AccessDenied("the keychain is locked".to_owned())
        );
        assert!(matches!(
            describe(&keyring::Error::NoDefaultStore),
            KeyStoreError::Unavailable(_)
        ));
        let platform = keyring::Error::Invalid("platform".to_owned(), "not supported".to_owned());
        assert_eq!(
            describe(&platform),
            KeyStoreError::Unavailable("not supported".to_owned())
        );
        let other = keyring::Error::Invalid("user".to_owned(), "cannot be empty".to_owned());
        assert!(matches!(describe(&other), KeyStoreError::Failed(_)));
        assert_eq!(
            describe(&keyring::Error::Ambiguous(Vec::new())),
            KeyStoreError::Failed("0 entries match where one was expected".to_owned())
        );
    }

    #[test]
    fn errors_that_carry_the_stored_bytes_never_print_them() {
        let bytes = KEY.as_bytes().to_vec();
        let cases = [
            keyring::Error::BadEncoding(bytes.clone()),
            keyring::Error::BadDataFormat(bytes, "bad header".into()),
        ];
        for error in cases {
            let described = describe(&error);
            assert!(matches!(described, KeyStoreError::Corrupt(_)));
            assert!(!described.to_string().contains(KEY));
            assert!(!format!("{described:?}").contains(KEY));
        }
    }

    #[test]
    fn a_key_quoted_in_a_platform_message_is_masked() {
        let error = keyring::Error::PlatformFailure(format!("could not store {KEY}").into());
        let message = describe(&error).to_string();
        assert!(!message.contains(KEY), "{message}");
        assert!(message.contains("[redacted]"));
    }
}
