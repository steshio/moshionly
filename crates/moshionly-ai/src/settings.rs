//! What the user chose: which providers to offer, which one the chat opens
//! with, and each provider's options.
//!
//! The settings are one JSON file in the app's configuration directory.
//! They never hold an API key; keys go to the [`KeyStore`](crate::KeyStore).
//! A file written by hand or by a newer version still loads: every field is
//! optional and fields this version does not know are ignored.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::api::anthropic::Effort;
use crate::paths::config_dir;
use crate::redact::scrub;

/// The name of the settings file inside [`config_dir`].
pub const SETTINGS_FILE: &str = "ai-settings.json";

/// Why settings could not be loaded or saved.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SettingsError {
    /// The platform has no configuration directory to keep the file in.
    #[error("this system has no configuration directory to keep settings in")]
    NoConfigDir,
    /// The file exists but could not be read.
    #[error("could not read {}: {message}", .path.display())]
    Read {
        /// The settings file.
        path: PathBuf,
        /// What the system said.
        message: String,
    },
    /// The file is not valid settings JSON. It is left as it was, so the
    /// user can repair it.
    #[error("{} is not a valid settings file: {message}", .path.display())]
    Parse {
        /// The settings file.
        path: PathBuf,
        /// Where and why parsing stopped.
        message: String,
    },
    /// The file could not be written.
    #[error("could not write {}: {message}", .path.display())]
    Write {
        /// The settings file.
        path: PathBuf,
        /// What the system said.
        message: String,
    },
    /// A server address holds a password or a key. Saving it would put a
    /// secret in a plain file, so it is refused.
    #[error(
        "the server address for {provider} contains a password or a key; \
         enter the key in the key field instead"
    )]
    SecretInAddress {
        /// The id of the provider whose address it is.
        provider: String,
    },
}

/// The user's choices for one provider.
///
/// Every field is optional and means "the provider's default" when absent.
/// A field that does not apply to a provider is ignored by it; each one
/// says which providers read it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderSettings {
    /// Whether the provider is offered in the chat. On unless the user
    /// turned it off.
    #[serde(skip_serializing_if = "is_true")]
    pub enabled: bool,
    /// The model to use when a request names none. All providers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The server address to use in place of the usual one. API providers;
    /// required for `custom`. Anthropic's is given without `/v1`, the
    /// others up to and including the version segment (`https://host/v1`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// Where the agent CLI is, when it should not be looked up.
    /// Subscription providers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binary: Option<PathBuf>,
    /// Run the agent without the user's own configuration for it. On by
    /// default. `claude-code` and `codex`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub isolate_settings: Option<bool>,
    /// Let the agent read files. Off by default. `cursor-agent`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_file_reads: Option<bool>,
    /// How thorough the model is. `anthropic`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<Effort>,
    /// The most the model may write in one request. `anthropic`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    /// How many rounds of tool calls one turn may take. API providers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tool_rounds: Option<u32>,
    /// Whether the model can be sent images, when the preset's answer is
    /// wrong for the model in use. OpenAI-compatible providers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub images: Option<bool>,
    /// Ask for token counts in the stream. On by default; a few older
    /// servers reject the request field for it. OpenAI-compatible providers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_usage: Option<bool>,
}

/// The settings of a provider the user has not touched.
static UNTOUCHED: ProviderSettings = ProviderSettings {
    enabled: true,
    model: None,
    base_url: None,
    binary: None,
    isolate_settings: None,
    allow_file_reads: None,
    effort: None,
    max_tokens: None,
    max_tool_rounds: None,
    images: None,
    stream_usage: None,
};

impl Default for ProviderSettings {
    fn default() -> Self {
        UNTOUCHED.clone()
    }
}

impl ProviderSettings {
    /// The chosen model, or `None` when the field is absent or blank.
    pub fn model(&self) -> Option<&str> {
        non_blank(self.model.as_deref())
    }

    /// The chosen server address, or `None` when the field is absent or
    /// blank.
    pub fn base_url(&self) -> Option<&str> {
        non_blank(self.base_url.as_deref())
    }

    /// The chosen CLI location, or `None` when the field is absent or
    /// empty.
    pub fn binary(&self) -> Option<&Path> {
        self.binary
            .as_deref()
            .filter(|path| !path.as_os_str().is_empty())
    }

    /// True when the chosen server address carries a credential.
    pub(crate) fn address_has_secret(&self) -> bool {
        self.base_url().is_some_and(address_has_secret)
    }
}

/// Everything the user chose about AI providers.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AiSettings {
    /// The id of the provider the chat opens with. `None` leaves the choice
    /// to the app.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_provider: Option<String>,
    /// Choices per provider, by provider id. A provider without an entry
    /// is enabled and uses its defaults.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub providers: BTreeMap<String, ProviderSettings>,
}

impl AiSettings {
    /// The choices for one provider; the defaults when it has no entry.
    pub fn provider(&self, id: &str) -> &ProviderSettings {
        self.providers.get(id).unwrap_or(&UNTOUCHED)
    }

    /// The choices for one provider, for changing them. Creates the entry
    /// when there is none.
    pub fn provider_mut(&mut self, id: &str) -> &mut ProviderSettings {
        self.providers.entry(id.to_owned()).or_default()
    }

    /// Whether the provider is offered in the chat.
    pub fn is_enabled(&self, id: &str) -> bool {
        self.provider(id).enabled
    }

    /// The id of the provider the chat opens with, when one is set.
    pub fn default_provider(&self) -> Option<&str> {
        non_blank(self.default_provider.as_deref())
    }

    /// Where the settings file lives: [`SETTINGS_FILE`] in [`config_dir`].
    /// `None` when the platform has no configuration directory.
    pub fn default_path() -> Option<PathBuf> {
        config_dir().map(|dir| dir.join(SETTINGS_FILE))
    }

    /// Loads the settings from their usual place. A missing file gives the
    /// defaults: that is simply a first run.
    pub fn load() -> Result<Self, SettingsError> {
        match Self::default_path() {
            Some(path) => Self::load_from(&path),
            None => Ok(Self::default()),
        }
    }

    /// Loads the settings from `path`. A missing file gives the defaults.
    pub fn load_from(path: &Path) -> Result<Self, SettingsError> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => {
                return Err(SettingsError::Read {
                    path: path.to_path_buf(),
                    message: error.to_string(),
                });
            }
        };
        if text.trim().is_empty() {
            return Ok(Self::default());
        }
        serde_json::from_str(&text).map_err(|error| SettingsError::Parse {
            path: path.to_path_buf(),
            // A parse error can quote the text it tripped over.
            message: scrub(&error.to_string()),
        })
    }

    /// Saves the settings to their usual place.
    pub fn save(&self) -> Result<(), SettingsError> {
        let path = Self::default_path().ok_or(SettingsError::NoConfigDir)?;
        self.save_to(&path)
    }

    /// Saves the settings to `path`, creating its directory when needed.
    ///
    /// The file is replaced in one step, so a crash while saving leaves the
    /// old settings and never half a file. Settings with a secret in a
    /// server address are refused; see [`validate`](Self::validate).
    pub fn save_to(&self, path: &Path) -> Result<(), SettingsError> {
        self.validate()?;
        let write_error = |error: std::io::Error| SettingsError::Write {
            path: path.to_path_buf(),
            message: error.to_string(),
        };
        let dir = path
            .parent()
            .filter(|dir| !dir.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(dir).map_err(write_error)?;

        let mut json =
            serde_json::to_string_pretty(self).map_err(|error| write_error(error.into()))?;
        json.push('\n');
        let mut file = tempfile::NamedTempFile::new_in(dir).map_err(write_error)?;
        file.write_all(json.as_bytes()).map_err(write_error)?;
        file.flush().map_err(write_error)?;
        file.persist(path)
            .map_err(|error| write_error(error.error))?;
        Ok(())
    }

    /// Checks that nothing secret would be written to the file.
    ///
    /// The settings have no field for a key, but a server address can carry
    /// one, as `https://user:password@host/v1` or `https://host/v1?key=...`.
    /// Such an address is refused: the key belongs in the
    /// [`KeyStore`](crate::KeyStore), which sends it as a header.
    pub fn validate(&self) -> Result<(), SettingsError> {
        match self
            .providers
            .iter()
            .find(|(_, settings)| settings.address_has_secret())
        {
            Some((provider, _)) => Err(SettingsError::SecretInAddress {
                provider: provider.clone(),
            }),
            None => Ok(()),
        }
    }
}

/// True when a server address carries a credential: a user name or password
/// in front of the host, or anything shaped like a key.
fn address_has_secret(address: &str) -> bool {
    let after_scheme = address.split_once("://").map_or(address, |(_, rest)| rest);
    let host_part = after_scheme.split(['/', '?', '#']).next().unwrap_or("");
    host_part.contains('@') || scrub(address) != address
}

fn non_blank(text: Option<&str>) -> Option<&str> {
    text.map(str::trim).filter(|text| !text.is_empty())
}

fn is_true(value: &bool) -> bool {
    *value
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn sample() -> AiSettings {
        let mut settings = AiSettings {
            default_provider: Some("claude-code".to_owned()),
            ..AiSettings::default()
        };
        settings.provider_mut("claude-code").model = Some("sonnet".to_owned());
        settings.provider_mut("cursor-agent").enabled = false;
        let anthropic = settings.provider_mut("anthropic");
        anthropic.effort = Some(Effort::High);
        anthropic.max_tokens = Some(32_000);
        let ollama = settings.provider_mut("ollama");
        ollama.base_url = Some("http://192.168.1.20:11434/v1".to_owned());
        ollama.model = Some("llama3.2".to_owned());
        ollama.images = Some(true);
        settings
    }

    #[test]
    fn an_untouched_provider_is_enabled_with_no_choices() {
        let settings = AiSettings::default();
        assert!(settings.is_enabled("claude-code"));
        assert_eq!(settings.provider("openai"), &ProviderSettings::default());
        assert_eq!(settings.provider("openai").model(), None);
        assert_eq!(settings.default_provider(), None);
        assert!(ProviderSettings::default().enabled);
    }

    #[test]
    fn default_settings_serialise_to_an_empty_object() {
        assert_eq!(
            serde_json::to_value(AiSettings::default()).unwrap(),
            json!({})
        );
        assert_eq!(
            serde_json::to_value(ProviderSettings::default()).unwrap(),
            json!({})
        );
    }

    #[test]
    fn only_what_the_user_changed_is_written() {
        assert_eq!(
            serde_json::to_value(sample()).unwrap(),
            json!({
                "default_provider": "claude-code",
                "providers": {
                    "anthropic": { "effort": "high", "max_tokens": 32000 },
                    "claude-code": { "model": "sonnet" },
                    "cursor-agent": { "enabled": false },
                    "ollama": {
                        "model": "llama3.2",
                        "base_url": "http://192.168.1.20:11434/v1",
                        "images": true
                    }
                }
            })
        );
    }

    #[test]
    fn settings_round_trip_through_json() {
        let json = serde_json::to_string(&sample()).unwrap();
        let back: AiSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(back, sample());
    }

    #[test]
    fn missing_and_unknown_fields_are_tolerated() {
        let settings: AiSettings = serde_json::from_value(json!({
            "added_in_a_later_version": 3,
            "providers": {
                "openai": { "model": "gpt-x", "not_known_yet": true },
                "a-provider-from-the-future": { "enabled": false }
            }
        }))
        .unwrap();
        assert_eq!(settings.provider("openai").model(), Some("gpt-x"));
        assert!(settings.is_enabled("openai"));
        assert!(!settings.is_enabled("a-provider-from-the-future"));
    }

    #[test]
    fn blank_choices_count_as_absent() {
        let settings = ProviderSettings {
            model: Some("  ".to_owned()),
            base_url: Some(String::new()),
            binary: Some(PathBuf::new()),
            ..ProviderSettings::default()
        };
        assert_eq!(settings.model(), None);
        assert_eq!(settings.base_url(), None);
        assert_eq!(settings.binary(), None);

        let settings = ProviderSettings {
            model: Some(" llama3.2\n".to_owned()),
            ..ProviderSettings::default()
        };
        assert_eq!(settings.model(), Some("llama3.2"));
    }

    #[test]
    fn a_missing_or_empty_file_gives_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SETTINGS_FILE);
        assert_eq!(AiSettings::load_from(&path).unwrap(), AiSettings::default());
        std::fs::write(&path, "\n").unwrap();
        assert_eq!(AiSettings::load_from(&path).unwrap(), AiSettings::default());
    }

    #[test]
    fn saving_creates_the_directory_and_loading_gives_the_same_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Moshionly").join(SETTINGS_FILE);
        sample().save_to(&path).unwrap();
        assert_eq!(AiSettings::load_from(&path).unwrap(), sample());

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.ends_with("}\n"));
        assert!(text.contains("\n  \"default_provider\""), "pretty printed");
    }

    #[test]
    fn saving_replaces_the_file_and_leaves_nothing_else_behind() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SETTINGS_FILE);
        sample().save_to(&path).unwrap();
        AiSettings::default().save_to(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{}\n");
        let files: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(files.len(), 1, "no temporary file is left over");
    }

    #[test]
    fn a_damaged_file_is_reported_and_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SETTINGS_FILE);
        std::fs::write(&path, "{ \"providers\": [").unwrap();
        let error = AiSettings::load_from(&path).unwrap_err();
        assert!(matches!(&error, SettingsError::Parse { path: at, .. } if at == &path));
        assert!(error.to_string().contains(SETTINGS_FILE));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{ \"providers\": ["
        );
    }

    #[test]
    fn a_parse_error_does_not_quote_a_key_from_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SETTINGS_FILE);
        // Someone pasted a key where a true or false belongs.
        std::fs::write(
            &path,
            r#"{"providers":{"openai":{"enabled":"sk-proj-Abc123def456ghi789jkl012"}}}"#,
        )
        .unwrap();
        let message = AiSettings::load_from(&path).unwrap_err().to_string();
        assert!(!message.contains("sk-proj"), "{message}");
    }

    #[test]
    fn an_unreadable_file_is_a_read_error() {
        let dir = tempfile::tempdir().unwrap();
        // A directory where the file should be cannot be read as one.
        let error = AiSettings::load_from(dir.path()).unwrap_err();
        assert!(matches!(error, SettingsError::Read { .. }));
    }

    #[test]
    fn an_unwritable_place_is_a_write_error() {
        let dir = tempfile::tempdir().unwrap();
        let in_the_way = dir.path().join("not-a-directory");
        std::fs::write(&in_the_way, "x").unwrap();
        let error = sample()
            .save_to(&in_the_way.join(SETTINGS_FILE))
            .unwrap_err();
        assert!(matches!(error, SettingsError::Write { .. }));
    }

    #[test]
    fn addresses_with_credentials_are_recognised() {
        for address in [
            "https://user:hunter2@example.com/v1",
            "https://token@example.com/v1",
            "https://example.com/v1?key=AIzaSyA1234567890abcdefghijklmnop",
            "https://example.com/v1?api-key=9f8e7d6c5b4a",
            "https://example.com/sk-proj-Abc123def456ghi789jkl012/v1",
        ] {
            assert!(address_has_secret(address), "{address}");
        }
        for address in [
            "https://api.openai.com/v1",
            "http://localhost:11434/v1",
            "http://192.168.1.20:1234/v1",
            "https://generativelanguage.googleapis.com/v1beta/openai",
            "https://gateway.example.com/v1/0123456789abcdef0123456789abcdef/openai",
            "https://example.com/v1?api-version=2026-10-21",
            "example.com/v1",
            "",
        ] {
            assert!(!address_has_secret(address), "{address}");
        }
    }

    #[test]
    fn settings_with_a_secret_in_an_address_are_not_saved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SETTINGS_FILE);
        let mut settings = sample();
        settings.provider_mut("custom").base_url =
            Some("https://example.com/v1?key=AIzaSyA1234567890abcdefghijklmnop".to_owned());

        let error = settings.save_to(&path).unwrap_err();
        assert_eq!(
            error,
            SettingsError::SecretInAddress {
                provider: "custom".to_owned()
            }
        );
        assert!(!error.to_string().contains("AIza"));
        assert!(!path.exists(), "nothing was written");
    }

    #[test]
    fn the_settings_file_has_its_usual_name_and_place() {
        if let Some(path) = AiSettings::default_path() {
            assert_eq!(path.file_name().unwrap(), SETTINGS_FILE);
            assert_eq!(path.parent(), config_dir().as_deref());
        }
    }
}
