//! What the API providers share: an HTTP client, retries for failures that
//! pass, cancellation, and turning HTTP failures into [`AiError`]s that never
//! contain a key.

pub(crate) mod sse;
#[cfg(test)]
pub(crate) mod test_support;

use std::future::Future;
use std::time::Duration;

use reqwest::header::HeaderMap;
use tokio_util::sync::CancellationToken;

use crate::error::{AiError, ErrorKind};
use crate::event::{EventSender, stopped};
use crate::redact::ApiKey;

/// How long to wait for a connection before giving up.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

/// How long a response may stay silent. Model APIs send keep-alive events
/// while they work, so a silence this long means the connection is dead.
const READ_TIMEOUT: Duration = Duration::from_secs(300);

/// The most of an error response that is read. Enough for any error
/// message; a misbehaving server cannot make us buffer more.
const MAX_ERROR_BODY_BYTES: usize = 64 * 1024;

/// The HTTP client the API providers use.
pub(crate) fn client() -> Result<reqwest::Client, AiError> {
    reqwest::Client::builder()
        .user_agent(concat!("moshionly/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(READ_TIMEOUT)
        .build()
        .map_err(|error| AiError::other(format!("could not set up the HTTP client: {error}")))
}

/// Why a request stopped before it produced a result.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Interrupt {
    /// The caller cancelled, or stopped listening.
    Cancelled,
    /// The request failed.
    Failed(AiError),
}

impl From<AiError> for Interrupt {
    fn from(error: AiError) -> Self {
        Self::Failed(error)
    }
}

/// A failed attempt, with what is needed to decide about another one.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Failure {
    /// What went wrong.
    pub error: AiError,
    /// Sending the same request again may work.
    pub retryable: bool,
    /// How long the server asked us to wait first.
    pub retry_after: Option<Duration>,
}

impl Failure {
    /// A failure that another attempt will not fix.
    pub fn permanent(error: AiError) -> Self {
        Self {
            error,
            retryable: false,
            retry_after: None,
        }
    }

    /// A failure that may pass.
    pub fn transient(error: AiError) -> Self {
        Self {
            error,
            retryable: true,
            retry_after: None,
        }
    }
}

/// How one attempt ended, when it did not succeed.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum AttemptError {
    /// The caller cancelled, or stopped listening.
    Cancelled,
    /// The attempt failed.
    Failed(Failure),
}

impl From<Failure> for AttemptError {
    fn from(failure: Failure) -> Self {
        Self::Failed(failure)
    }
}

/// When and how often to try again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RetryPolicy {
    /// Attempts after the first one.
    pub max_retries: u32,
    /// The wait before the first retry; it doubles each time.
    pub base_delay: Duration,
    /// The longest wait between attempts. A server that asks for more than
    /// this is not waited for: the error goes to the user, who can see how
    /// long the limit lasts.
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 2,
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(20),
        }
    }
}

impl RetryPolicy {
    /// The wait before retry number `retry` (counted from zero), or `None`
    /// when the failure should be reported instead.
    fn delay(&self, retry: u32, failure: &Failure) -> Option<Duration> {
        if !failure.retryable || retry >= self.max_retries {
            return None;
        }
        match failure.retry_after {
            Some(asked) if asked > self.max_delay => None,
            Some(asked) => Some(asked),
            None => {
                let doubled = self.base_delay.saturating_mul(2u32.saturating_pow(retry));
                Some(doubled.min(self.max_delay) + jitter(self.base_delay))
            }
        }
    }
}

/// A small random extra wait, so that clients which failed together do not
/// retry together.
fn jitter(base: Duration) -> Duration {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.subsec_nanos())
        .unwrap_or(0);
    base.mul_f64(f64::from(nanos % 1000) / 4000.0)
}

/// Runs `attempt` until it succeeds, fails for good, or runs out of tries.
/// The wait between attempts ends early when the request is cancelled.
pub(crate) async fn with_retries<T, F, Fut>(
    policy: &RetryPolicy,
    cancel: &CancellationToken,
    events: &EventSender,
    mut attempt: F,
) -> Result<T, Interrupt>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, AttemptError>>,
{
    let mut retry = 0;
    loop {
        let failure = match attempt().await {
            Ok(value) => return Ok(value),
            Err(AttemptError::Cancelled) => return Err(Interrupt::Cancelled),
            Err(AttemptError::Failed(failure)) => failure,
        };
        let Some(delay) = policy.delay(retry, &failure) else {
            return Err(Interrupt::Failed(failure.error));
        };
        retry += 1;
        tokio::select! {
            () = stopped(cancel, events) => return Err(Interrupt::Cancelled),
            () = tokio::time::sleep(delay) => {}
        }
    }
}

/// Sends a request and returns the response once its status line and
/// headers are in. A response with an error status is returned as it is.
pub(crate) async fn send(
    request: reqwest::RequestBuilder,
    key: Option<&ApiKey>,
    cancel: &CancellationToken,
    events: &EventSender,
) -> Result<reqwest::Response, AttemptError> {
    tokio::select! {
        () = stopped(cancel, events) => Err(AttemptError::Cancelled),
        response = request.send() => response.map_err(|error| transport_failure(&error, key).into()),
    }
}

/// Reads the next piece of a response body, or `None` at its end.
pub(crate) async fn next_chunk(
    response: &mut reqwest::Response,
    key: Option<&ApiKey>,
    cancel: &CancellationToken,
    events: &EventSender,
) -> Result<Option<Vec<u8>>, AttemptError> {
    tokio::select! {
        () = stopped(cancel, events) => Err(AttemptError::Cancelled),
        chunk = response.chunk() => match chunk {
            Ok(chunk) => Ok(chunk.map(|bytes| bytes.to_vec())),
            Err(error) => Err(transport_failure(&error, key).into()),
        },
    }
}

/// Reads the body of an error response as text, up to a sensible size.
pub(crate) async fn read_error_body(mut response: reqwest::Response) -> String {
    let mut body = Vec::new();
    while body.len() < MAX_ERROR_BODY_BYTES {
        match response.chunk().await {
            Ok(Some(chunk)) => body.extend_from_slice(&chunk),
            _ => break,
        }
    }
    body.truncate(MAX_ERROR_BODY_BYTES);
    String::from_utf8_lossy(&body).into_owned()
}

/// The failure for a request that never got an HTTP answer: no connection,
/// a timeout, a connection that broke half way.
pub(crate) fn transport_failure(error: &reqwest::Error, key: Option<&ApiKey>) -> Failure {
    // Without the URL: it is long, and a base URL can carry credentials.
    let mut message = error_chain(error);
    if error.is_timeout() {
        message = format!("the server took too long to answer ({message})");
    } else if error.is_connect() {
        message = format!("could not connect to the server ({message})");
    }
    Failure::transient(AiError::network(clean(&message, key)))
}

/// An error and its causes as one line, with URLs left out.
fn error_chain(error: &reqwest::Error) -> String {
    use std::error::Error as _;
    let mut parts = vec![strip_url(&error.to_string())];
    let mut source = error.source();
    while let Some(cause) = source {
        let text = strip_url(&cause.to_string());
        if !parts.contains(&text) {
            parts.push(text);
        }
        source = cause.source();
    }
    parts.join(": ")
}

/// Removes the " for url (...)" suffix reqwest adds to its messages.
fn strip_url(message: &str) -> String {
    match message.find(" for url (") {
        Some(start) => {
            let rest = &message[start..];
            let end = rest.find(')').map_or(rest.len(), |index| index + 1);
            format!("{}{}", &message[..start], &rest[end..])
        }
        None => message.to_owned(),
    }
}

/// The kind of error an HTTP status stands for across model APIs.
pub(crate) fn kind_for_status(status: u16) -> ErrorKind {
    match status {
        401 => ErrorKind::InvalidKey,
        404 => ErrorKind::ModelUnavailable,
        400 | 413 | 422 => ErrorKind::InvalidRequest,
        408 => ErrorKind::Network,
        429 => ErrorKind::RateLimited,
        500..=599 => ErrorKind::Overloaded,
        _ => ErrorKind::Other,
    }
}

/// True for statuses where the same request may succeed a moment later.
pub(crate) fn is_retryable_status(status: u16) -> bool {
    matches!(status, 408 | 409 | 429 | 500..=599)
}

/// Builds the failure for an error response.
///
/// `message` is what the service said, already picked out of its body.
/// `kind` lets a provider override the classification by status when the
/// body is more precise.
pub(crate) fn status_failure(
    status: u16,
    kind: Option<ErrorKind>,
    message: &str,
    headers: &HeaderMap,
    key: Option<&ApiKey>,
) -> Failure {
    let kind = kind.unwrap_or_else(|| kind_for_status(status));
    let retry_after = retry_after(headers);
    let mut message = clean(message.trim(), key);
    if message.is_empty() {
        message = format!("the server answered with HTTP status {status}");
    }
    let error = match kind {
        ErrorKind::RateLimited => AiError::RateLimited {
            message,
            retry_after,
        },
        kind => AiError::from_kind(kind, message),
    };
    Failure {
        error,
        retryable: is_retryable_status(status),
        retry_after,
    }
}

/// How long the server asked the client to wait, from `retry-after-ms` or
/// `retry-after` in seconds.
pub(crate) fn retry_after(headers: &HeaderMap) -> Option<Duration> {
    let number = |name: &str| -> Option<f64> {
        headers
            .get(name)?
            .to_str()
            .ok()?
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite() && *value >= 0.0)
    };
    if let Some(millis) = number("retry-after-ms") {
        return Some(Duration::from_secs_f64(millis / 1000.0));
    }
    number("retry-after").map(Duration::from_secs_f64)
}

/// Removes the key from text that came from outside, by value and by shape.
pub(crate) fn clean(text: &str, key: Option<&ApiKey>) -> String {
    match key {
        Some(key) => key.redact(text),
        None => crate::redact::scrub(text),
    }
}

/// Joins a base URL and a path without doubling or dropping the slash.
pub(crate) fn join_url(base: &str, path: &str) -> String {
    format!(
        "{}/{}",
        base.trim().trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::event_channel;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn fast_policy() -> RetryPolicy {
        RetryPolicy {
            max_retries: 2,
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(50),
        }
    }

    fn overloaded() -> Failure {
        Failure::transient(AiError::from_kind(ErrorKind::Overloaded, "busy"))
    }

    #[tokio::test]
    async fn a_transient_failure_is_retried_until_it_passes() {
        let attempts = AtomicU32::new(0);
        let (events, _receiver) = event_channel();
        let result = with_retries(
            &fast_policy(),
            &CancellationToken::new(),
            &events,
            || async {
                if attempts.fetch_add(1, Ordering::SeqCst) < 2 {
                    Err(AttemptError::Failed(overloaded()))
                } else {
                    Ok("answer")
                }
            },
        )
        .await;
        assert_eq!(result, Ok("answer"));
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn retries_stop_at_the_limit_and_report_the_last_error() {
        let attempts = AtomicU32::new(0);
        let (events, _receiver) = event_channel();
        let result: Result<(), _> = with_retries(
            &fast_policy(),
            &CancellationToken::new(),
            &events,
            || async {
                attempts.fetch_add(1, Ordering::SeqCst);
                Err(AttemptError::Failed(overloaded()))
            },
        )
        .await;
        assert_eq!(result, Err(Interrupt::Failed(overloaded().error)));
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn a_permanent_failure_is_not_retried() {
        let attempts = AtomicU32::new(0);
        let (events, _receiver) = event_channel();
        let error = AiError::from_kind(ErrorKind::InvalidKey, "no");
        let result: Result<(), _> = with_retries(
            &fast_policy(),
            &CancellationToken::new(),
            &events,
            || async {
                attempts.fetch_add(1, Ordering::SeqCst);
                Err(AttemptError::Failed(Failure::permanent(error.clone())))
            },
        )
        .await;
        assert_eq!(result, Err(Interrupt::Failed(error.clone())));
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn cancelling_ends_the_wait_between_attempts() {
        let policy = RetryPolicy {
            max_retries: 5,
            base_delay: Duration::from_secs(60),
            max_delay: Duration::from_secs(60),
        };
        let cancel = CancellationToken::new();
        let (events, _receiver) = event_channel();
        let trigger = cancel.clone();
        let started = std::time::Instant::now();
        let result: Result<(), _> = with_retries(&policy, &cancel, &events, || {
            trigger.cancel();
            async { Err(AttemptError::Failed(overloaded())) }
        })
        .await;
        assert_eq!(result, Err(Interrupt::Cancelled));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn the_servers_wait_is_honoured_unless_it_is_too_long() {
        let policy = fast_policy();
        let mut failure = overloaded();
        failure.retry_after = Some(Duration::from_millis(20));
        assert_eq!(policy.delay(0, &failure), Some(Duration::from_millis(20)));
        failure.retry_after = Some(Duration::from_secs(3600));
        assert_eq!(policy.delay(0, &failure), None);
    }

    #[test]
    fn backoff_grows_and_is_capped() {
        let policy = RetryPolicy {
            max_retries: 10,
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_millis(350),
        };
        let failure = overloaded();
        let first = policy.delay(0, &failure).unwrap();
        let third = policy.delay(2, &failure).unwrap();
        assert!(first >= Duration::from_millis(100) && first < Duration::from_millis(130));
        // 400 ms capped to 350 ms, plus at most a quarter of the base.
        assert!(third >= Duration::from_millis(350) && third < Duration::from_millis(380));
        assert_eq!(policy.delay(10, &failure), None);
    }

    #[test]
    fn statuses_map_to_error_kinds() {
        let cases = [
            (400, ErrorKind::InvalidRequest, false),
            (401, ErrorKind::InvalidKey, false),
            (403, ErrorKind::Other, false),
            (404, ErrorKind::ModelUnavailable, false),
            (413, ErrorKind::InvalidRequest, false),
            (429, ErrorKind::RateLimited, true),
            (500, ErrorKind::Overloaded, true),
            (529, ErrorKind::Overloaded, true),
        ];
        for (status, kind, retryable) in cases {
            let failure = status_failure(status, None, "message", &HeaderMap::new(), None);
            assert_eq!(failure.error.kind(), kind, "{status}");
            assert_eq!(failure.retryable, retryable, "{status}");
        }
    }

    #[test]
    fn a_rate_limit_carries_the_wait_the_server_asked_for() {
        let mut headers = HeaderMap::new();
        headers.insert("retry-after", "7".parse().unwrap());
        let failure = status_failure(429, None, "slow down", &headers, None);
        assert_eq!(
            failure.error,
            AiError::RateLimited {
                message: "slow down".to_owned(),
                retry_after: Some(Duration::from_secs(7)),
            }
        );
        assert_eq!(failure.retry_after, Some(Duration::from_secs(7)));

        headers.insert("retry-after-ms", "250".parse().unwrap());
        assert_eq!(retry_after(&headers), Some(Duration::from_millis(250)));
        headers.insert("retry-after-ms", "soon".parse().unwrap());
        assert_eq!(retry_after(&headers), Some(Duration::from_secs(7)));
    }

    #[test]
    fn error_messages_never_contain_the_key() {
        let key = ApiKey::new("local-secret-key");
        let failure = status_failure(
            401,
            None,
            "key local-secret-key is not valid, nor is sk-proj-Abc123def456ghi789jkl012",
            &HeaderMap::new(),
            Some(&key),
        );
        let message = failure.error.to_string();
        assert!(!message.contains("local-secret-key"));
        assert!(!message.contains("sk-proj"));
    }

    #[test]
    fn an_empty_error_body_still_gives_a_message() {
        let failure = status_failure(502, None, "  ", &HeaderMap::new(), None);
        assert!(failure.error.to_string().contains("HTTP status 502"));
    }

    #[test]
    fn urls_are_removed_from_transport_messages() {
        assert_eq!(
            strip_url("error sending request for url (https://user:pw@example.com/v1/x)"),
            "error sending request"
        );
        assert_eq!(strip_url("plain message"), "plain message");
    }

    #[test]
    fn urls_are_joined_with_exactly_one_slash() {
        for (base, path) in [
            ("https://api.example.com", "v1/models"),
            ("https://api.example.com/", "/v1/models"),
            (" https://api.example.com// ", "v1/models"),
        ] {
            assert_eq!(join_url(base, path), "https://api.example.com/v1/models");
        }
    }

    #[tokio::test]
    async fn an_unreachable_server_is_a_network_failure_that_can_be_retried() {
        // A port that was just free: nothing is listening on it.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let (events, _receiver) = event_channel();
        let request = client().unwrap().get(format!("http://{addr}/v1/models"));
        let error = send(request, None, &CancellationToken::new(), &events)
            .await
            .unwrap_err();
        match error {
            AttemptError::Failed(failure) => {
                assert_eq!(failure.error.kind(), ErrorKind::Network);
                assert!(failure.retryable);
                assert!(!failure.error.to_string().contains("127.0.0.1"));
            }
            AttemptError::Cancelled => panic!("expected a failure"),
        }
    }
}
