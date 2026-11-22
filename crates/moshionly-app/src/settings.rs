//! What the app remembers between runs.

use serde::{Deserialize, Serialize};

use crate::{
    shell::PanelSizes,
    theme::{Theme, ThemeMode},
};

/// The key the settings are stored under in the app's storage.
pub const STORAGE_KEY: &str = "moshionly-settings";

/// Everything that is saved when the app closes and restored when it opens.
///
/// Unknown or missing fields fall back to their defaults, so settings written
/// by an older or newer version still load.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Light, dark, or following the system.
    pub theme: ThemeMode,
    /// The sizes of the resizable panels.
    pub panels: PanelSizes,
}

impl Settings {
    /// Reads the settings from the app's storage. Anything missing or
    /// unreadable becomes the default, and odd panel sizes are repaired.
    pub fn load(storage: Option<&dyn eframe::Storage>) -> Self {
        let mut settings: Self = storage
            .and_then(|storage| eframe::get_value(storage, STORAGE_KEY))
            .unwrap_or_default();
        settings.panels = settings.panels.sanitized(&Theme::light().size);
        settings
    }

    /// Writes the settings to the app's storage.
    pub fn save(&self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, STORAGE_KEY, self);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use eframe::Storage as _;

    use super::*;

    #[derive(Default)]
    struct MemoryStorage(HashMap<String, String>);

    impl eframe::Storage for MemoryStorage {
        fn get_string(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }

        fn set_string(&mut self, key: &str, value: String) {
            self.0.insert(key.to_owned(), value);
        }

        fn remove_string(&mut self, key: &str) {
            self.0.remove(key);
        }

        fn flush(&mut self) {}
    }

    #[test]
    fn settings_survive_a_round_trip() {
        let mut storage = MemoryStorage::default();
        let settings = Settings {
            theme: ThemeMode::Dark,
            panels: PanelSizes {
                left: 300.0,
                right: 280.0,
                chat: 360.0,
                timeline: 320.0,
                chat_open: true,
            },
        };
        settings.save(&mut storage);
        assert_eq!(Settings::load(Some(&storage)), settings);
    }

    #[test]
    fn missing_or_broken_settings_fall_back_to_defaults() {
        assert_eq!(Settings::load(None), Settings::default());
        let mut storage = MemoryStorage::default();
        storage.set_string(STORAGE_KEY, "not settings at all {".to_owned());
        assert_eq!(Settings::load(Some(&storage)), Settings::default());
    }

    #[test]
    fn partial_settings_keep_what_they_have() {
        let mut storage = MemoryStorage::default();
        storage.set_string(STORAGE_KEY, "(theme: Light)".to_owned());
        let settings = Settings::load(Some(&storage));
        assert_eq!(settings.theme, ThemeMode::Light);
        assert_eq!(settings.panels, PanelSizes::default());

        storage.set_string(
            STORAGE_KEY,
            "(panels: (left: 99999.0, chat_open: true))".to_owned(),
        );
        let settings = Settings::load(Some(&storage));
        assert_eq!(settings.panels.left, Theme::light().size.panel_max);
        assert!(settings.panels.chat_open);
    }
}
