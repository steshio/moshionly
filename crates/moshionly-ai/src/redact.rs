//! Keeping API keys out of errors, logs and debug output.
//!
//! Two layers work together. [`ApiKey`] never prints its value, so a key
//! cannot leak through `{:?}`. [`scrub`] masks anything that looks like a
//! key in text that came from outside (an HTTP error body, a CLI's stderr),
//! and every error message in this crate passes through it.

use std::fmt;

/// What a masked secret is replaced with.
pub const REDACTED: &str = "[redacted]";

/// An API key held in memory.
///
/// It has no `Serialize` implementation on purpose: keys live in the OS
/// keychain (see [`KeyStore`](crate::KeyStore)) and never in a settings file.
#[derive(Clone, PartialEq, Eq)]
pub struct ApiKey(String);

impl ApiKey {
    /// Wraps a key. Surrounding whitespace, which pasted keys often carry, is
    /// removed.
    pub fn new(key: impl Into<String>) -> Self {
        let key: String = key.into();
        Self(key.trim().to_owned())
    }

    /// The key itself, for building a request header. Do not log the result.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// True when the key is empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Replaces every occurrence of this key in `text` with [`REDACTED`], then
    /// applies [`scrub`] for keys it does not know about.
    pub fn redact(&self, text: &str) -> String {
        if self.0.is_empty() {
            return scrub(text);
        }
        scrub(&text.replace(&self.0, REDACTED))
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ApiKey(")?;
        f.write_str(REDACTED)?;
        f.write_str(")")
    }
}

/// Prefixes that the common providers put at the start of a key.
const KEY_PREFIXES: &[&str] = &[
    "sk-", "sk_", "pk-", "rk-", "xai-", "gsk_", "AIza", "csk-", "pplx-", "hf_", "key-", "ya29.",
];

/// The shortest run of characters treated as a key after a known prefix.
const MIN_PREFIXED_KEY_CHARS: usize = 20;

/// The shortest run of characters treated as a key after a marker such as
/// `Bearer` or `x-api-key`.
const MIN_MARKED_KEY_CHARS: usize = 8;

/// Words that announce a credential in the token that follows.
const KEY_MARKERS: &[&str] = &[
    "bearer",
    "x-api-key",
    "api-key",
    "api_key",
    "apikey",
    "authorization",
    "key",
    "token",
    "access_token",
];

/// Masks anything in `text` that looks like an API key.
///
/// This is a safety net for text this crate did not write. It recognises the
/// key prefixes of common providers and the token that follows markers such
/// as `Bearer`, `x-api-key:` and `key=`.
pub fn scrub(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut previous_word = String::new();
    let mut rest = text;
    while !rest.is_empty() {
        let word_len = rest
            .char_indices()
            .find(|(_, c)| !is_key_char(*c))
            .map_or(rest.len(), |(index, _)| index);
        if word_len == 0 {
            // A separator character: copy it through unchanged.
            let c = rest.chars().next().expect("rest is not empty");
            out.push(c);
            rest = &rest[c.len_utf8()..];
            continue;
        }
        // A full stop that ends a sentence is not part of the key before it.
        let word = rest[..word_len].trim_end_matches('.');
        let trailing_dots = &rest[word.len()..word_len];
        if looks_like_key(word, &previous_word) {
            out.push_str(REDACTED);
        } else {
            out.push_str(word);
        }
        out.push_str(trailing_dots);
        previous_word = word.to_ascii_lowercase();
        rest = &rest[word_len..];
    }
    out
}

fn is_key_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')
}

fn looks_like_key(word: &str, previous_word: &str) -> bool {
    let has_digit_or_upper = word
        .chars()
        .any(|c| c.is_ascii_digit() || c.is_ascii_uppercase());
    if word.len() >= MIN_PREFIXED_KEY_CHARS
        && KEY_PREFIXES.iter().any(|prefix| word.starts_with(prefix))
        && has_digit_or_upper
    {
        return true;
    }
    // After a marker, ordinary words ("Bearer token is missing") must survive,
    // so the candidate has to look random as well.
    word.len() >= MIN_MARKED_KEY_CHARS
        && KEY_MARKERS.contains(&previous_word)
        && word.chars().any(|c| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_never_prints_the_key() {
        let key = ApiKey::new("sk-ant-api03-abcdefghijklmnop0123456789");
        assert_eq!(format!("{key:?}"), "ApiKey([redacted])");
    }

    #[test]
    fn new_trims_pasted_whitespace() {
        assert_eq!(ApiKey::new("  abc123\n").expose(), "abc123");
    }

    #[test]
    fn redact_replaces_the_exact_key_even_without_a_known_prefix() {
        let key = ApiKey::new("local-secret");
        assert_eq!(
            key.redact("401: key local-secret is not valid"),
            "401: key [redacted] is not valid"
        );
    }

    #[test]
    fn scrub_masks_prefixed_keys() {
        let text = "Incorrect API key provided: sk-proj-Abc123def456ghi789jkl012. Check it.";
        assert_eq!(
            scrub(text),
            "Incorrect API key provided: [redacted]. Check it."
        );
        assert_eq!(
            scrub("url?key=AIzaSyA1234567890abcdefghijklmnop"),
            "url?key=[redacted]"
        );
    }

    #[test]
    fn scrub_masks_the_token_after_a_marker() {
        assert_eq!(
            scrub("Authorization: Bearer abc123def456"),
            "Authorization: Bearer [redacted]"
        );
        assert_eq!(scrub("x-api-key: 9f8e7d6c5b4a"), "x-api-key: [redacted]");
    }

    #[test]
    fn scrub_leaves_ordinary_text_alone() {
        for text in [
            "The bearer token is missing from the request",
            "model claude-opus-5-5 is overloaded, retry in 20 seconds",
            "invalid x-api-key header",
            "tool sk-etch is not a key",
            "see https://example.com/v1/models/gpt-5.6-sol for the key list",
            "",
        ] {
            assert_eq!(scrub(text), text);
        }
    }

    #[test]
    fn scrub_handles_non_ascii_text() {
        assert_eq!(
            scrub("clé « sk-proj-Abc123def456ghi789jkl012 » refusée"),
            "clé « [redacted] » refusée"
        );
    }
}
