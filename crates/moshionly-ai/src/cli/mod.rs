//! Subscription providers: agent CLIs that the user installed and signed in
//! to. Moshionly runs the binary and reads its output, nothing more.

pub(crate) mod claude_code;
pub(crate) mod codex;
pub(crate) mod cursor_agent;
pub(crate) mod discover;
pub(crate) mod files;
pub(crate) mod process;
pub(crate) mod prompt;
#[cfg(all(test, unix))]
pub(crate) mod test_support;

use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::error::{AiError, ErrorKind};
use crate::event::EventSender;
use crate::tool::summarize_text;

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

/// One line describing a tool result whose content is either a string or a
/// list of MCP-style text and image blocks.
pub(crate) fn summarize_tool_content(content: Option<&Value>, failed: bool) -> String {
    let mut text = String::new();
    let mut images = 0usize;
    match content {
        Some(Value::String(value)) => text.push_str(value),
        Some(Value::Array(blocks)) => {
            for block in blocks {
                match block.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        if !text.is_empty() {
                            text.push('\n');
                        }
                        text.push_str(block.get("text").and_then(Value::as_str).unwrap_or(""));
                    }
                    Some("image") => images += 1,
                    _ => {}
                }
            }
        }
        _ => {}
    }
    let mut summary = summarize_text(&text);
    if images > 0 {
        if !summary.is_empty() {
            summary.push_str(", ");
        }
        summary.push_str(&format!(
            "{images} image{}",
            if images == 1 { "" } else { "s" }
        ));
    }
    if summary.is_empty() {
        summary.push_str(if failed { "failed" } else { "ok" });
    }
    summary
}

/// The error for an agent CLI that ended without finishing its answer,
/// built from whatever it printed on the way out.
///
/// `product` is the name shown to the user and `sign_in_hint` what to tell
/// them when the text says they are not signed in.
pub(crate) fn exit_failure(
    product: &str,
    sign_in_hint: &str,
    code: Option<i32>,
    detail: &str,
) -> AiError {
    let kind = classify_failure_text(detail);
    let line = most_telling_line(detail);
    let line = line.strip_prefix("Error: ").unwrap_or(line);
    let message = match (kind, code) {
        (ErrorKind::NotSignedIn, _) => sign_in_hint.to_owned(),
        (_, _) if !line.is_empty() => summarize_text(line),
        (_, Some(code)) => format!("{product} exited with code {code}"),
        (_, None) => format!("{product} stopped without an answer"),
    };
    AiError::from_kind(kind, message)
}

/// The line of a CLI's parting words that best explains a failure: the
/// first one that mentions an error, otherwise the last one.
fn most_telling_line(detail: &str) -> &str {
    let mut lines = detail
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty());
    let mut last = "";
    for line in lines.by_ref() {
        if line.to_ascii_lowercase().contains("error") {
            return line;
        }
        last = line;
    }
    last
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
    #[test]
    fn a_tool_result_with_an_image_mentions_it() {
        let content = json!([
            {"type":"text","text":"frame at 500 ms"},
            {"type":"image","source":{"type":"base64","media_type":"image/png","data":"AAAA"}}
        ]);
        assert_eq!(
            summarize_tool_content(Some(&content), false),
            "frame at 500 ms, 1 image"
        );
        assert_eq!(summarize_tool_content(None, false), "ok");
        assert_eq!(summarize_tool_content(None, true), "failed");
    }

    #[test]
    fn an_exit_failure_quotes_the_most_telling_line() {
        let error = exit_failure(
            "Codex",
            "sign in",
            Some(1),
            "warning: something minor\nError: unexpected status 500\n  at line 3",
        );
        assert_eq!(error.to_string(), "unexpected status 500");

        let error = exit_failure("Codex", "sign in", Some(1), "first\nlast words");
        assert_eq!(error.to_string(), "last words");
    }

    #[test]
    fn an_exit_failure_without_text_reports_the_exit_code() {
        let error = exit_failure("Gemini CLI", "sign in", Some(42), "");
        assert_eq!(error.to_string(), "Gemini CLI exited with code 42");
        let error = exit_failure("Gemini CLI", "sign in", None, "");
        assert_eq!(error.to_string(), "Gemini CLI stopped without an answer");
    }

    #[test]
    fn an_exit_failure_that_is_a_sign_in_problem_gives_the_hint() {
        let error = exit_failure("Codex", "run `codex login`", Some(1), "401 Unauthorized");
        assert_eq!(error.kind(), ErrorKind::NotSignedIn);
        assert_eq!(error.to_string(), "not signed in: run `codex login`");
    }
}
