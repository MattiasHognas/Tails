//! Timeouts, deadlines and bounded retries for upstream HTTP calls.
//!
//! - [`HttpConfig`] builds reqwest clients with connect and request timeouts.
//! - [`RetryPolicy`] + [`send_with_retry`] retry transient failures only
//!   (connect errors, timeouts, HTTP 429 honoring `Retry-After`, 5xx) with
//!   exponential backoff and jitter. Other 4xx are never retried.
//! - [`run_stage`] bounds a pipeline stage by its own timeout and by the
//!   enclosing deadline. The deadline is carried in a task-local so retries
//!   never sleep past it, without threading it through every client method.

use crate::error::{RagError, Stage, UpstreamError, UpstreamFailure};
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::Instant;

/// Read a duration in milliseconds from `name`, falling back to `default`.
pub fn env_duration_ms(name: &str, default: Duration) -> Duration {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or(default)
}

fn env_u32(name: &str, default: u32) -> u32 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}

/// Transport-level timeouts applied to every request of a reqwest client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HttpConfig {
    pub connect_timeout: Duration,
    pub request_timeout: Duration,
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(60),
        }
    }
}

impl HttpConfig {
    /// `RAG_HTTP_CONNECT_TIMEOUT_MS` (default 5000) and
    /// `RAG_HTTP_REQUEST_TIMEOUT_MS` (default 60000).
    pub fn from_env() -> Self {
        let d = Self::default();
        Self {
            connect_timeout: env_duration_ms("RAG_HTTP_CONNECT_TIMEOUT_MS", d.connect_timeout),
            request_timeout: env_duration_ms("RAG_HTTP_REQUEST_TIMEOUT_MS", d.request_timeout),
        }
    }

    pub fn build_client(&self) -> reqwest::Client {
        reqwest::Client::builder()
            .connect_timeout(self.connect_timeout)
            .timeout(self.request_timeout)
            .build()
            // Only fails if the TLS backend cannot initialise; fall back to the
            // default client rather than aborting startup.
            .unwrap_or_else(|_| reqwest::Client::new())
    }
}

/// Bounded retry policy for transient upstream failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Total attempts including the first one (1 = no retries).
    pub max_attempts: u32,
    /// Backoff before the first retry; doubles per retry.
    pub base_delay: Duration,
    /// Cap for a single backoff. A `Retry-After` longer than this is not
    /// waited for: the call fails as unavailable instead.
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            base_delay: Duration::from_millis(200),
            max_delay: Duration::from_secs(10),
        }
    }
}

impl RetryPolicy {
    /// A single attempt, no retries.
    pub fn none() -> Self {
        Self {
            max_attempts: 1,
            ..Self::default()
        }
    }

    /// `RAG_RETRY_MAX_ATTEMPTS` (default 3), `RAG_RETRY_BASE_DELAY_MS`
    /// (default 200) and `RAG_RETRY_MAX_DELAY_MS` (default 10000).
    pub fn from_env() -> Self {
        let d = Self::default();
        Self {
            max_attempts: env_u32("RAG_RETRY_MAX_ATTEMPTS", d.max_attempts).max(1),
            base_delay: env_duration_ms("RAG_RETRY_BASE_DELAY_MS", d.base_delay),
            max_delay: env_duration_ms("RAG_RETRY_MAX_DELAY_MS", d.max_delay),
        }
    }

    /// Backoff before retry number `retry` (1-based): exponential, capped at
    /// `max_delay`, with "equal jitter" (uniform in `[d/2, d]`).
    pub fn backoff(&self, retry: u32) -> Duration {
        let exp = self
            .base_delay
            .saturating_mul(1u32 << retry.saturating_sub(1).min(16));
        let capped = exp.min(self.max_delay);
        capped / 2 + capped.mul_f64(jitter_fraction() / 2.0)
    }
}

/// Cheap uniform value in `[0, 1)`; avoids pulling in `rand` for jitter.
fn jitter_fraction() -> f64 {
    static STATE: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    // splitmix64
    let mut z = nanos ^ STATE.fetch_add(0x9E37_79B9_7F4A_7C15, Ordering::Relaxed);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 11) as f64 / (1u64 << 53) as f64
}

tokio::task_local! {
    static DEADLINE: Instant;
}

/// The innermost deadline in scope for the current task, if any.
pub fn current_deadline() -> Option<Instant> {
    DEADLINE.try_with(|d| *d).ok()
}

/// Run `fut` with `deadline` in scope (never extending an outer deadline).
pub async fn with_deadline<F: Future>(deadline: Instant, fut: F) -> F::Output {
    let effective = current_deadline().map_or(deadline, |outer| outer.min(deadline));
    DEADLINE.scope(effective, fut).await
}

/// Run one pipeline stage bounded by `timeout` and any enclosing deadline.
/// Elapsing maps to [`RagError::Timeout`] for `stage`.
pub async fn run_stage<T, F>(stage: Stage, timeout: Duration, fut: F) -> Result<T, RagError>
where
    F: Future<Output = Result<T, RagError>>,
{
    let mut deadline = Instant::now() + timeout;
    if let Some(outer) = current_deadline() {
        deadline = deadline.min(outer);
    }
    match with_deadline(deadline, tokio::time::timeout_at(deadline, fut)).await {
        Ok(res) => res,
        Err(_) => {
            tracing::warn!(stage = %stage, timeout_ms = timeout.as_millis() as u64, "stage deadline exceeded");
            Err(RagError::Timeout { stage })
        }
    }
}

/// Send a request built by `build`, retrying transient failures per `policy`.
///
/// Returns the first 2xx response. Non-2xx bodies are logged (truncated) and
/// never included in the returned error. No retry is started if its backoff
/// would end past the current deadline.
pub async fn send_with_retry<F>(
    policy: &RetryPolicy,
    what: &str,
    build: F,
) -> Result<reqwest::Response, UpstreamFailure>
where
    F: Fn() -> reqwest::RequestBuilder,
{
    let max_attempts = policy.max_attempts.max(1);
    let mut attempt = 0;
    loop {
        attempt += 1;
        let error = match build().send().await {
            Ok(r) if r.status().is_success() => return Ok(r),
            Ok(r) => status_error(what, r).await,
            Err(e) => {
                let err = UpstreamError::from_reqwest(e);
                tracing::warn!(upstream = what, attempt, error = %err, "upstream request failed");
                err
            }
        };

        let fail = |error, exhausted| UpstreamFailure {
            error,
            attempts: attempt,
            exhausted,
        };
        if !error.is_transient() {
            return Err(fail(error, false));
        }
        if attempt >= max_attempts {
            return Err(fail(error, true));
        }
        let delay = match error.retry_after() {
            Some(ra) if ra > policy.max_delay => {
                tracing::warn!(
                    upstream = what,
                    retry_after_ms = ra.as_millis() as u64,
                    "Retry-After exceeds max retry delay; giving up"
                );
                return Err(fail(error, true));
            }
            Some(ra) => ra,
            None => policy.backoff(attempt),
        };
        if let Some(deadline) = current_deadline()
            && Instant::now() + delay >= deadline
        {
            tracing::warn!(
                upstream = what,
                attempt,
                "not retrying: backoff would exceed deadline"
            );
            return Err(fail(error, true));
        }
        tracing::info!(
            upstream = what,
            attempt,
            delay_ms = delay.as_millis() as u64,
            "retrying upstream request"
        );
        tokio::time::sleep(delay).await;
    }
}

async fn status_error(what: &str, r: reqwest::Response) -> UpstreamError {
    let status = r.status().as_u16();
    let retry_after = parse_retry_after(r.headers());
    let body = r.text().await.unwrap_or_default();
    let body = crate::text::truncate_bytes(&body, 512);
    tracing::warn!(upstream = what, status, body = %body, "upstream returned error status");
    UpstreamError::Status {
        status,
        retry_after,
    }
}

/// Parse `retry-after-ms` (OpenAI) or `Retry-After` (seconds or HTTP-date).
pub fn parse_retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    let get = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    if let Some(ms) = get("retry-after-ms").and_then(|v| v.trim().parse::<f64>().ok())
        && ms.is_finite()
        && ms >= 0.0
    {
        return Some(Duration::from_secs_f64(ms / 1000.0));
    }
    let v = get("retry-after")?.trim();
    if let Ok(secs) = v.parse::<u64>() {
        return Some(Duration::from_secs(secs));
    }
    let at = chrono::DateTime::parse_from_rfc2822(v).ok()?;
    let secs = (at.timestamp() - chrono::Utc::now().timestamp()).max(0);
    Some(Duration::from_secs(secs as u64))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn fast_policy(max_attempts: u32) -> RetryPolicy {
        RetryPolicy {
            max_attempts,
            base_delay: Duration::from_millis(5),
            max_delay: Duration::from_millis(500),
        }
    }

    #[test]
    fn backoff_is_exponential_capped_and_jittered() {
        let p = RetryPolicy {
            max_attempts: 5,
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_millis(300),
        };
        for _ in 0..50 {
            let d1 = p.backoff(1);
            assert!(d1 >= Duration::from_millis(50) && d1 <= Duration::from_millis(100));
            let d2 = p.backoff(2);
            assert!(d2 >= Duration::from_millis(100) && d2 <= Duration::from_millis(200));
            let d5 = p.backoff(5);
            assert!(d5 >= Duration::from_millis(150) && d5 <= Duration::from_millis(300));
        }
    }

    #[test]
    fn parses_retry_after_variants() {
        let mut h = reqwest::header::HeaderMap::new();
        h.insert("retry-after", "2".parse().unwrap());
        assert_eq!(parse_retry_after(&h), Some(Duration::from_secs(2)));
        h.insert("retry-after-ms", "150".parse().unwrap());
        assert_eq!(parse_retry_after(&h), Some(Duration::from_millis(150)));
        let mut h = reqwest::header::HeaderMap::new();
        h.insert(
            "retry-after",
            "Wed, 21 Oct 2015 07:28:00 GMT".parse().unwrap(),
        );
        assert_eq!(parse_retry_after(&h), Some(Duration::ZERO));
        h.insert("retry-after", "garbage".parse().unwrap());
        assert_eq!(parse_retry_after(&h), None);
    }

    #[tokio::test]
    async fn retries_429_honoring_retry_after_then_succeeds() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/x"))
            .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "0"))
            .up_to_n_times(1)
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/x"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let url = format!("{}/x", server.uri());
        let r = send_with_retry(&fast_policy(3), "test", || http.get(&url))
            .await
            .unwrap();
        // The second attempt's response is returned; `expect` checks both were sent.
        assert_eq!(r.status(), 200);
    }

    #[tokio::test]
    async fn multibyte_error_bodies_are_logged_without_panicking() {
        // 1 + 2·400 bytes: the 512-byte log cut falls inside an "å".
        let body = format!("x{}", "å".repeat(400));
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(422).set_body_string(body))
            .expect(1)
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let url = format!("{}/x", server.uri());
        let f = send_with_retry(&fast_policy(3), "test", || http.get(&url))
            .await
            .unwrap_err();
        assert!(matches!(f.error, UpstreamError::Status { status: 422, .. }));
        assert!(!f.error.to_string().contains('å'));
    }

    #[tokio::test]
    async fn persistent_5xx_stops_after_max_attempts() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(503))
            .expect(3)
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let url = format!("{}/x", server.uri());
        let f = send_with_retry(&fast_policy(3), "test", || http.get(&url))
            .await
            .unwrap_err();
        assert_eq!(f.attempts, 3);
        assert!(f.exhausted);
    }

    #[tokio::test]
    async fn client_errors_are_not_retried() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(400).set_body_string("secret-ish body"))
            .expect(1)
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let url = format!("{}/x", server.uri());
        let f = send_with_retry(&fast_policy(5), "test", || http.get(&url))
            .await
            .unwrap_err();
        assert_eq!(f.attempts, 1);
        assert!(!f.exhausted);
        assert!(!f.error.to_string().contains("secret"));
    }

    #[tokio::test]
    async fn retry_after_beyond_max_delay_is_not_waited_for() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "3600"))
            .expect(1)
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let url = format!("{}/x", server.uri());
        let f = send_with_retry(&fast_policy(5), "test", || http.get(&url))
            .await
            .unwrap_err();
        assert!(f.exhausted);
        assert_eq!(f.attempts, 1);
    }

    #[tokio::test]
    async fn retries_do_not_sleep_past_deadline() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(500))
            .expect(1)
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let url = format!("{}/x", server.uri());
        let policy = RetryPolicy {
            max_attempts: 5,
            base_delay: Duration::from_secs(5),
            max_delay: Duration::from_secs(5),
        };
        let started = Instant::now();
        let f = with_deadline(
            Instant::now() + Duration::from_millis(500),
            send_with_retry(&policy, "test", || http.get(&url)),
        )
        .await
        .unwrap_err();
        assert!(f.exhausted);
        assert!(started.elapsed() < Duration::from_millis(500));
    }

    #[tokio::test]
    async fn run_stage_times_out() {
        let r: Result<(), RagError> =
            run_stage(Stage::Retrieval, Duration::from_millis(20), async {
                tokio::time::sleep(Duration::from_secs(5)).await;
                Ok(())
            })
            .await;
        assert!(matches!(
            r,
            Err(RagError::Timeout {
                stage: Stage::Retrieval
            })
        ));
    }

    #[tokio::test]
    async fn run_stage_respects_outer_deadline() {
        let started = Instant::now();
        let r: Result<(), RagError> = with_deadline(
            Instant::now() + Duration::from_millis(20),
            run_stage(Stage::Generation, Duration::from_secs(5), async {
                tokio::time::sleep(Duration::from_secs(5)).await;
                Ok(())
            }),
        )
        .await;
        assert!(matches!(r, Err(RagError::Timeout { .. })));
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}
