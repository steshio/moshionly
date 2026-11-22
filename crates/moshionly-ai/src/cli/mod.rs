//! Subscription providers: agent CLIs that the user installed and signed in
//! to. Moshionly runs the binary and reads its output, nothing more.

pub(crate) mod claude_code;
pub(crate) mod discover;
pub(crate) mod process;
pub(crate) mod prompt;
#[cfg(all(test, unix))]
pub(crate) mod test_support;

use tokio_util::sync::CancellationToken;

use crate::error::ErrorKind;
use crate::event::EventSender;

/// Completes when a run should stop: the caller cancelled, or nobody is
/// listening to the events any more.
pub(crate) async fn stopped(cancel: &CancellationToken, events: &EventSender) {
    tokio::select! {
        () = cancel.cancelled() => {}
        () = events.closed() => {}
    }
}

/// Guesses what went wrong from text an agent CLI printed when it failed
/// without a structured error. Each CLI words these differently, so this
/// looks for the terms they share.
pub(crate) fn classify_failure_text(text: &str) -> ErrorKind {
    let text = text.to_ascii_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|needle| text.contains(needle));
    if has(&[
        "not logged in",
        "not signed in",
        "authentication required",
        "authentication failed",
        "please run /login",
        "login required",
        "unauthorized",
        "invalid api key",
        "no credentials",
        "not authenticated",
    ]) {
        ErrorKind::NotSignedIn
    } else if has(&[
        "rate limit",
        "rate_limit",
        "usage limit",
        "too many requests",
        "quota",
        "resource_exhausted",
    ]) {
        ErrorKind::RateLimited
    } else if has(&["overloaded", "service unavailable", "at capacity"]) {
        ErrorKind::Overloaded
    } else if text.contains("model")
        && has(&[
            "not found",
            "does not exist",
            "unrecognized",
            "unknown model",
            "invalid model",
            "not available",
            "not supported",
            "unsupported",
            "cannot use",
            "do not have access",
            "issue with the selected model",
        ])
    {
        ErrorKind::ModelUnavailable
    } else if has(&[
        "enotfound",
        "econnrefused",
        "econnreset",
        "etimedout",
        "network error",
        "fetch failed",
        "connection error",
        "offline",
    ]) {
        ErrorKind::Network
    } else {
        ErrorKind::Other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_text_is_classified_by_shared_terms() {
        let cases = [
            ("Not logged in · Please run /login", ErrorKind::NotSignedIn),
            (
                "Error: Authentication required. Please run 'agent login' first, or set CURSOR_API_KEY environment variable.",
                ErrorKind::NotSignedIn,
            ),
            ("You've hit your usage limit", ErrorKind::RateLimited),
            ("429 Too Many Requests", ErrorKind::RateLimited),
            ("The model is overloaded. Try again.", ErrorKind::Overloaded),
            (
                "[claude-code:unrecognized_model] {\"model\":\"nope\"}",
                ErrorKind::ModelUnavailable,
            ),
            (
                "The model `o9` does not exist or you do not have access to it.",
                ErrorKind::ModelUnavailable,
            ),
            (
                "request failed: getaddrinfo ENOTFOUND api.example.com",
                ErrorKind::Network,
            ),
            ("error: unknown option '--frobnicate'", ErrorKind::Other),
            ("", ErrorKind::Other),
        ];
        for (text, expected) in cases {
            assert_eq!(classify_failure_text(text), expected, "{text:?}");
        }
    }

    #[tokio::test]
    async fn stopped_completes_on_cancel_or_when_the_receiver_goes_away() {
        let cancel = CancellationToken::new();
        let (events, receiver) = crate::event::event_channel();
        cancel.cancel();
        stopped(&cancel, &events).await;

        let cancel = CancellationToken::new();
        drop(receiver);
        stopped(&cancel, &events).await;
    }
}
