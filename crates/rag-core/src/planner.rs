use crate::domain::SourceKind;
use crate::error::RagError;
use crate::openai::OpenAiClient;
use anyhow::{Result, anyhow};
use chrono::{DateTime, Duration, NaiveDate, SecondsFormat, TimeZone, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Intent {
    RootCauseWindow,
    IncidentSummary,
    MonitorExplanation,
    SemanticLogSearch,
    MetricQuestion,
    DashboardLookup,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeRange {
    pub from_utc: Option<String>,
    pub to_utc: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryPlan {
    pub intent: Intent,
    pub service: Option<String>,
    pub environment: Option<String>,
    pub monitor_id: Option<String>,
    pub incident_id: Option<String>,
    pub metric: Option<String>,
    pub slo_id: Option<String>,
    pub window: Option<TimeRange>,
    pub filters: Vec<String>,
    pub missing_fields: Vec<String>,
    pub clarifying_questions: Vec<String>,
    pub rewritten_query: Option<String>,
}

/// Source of "now", injectable so planning is deterministic in tests.
pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct FixedClock(pub DateTime<Utc>);

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

/// Parses an IANA timezone name (e.g. `Europe/Stockholm`). Missing or blank means UTC.
pub fn parse_timezone(name: Option<&str>) -> Result<Tz> {
    match name.map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(Tz::UTC),
        Some(n) => n
            .parse::<Tz>()
            .map_err(|_| anyhow!("unknown IANA timezone {n:?}")),
    }
}

/// When the question is asked and which timezone the user lives in.
#[derive(Debug, Clone, Copy)]
pub struct PlanContext {
    pub now: DateTime<Utc>,
    pub tz: Tz,
}

impl PlanContext {
    pub fn new(now: DateTime<Utc>, tz: Tz) -> Self {
        Self { now, tz }
    }
}

/// A validated, half-open `[from, to)` UTC window. Either bound may be open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
}

/// Oldest `from` accepted from the planner, relative to now.
const MAX_LOOKBACK_DAYS: i64 = 5 * 365;
/// Longest window accepted from the planner.
const MAX_SPAN_DAYS: i64 = 366;
/// How far into the future a planner bound may reach (clock skew, "today").
const MAX_FUTURE_DAYS: i64 = 1;
const MAX_FILTERS: usize = 20;

fn planner_system_prompt(ctx: &PlanContext) -> String {
    let local_now = ctx.now.with_timezone(&ctx.tz);
    format!(
        r#"
You are a planning assistant for an SRE RAG over Datadog.
Return strictly valid JSON with:
  intent, service, environment, monitorId, incidentId, metric, sloId,
  window {{ fromUtc, toUtc }} in UTC RFC 3339 (e.g. 2025-01-31T13:00:00Z) if inferred,
  filters[], missingFields[], clarifyingQuestions[], rewrittenQuery.

Context:
- The current time is {local_now} in the user's timezone {tz} ({utc_now} in UTC).
- Interpret relative phrases ("yesterday", "this morning", "last 2 hours") and times
  without an explicit zone in {tz}, then convert to UTC. "Yesterday" means local midnight
  to local midnight in {tz}.

Inference rules:
- Try to INFER `service` and `environment` from the user text and common Datadog tag patterns
  (e.g., "auth-api", "payment", "env:prod", "prod", "staging", "dev").
- If you can infer them confidently, fill `service` and/or `environment`.
- If not confident, add them to `missingFields` and include precise `clarifyingQuestions`.
- If a concrete time is mentioned (e.g., "yesterday 14:00-15:00 CET"),
  convert to UTC and set `window.fromUtc` and `window.toUtc`.
- `filters` may only contain `kind:<logs|metrics|monitor|incident|dashboard|slo|git>`
  entries, and only when the user explicitly restricts the kind of evidence.
- Set `rewrittenQuery` to a crisp, search-friendly paraphrase (include inferred service/env words).
"#,
        local_now = local_now.to_rfc3339_opts(SecondsFormat::Secs, false),
        tz = ctx.tz.name(),
        utc_now = ctx.now.to_rfc3339_opts(SecondsFormat::Secs, true),
    )
}

/// Asks the LLM for a plan, then validates it. The LLM output is untrusted: invalid
/// fields are dropped individually, and relative ranges ("yesterday") are resolved in
/// code against `ctx` rather than taken from the model's arithmetic.
pub async fn plan_query(
    oa: &OpenAiClient,
    user_query: &str,
    ctx: &PlanContext,
) -> Result<QueryPlan, RagError> {
    let raw: Value = oa
        .chat_json(&planner_system_prompt(ctx), user_query)
        .await?;
    Ok(sanitize_plan(&raw, user_query, ctx))
}

/// Builds a plan from untrusted JSON (LLM output or a caller-supplied plan).
/// Never fails: every invalid field is logged and dropped.
pub fn sanitize_plan(raw: &Value, user_query: &str, ctx: &PlanContext) -> QueryPlan {
    let intent = raw
        .get("intent")
        .and_then(|v| serde_json::from_value::<Intent>(v.clone()).ok())
        .unwrap_or_default();

    let service = raw.get("service").and_then(|v| {
        let out = sanitize_tag_value(v, "service");
        if out.is_none() && !v.is_null() {
            tracing::warn!(value = %v, "planner: dropping invalid service");
        }
        out
    });
    let environment = raw.get("environment").and_then(|v| {
        let out = sanitize_tag_value(v, "env");
        if out.is_none() && !v.is_null() {
            tracing::warn!(value = %v, "planner: dropping invalid environment");
        }
        out
    });

    let llm_window = raw
        .get("window")
        .and_then(|w| validate_llm_window(w, ctx.now));
    let relative = resolve_relative_window(user_query, ctx);
    let window = match (relative, llm_window) {
        (Some(rel), Some(llm)) if window_contains(&rel, &llm) => Some(llm),
        (Some(rel), llm) => {
            if llm.is_some() {
                tracing::warn!(
                    ?llm,
                    ?rel,
                    "planner: window disagrees with relative phrase; using deterministic range"
                );
            }
            Some(rel)
        }
        (None, llm) => llm,
    };

    QueryPlan {
        intent,
        service,
        environment,
        monitor_id: clean_string(raw.get("monitorId"), 200),
        incident_id: clean_string(raw.get("incidentId"), 200),
        metric: clean_string(raw.get("metric"), 200),
        slo_id: clean_string(raw.get("sloId"), 200),
        window: window.map(|w| TimeRange {
            from_utc: w.from.map(format_utc),
            to_utc: w.to.map(format_utc),
        }),
        filters: sanitize_filters(raw.get("filters")),
        missing_fields: string_list(raw.get("missingFields"), 20, 100),
        clarifying_questions: string_list(raw.get("clarifyingQuestions"), 10, 500),
        rewritten_query: clean_string(raw.get("rewrittenQuery"), 1000),
    }
}

pub fn format_utc(dt: DateTime<Utc>) -> String {
    dt.to_rfc3339_opts(SecondsFormat::AutoSi, true)
}

/// Parses an RFC 3339 timestamp (any offset) into UTC.
pub fn parse_utc(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s.trim())
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

fn window_contains(outer: &Window, inner: &Window) -> bool {
    let (Some(from), Some(to)) = (inner.from, inner.to) else {
        return false;
    };
    outer.from.is_none_or(|f| f <= from) && outer.to.is_none_or(|t| to <= t)
}

fn validate_llm_window(w: &Value, now: DateTime<Utc>) -> Option<Window> {
    if w.is_null() {
        return None;
    }
    let bound = |key: &str| -> std::result::Result<Option<DateTime<Utc>>, ()> {
        match w.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(s)) if s.trim().is_empty() => Ok(None),
            Some(Value::String(s)) => parse_utc(s).map(Some).ok_or(()),
            Some(_) => Err(()),
        }
    };
    let (Ok(from), Ok(to)) = (bound("fromUtc"), bound("toUtc")) else {
        tracing::warn!(window = %w, "planner: dropping window with unparseable timestamps");
        return None;
    };
    if from.is_none() && to.is_none() {
        return None;
    }
    let oldest = now - Duration::days(MAX_LOOKBACK_DAYS);
    let newest = now + Duration::days(MAX_FUTURE_DAYS);
    let in_bounds = |t: DateTime<Utc>| t >= oldest && t <= newest;
    let reason = match (from, to) {
        (Some(f), Some(t)) if f >= t => Some("from is not before to"),
        (Some(f), Some(t)) if t - f > Duration::days(MAX_SPAN_DAYS) => Some("window too long"),
        (Some(f), _) if !in_bounds(f) => Some("from out of range"),
        (_, Some(t)) if !in_bounds(t) => Some("to out of range"),
        _ => None,
    };
    if let Some(reason) = reason {
        tracing::warn!(window = %w, reason, "planner: dropping invalid window");
        return None;
    }
    Some(Window { from, to })
}

/// Local midnight at the start of `date` in `tz`, as UTC. If midnight does not exist
/// (DST gap at 00:00), the first valid local time after it is used.
pub fn local_midnight(tz: Tz, date: NaiveDate) -> DateTime<Utc> {
    let midnight = date.and_hms_opt(0, 0, 0).expect("midnight is valid");
    (0..=3 * 60)
        .find_map(|m| {
            tz.from_local_datetime(&(midnight + Duration::minutes(m)))
                .earliest()
        })
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|| Utc.from_utc_datetime(&midnight))
}

/// Deterministically resolves common relative phrases in the question:
/// `today`, `yesterday`, `day before yesterday`, `since today|yesterday`, and
/// `last|past [N] minute(s)|hour(s)|day(s)|week(s)` (also `last 24h`).
/// Several matches are unioned. Returns `None` when nothing is recognised.
pub fn resolve_relative_window(question: &str, ctx: &PlanContext) -> Option<Window> {
    let lower = question.to_lowercase();
    let tokens: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect();
    let today = ctx.now.with_timezone(&ctx.tz).date_naive();
    let day = |offset: i64| {
        let from = today - Duration::days(offset);
        Window {
            from: Some(local_midnight(ctx.tz, from)),
            to: Some(local_midnight(ctx.tz, from + Duration::days(1))),
        }
    };

    let mut found: Vec<Window> = Vec::new();
    for (i, tok) in tokens.iter().enumerate() {
        let prev = |n: usize| i.checked_sub(n).map(|j| tokens[j]);
        let mut w = match *tok {
            "today" => day(0),
            "yesterday" if prev(2) == Some("day") && prev(1) == Some("before") => day(2),
            "yesterday" => day(1),
            "last" | "past" => match rolling(&tokens[i + 1..]) {
                Some(d) => Window {
                    from: Some(ctx.now - d),
                    to: Some(ctx.now),
                },
                None => continue,
            },
            _ => continue,
        };
        if prev(1) == Some("since") {
            w.to = None;
        }
        found.push(w);
    }
    found.into_iter().reduce(|a, b| Window {
        from: a.from.min(b.from),
        to: match (a.to, b.to) {
            (Some(x), Some(y)) => Some(x.max(y)),
            _ => None,
        },
    })
}

/// Parses `[N] unit` or `Nh`-style tokens following "last"/"past".
fn rolling(rest: &[&str]) -> Option<Duration> {
    let unit = |u: &str| -> Option<Duration> {
        Some(match u {
            "minute" | "minutes" | "min" | "mins" | "m" => Duration::minutes(1),
            "hour" | "hours" | "hr" | "hrs" | "h" => Duration::hours(1),
            "day" | "days" | "d" => Duration::days(1),
            "week" | "weeks" | "w" => Duration::weeks(1),
            _ => return None,
        })
    };
    let first = *rest.first()?;
    let (n, u) = if let Some(u) = unit(first).filter(|_| first.len() > 1) {
        (1, u)
    } else if let Ok(n) = first.parse::<i64>() {
        (n, unit(rest.get(1)?)?)
    } else {
        let split = first.find(|c: char| !c.is_ascii_digit())?;
        let (num, suffix) = first.split_at(split);
        (num.parse().ok()?, unit(suffix)?)
    };
    let d = u.checked_mul(i32::try_from(n).ok()?)?;
    (n > 0 && d <= Duration::days(MAX_SPAN_DAYS)).then_some(d)
}

const PLACEHOLDERS: &[&str] = &[
    "null",
    "none",
    "unknown",
    "n/a",
    "na",
    "any",
    "all",
    "*",
    "undefined",
    "unspecified",
];

/// Validates a service or environment value from untrusted input: Datadog tag
/// charset, lowercased, an optional `key:` prefix stripped (`env:` or
/// `environment:` for environments), placeholders rejected.
pub fn sanitize_tag_value(v: &Value, key: &str) -> Option<String> {
    let s = v.as_str()?.trim().to_lowercase();
    let keys: &[&str] = match key {
        "env" | "environment" => &["environment", "env"],
        _ => &[key],
    };
    let s = keys
        .iter()
        .find_map(|k| s.strip_prefix(k)?.strip_prefix(':'))
        .map(str::trim)
        .unwrap_or(&s)
        .to_string();
    let valid = !s.is_empty()
        && s.len() <= 100
        && s.starts_with(|c: char| c.is_ascii_alphanumeric())
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':' | '/'))
        && !PLACEHOLDERS.contains(&s.as_str());
    valid.then_some(s)
}

/// Normalizes planner filters to the recognised `kind:`, `service:` and `env:` tags.
/// Anything else is dropped.
pub fn sanitize_filters(v: Option<&Value>) -> Vec<String> {
    let Some(items) = v.and_then(Value::as_array) else {
        return vec![];
    };
    let mut out: Vec<String> = Vec::new();
    for item in items {
        match item.as_str().and_then(normalize_filter) {
            Some(f) if !out.contains(&f) => out.push(f),
            Some(_) => {}
            None => tracing::warn!(filter = %item, "planner: dropping unrecognised filter"),
        }
        if out.len() >= MAX_FILTERS {
            break;
        }
    }
    out
}

/// `kind:logs`, `source:Logs`, `service:x`, `env:prod`, `environment:prod` → canonical form.
pub fn normalize_filter(f: &str) -> Option<String> {
    let (key, value) = f.trim().split_once(':')?;
    let value = Value::String(value.trim().to_string());
    match key.trim().to_lowercase().as_str() {
        "kind" | "source" => {
            SourceKind::parse_lenient(value.as_str()?).map(|k| format!("kind:{}", k.name()))
        }
        "service" => sanitize_tag_value(&value, "service").map(|s| format!("service:{s}")),
        "env" | "environment" => sanitize_tag_value(&value, "env").map(|s| format!("env:{s}")),
        _ => None,
    }
}

fn clean_string(v: Option<&Value>, max_chars: usize) -> Option<String> {
    let s = v?.as_str()?.trim();
    if s.is_empty() || PLACEHOLDERS.contains(&s.to_lowercase().as_str()) {
        return None;
    }
    Some(s.chars().take(max_chars).collect())
}

fn string_list(v: Option<&Value>, max_items: usize, max_chars: usize) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| clean_string(Some(x), max_chars))
                .take(max_items)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> DateTime<Utc> {
        parse_utc(s).unwrap()
    }

    fn stockholm(now: &str) -> PlanContext {
        PlanContext::new(at(now), parse_timezone(Some("Europe/Stockholm")).unwrap())
    }

    fn window(from: &str, to: &str) -> Option<Window> {
        Some(Window {
            from: Some(at(from)),
            to: Some(at(to)),
        })
    }

    #[test]
    fn yesterday_is_local_midnight_to_local_midnight_in_utc() {
        // 10:00 CEST on 2026-09-24.
        let ctx = stockholm("2026-09-24T08:00:00Z");
        assert_eq!(
            resolve_relative_window("why did auth-api fail yesterday?", &ctx),
            window("2026-09-22T22:00:00Z", "2026-09-23T22:00:00Z")
        );
        // 01:30 local on the 24th is still the 23rd in UTC; yesterday is the local 23rd.
        let ctx = stockholm("2026-09-23T23:30:00Z");
        assert_eq!(
            resolve_relative_window("errors yesterday", &ctx),
            window("2026-09-22T22:00:00Z", "2026-09-23T22:00:00Z")
        );
    }

    #[test]
    fn yesterday_across_dst_boundaries_is_25_or_23_hours() {
        // DST ends 2026-10-25 03:00 CEST -> 02:00 CET: yesterday is 25 hours long.
        let w = resolve_relative_window("yesterday", &stockholm("2026-10-26T09:00:00Z"));
        assert_eq!(w, window("2026-10-24T22:00:00Z", "2026-10-25T23:00:00Z"));
        // DST starts 2026-03-29 02:00 CET -> 03:00 CEST: yesterday is 23 hours long.
        let w = resolve_relative_window("yesterday", &stockholm("2026-03-30T09:00:00Z"));
        assert_eq!(w, window("2026-03-28T23:00:00Z", "2026-03-29T22:00:00Z"));
    }

    #[test]
    fn relative_phrases_default_to_utc() {
        let ctx = PlanContext::new(at("2026-09-24T08:00:00Z"), parse_timezone(None).unwrap());
        assert_eq!(
            resolve_relative_window("yesterday", &ctx),
            window("2026-09-23T00:00:00Z", "2026-09-24T00:00:00Z")
        );
    }

    #[test]
    fn other_relative_phrases() {
        let ctx = stockholm("2026-09-24T08:00:00Z");
        let r = |q: &str| resolve_relative_window(q, &ctx);
        assert_eq!(
            r("today"),
            window("2026-09-23T22:00:00Z", "2026-09-24T22:00:00Z")
        );
        assert_eq!(
            r("the day before yesterday"),
            window("2026-09-21T22:00:00Z", "2026-09-22T22:00:00Z")
        );
        assert_eq!(
            r("errors since yesterday"),
            Some(Window {
                from: Some(at("2026-09-22T22:00:00Z")),
                to: None
            })
        );
        assert_eq!(
            r("in the last 2 hours"),
            window("2026-09-24T06:00:00Z", "2026-09-24T08:00:00Z")
        );
        assert_eq!(
            r("past 24h"),
            window("2026-09-23T08:00:00Z", "2026-09-24T08:00:00Z")
        );
        assert_eq!(
            r("last week"),
            window("2026-09-17T08:00:00Z", "2026-09-24T08:00:00Z")
        );
        assert_eq!(
            r("yesterday and today"),
            window("2026-09-22T22:00:00Z", "2026-09-24T22:00:00Z")
        );
        assert_eq!(r("what changed in the last deploy?"), None);
        assert_eq!(r("last 5 errors"), None);
        assert_eq!(r("last 100000 days"), None);
    }

    #[test]
    fn parse_timezone_accepts_iana_names_only() {
        assert_eq!(parse_timezone(None).unwrap(), Tz::UTC);
        assert_eq!(parse_timezone(Some("  ")).unwrap(), Tz::UTC);
        assert_eq!(
            parse_timezone(Some("Europe/Stockholm")).unwrap(),
            Tz::Europe__Stockholm
        );
        assert!(parse_timezone(Some("Mars/Olympus")).is_err());
    }

    #[test]
    fn prompt_carries_local_now_and_timezone() {
        let prompt = planner_system_prompt(&stockholm("2026-09-24T08:00:00Z"));
        assert!(prompt.contains("2026-09-24T10:00:00+02:00"));
        assert!(prompt.contains("Europe/Stockholm"));
        assert!(prompt.contains("2026-09-24T08:00:00Z"));
    }

    #[test]
    fn sanitize_replaces_wrong_llm_arithmetic_for_yesterday() {
        let ctx = stockholm("2026-09-24T08:00:00Z");
        // UTC-day "yesterday" instead of the Stockholm one.
        let raw = serde_json::json!({"window": {
            "fromUtc": "2026-09-23T00:00:00Z", "toUtc": "2026-09-24T00:00:00Z"
        }});
        let plan = sanitize_plan(&raw, "errors yesterday", &ctx);
        let w = plan.window.unwrap();
        assert_eq!(w.from_utc.as_deref(), Some("2026-09-22T22:00:00Z"));
        assert_eq!(w.to_utc.as_deref(), Some("2026-09-23T22:00:00Z"));
    }

    #[test]
    fn sanitize_keeps_a_precise_llm_window_inside_the_relative_range() {
        let ctx = stockholm("2026-09-24T08:00:00Z");
        let raw = serde_json::json!({"window": {
            "fromUtc": "2026-09-23T14:00:00+02:00", "toUtc": "2026-09-23T13:00:00Z"
        }});
        let plan = sanitize_plan(&raw, "yesterday 14:00-15:00", &ctx);
        let w = plan.window.unwrap();
        assert_eq!(w.from_utc.as_deref(), Some("2026-09-23T12:00:00Z"));
        assert_eq!(w.to_utc.as_deref(), Some("2026-09-23T13:00:00Z"));
    }

    #[test]
    fn sanitize_drops_invalid_windows_but_keeps_other_fields() {
        let ctx = stockholm("2026-09-24T08:00:00Z");
        for w in [
            serde_json::json!({"fromUtc": "yesterday", "toUtc": "2026-09-24T00:00:00Z"}),
            serde_json::json!({"fromUtc": "2026-09-24T00:00:00Z", "toUtc": "2026-09-23T00:00:00Z"}),
            serde_json::json!({"fromUtc": "2026-09-24T00:00:00Z", "toUtc": "2026-09-24T00:00:00Z"}),
            serde_json::json!({"fromUtc": "1970-01-01T00:00:00Z", "toUtc": "1970-01-02T00:00:00Z"}),
            serde_json::json!({"fromUtc": "2020-01-01T00:00:00Z", "toUtc": "2026-01-01T00:00:00Z"}),
            serde_json::json!({"fromUtc": "2030-01-01T00:00:00Z"}),
            serde_json::json!({"fromUtc": 12345}),
            serde_json::json!("garbage"),
        ] {
            let raw = serde_json::json!({"service": "auth-api", "window": w});
            let plan = sanitize_plan(&raw, "auth-api errors", &ctx);
            assert!(plan.window.is_none(), "window {w} should be dropped");
            assert_eq!(plan.service.as_deref(), Some("auth-api"));
        }
        let raw = serde_json::json!({"window": {"fromUtc": "2026-09-24T06:00:00Z", "toUtc": null}});
        let plan = sanitize_plan(&raw, "auth-api errors", &ctx);
        let w = plan.window.unwrap();
        assert_eq!(w.from_utc.as_deref(), Some("2026-09-24T06:00:00Z"));
        assert_eq!(w.to_utc, None);
    }

    #[test]
    fn sanitize_validates_entities_intent_and_filters() {
        let ctx = stockholm("2026-09-24T08:00:00Z");
        let raw = serde_json::json!({
            "intent": "somethingNew",
            "service": " Service:Auth-API ",
            "environment": "unknown",
            "metric": "  ",
            "monitorId": 42,
            "filters": ["kind:Logs", "kind:tweets", "env:PROD", "status:error", 7],
            "missingFields": ["environment", ""],
            "rewrittenQuery": "auth-api errors"
        });
        let plan = sanitize_plan(&raw, "auth-api errors", &ctx);
        assert_eq!(plan.intent, Intent::Unknown);
        assert_eq!(plan.service.as_deref(), Some("auth-api"));
        assert_eq!(plan.environment, None);
        assert_eq!(plan.metric, None);
        assert_eq!(plan.monitor_id, None);
        assert_eq!(plan.filters, vec!["kind:logs", "env:prod"]);
        assert_eq!(plan.missing_fields, vec!["environment"]);
        assert_eq!(plan.rewritten_query.as_deref(), Some("auth-api errors"));

        for bad in ["", "prod; drop", "-x", "none", "*"] {
            let plan = sanitize_plan(&serde_json::json!({"service": bad}), "q", &ctx);
            assert_eq!(plan.service, None, "{bad:?} should be rejected");
        }
        for env in ["environment:Prod", "env:prod", "prod"] {
            let plan = sanitize_plan(&serde_json::json!({"environment": env}), "q", &ctx);
            assert_eq!(plan.environment.as_deref(), Some("prod"), "{env}");
        }
        let plan = sanitize_plan(&serde_json::json!({"intent": "rootCauseWindow"}), "q", &ctx);
        assert_eq!(plan.intent, Intent::RootCauseWindow);
    }

    #[test]
    fn sanitize_of_non_object_still_resolves_relative_phrases() {
        let ctx = stockholm("2026-09-24T08:00:00Z");
        let plan = sanitize_plan(&serde_json::json!([1, 2]), "errors yesterday", &ctx);
        assert_eq!(plan.intent, Intent::Unknown);
        assert!(plan.window.is_some());
    }

    #[test]
    fn local_midnight_skips_a_dst_gap_at_midnight() {
        // America/Santiago springs forward at 00:00 -> 01:00 on 2026-09-06.
        let tz: Tz = "America/Santiago".parse().unwrap();
        let date = NaiveDate::from_ymd_opt(2026, 9, 6).unwrap();
        let m = local_midnight(tz, date);
        assert_eq!(m.with_timezone(&tz).date_naive(), date);
        assert_eq!(m, at("2026-09-06T04:00:00Z"));
    }

    #[tokio::test]
    async fn plan_query_sends_context_and_sanitizes_the_response() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let content = serde_json::json!({
            "intent": "rootCauseWindow",
            "service": "auth-api",
            "window": {"fromUtc": "not a date"}
        })
        .to_string();
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{"message": {"content": content}}]
            })))
            .mount(&server)
            .await;
        let oa = OpenAiClient::new("k".into(), server.uri(), "e".into(), "c".into());
        let ctx = stockholm("2026-09-24T08:00:00Z");
        let plan = plan_query(&oa, "why did auth-api fail yesterday", &ctx)
            .await
            .unwrap();
        assert_eq!(plan.service.as_deref(), Some("auth-api"));
        let w = plan.window.unwrap();
        assert_eq!(w.from_utc.as_deref(), Some("2026-09-22T22:00:00Z"));

        let body: serde_json::Value =
            serde_json::from_slice(&server.received_requests().await.unwrap()[0].body).unwrap();
        let system = body["messages"][0]["content"].as_str().unwrap();
        assert!(system.contains("Europe/Stockholm"));
        assert!(system.contains("2026-09-24T10:00:00+02:00"));
    }

    #[test]
    fn test_intent_serialization() {
        let intents = vec![
            Intent::RootCauseWindow,
            Intent::IncidentSummary,
            Intent::MonitorExplanation,
            Intent::SemanticLogSearch,
            Intent::MetricQuestion,
            Intent::DashboardLookup,
            Intent::Unknown,
        ];

        for intent in intents {
            let json = serde_json::to_string(&intent).unwrap();
            let deserialized: Intent = serde_json::from_str(&json).unwrap();
            // Cannot use PartialEq on Intent enum, so just check it deserializes
            let _ = deserialized;
        }
    }

    #[test]
    fn test_intent_camel_case() {
        let json = r#""rootCauseWindow""#;
        let intent: Intent = serde_json::from_str(json).unwrap();
        let serialized = serde_json::to_string(&intent).unwrap();
        assert_eq!(serialized, r#""rootCauseWindow""#);
    }

    #[test]
    fn test_time_range_serialization() {
        let time_range = TimeRange {
            from_utc: Some("2025-01-01T00:00:00Z".to_string()),
            to_utc: Some("2025-01-01T12:00:00Z".to_string()),
        };

        let json = serde_json::to_string(&time_range).unwrap();
        let deserialized: TimeRange = serde_json::from_str(&json).unwrap();

        assert_eq!(
            deserialized.from_utc,
            Some("2025-01-01T00:00:00Z".to_string())
        );
        assert_eq!(
            deserialized.to_utc,
            Some("2025-01-01T12:00:00Z".to_string())
        );
    }

    #[test]
    fn test_time_range_none_values() {
        let time_range = TimeRange {
            from_utc: None,
            to_utc: None,
        };

        let json = serde_json::to_string(&time_range).unwrap();
        let deserialized: TimeRange = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.from_utc, None);
        assert_eq!(deserialized.to_utc, None);
    }

    #[test]
    fn test_query_plan_serialization() {
        let plan = QueryPlan {
            intent: Intent::RootCauseWindow,
            service: Some("auth-api".to_string()),
            environment: Some("production".to_string()),
            monitor_id: None,
            incident_id: Some("incident-123".to_string()),
            metric: None,
            slo_id: None,
            window: Some(TimeRange {
                from_utc: Some("2025-01-01T00:00:00Z".to_string()),
                to_utc: Some("2025-01-01T12:00:00Z".to_string()),
            }),
            filters: vec!["service:auth-api".to_string(), "env:production".to_string()],
            missing_fields: vec![],
            clarifying_questions: vec![],
            rewritten_query: Some("auth-api production root cause analysis".to_string()),
        };

        let json = serde_json::to_string(&plan).unwrap();
        let deserialized: QueryPlan = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.service, Some("auth-api".to_string()));
        assert_eq!(deserialized.environment, Some("production".to_string()));
        assert_eq!(deserialized.filters.len(), 2);
    }

    #[test]
    fn test_query_plan_camel_case_fields() {
        let json = r#"{
            "intent": "metricQuestion",
            "service": "api-service",
            "environment": "staging",
            "monitorId": null,
            "incidentId": null,
            "metric": "cpu.usage",
            "sloId": null,
            "window": null,
            "filters": [],
            "missingFields": ["time_range"],
            "clarifyingQuestions": ["What time range?"],
            "rewrittenQuery": "cpu usage for api-service staging"
        }"#;

        let plan: QueryPlan = serde_json::from_str(json).unwrap();
        assert_eq!(plan.service, Some("api-service".to_string()));
        assert_eq!(plan.metric, Some("cpu.usage".to_string()));
        assert_eq!(plan.missing_fields, vec!["time_range"]);
        assert_eq!(plan.clarifying_questions, vec!["What time range?"]);

        let serialized = serde_json::to_value(&plan).unwrap();
        assert!(serialized.get("monitorId").is_some());
        assert!(serialized.get("incidentId").is_some());
        assert!(serialized.get("sloId").is_some());
        assert!(serialized.get("missingFields").is_some());
        assert!(serialized.get("clarifyingQuestions").is_some());
        assert!(serialized.get("rewrittenQuery").is_some());
    }

    #[test]
    fn test_query_plan_all_optional_fields() {
        let plan = QueryPlan {
            intent: Intent::Unknown,
            service: None,
            environment: None,
            monitor_id: None,
            incident_id: None,
            metric: None,
            slo_id: None,
            window: None,
            filters: vec![],
            missing_fields: vec![],
            clarifying_questions: vec![],
            rewritten_query: None,
        };

        let json = serde_json::to_string(&plan).unwrap();
        let deserialized: QueryPlan = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.service, None);
        assert_eq!(deserialized.environment, None);
        assert_eq!(deserialized.rewritten_query, None);
    }
}
