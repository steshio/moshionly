//! Node identifiers.

use std::borrow::Borrow;
use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Number of characters in a generated [`NodeId`].
pub const GENERATED_ID_LEN: usize = 12;

/// Longest id the validator accepts.
///
/// Generated ids are always [`GENERATED_ID_LEN`] characters. Shorter and longer
/// ids are accepted so that hand-written files and tests can use readable names.
pub const MAX_ID_LEN: usize = 64;

/// The identifier of a node, unique within one document.
///
/// An id is a short URL-safe string: letters, digits, `_` and `-`. New ids are
/// 12 random characters, which is enough that two documents can be merged
/// without realistic risk of a collision.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct NodeId(#[schemars(length(min = 1, max = 64), pattern(r"^[A-Za-z0-9_-]+$"))] String);

impl NodeId {
    /// Generates a fresh random id.
    pub fn generate() -> Self {
        Self(nanoid::nanoid!(GENERATED_ID_LEN))
    }

    /// The id as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether the id is well formed: 1 to 64 characters from `A-Z a-z 0-9 _ -`.
    pub fn is_valid(&self) -> bool {
        !self.0.is_empty()
            && self.0.len() <= MAX_ID_LEN
            && self
                .0
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for NodeId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for NodeId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&NodeId> for NodeId {
    fn from(value: &NodeId) -> Self {
        value.clone()
    }
}

impl AsRef<str> for NodeId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl Borrow<str> for NodeId {
    fn borrow(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn generated_ids_are_valid_and_distinct() {
        let ids: HashSet<NodeId> = (0..2000).map(|_| NodeId::generate()).collect();
        assert_eq!(ids.len(), 2000);
        for id in &ids {
            assert_eq!(id.as_str().len(), GENERATED_ID_LEN);
            assert!(id.is_valid(), "{id} should be valid");
        }
    }

    #[test]
    fn validity_rules() {
        assert!(NodeId::from("ab12").is_valid());
        assert!(NodeId::from("a_b-C9").is_valid());
        assert!(!NodeId::from("").is_valid());
        assert!(!NodeId::from("has space").is_valid());
        assert!(!NodeId::from("slash/").is_valid());
        assert!(!NodeId::from("x".repeat(MAX_ID_LEN + 1)).is_valid());
    }

    #[test]
    fn serialises_as_a_plain_string() {
        let id = NodeId::from("cd34");
        assert_eq!(serde_json::to_string(&id).unwrap(), "\"cd34\"");
        let back: NodeId = serde_json::from_str("\"cd34\"").unwrap();
        assert_eq!(back, id);
    }
}
