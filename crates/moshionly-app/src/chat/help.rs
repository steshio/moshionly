//! The way forward after a failure: what to tell the person and which
//! buttons to offer, by kind of failure and provider.

use moshionly_ai::ErrorKind;

/// A button offered under an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Remedy {
    /// Open the AI settings.
    OpenSettings,
    /// Send the same message again.
    TryAgain,
}

/// What the panel says about a failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Help {
    /// A short headline.
    pub title: String,
    /// One or two sentences saying what to do. Text in backticks is a
    /// command to type.
    pub advice: String,
    /// The buttons to offer, most useful first.
    pub remedies: Vec<Remedy>,
}

/// The help for a failure of `kind` from the provider `provider_id`, called
/// `provider_name` in the interface.
pub fn help(kind: ErrorKind, provider_id: &str, provider_name: &str) -> Help {
    use Remedy::{OpenSettings, TryAgain};
    let name = provider_name;
    let (title, advice, remedies) = match kind {
        ErrorKind::NotInstalled => (
            format!("{name} is not installed"),
            match sign_in_command(provider_id) {
                Some(command) => format!(
                    "Install {name}, then open Terminal and run `{command}` once to sign in. \
                     Or choose another AI in the settings."
                ),
                None => format!("Install {name}, or choose another AI in the settings."),
            },
            vec![OpenSettings, TryAgain],
        ),
        ErrorKind::NotSignedIn => (
            format!("{name} is not signed in"),
            match sign_in_command(provider_id) {
                Some(command) => {
                    format!("Open Terminal and run `{command}` once to sign in, then try again.")
                }
                None => format!("Sign in to {name}, then try again."),
            },
            vec![TryAgain, OpenSettings],
        ),
        ErrorKind::MissingKey => (
            format!("{name} needs an API key"),
            "Add your key in the AI settings. It is kept in your system's keychain.".to_owned(),
            vec![OpenSettings],
        ),
        ErrorKind::InvalidKey => (
            format!("{name} did not accept the key"),
            "Check the key in the AI settings, or create a new one with the provider.".to_owned(),
            vec![OpenSettings],
        ),
        ErrorKind::RateLimited => (
            "Usage limit reached".to_owned(),
            format!(
                "{name} asked to slow down. Wait a little and try again, or switch to another AI."
            ),
            vec![TryAgain, OpenSettings],
        ),
        ErrorKind::Overloaded => (
            format!("{name} is busy"),
            "The service is over capacity right now. Try again in a moment.".to_owned(),
            vec![TryAgain],
        ),
        ErrorKind::ModelUnavailable => (
            "That model is not available".to_owned(),
            "Pick another model from the menu at the top of the chat.".to_owned(),
            vec![OpenSettings],
        ),
        ErrorKind::InvalidRequest => (
            "The request was refused".to_owned(),
            "Try a shorter message, or start a new chat.".to_owned(),
            vec![TryAgain],
        ),
        ErrorKind::Network if is_local(provider_id) => (
            format!("Could not reach {name}"),
            format!("Make sure {name} is running on this computer, then try again."),
            vec![TryAgain, OpenSettings],
        ),
        ErrorKind::Network => (
            format!("Could not reach {name}"),
            "Check your internet connection, then try again.".to_owned(),
            vec![TryAgain, OpenSettings],
        ),
        ErrorKind::Protocol => (
            format!("Unexpected answer from {name}"),
            "Try again. If it keeps happening, try another AI.".to_owned(),
            vec![TryAgain, OpenSettings],
        ),
        ErrorKind::Other => (
            "Something went wrong".to_owned(),
            "Try again, or check the AI settings.".to_owned(),
            vec![TryAgain, OpenSettings],
        ),
    };
    Help {
        title,
        advice,
        remedies,
    }
}

/// The button text for a remedy.
pub fn remedy_label(remedy: Remedy) -> &'static str {
    match remedy {
        Remedy::OpenSettings => "Open AI settings",
        Remedy::TryAgain => "Try again",
    }
}

/// The command that signs a person in to an agent CLI.
fn sign_in_command(provider_id: &str) -> Option<&'static str> {
    match provider_id {
        "claude-code" => Some("claude"),
        "cursor-agent" => Some("cursor-agent login"),
        "codex" => Some("codex login"),
        "gemini-cli" => Some("gemini"),
        _ => None,
    }
}

/// True for providers that run on the person's own computer.
fn is_local(provider_id: &str) -> bool {
    moshionly_ai::openai_preset(provider_id).is_some_and(|preset| preset.local)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cli_that_is_not_signed_in_says_what_to_run() {
        let help = help(ErrorKind::NotSignedIn, "claude-code", "Claude Code");
        assert_eq!(help.title, "Claude Code is not signed in");
        assert!(help.advice.contains("run `claude` once"), "{}", help.advice);
        assert_eq!(help.remedies, vec![Remedy::TryAgain, Remedy::OpenSettings]);
        let help = super::help(ErrorKind::NotInstalled, "cursor-agent", "Cursor Agent");
        assert!(
            help.advice.contains("`cursor-agent login`"),
            "{}",
            help.advice
        );
    }

    #[test]
    fn key_problems_lead_to_the_settings() {
        for kind in [ErrorKind::MissingKey, ErrorKind::InvalidKey] {
            assert_eq!(
                help(kind, "anthropic", "Anthropic").remedies[0],
                Remedy::OpenSettings
            );
        }
    }

    #[test]
    fn local_servers_are_asked_to_be_started() {
        let help = help(ErrorKind::Network, "ollama", "Ollama");
        assert!(help.advice.contains("running on this computer"));
        let help = super::help(ErrorKind::Network, "openai", "OpenAI");
        assert!(help.advice.contains("internet connection"));
    }

    #[test]
    fn every_kind_offers_a_way_forward() {
        let kinds = [
            ErrorKind::NotInstalled,
            ErrorKind::NotSignedIn,
            ErrorKind::MissingKey,
            ErrorKind::InvalidKey,
            ErrorKind::RateLimited,
            ErrorKind::Overloaded,
            ErrorKind::ModelUnavailable,
            ErrorKind::InvalidRequest,
            ErrorKind::Network,
            ErrorKind::Protocol,
            ErrorKind::Other,
        ];
        for kind in kinds {
            let help = help(kind, "custom", "Custom");
            assert!(!help.title.is_empty() && !help.advice.is_empty());
            assert!(!help.remedies.is_empty(), "{kind:?}");
        }
    }
}
