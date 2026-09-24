//! Live evidence for diagnostic questions.
//!
//! The indexed documents say which monitors, metrics and services exist; they
//! cannot say whether latency actually spiked yesterday. For diagnostic
//! questions with a time window this module queries Datadog for the window
//! (plus an equal-length baseline before it), analyses the data in code
//! ([`analysis`]) and returns a [`Timeline`] of observations, hypotheses that
//! must cite observations, and missing evidence.
//!
//! Interface: [`gate`] decides whether to run and resolves the window,
//! [`discovery::discover`] picks services and metrics from retrieved hits, and
//! [`collect_timeline`] takes services, environment, metrics, a UTC window and
//! the question and returns the timeline. Failures never fail the request:
//! each is logged and recorded as missing evidence.

pub mod analysis;
pub mod discovery;
pub mod timeline;

pub use discovery::{Discovery, DiscoveryCaps, MetricTarget};
pub use timeline::{
    Hypothesis, MissingEvidence, MissingReason, Observation, ObservationKind, ObservationSource,
    SkipReason, TimeSpan, Timeline, TimelineStatus,
};

use crate::datadog::{Datadog, LogEvents, MetricSeries};
use crate::error::{RagError, Stage};
use crate::openai::OpenAiClient;
use crate::planner::{Intent, format_utc};
use crate::resilience::{run_stage, with_deadline};
use analysis::{Direction, Finding};
use chrono::{DateTime, Duration, Utc};
use serde_json::json;
use std::collections::HashSet;
use tokio::time::Instant;

/// `RAG_LIVE_EVIDENCE` and the caps on what one question may query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiveEvidenceConfig {
    pub enabled: bool,
    pub caps: DiscoveryCaps,
    /// Most error/warning log events fetched per service.
    pub max_log_events: usize,
    /// Longest window queried live.
    pub max_window: Duration,
}

impl Default for LiveEvidenceConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            caps: DiscoveryCaps {
                max_services: 3,
                max_metrics: 5,
            },
            max_log_events: 1000,
            max_window: Duration::days(7),
        }
    }
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}

impl LiveEvidenceConfig {
    /// `RAG_LIVE_EVIDENCE` (`on`/`off`, default on), `RAG_LIVE_MAX_SERVICES`
    /// (3), `RAG_LIVE_MAX_METRICS` (5), `RAG_LIVE_MAX_LOG_EVENTS` (1000) and
    /// `RAG_LIVE_MAX_WINDOW_HOURS` (168).
    pub fn from_env() -> Self {
        let d = Self::default();
        let enabled = match std::env::var("RAG_LIVE_EVIDENCE") {
            Ok(v) => !matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "off" | "false" | "0" | "no"
            ),
            Err(_) => d.enabled,
        };
        Self {
            enabled,
            caps: DiscoveryCaps {
                max_services: env_usize("RAG_LIVE_MAX_SERVICES", d.caps.max_services),
                max_metrics: env_usize("RAG_LIVE_MAX_METRICS", d.caps.max_metrics),
            },
            max_log_events: env_usize("RAG_LIVE_MAX_LOG_EVENTS", d.max_log_events).max(1),
            max_window: Duration::hours(env_usize(
                "RAG_LIVE_MAX_WINDOW_HOURS",
                d.max_window.num_hours() as usize,
            ) as i64),
        }
    }
}

/// Whether a question asks *why/whether* something happened, so live data
/// matters. The planner intent decides; the keyword heuristic is only a
/// fallback when the intent is unknown.
pub fn is_diagnostic(intent: &Intent, question: &str) -> bool {
    match intent {
        Intent::RootCauseWindow | Intent::MetricQuestion => true,
        Intent::Unknown => {
            let q = question.to_lowercase();
            [
                "why",
                "root cause",
                "rca",
                "spike",
                "fail",
                "error",
                "latency",
                "outage",
                "degrad",
                "slow",
                "drop",
                "down",
                "5xx",
                "timeout",
            ]
            .iter()
            .any(|k| q.contains(k))
        }
        _ => false,
    }
}

/// Inputs to [`gate`].
#[derive(Debug, Clone, Copy)]
pub struct GateInput<'a> {
    pub config: &'a LiveEvidenceConfig,
    /// Datadog credentials are configured.
    pub configured: bool,
    /// The request's `live_evidence` field.
    pub requested: Option<bool>,
    pub diagnostic: bool,
    /// The resolved retrieval window.
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    pub now: DateTime<Utc>,
}

/// Decides whether live evidence runs and resolves the window: an open `to`
/// (or one in the future) becomes `now`. Returns the skipped timeline otherwise.
pub fn gate(input: GateInput<'_>) -> Result<TimeSpan, Box<Timeline>> {
    let skip = |r, d: &str| Err(Box::new(Timeline::skipped(r, d)));
    if input.requested == Some(false) {
        return skip(
            SkipReason::DisabledByRequest,
            "live evidence was disabled by the request",
        );
    }
    if !input.config.enabled {
        return skip(
            SkipReason::Disabled,
            "live evidence is disabled (RAG_LIVE_EVIDENCE=off)",
        );
    }
    if !input.diagnostic && input.requested != Some(true) {
        return skip(SkipReason::NotDiagnostic, "not a diagnostic question");
    }
    if !input.configured {
        return skip(
            SkipReason::NotConfigured,
            "Datadog credentials (DD_API_KEY, DD_APP_KEY) are not configured for the API",
        );
    }
    let Some(from) = input.from else {
        return skip(
            SkipReason::WindowNotSpecified,
            "no time window was given or inferred, so live metrics and logs were not queried",
        );
    };
    let to = input.to.unwrap_or(input.now).min(input.now);
    if from >= to {
        return skip(
            SkipReason::WindowNotSpecified,
            "the time window has not started yet",
        );
    }
    if to - from > input.config.max_window {
        return skip(
            SkipReason::WindowTooLong,
            &format!(
                "the window is longer than {} hours (RAG_LIVE_MAX_WINDOW_HOURS)",
                input.config.max_window.num_hours()
            ),
        );
    }
    Ok(TimeSpan {
        from_utc: from,
        to_utc: to,
    })
}

/// What to query live for one question.
#[derive(Debug, Clone)]
pub struct LiveEvidenceRequest {
    pub question: String,
    pub services: Vec<String>,
    pub environment: Option<String>,
    pub metrics: Vec<MetricTarget>,
    pub window: TimeSpan,
    /// Missing evidence already known (e.g. from discovery caps).
    pub missing: Vec<MissingEvidence>,
}

/// The Datadog tag filter for a service/environment, e.g. `service:auth-api,env:prod`.
fn tag_scope(service: Option<&str>, env: Option<&str>) -> String {
    let tags: Vec<String> = [("service", service), ("env", env)]
        .into_iter()
        .filter_map(|(k, v)| {
            v.filter(|v| discovery::is_tag_value(v))
                .map(|v| format!("{k}:{v}"))
        })
        .collect();
    if tags.is_empty() {
        "*".into()
    } else {
        tags.join(",")
    }
}

/// `avg:trace.http.request.duration{service:auth-api,env:prod}`.
pub fn metric_query(target: &MetricTarget, env: Option<&str>) -> String {
    format!(
        "{}:{}{{{}}}",
        target.aggregation,
        target.metric,
        tag_scope(target.service.as_deref(), env)
    )
}

/// `service:auth-api env:prod status:(error OR warn)`.
pub fn log_query(service: &str, env: Option<&str>) -> String {
    let mut q = format!("service:{service}");
    if let Some(env) = env.filter(|e| discovery::is_tag_value(e)) {
        q.push_str(&format!(" env:{env}"));
    }
    q.push_str(" status:(error OR warn)");
    q
}

fn metric_link(
    site: &str,
    target: &MetricTarget,
    env: Option<&str>,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> String {
    format!(
        "https://app.{site}/metric/explorer?exp_metric={}&exp_scope={}&exp_agg={}&from_ts={}&to_ts={}&live=false",
        urlencoding::encode(&target.metric),
        urlencoding::encode(&tag_scope(target.service.as_deref(), env)),
        target.aggregation,
        from.timestamp_millis(),
        to.timestamp_millis()
    )
}

fn logs_link(site: &str, query: &str, from: DateTime<Utc>, to: DateTime<Utc>) -> String {
    format!(
        "https://app.{site}/logs?query={}&from_ts={}&to_ts={}&live=false",
        urlencoding::encode(query),
        from.timestamp_millis(),
        to.timestamp_millis()
    )
}

fn ms_utc(ms: i64) -> String {
    DateTime::<Utc>::from_timestamp_millis(ms)
        .map(format_utc)
        .unwrap_or_default()
}

/// Rounds for display; the full-precision values stay in `values`.
fn r(v: f64) -> f64 {
    if v.abs() >= 100.0 {
        (v * 10.0).round() / 10.0
    } else {
        (v * 1000.0).round() / 1000.0
    }
}

fn stats_json(s: &analysis::Stats) -> serde_json::Value {
    json!({
        "count": s.count, "min": s.min, "max": s.max, "mean": s.mean,
        "stddev": s.stddev, "p5": s.p5, "p50": s.p50, "p95": s.p95,
    })
}

/// Unnumbered observation; IDs are assigned once all are collected.
#[allow(clippy::too_many_arguments)]
fn obs(
    kind: ObservationKind,
    source: ObservationSource,
    service: Option<&str>,
    query: &str,
    start_utc: String,
    end_utc: String,
    summary: String,
    values: serde_json::Value,
    link: &str,
) -> Observation {
    Observation {
        id: String::new(),
        kind,
        source,
        service: service.map(str::to_string),
        query: query.to_string(),
        start_utc,
        end_utc,
        summary,
        values,
        link: link.to_string(),
    }
}

fn failure_reason(e: &RagError) -> MissingReason {
    match e {
        RagError::Timeout { .. } => MissingReason::TimedOut,
        _ => MissingReason::QueryFailed,
    }
}

/// Turns one metric query result into observations or missing evidence.
pub fn metric_observations(
    target: &MetricTarget,
    query: &str,
    link: &str,
    series: &[MetricSeries],
    window: &TimeSpan,
    out: &mut Vec<Observation>,
    missing: &mut Vec<MissingEvidence>,
) {
    let (from_ms, to_ms) = (
        window.from_utc.timestamp_millis(),
        window.to_utc.timestamp_millis(),
    );
    let svc = target.service.as_deref();
    let mut any_data = false;
    for s in series {
        let interval_ms = s.interval_secs.map(|i| i as i64 * 1000);
        let a = analysis::analyse_series(&s.points, from_ms, to_ms, interval_ms);
        let label = if s.expression.is_empty() {
            query
        } else {
            &s.expression
        };
        let Some(w) = a.window else {
            continue;
        };
        any_data = true;
        let values = json!({
            "window": stats_json(&w),
            "baseline": a.baseline.as_ref().map(stats_json),
            "intervalSeconds": s.interval_secs,
            "scope": s.scope,
        });
        let summary = match &a.baseline {
            Some(b) => format!(
                "{label}: window mean {}, max {} at {}; baseline mean {}, p95 {}",
                r(w.mean),
                r(w.max),
                a.window_max_ms.map(ms_utc).unwrap_or_default(),
                r(b.mean),
                r(b.p95)
            ),
            None => format!(
                "{label}: window mean {}, max {} at {} (no baseline to compare)",
                r(w.mean),
                r(w.max),
                a.window_max_ms.map(ms_utc).unwrap_or_default()
            ),
        };
        out.push(obs(
            ObservationKind::SeriesSummary,
            ObservationSource::MetricQuery,
            svc,
            label,
            format_utc(window.from_utc),
            format_utc(window.to_utc),
            summary,
            values,
            link,
        ));
        if a.baseline.is_none() {
            missing.push(MissingEvidence::new(
                label,
                MissingReason::NoBaseline,
                format!(
                    "fewer than {} data points before the window; spikes and drops were not assessed",
                    analysis::MIN_BASELINE_POINTS
                ),
            ));
        }
        for f in &a.findings {
            match f {
                Finding::Excursion {
                    direction,
                    start_ms,
                    end_ms,
                    points,
                    peak,
                    peak_ms,
                    threshold,
                } => {
                    let (kind, word) = match direction {
                        Direction::Above => (ObservationKind::Spike, "above"),
                        Direction::Below => (ObservationKind::Drop, "below"),
                    };
                    let b = a.baseline.as_ref().expect("excursions need a baseline");
                    let reference = if *direction == Direction::Above {
                        b.p95
                    } else {
                        b.p5
                    };
                    let ratio = (reference.abs() > f64::EPSILON).then(|| peak / reference);
                    out.push(obs(
                        kind,
                        ObservationSource::MetricQuery,
                        svc,
                        label,
                        ms_utc(*start_ms),
                        ms_utc(*end_ms),
                        format!(
                            "{label}: {points} point(s) {word} the baseline band ({}); extreme {} at {} vs baseline {} {}",
                            r(*threshold),
                            r(*peak),
                            ms_utc(*peak_ms),
                            if *direction == Direction::Above { "p95" } else { "p5" },
                            r(reference)
                        ),
                        json!({
                            "points": points, "peak": peak, "peakUtc": ms_utc(*peak_ms),
                            "threshold": threshold, "baselineReference": reference,
                            "ratioToBaseline": ratio,
                        }),
                        link,
                    ));
                }
                Finding::Gap {
                    start_ms,
                    end_ms,
                    missing_points,
                } => out.push(obs(
                    ObservationKind::Gap,
                    ObservationSource::MetricQuery,
                    svc,
                    label,
                    ms_utc(*start_ms),
                    ms_utc(*end_ms),
                    format!("{label}: no data for {missing_points} consecutive interval(s)"),
                    json!({ "missingPoints": missing_points }),
                    link,
                )),
            }
        }
    }
    if !any_data {
        missing.push(MissingEvidence::new(
            query,
            MissingReason::SeriesEmpty,
            "the query returned no data points in the window (the metric may not carry these tags)",
        ));
    }
}

/// Turns one log query result into observations (and a cap note).
#[allow(clippy::too_many_arguments)]
pub fn log_observations(
    service: &str,
    query: &str,
    link: &str,
    logs: &LogEvents,
    window: &TimeSpan,
    max_events: usize,
    out: &mut Vec<Observation>,
    missing: &mut Vec<MissingEvidence>,
) {
    let events: Vec<(DateTime<Utc>, String, String)> = logs
        .events
        .iter()
        .filter_map(|d| {
            let ts = d.timestamp.as_deref().and_then(crate::planner::parse_utc)?;
            let status = d
                .metadata
                .get("status")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string();
            Some((ts, status, d.text.clone()))
        })
        .collect();
    let a = analysis::analyse_logs(&events, window.from_utc, window.to_utc);
    let at_least = if logs.truncated { "at least " } else { "" };
    if logs.truncated {
        missing.push(MissingEvidence::new(
            query,
            MissingReason::Capped,
            format!(
                "only the first {max_events} matching logs were fetched (RAG_LIVE_MAX_LOG_EVENTS); counts are lower bounds and later bursts may be missed"
            ),
        ));
    }
    let by_status: serde_json::Map<String, serde_json::Value> = a
        .by_status
        .iter()
        .map(|(s, c)| (s.clone(), json!(c)))
        .collect();
    let top: Vec<_> = a
        .top_messages
        .iter()
        .map(|(m, c)| json!({"message": m, "count": c}))
        .collect();
    let summary = if a.total == 0 {
        format!("{query}: no error/warn logs in the window")
    } else {
        format!(
            "{query}: {at_least}{} error/warn logs; first {}, last {}; peak {} per {} min at {}",
            a.total,
            a.first.map(format_utc).unwrap_or_default(),
            a.last.map(format_utc).unwrap_or_default(),
            a.peak_bucket.map(|p| p.1).unwrap_or(0),
            a.bucket.num_minutes(),
            a.peak_bucket.map(|p| format_utc(p.0)).unwrap_or_default(),
        )
    };
    out.push(obs(
        ObservationKind::LogSummary,
        ObservationSource::LogQuery,
        Some(service),
        query,
        a.first
            .map(format_utc)
            .unwrap_or_else(|| format_utc(window.from_utc)),
        a.last
            .map(format_utc)
            .unwrap_or_else(|| format_utc(window.to_utc)),
        summary,
        json!({
            "total": a.total, "truncated": logs.truncated, "byStatus": by_status,
            "firstUtc": a.first.map(format_utc), "lastUtc": a.last.map(format_utc),
            "bucketMinutes": a.bucket.num_minutes(), "topMessages": top,
        }),
        link,
    ));
    for b in &a.bursts {
        out.push(obs(
            ObservationKind::LogBurst,
            ObservationSource::LogQuery,
            Some(service),
            query,
            format_utc(b.start),
            format_utc(b.end),
            format!(
                "{query}: burst of {}{} error/warn logs between {} and {} (burst threshold {} per {} min)",
                at_least,
                b.count,
                format_utc(b.start),
                format_utc(b.end),
                b.threshold,
                a.bucket.num_minutes()
            ),
            json!({"count": b.count, "thresholdPerBucket": b.threshold, "bucketMinutes": a.bucket.num_minutes()}),
            link,
        ));
    }
}

enum Fetched {
    Metric(usize, Result<Vec<MetricSeries>, RagError>),
    Logs(usize, Result<LogEvents, RagError>),
}

/// Queries Datadog for `req` and builds the timeline. Queries run concurrently;
/// each is bounded by `timeout` and they share one deadline `timeout` from now
/// (and any enclosing request deadline), so partial results survive a slow
/// query. Hypotheses are requested from `oa` only when there are observations.
pub async fn collect_timeline(
    dd: &Datadog,
    oa: Option<&OpenAiClient>,
    req: LiveEvidenceRequest,
    config: &LiveEvidenceConfig,
    timeout: std::time::Duration,
) -> Timeline {
    let window = req.window;
    let span = window.to_utc - window.from_utc;
    let baseline = TimeSpan {
        from_utc: window.from_utc - span,
        to_utc: window.from_utc,
    };
    let env = req.environment.as_deref();
    let metric_queries: Vec<String> = req.metrics.iter().map(|m| metric_query(m, env)).collect();
    let log_queries: Vec<String> = req.services.iter().map(|s| log_query(s, env)).collect();

    let deadline = Instant::now() + timeout;
    let mut futs: Vec<std::pin::Pin<Box<dyn std::future::Future<Output = Fetched> + Send + '_>>> =
        Vec::new();
    for (i, q) in metric_queries.iter().enumerate() {
        // One query covers baseline + window.
        futs.push(Box::pin(async move {
            let r = run_stage(
                Stage::LiveEvidence,
                timeout,
                dd.query_metrics(q, baseline.from_utc, window.to_utc),
            )
            .await;
            Fetched::Metric(i, r)
        }));
    }
    for (i, q) in log_queries.iter().enumerate() {
        let max = config.max_log_events;
        futs.push(Box::pin(async move {
            let r = run_stage(
                Stage::LiveEvidence,
                timeout,
                dd.search_log_events(q, window.from_utc, window.to_utc, max),
            )
            .await;
            Fetched::Logs(i, r)
        }));
    }
    let results = with_deadline(deadline, futures::future::join_all(futs)).await;

    let mut observations = Vec::new();
    let mut missing = req.missing;
    for fetched in results {
        match fetched {
            Fetched::Metric(i, Ok(series)) => {
                let t = &req.metrics[i];
                let link = metric_link(&dd.site, t, env, baseline.from_utc, window.to_utc);
                metric_observations(
                    t,
                    &metric_queries[i],
                    &link,
                    &series,
                    &window,
                    &mut observations,
                    &mut missing,
                );
            }
            Fetched::Logs(i, Ok(logs)) => {
                let link = logs_link(&dd.site, &log_queries[i], window.from_utc, window.to_utc);
                log_observations(
                    &req.services[i],
                    &log_queries[i],
                    &link,
                    &logs,
                    &window,
                    config.max_log_events,
                    &mut observations,
                    &mut missing,
                );
            }
            Fetched::Metric(i, Err(e)) => {
                tracing::warn!(query = %metric_queries[i], error = %e, "live evidence: metric query failed");
                missing.push(MissingEvidence::new(
                    &metric_queries[i],
                    failure_reason(&e),
                    e.to_string(),
                ));
            }
            Fetched::Logs(i, Err(e)) => {
                tracing::warn!(query = %log_queries[i], error = %e, "live evidence: log query failed");
                missing.push(MissingEvidence::new(
                    &log_queries[i],
                    failure_reason(&e),
                    e.to_string(),
                ));
            }
        }
    }

    observations.sort_by(|a, b| {
        a.start_utc
            .cmp(&b.start_utc)
            .then(a.end_utc.cmp(&b.end_utc))
    });
    for (i, o) in observations.iter_mut().enumerate() {
        o.id = format!("obs-{}", i + 1);
    }

    let mut hypotheses = Vec::new();
    if let Some(oa) = oa.filter(|_| !observations.is_empty()) {
        let known: HashSet<&str> = observations.iter().map(|o| o.id.as_str()).collect();
        let prompt = hypothesis_prompt(&req.question, &observations);
        let r = with_deadline(
            deadline,
            run_stage(
                Stage::LiveEvidence,
                timeout,
                oa.chat_json::<serde_json::Value>(HYPOTHESIS_SYSTEM, &prompt),
            ),
        )
        .await;
        match r {
            Ok(raw) => hypotheses = timeline::validate_hypotheses(&raw, &known),
            Err(e) => {
                tracing::warn!(error = %e, "live evidence: hypothesis generation failed");
                missing.push(MissingEvidence::new(
                    "hypotheses",
                    MissingReason::HypothesesFailed,
                    e.to_string(),
                ));
            }
        }
    }

    Timeline {
        status: TimelineStatus::Collected,
        skip_reason: None,
        window: Some(window),
        baseline: Some(baseline),
        observations,
        hypotheses,
        missing_evidence: missing,
    }
}

const HYPOTHESIS_SYSTEM: &str = "You are an SRE assistant. Propose candidate explanations for the question \
using ONLY the numbered observations measured from Datadog. Return strictly valid JSON: \
{\"hypotheses\": [{\"statement\": \"...\", \"observationIds\": [\"obs-1\"]}]}. Every hypothesis must cite \
at least one observation ID from the list. Return at most 5, most likely first. Return an empty list if \
the observations do not suggest an explanation.";

fn hypothesis_prompt(question: &str, observations: &[Observation]) -> String {
    let mut s = format!("Question: {question}\n\nObservations:\n");
    for o in observations {
        s.push_str(&format!(
            "- {} ({} to {}): {}\n",
            o.id, o.start_utc, o.end_utc, o.summary
        ));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resilience::RetryPolicy;
    use serde_json::Value;
    use wiremock::matchers::{body_partial_json, header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn utc(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    fn fixture(name: &str) -> Value {
        let path = format!(
            "{}/tests/fixtures/datadog/{}",
            env!("CARGO_MANIFEST_DIR"),
            name
        );
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    fn client(server: &MockServer) -> Datadog {
        let mut dd = Datadog::new("api".into(), "app".into(), "datadoghq.eu".into());
        dd.api_base = server.uri();
        dd.retry = RetryPolicy {
            max_attempts: 2,
            base_delay: std::time::Duration::from_millis(1),
            max_delay: std::time::Duration::from_millis(10),
        };
        dd
    }

    /// The recorded `/api/v1/query` cassette covers 2022-01-05T00:50:52Z to
    /// 2022-01-06T00:50:52Z; its second half is the question's window and the
    /// first half the baseline.
    fn cassette_window() -> TimeSpan {
        TimeSpan {
            from_utc: utc("2022-01-05T12:50:52Z"),
            to_utc: utc("2022-01-06T00:50:52Z"),
        }
    }

    fn cpu_idle() -> MetricTarget {
        MetricTarget {
            metric: "system.cpu.idle".into(),
            aggregation: "avg".into(),
            service: None,
        }
    }

    fn request(
        metrics: Vec<MetricTarget>,
        services: Vec<&str>,
        window: TimeSpan,
    ) -> LiveEvidenceRequest {
        LiveEvidenceRequest {
            question: "why did auth-api fail?".into(),
            services: services.into_iter().map(str::to_string).collect(),
            environment: Some("prod".into()),
            metrics,
            window,
            missing: vec![],
        }
    }

    fn config() -> LiveEvidenceConfig {
        LiveEvidenceConfig {
            max_log_events: 50,
            ..LiveEvidenceConfig::default()
        }
    }

    const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

    #[tokio::test]
    async fn sends_documented_metric_and_log_requests_for_window_and_service() {
        let server = MockServer::start().await;
        // Window 2026-09-23T10:00Z..12:00Z; the baseline starts two hours earlier.
        Mock::given(method("GET"))
            .and(path("/api/v1/query"))
            .and(header("DD-API-KEY", "api"))
            .and(header("DD-APPLICATION-KEY", "app"))
            .and(query_param("from", "1790150400"))
            .and(query_param("to", "1790164800"))
            .and(query_param(
                "query",
                "sum:trace.http.request.errors{service:auth-api,env:prod}",
            ))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"status": "ok", "series": []})),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/v2/logs/events/search"))
            .and(header("DD-API-KEY", "api"))
            .and(header("DD-APPLICATION-KEY", "app"))
            .and(header("content-type", "application/json"))
            .and(body_partial_json(json!({
                "filter": {
                    "from": "2026-09-23T10:00:00Z",
                    "to": "2026-09-23T12:00:00Z",
                    "query": "service:auth-api env:prod status:(error OR warn)"
                },
                "page": {"limit": 50},
                "sort": "timestamp"
            })))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"data": [], "meta": {"page": {}}})),
            )
            .expect(1)
            .mount(&server)
            .await;

        let window = TimeSpan {
            from_utc: utc("2026-09-23T10:00:00Z"),
            to_utc: utc("2026-09-23T12:00:00Z"),
        };
        let target = MetricTarget {
            metric: "trace.http.request.errors".into(),
            aggregation: "sum".into(),
            service: Some("auth-api".into()),
        };
        let t = collect_timeline(
            &client(&server),
            None,
            request(vec![target], vec!["auth-api"], window),
            &config(),
            TIMEOUT,
        )
        .await;

        assert_eq!(t.status, TimelineStatus::Collected);
        assert_eq!(t.baseline.unwrap().from_utc, utc("2026-09-23T08:00:00Z"));
        // An empty series is missing evidence; zero logs is a measured fact.
        assert_eq!(t.missing_evidence.len(), 1);
        assert_eq!(t.missing_evidence[0].reason, MissingReason::SeriesEmpty);
        assert_eq!(t.observations.len(), 1);
        let o = &t.observations[0];
        assert_eq!(
            (o.id.as_str(), o.kind),
            ("obs-1", ObservationKind::LogSummary)
        );
        assert_eq!(o.values["total"], 0);
        assert_eq!(
            o.link,
            "https://app.datadoghq.eu/logs?query=service%3Aauth-api%20env%3Aprod%20status%3A%28error%20OR%20warn%29&from_ts=1790157600000&to_ts=1790164800000&live=false"
        );
    }

    async fn run_with_series(body: Value) -> Timeline {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/query"))
            .and(query_param("from", "1641343852"))
            .and(query_param("to", "1641430252"))
            .and(query_param("query", "avg:system.cpu.idle{env:prod}"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .expect(1)
            .mount(&server)
            .await;
        collect_timeline(
            &client(&server),
            None,
            request(vec![cpu_idle()], vec![], cassette_window()),
            &config(),
            TIMEOUT,
        )
        .await
    }

    #[tokio::test]
    async fn recorded_flat_series_yields_only_a_summary() {
        let t = run_with_series(fixture("metrics_query.json")).await;
        assert!(t.missing_evidence.is_empty(), "{:?}", t.missing_evidence);
        assert_eq!(t.observations.len(), 1);
        let o = &t.observations[0];
        assert_eq!(o.kind, ObservationKind::SeriesSummary);
        assert_eq!(o.source, ObservationSource::MetricQuery);
        assert_eq!(o.values["window"]["count"], 144);
        assert_eq!(o.values["baseline"]["count"], 144);
        assert!(o.link.starts_with(
            "https://app.datadoghq.eu/metric/explorer?exp_metric=system.cpu.idle&exp_scope=env%3Aprod&exp_agg=avg&from_ts=1641343852000&to_ts=1641430252000"
        ));
    }

    #[tokio::test]
    async fn drop_against_baseline_is_detected_on_recorded_series() {
        let mut body = fixture("metrics_query.json");
        let points = body["series"][0]["pointlist"].as_array_mut().unwrap();
        // Two consecutive points inside the window (after 12:50:52Z) collapse.
        let t1 = points[200][0].as_f64().unwrap() as i64;
        let t2 = points[201][0].as_f64().unwrap() as i64;
        points[200][1] = json!(20.0);
        points[201][1] = json!(15.0);
        let t = run_with_series(body).await;

        let kinds: Vec<_> = t.observations.iter().map(|o| o.kind).collect();
        assert_eq!(
            kinds,
            vec![ObservationKind::SeriesSummary, ObservationKind::Drop]
        );
        let drop = &t.observations[1];
        assert_eq!(drop.id, "obs-2");
        assert_eq!(drop.start_utc, ms_utc(t1));
        assert_eq!(drop.end_utc, ms_utc(t2));
        assert_eq!(drop.values["points"], 2);
        assert_eq!(drop.values["peak"], 15.0);
        assert!(drop.values["baselineReference"].as_f64().unwrap() > 90.0);
    }

    #[tokio::test]
    async fn spike_against_baseline_is_detected_on_recorded_series() {
        let mut body = fixture("metrics_query.json");
        let points = body["series"][0]["pointlist"].as_array_mut().unwrap();
        for p in &mut points[250..254] {
            p[1] = json!(400.0);
        }
        let t = run_with_series(body).await;
        let spike = t
            .observations
            .iter()
            .find(|o| o.kind == ObservationKind::Spike)
            .unwrap();
        assert_eq!(spike.values["points"], 4);
        assert!(spike.values["ratioToBaseline"].as_f64().unwrap() > 4.0);
    }

    #[tokio::test]
    async fn empty_or_all_null_series_is_missing_evidence() {
        let t = run_with_series(json!({"status": "ok", "series": []})).await;
        assert!(t.observations.is_empty());
        assert_eq!(t.missing_evidence[0].reason, MissingReason::SeriesEmpty);
        assert_eq!(
            t.missing_evidence[0].subject,
            "avg:system.cpu.idle{env:prod}"
        );

        let mut body = fixture("metrics_query.json");
        for p in body["series"][0]["pointlist"].as_array_mut().unwrap() {
            p[1] = Value::Null;
        }
        let t = run_with_series(body).await;
        assert!(t.observations.is_empty());
        assert_eq!(t.missing_evidence[0].reason, MissingReason::SeriesEmpty);
    }

    #[tokio::test]
    async fn gap_inside_recorded_series_is_an_observation() {
        let mut body = fixture("metrics_query.json");
        let points = body["series"][0]["pointlist"].as_array_mut().unwrap();
        for p in &mut points[220..226] {
            p[1] = Value::Null;
        }
        let t = run_with_series(body).await;
        let gap = t
            .observations
            .iter()
            .find(|o| o.kind == ObservationKind::Gap)
            .unwrap();
        assert_eq!(gap.values["missingPoints"], 6);
    }

    #[tokio::test]
    async fn datadog_5xx_and_timeouts_become_missing_evidence() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/query"))
            .respond_with(ResponseTemplate::new(500))
            .expect(2)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/v2/logs/events/search"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"data": []}))
                    .set_delay(std::time::Duration::from_secs(3)),
            )
            .mount(&server)
            .await;
        let t = collect_timeline(
            &client(&server),
            None,
            request(vec![cpu_idle()], vec!["auth-api"], cassette_window()),
            &config(),
            std::time::Duration::from_millis(300),
        )
        .await;
        assert!(t.observations.is_empty());
        let reasons: Vec<_> = t
            .missing_evidence
            .iter()
            .map(|m| (m.reason, m.subject.as_str()))
            .collect();
        assert_eq!(
            reasons,
            vec![
                (MissingReason::QueryFailed, "avg:system.cpu.idle{env:prod}"),
                (
                    MissingReason::TimedOut,
                    "service:auth-api env:prod status:(error OR warn)"
                ),
            ]
        );
        assert!(t.missing_evidence[0].detail.contains("HTTP 500"));
    }

    #[tokio::test]
    async fn hypotheses_citing_unknown_observations_are_dropped() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/query"))
            .respond_with(ResponseTemplate::new(200).set_body_json(fixture("metrics_query.json")))
            .mount(&server)
            .await;
        let openai = MockServer::start().await;
        let content = json!({"hypotheses": [
            {"statement": "CPU was not the bottleneck", "observationIds": ["obs-1"]},
            {"statement": "A bad deploy at 14:00", "observationIds": ["obs-7"]},
            {"statement": "Uncited guess", "observationIds": []}
        ]});
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(body_partial_json(
                json!({"response_format": {"type": "json_object"}}),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{"message": {"content": content.to_string()}}]
            })))
            .expect(1)
            .mount(&openai)
            .await;
        let oa = OpenAiClient::new("k".into(), openai.uri(), "e".into(), "c".into());
        let t = collect_timeline(
            &client(&server),
            Some(&oa),
            request(vec![cpu_idle()], vec![], cassette_window()),
            &config(),
            TIMEOUT,
        )
        .await;
        assert_eq!(
            t.hypotheses,
            vec![Hypothesis {
                statement: "CPU was not the bottleneck".into(),
                observation_ids: vec!["obs-1".into()],
            }]
        );
        let reqs = openai.received_requests().await.unwrap();
        let prompt: Value = serde_json::from_slice(&reqs[0].body).unwrap();
        assert!(
            prompt["messages"][1]["content"]
                .as_str()
                .unwrap()
                .contains("- obs-1 (")
        );

        let rendered = t.prompt_context().unwrap();
        assert!(rendered.contains("[obs-1]"));
        assert!(rendered.contains("CPU was not the bottleneck [obs-1]"));
        assert!(!rendered.contains("bad deploy"));
    }

    fn gate_input(config: &LiveEvidenceConfig) -> GateInput<'_> {
        GateInput {
            config,
            configured: true,
            requested: None,
            diagnostic: true,
            from: Some(utc("2026-09-22T22:00:00Z")),
            to: Some(utc("2026-09-23T22:00:00Z")),
            now: utc("2026-09-24T08:00:00Z"),
        }
    }

    #[test]
    fn gate_runs_only_for_enabled_diagnostic_questions_with_a_window() {
        let cfg = LiveEvidenceConfig::default();
        let ok = gate(gate_input(&cfg)).unwrap();
        assert_eq!(ok.to_utc, utc("2026-09-23T22:00:00Z"));

        let reason = |input: GateInput<'_>| gate(input).unwrap_err().skip_reason.unwrap();
        let base = gate_input(&cfg);
        assert_eq!(
            reason(GateInput { from: None, ..base }),
            SkipReason::WindowNotSpecified
        );
        assert_eq!(
            reason(GateInput {
                requested: Some(false),
                ..base
            }),
            SkipReason::DisabledByRequest
        );
        assert_eq!(
            reason(GateInput {
                configured: false,
                ..base
            }),
            SkipReason::NotConfigured
        );
        assert_eq!(
            reason(GateInput {
                diagnostic: false,
                ..base
            }),
            SkipReason::NotDiagnostic
        );
        assert!(
            gate(GateInput {
                diagnostic: false,
                requested: Some(true),
                ..base
            })
            .is_ok()
        );
        assert_eq!(
            reason(GateInput {
                from: Some(utc("2026-09-01T00:00:00Z")),
                ..base
            }),
            SkipReason::WindowTooLong
        );
        let off = LiveEvidenceConfig {
            enabled: false,
            ..cfg
        };
        assert_eq!(reason(gate_input(&off)), SkipReason::Disabled);

        // An open end is clamped to now.
        let w = gate(GateInput { to: None, ..base }).unwrap();
        assert_eq!(w.to_utc, utc("2026-09-24T08:00:00Z"));
    }

    #[test]
    fn intent_decides_and_keywords_are_only_a_fallback() {
        assert!(is_diagnostic(&Intent::RootCauseWindow, "summarize"));
        assert!(is_diagnostic(&Intent::MetricQuestion, "p95 of checkout"));
        assert!(!is_diagnostic(
            &Intent::DashboardLookup,
            "why is the dashboard slow"
        ));
        assert!(is_diagnostic(&Intent::Unknown, "did latency spike?"));
        assert!(!is_diagnostic(&Intent::Unknown, "list auth-api monitors"));
    }

    #[test]
    fn queries_scope_by_service_and_env() {
        let t = MetricTarget {
            metric: "trace.http.request.duration".into(),
            aggregation: "avg".into(),
            service: Some("auth-api".into()),
        };
        assert_eq!(
            metric_query(&t, Some("prod")),
            "avg:trace.http.request.duration{service:auth-api,env:prod}"
        );
        assert_eq!(metric_query(&cpu_idle(), None), "avg:system.cpu.idle{*}");
        assert_eq!(
            log_query("auth-api", None),
            "service:auth-api status:(error OR warn)"
        );
    }
}
