//! Turns a validated query plan plus explicit caller input into a Qdrant filter.
//!
//! Precedence is per field: an explicit caller value always wins over anything
//! inferred by the planner. For service/environment the order is explicit field,
//! explicit `service:`/`env:` filter tag, planner field, planner filter tag. The time
//! window is taken as a whole from the caller if either bound was given, otherwise
//! from the plan. Source kinds come from explicit `kinds`, else explicit `kind:`
//! filter tags, else planner `kind:` filter tags.

use crate::domain::SourceKind;
use crate::planner::{QueryPlan, Window, format_utc, normalize_filter, parse_utc};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};

/// Payload key of the first log of a log pattern day (RFC 3339).
const FIRST_SEEN_KEY: &str = "Metadata.first_seen";

/// Kinds that describe configuration or state rather than events. Their `Timestamp`,
/// when set at all, is a creation date (monitors, dashboards, SLOs) or the time the
/// indexer last saw the metric active (metric catalog entries), so a time window never
/// excludes them.
pub const TIMELESS_KINDS: [SourceKind; 4] = [
    SourceKind::Metrics,
    SourceKind::Monitor,
    SourceKind::Dashboard,
    SourceKind::SLO,
];

/// Values the caller set explicitly on the request. These are already validated.
#[derive(Debug, Clone, Default)]
pub struct ExplicitScope {
    pub service: Option<String>,
    pub environment: Option<String>,
    pub window: Option<Window>,
    pub kinds: Option<Vec<SourceKind>>,
    /// Normalized filter tags (see [`normalize_filter`]).
    pub filters: Vec<String>,
}

/// The effective retrieval constraints for one question.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RetrievalScope {
    pub service: Option<String>,
    pub environment: Option<String>,
    pub from_utc: Option<DateTime<Utc>>,
    pub to_utc: Option<DateTime<Utc>>,
    pub kinds: Vec<SourceKind>,
}

fn filter_tag(filters: &[String], key: &str) -> Option<String> {
    filters
        .iter()
        .find_map(|f| f.strip_prefix(key)?.strip_prefix(':').map(str::to_string))
}

fn filter_kinds(filters: &[String]) -> Vec<SourceKind> {
    filters
        .iter()
        .filter_map(|f| SourceKind::parse_lenient(f.strip_prefix("kind:")?))
        .collect()
}

/// Normalizes caller-supplied filter tags, silently ignoring unrecognised ones
/// (older clients forwarded arbitrary planner tags that were never applied).
pub fn normalize_filters(filters: &[String]) -> Vec<String> {
    filters
        .iter()
        .filter_map(|f| {
            let out = normalize_filter(f);
            if out.is_none() {
                tracing::debug!(filter = %f, "ignoring unrecognised request filter");
            }
            out
        })
        .collect()
}

impl RetrievalScope {
    pub fn resolve(explicit: &ExplicitScope, plan: &QueryPlan) -> Self {
        let service = explicit
            .service
            .clone()
            .or_else(|| filter_tag(&explicit.filters, "service"))
            .or_else(|| plan.service.clone())
            .or_else(|| filter_tag(&plan.filters, "service"));
        let environment = explicit
            .environment
            .clone()
            .or_else(|| filter_tag(&explicit.filters, "env"))
            .or_else(|| plan.environment.clone())
            .or_else(|| filter_tag(&plan.filters, "env"));
        let window = explicit.window.or_else(|| {
            plan.window.as_ref().map(|w| Window {
                from: w.from_utc.as_deref().and_then(parse_utc),
                to: w.to_utc.as_deref().and_then(parse_utc),
            })
        });
        let kinds = explicit
            .kinds
            .clone()
            .filter(|k| !k.is_empty())
            .or_else(|| Some(filter_kinds(&explicit.filters)).filter(|k| !k.is_empty()))
            .unwrap_or_else(|| filter_kinds(&plan.filters));
        let mut dedup: Vec<SourceKind> = Vec::new();
        for k in kinds {
            if !dedup.contains(&k) {
                dedup.push(k);
            }
        }
        Self {
            service,
            environment,
            from_utc: window.and_then(|w| w.from),
            to_utc: window.and_then(|w| w.to),
            kinds: dedup,
        }
    }

    /// Qdrant filter for this scope, or `None` when unconstrained.
    ///
    /// The time window is a half-open `[from, to)` `range` on the RFC 3339 `Timestamp`
    /// payload (Qdrant's datetime range; no numeric field or payload index is required).
    /// It is wrapped in a `should` so that documents without a timestamp and
    /// [`TIMELESS_KINDS`] still match: a monitor or SLO is relevant evidence for
    /// "yesterday" even though it is not an event from yesterday. A log pattern document
    /// covers one pattern on one UTC day (see [`crate::log_patterns`]) and is stamped with
    /// that day's last log; it matches when the day's `[Metadata.first_seen, Timestamp]`
    /// overlaps the window, so a day whose logs started before a short window and went on
    /// after it is kept. Whether such a day logged inside the window is decided per hour
    /// when the answer is built.
    pub fn to_qdrant_filter(&self) -> Option<Value> {
        let mut must = vec![];
        if let Some(svc) = &self.service {
            must.push(json!({"key": "Service", "match": {"value": svc}}));
        }
        if let Some(env) = &self.environment {
            must.push(json!({"key": "Environment", "match": {"value": env}}));
        }
        if !self.kinds.is_empty() {
            let any: Vec<Value> = self.kinds.iter().map(SourceKind::payload_value).collect();
            must.push(json!({"key": "Kind", "match": {"any": any}}));
        }
        if self.from_utc.is_some() || self.to_utc.is_some() {
            let mut range = serde_json::Map::new();
            if let Some(from) = self.from_utc {
                range.insert("gte".into(), json!(format_utc(from)));
            }
            if let Some(to) = self.to_utc {
                range.insert("lt".into(), json!(format_utc(to)));
            }
            let timeless: Vec<Value> = TIMELESS_KINDS
                .iter()
                .map(SourceKind::payload_value)
                .collect();
            // A log pattern day spans `[first_seen, Timestamp]` (its last log): it belongs
            // to the window when that span overlaps it.
            let mut pattern =
                vec![json!({"key": "Kind", "match": {"value": SourceKind::Logs.payload_value()}})];
            if let Some(from) = self.from_utc {
                pattern.push(json!({"key": "Timestamp", "range": {"gte": format_utc(from)}}));
            }
            if let Some(to) = self.to_utc {
                pattern.push(json!({"key": FIRST_SEEN_KEY, "range": {"lt": format_utc(to)}}));
            }
            must.push(json!({"should": [
                {"key": "Timestamp", "range": range},
                {"is_empty": {"key": "Timestamp"}},
                {"key": "Kind", "match": {"any": timeless}},
                {"must": pattern},
            ]}));
        }
        (!must.is_empty()).then(|| json!({ "must": must }))
    }

    /// Public, human-readable form returned by the API.
    pub fn to_json(&self) -> Value {
        json!({
            "service": self.service,
            "environment": self.environment,
            "fromUtc": self.from_utc.map(format_utc),
            "toUtc": self.to_utc.map(format_utc),
            "kinds": self.kinds.iter().map(SourceKind::name).collect::<Vec<_>>(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::planner::TimeRange;

    fn ts(s: &str) -> DateTime<Utc> {
        parse_utc(s).unwrap()
    }

    fn inferred() -> QueryPlan {
        QueryPlan {
            service: Some("auth-api".into()),
            environment: Some("prod".into()),
            window: Some(TimeRange {
                from_utc: Some("2026-09-22T22:00:00Z".into()),
                to_utc: Some("2026-09-23T22:00:00Z".into()),
            }),
            filters: vec!["kind:logs".into(), "service:other".into()],
            ..Default::default()
        }
    }

    #[test]
    fn inferred_values_apply_when_caller_is_silent() {
        let scope = RetrievalScope::resolve(&ExplicitScope::default(), &inferred());
        assert_eq!(scope.service.as_deref(), Some("auth-api"));
        assert_eq!(scope.environment.as_deref(), Some("prod"));
        assert_eq!(scope.from_utc, Some(ts("2026-09-22T22:00:00Z")));
        assert_eq!(scope.to_utc, Some(ts("2026-09-23T22:00:00Z")));
        assert_eq!(scope.kinds, vec![SourceKind::Logs]);
    }

    #[test]
    fn explicit_values_win_field_by_field() {
        let explicit = ExplicitScope {
            service: Some("payments".into()),
            window: Some(Window {
                from: Some(ts("2026-09-01T00:00:00Z")),
                to: None,
            }),
            filters: vec!["env:staging".into(), "kind:monitor".into()],
            ..Default::default()
        };
        let scope = RetrievalScope::resolve(&explicit, &inferred());
        assert_eq!(scope.service.as_deref(), Some("payments"));
        assert_eq!(scope.environment.as_deref(), Some("staging"));
        // A caller bound replaces the whole inferred window.
        assert_eq!(scope.from_utc, Some(ts("2026-09-01T00:00:00Z")));
        assert_eq!(scope.to_utc, None);
        assert_eq!(scope.kinds, vec![SourceKind::Monitor]);

        let explicit = ExplicitScope {
            kinds: Some(vec![SourceKind::SLO, SourceKind::SLO]),
            filters: vec!["kind:monitor".into()],
            ..Default::default()
        };
        let scope = RetrievalScope::resolve(&explicit, &inferred());
        assert_eq!(scope.kinds, vec![SourceKind::SLO]);
    }

    #[test]
    fn planner_filter_tags_are_a_fallback_for_missing_fields() {
        let plan = QueryPlan {
            filters: vec!["service:checkout".into(), "env:dev".into()],
            ..Default::default()
        };
        let scope = RetrievalScope::resolve(&ExplicitScope::default(), &plan);
        assert_eq!(scope.service.as_deref(), Some("checkout"));
        assert_eq!(scope.environment.as_deref(), Some("dev"));
    }

    #[test]
    fn empty_scope_has_no_filter() {
        assert_eq!(RetrievalScope::default().to_qdrant_filter(), None);
    }

    #[test]
    fn filter_contains_entities_kinds_and_time_window() {
        let scope = RetrievalScope {
            service: Some("auth-api".into()),
            environment: Some("prod".into()),
            from_utc: Some(ts("2026-09-22T22:00:00Z")),
            to_utc: Some(ts("2026-09-23T22:00:00Z")),
            kinds: vec![SourceKind::Logs, SourceKind::SLO],
        };
        assert_eq!(
            scope.to_qdrant_filter().unwrap(),
            json!({"must": [
                {"key": "Service", "match": {"value": "auth-api"}},
                {"key": "Environment", "match": {"value": "prod"}},
                {"key": "Kind", "match": {"any": ["logs", "sLO"]}},
                {"should": [
                    {"key": "Timestamp", "range": {
                        "gte": "2026-09-22T22:00:00Z",
                        "lt": "2026-09-23T22:00:00Z"
                    }},
                    {"is_empty": {"key": "Timestamp"}},
                    {"key": "Kind", "match": {"any": ["metrics", "monitor", "dashboard", "sLO"]}},
                    {"must": [
                        {"key": "Kind", "match": {"value": "logs"}},
                        {"key": "Timestamp", "range": {"gte": "2026-09-22T22:00:00Z"}},
                        {"key": "Metadata.first_seen", "range": {"lt": "2026-09-23T22:00:00Z"}}
                    ]}
                ]}
            ]})
        );
    }

    /// A pattern day whose first log is before the window's end and whose last is at or
    /// after its start overlaps it; with an open start only the first log counts.
    #[test]
    fn log_pattern_days_match_when_their_span_overlaps_the_window() {
        let scope = RetrievalScope {
            to_utc: Some(ts("2026-09-23T22:00:00Z")),
            ..Default::default()
        };
        assert_eq!(
            scope.to_qdrant_filter().unwrap()["must"][0]["should"][3],
            json!({"must": [
                {"key": "Kind", "match": {"value": "logs"}},
                {"key": "Metadata.first_seen", "range": {"lt": "2026-09-23T22:00:00Z"}}
            ]})
        );
    }

    #[test]
    fn open_ended_window_only_sets_one_bound() {
        let scope = RetrievalScope {
            from_utc: Some(ts("2026-09-22T22:00:00Z")),
            ..Default::default()
        };
        let filter = scope.to_qdrant_filter().unwrap();
        assert_eq!(
            filter["must"][0]["should"][0]["range"],
            json!({"gte": "2026-09-22T22:00:00Z"})
        );
    }

    #[test]
    fn request_filters_are_normalized_and_unknown_ones_ignored() {
        let out = normalize_filters(&[
            "Kind:Logs".into(),
            "status:error".into(),
            "environment:PROD".into(),
            "kind:bogus".into(),
        ]);
        assert_eq!(out, vec!["kind:logs", "env:prod"]);
    }
}
