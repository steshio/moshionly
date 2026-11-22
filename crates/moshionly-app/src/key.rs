//! Keys: how the visual pieces name the things they show.
//!
//! The timeline and the viewport are driven by plain data and know nothing
//! about documents. So that the app can tell which bar was dragged or which
//! artboard was clicked, every item it hands to those widgets carries a
//! [`Key`], and the widgets hand the key back in their events. In practice a
//! key is a node id.

use std::{fmt, sync::Arc};

/// A stable name for one item shown by a widget. Cheap to clone and compare.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Key(Arc<str>);

impl Key {
    /// A key from its text.
    pub fn new(text: impl AsRef<str>) -> Self {
        Self(Arc::from(text.as_ref()))
    }

    /// The key's text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for Key {
    fn from(text: &str) -> Self {
        Self::new(text)
    }
}

impl From<String> for Key {
    fn from(text: String) -> Self {
        Self(Arc::from(text))
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl PartialEq<str> for Key {
    fn eq(&self, other: &str) -> bool {
        &*self.0 == other
    }
}

impl PartialEq<&str> for Key {
    fn eq(&self, other: &&str) -> bool {
        &*self.0 == *other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_compare_by_text() {
        let a = Key::new("ab12");
        let b: Key = String::from("ab12").into();
        assert_eq!(a, b);
        assert_eq!(a, "ab12");
        assert_ne!(a, Key::from("cd34"));
        assert_eq!(a.to_string(), "ab12");
        assert_eq!(a.clone().as_str(), "ab12");
    }
}
