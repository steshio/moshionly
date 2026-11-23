//! The services reachable through the OpenAI-compatible provider.
//!
//! A preset is only data: an address, whether a key is needed, and a few
//! facts about what the service accepts. Adding a service that speaks the
//! Chat Completions format means adding an entry here and nothing else.

/// One service that speaks the OpenAI Chat Completions format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenAiPreset {
    /// The provider id used in settings and on the command line.
    pub id: &'static str,
    /// The name shown to the user.
    pub name: &'static str,
    /// The address requests go to, up to and including the version segment.
    /// Empty for the `custom` preset, where the user supplies it.
    pub base_url: &'static str,
    /// The service refuses requests without an API key.
    pub needs_api_key: bool,
    /// The service runs on the user's own machine.
    pub local: bool,
    /// The service's usual models accept images. When false, images in
    /// messages and tool results are replaced by a short note.
    pub images: bool,
    /// Models to prefer, best first, as the service's documentation named
    /// them in November 2026. The first one is used when nothing else was
    /// chosen; an empty list means "the first model the server lists".
    pub suggested_models: &'static [&'static str],
    /// Where the user can create a key, for the settings screen.
    pub key_url: &'static str,
}

/// OpenAI's own API.
pub const OPENAI: OpenAiPreset = OpenAiPreset {
    id: "openai",
    name: "OpenAI",
    base_url: "https://api.openai.com/v1",
    needs_api_key: true,
    local: false,
    images: true,
    suggested_models: &["gpt-6.2-sol", "gpt-6.2-astra", "gpt-6-luna"],
    key_url: "https://platform.openai.com/api-keys",
};

/// OpenRouter, one key for models from many companies.
pub const OPENROUTER: OpenAiPreset = OpenAiPreset {
    id: "openrouter",
    name: "OpenRouter",
    base_url: "https://openrouter.ai/api/v1",
    needs_api_key: true,
    local: false,
    images: true,
    suggested_models: &["openrouter/auto"],
    key_url: "https://openrouter.ai/keys",
};

/// Google Gemini through its OpenAI-compatible endpoint.
pub const GEMINI: OpenAiPreset = OpenAiPreset {
    id: "gemini",
    name: "Google Gemini",
    base_url: "https://generativelanguage.googleapis.com/v1beta/openai",
    needs_api_key: true,
    local: false,
    images: true,
    suggested_models: &["gemini-flash-latest", "gemini-pro-latest"],
    key_url: "https://aistudio.google.com/apikey",
};

/// Groq, fast hosting of open models.
pub const GROQ: OpenAiPreset = OpenAiPreset {
    id: "groq",
    name: "Groq",
    base_url: "https://api.groq.com/openai/v1",
    needs_api_key: true,
    local: false,
    images: false,
    suggested_models: &["openai/gpt-oss-120b", "llama-3.3-70b-versatile"],
    key_url: "https://console.groq.com/keys",
};

/// Mistral's API.
pub const MISTRAL: OpenAiPreset = OpenAiPreset {
    id: "mistral",
    name: "Mistral",
    base_url: "https://api.mistral.ai/v1",
    needs_api_key: true,
    local: false,
    images: true,
    suggested_models: &["mistral-large-latest", "mistral-medium-latest"],
    key_url: "https://console.mistral.ai/api-keys",
};

/// xAI's API.
pub const XAI: OpenAiPreset = OpenAiPreset {
    id: "xai",
    name: "xAI",
    base_url: "https://api.x.ai/v1",
    needs_api_key: true,
    local: false,
    images: true,
    suggested_models: &["grok-4.7"],
    key_url: "https://console.x.ai",
};

/// Ollama running on this machine. No key.
pub const OLLAMA: OpenAiPreset = OpenAiPreset {
    id: "ollama",
    name: "Ollama",
    base_url: "http://localhost:11434/v1",
    needs_api_key: false,
    local: true,
    images: false,
    suggested_models: &[],
    key_url: "",
};

/// LM Studio's local server. No key.
pub const LM_STUDIO: OpenAiPreset = OpenAiPreset {
    id: "lm-studio",
    name: "LM Studio",
    base_url: "http://localhost:1234/v1",
    needs_api_key: false,
    local: true,
    images: false,
    suggested_models: &[],
    key_url: "",
};

/// Any other server that speaks the format. The user enters its address,
/// and a key if it wants one.
pub const CUSTOM: OpenAiPreset = OpenAiPreset {
    id: "custom",
    name: "Custom",
    base_url: "",
    needs_api_key: false,
    local: false,
    images: false,
    suggested_models: &[],
    key_url: "",
};

/// Every preset, in the order a settings screen should list them.
pub const OPENAI_PRESETS: &[OpenAiPreset] = &[
    OPENAI, OPENROUTER, GEMINI, GROQ, MISTRAL, XAI, OLLAMA, LM_STUDIO, CUSTOM,
];

/// The preset with the given id.
pub fn openai_preset(id: &str) -> Option<&'static OpenAiPreset> {
    OPENAI_PRESETS.iter().find(|preset| preset.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn ids_are_unique_and_safe_for_settings_and_the_command_line() {
        let mut seen = HashSet::new();
        for preset in OPENAI_PRESETS {
            assert!(seen.insert(preset.id), "{} appears twice", preset.id);
            assert!(
                preset
                    .id
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "{}",
                preset.id
            );
            assert!(!preset.name.is_empty());
        }
        assert_eq!(OPENAI_PRESETS.len(), 9);
    }

    #[test]
    fn hosted_services_use_https_and_need_a_key() {
        for preset in OPENAI_PRESETS {
            if preset.local || preset.id == CUSTOM.id {
                continue;
            }
            assert!(preset.base_url.starts_with("https://"), "{}", preset.id);
            assert!(!preset.base_url.ends_with('/'), "{}", preset.id);
            assert!(preset.needs_api_key, "{}", preset.id);
            assert!(preset.key_url.starts_with("https://"), "{}", preset.id);
            assert!(!preset.suggested_models.is_empty(), "{}", preset.id);
        }
    }

    #[test]
    fn local_servers_need_no_key() {
        assert_eq!(OLLAMA.base_url, "http://localhost:11434/v1");
        assert_eq!(LM_STUDIO.base_url, "http://localhost:1234/v1");
        for preset in [OLLAMA, LM_STUDIO] {
            assert!(preset.local && !preset.needs_api_key);
        }
    }

    #[test]
    fn the_custom_preset_leaves_the_address_to_the_user() {
        let custom = openai_preset("custom").unwrap();
        assert_eq!(custom.base_url, "");
        assert!(!custom.needs_api_key);
    }

    #[test]
    fn presets_are_found_by_id() {
        assert_eq!(openai_preset("groq"), Some(&GROQ));
        assert_eq!(openai_preset("nope"), None);
    }
}
