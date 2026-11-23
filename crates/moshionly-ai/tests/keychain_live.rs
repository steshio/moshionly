//! Uses the real OS keychain. Ignored by default: every other test keeps
//! keys in memory and leaves the user's keychain alone.
//!
//! ```text
//! cargo test -p moshionly-ai --test keychain_live -- --ignored --nocapture
//! ```
//!
//! The test stores a made-up value under its own service name, so it cannot
//! meet a real key, and removes it again whether or not it passes.

use moshionly_ai::{ApiKey, KeyStore, KeychainStore};

/// Kept apart from the app's entries, which use `Moshionly AI`.
const TEST_SERVICE: &str = "Moshionly AI (test)";

/// Removes the test entry when the test ends, also when it fails.
struct Cleanup<'a> {
    store: &'a KeychainStore,
    provider_id: &'a str,
}

impl Drop for Cleanup<'_> {
    fn drop(&mut self) {
        let _ = self.store.delete(self.provider_id);
    }
}

#[test]
#[ignore = "writes to and removes from the real OS keychain"]
fn a_key_round_trips_through_the_os_keychain() {
    let store = KeychainStore::with_service(TEST_SERVICE);
    let provider_id = format!("round-trip-{}", std::process::id());
    let _cleanup = Cleanup {
        store: &store,
        provider_id: &provider_id,
    };

    assert_eq!(store.get(&provider_id).unwrap(), None, "nothing stored yet");
    store.delete(&provider_id).unwrap();

    store
        .set(&provider_id, &ApiKey::new("not-a-real-key-1"))
        .unwrap();
    assert_eq!(
        store.get(&provider_id).unwrap().unwrap().expose(),
        "not-a-real-key-1"
    );

    // A second write replaces the first rather than adding an entry.
    store
        .set(&provider_id, &ApiKey::new("not-a-real-key-2"))
        .unwrap();
    assert_eq!(
        store.get(&provider_id).unwrap().unwrap().expose(),
        "not-a-real-key-2"
    );

    // An empty key forgets the stored one.
    store.set(&provider_id, &ApiKey::new("")).unwrap();
    assert_eq!(store.get(&provider_id).unwrap(), None);

    store
        .set(&provider_id, &ApiKey::new("not-a-real-key-3"))
        .unwrap();
    store.delete(&provider_id).unwrap();
    assert_eq!(store.get(&provider_id).unwrap(), None);
}
