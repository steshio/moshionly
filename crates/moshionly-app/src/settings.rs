//! What the app remembers between runs.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{
    export::ExportChoices,
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
    /// Projects opened or saved lately, the most recent first.
    pub recent: Vec<PathBuf>,
    /// Whether agents outside the app may connect.
    pub agents: AgentSettings,
    /// The Export dialog's last choices.
    pub export: ExportChoices,
}

/// Whether agents outside the app (Claude Code, Cursor, the Figma plugin)
/// may reach the open document over MCP.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentSettings {
    /// Run the MCP server on this machine's loopback address while the app
    /// is open. On unless turned off.
    pub serve: bool,
}

impl Default for AgentSettings {
    fn default() -> Self {
        Self { serve: true }
    }
}

/// How many recent projects are remembered.
pub const RECENT_LIMIT: usize = 8;

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

    /// Puts `path` at the top of the recent projects, once, keeping at most
    /// [`RECENT_LIMIT`].
    pub fn remember(&mut self, path: &Path) {
        self.recent.retain(|known| known != path);
        self.recent.insert(0, path.to_owned());
        self.recent.truncate(RECENT_LIMIT);
    }

    /// Takes `path` off the recent projects, for example when it cannot be
    /// opened any more.
    pub fn forget(&mut self, path: &Path) {
        self.recent.retain(|known| known != path);
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
            recent: vec![PathBuf::from("/projects/teaser.moshi")],
            agents: AgentSettings { serve: false },
            export: ExportChoices {
                scale: 2.0,
                ..ExportChoices::default()
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
    fn recent_projects_keep_the_latest_first_and_once() {
        let mut settings = Settings::default();
        for n in 0..12 {
            settings.remember(Path::new(&format!("/p/{n}.moshi")));
        }
        assert_eq!(settings.recent.len(), RECENT_LIMIT);
        assert_eq!(settings.recent[0], Path::new("/p/11.moshi"));
        settings.remember(Path::new("/p/8.moshi"));
        assert_eq!(settings.recent[0], Path::new("/p/8.moshi"));
        assert_eq!(
            settings
                .recent
                .iter()
                .filter(|p| p.as_path() == Path::new("/p/8.moshi"))
                .count(),
            1
        );
        settings.forget(Path::new("/p/8.moshi"));
        assert_eq!(settings.recent[0], Path::new("/p/11.moshi"));
    }

    #[test]
    fn partial_settings_keep_what_they_have() {
        let mut storage = MemoryStorage::default();
        storage.set_string(STORAGE_KEY, "(theme: Light)".to_owned());
        let settings = Settings::load(Some(&storage));
        assert_eq!(settings.theme, ThemeMode::Light);
        assert_eq!(settings.panels, PanelSizes::default());
        assert!(
            settings.agents.serve,
            "agents may connect unless turned off"
        );

        storage.set_string(
            STORAGE_KEY,
            "(panels: (left: 99999.0, chat_open: true))".to_owned(),
        );
        let settings = Settings::load(Some(&storage));
        assert_eq!(settings.panels.left, Theme::light().size.panel_max);
        assert!(settings.panels.chat_open);
    }
}
