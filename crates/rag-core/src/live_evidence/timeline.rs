//! The `timeline` returned by `/ask`: measured observations, hypotheses that
//! must cite them, and evidence that could not be checked.

use crate::planner::format_utc;
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashSet;
use std::fmt::Write;

/// Most hypotheses kept from the LLM.
const MAX_HYPOTHESES: usize = 5;
/// Longest hypothesis statement kept, in characters.
const MAX_STATEMENT_CHARS: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TimelineStatus {
    /// Live queries ran (some may still have failed; see `missingEvidence`).
    Collected,
    /// No live queries ran; `skipReason` says why.
    Skipped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    /// Not a diagnostic question and not requested explicitly.
    NotDiagnostic,
    /// `RAG_LIVE_EVIDENCE=off`.
    Disabled,
    /// The request set `live_evidence: false`.
    DisabledByRequest,
    /// `DD_API_KEY`/`DD_APP_KEY` are not configured for the API.
    NotConfigured,
    /// No time window was given or inferred.
    WindowNotSpecified,
    /// The window is longer than `RAG_LIVE_MAX_WINDOW_HOURS`.
    WindowTooLong,
    /// Neither services nor metrics could be discovered.
    NothingToQuery,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ObservationKind {
    /// Sustained values above the baseline.
    Spike,
    /// Sustained values below the baseline.
    Drop,
    /// Consecutive missing points inside the window.
    Gap,
    /// Window statistics compared to the baseline (always reported for a series with data).
    SeriesSummary,
    /// An interval with a burst of error/warning logs.
    LogBurst,
    /// Error/warning log counts over the window (reported for every log query).
    LogSummary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ObservationSource {
    MetricQuery,
    LogQuery,
}

/// A fact measured from live Datadog data. `values` holds the numbers the
/// summary was built from; they were computed in code, not by the LLM.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Observation {
    /// `obs-N`, assigned in chronological order.
    pub id: String,
    pub kind: ObservationKind,
    pub source: ObservationSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service: Option<String>,
    /// The Datadog metric or log query that produced this observation.
    pub query: String,
    pub start_utc: String,
    pub end_utc: String,
    pub summary: String,
    pub values: Value,
    /// Datadog app URL showing the same data.
    pub link: String,
}

/// A candidate explanation. Kept only if it cites known observation IDs.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hypothesis {
    pub statement: String,
    pub observation_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MissingReason {
    /// Live evidence did not run at all (see the timeline's `skipReason`).
    Skipped,
    /// No metric could be associated with this service.
    NoMetricsDiscovered,
    /// The Datadog query failed (after retries).
    QueryFailed,
    /// The Datadog query did not finish within `RAG_LIVE_EVIDENCE_TIMEOUT_MS`.
    TimedOut,
    /// The query succeeded but returned no data points in the window.
    SeriesEmpty,
    /// Too few baseline points to judge whether the window is anomalous.
    NoBaseline,
    /// A cap (services, metrics, log events) left something unchecked.
    Capped,
    /// Hypotheses could not be generated.
    HypothesesFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MissingEvidence {
    /// What could not be checked (a query, service or metric).
    pub subject: String,
    pub reason: MissingReason,
    pub detail: String,
}

impl MissingEvidence {
    pub fn new(
        subject: impl Into<String>,
        reason: MissingReason,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            subject: subject.into(),
            reason,
            detail: detail.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeSpan {
    #[serde(serialize_with = "ser_utc")]
    pub from_utc: DateTime<Utc>,
    #[serde(serialize_with = "ser_utc")]
    pub to_utc: DateTime<Utc>,
}

fn ser_utc<S: serde::Serializer>(t: &DateTime<Utc>, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&format_utc(*t))
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Timeline {
    pub status: TimelineStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skip_reason: Option<SkipReason>,
    /// The window the question is about.
    pub window: Option<TimeSpan>,
    /// The equal-length window just before, used as the "normal" reference.
    pub baseline: Option<TimeSpan>,
    pub observations: Vec<Observation>,
    pub hypotheses: Vec<Hypothesis>,
    pub missing_evidence: Vec<MissingEvidence>,
}

impl Timeline {
    /// A timeline for which no live query ran. Unless the question simply was
    /// not diagnostic, the reason is also recorded as missing evidence so the
    /// answer does not imply live data was checked.
    pub fn skipped(reason: SkipReason, detail: &str) -> Self {
        let missing_evidence = if reason == SkipReason::NotDiagnostic {
            vec![]
        } else {
            vec![MissingEvidence::new(
                "live Datadog data",
                MissingReason::Skipped,
                detail,
            )]
        };
        Self {
            status: TimelineStatus::Skipped,
            skip_reason: Some(reason),
            window: None,
            baseline: None,
            observations: vec![],
            hypotheses: vec![],
            missing_evidence,
        }
    }

    pub fn has_observations(&self) -> bool {
        !self.observations.is_empty()
    }

    /// The timeline rendered for the answer prompt, or `None` when there is
    /// nothing to tell the LLM (a non-diagnostic question).
    pub fn prompt_context(&self) -> Option<String> {
        if self.skip_reason == Some(SkipReason::NotDiagnostic) {
            return None;
        }
        let mut sb = String::new();
        if let Some(w) = &self.window {
            let _ = writeln!(
                sb,
                "Window: {} to {}",
                format_utc(w.from_utc),
                format_utc(w.to_utc)
            );
        }
        if let Some(b) = &self.baseline {
            let _ = writeln!(
                sb,
                "Baseline: {} to {}",
                format_utc(b.from_utc),
                format_utc(b.to_utc)
            );
        }
        sb.push_str("Observations (measured facts):\n");
        if self.observations.is_empty() {
            sb.push_str("- none\n");
        }
        for o in &self.observations {
            let _ = writeln!(
                sb,
                "- [{}] {} to {}: {} (query: {}; link: {})",
                o.id, o.start_utc, o.end_utc, o.summary, o.query, o.link
            );
        }
        sb.push_str("Hypotheses (unverified, cite observations):\n");
        if self.hypotheses.is_empty() {
            sb.push_str("- none\n");
        }
        for h in &self.hypotheses {
            let _ = writeln!(sb, "- {} [{}]", h.statement, h.observation_ids.join(", "));
        }
        sb.push_str("Missing evidence (not checked):\n");
        if self.missing_evidence.is_empty() {
            sb.push_str("- none\n");
        }
        for m in &self.missing_evidence {
            let _ = writeln!(sb, "- {}: {}", m.subject, m.detail);
        }
        Some(sb)
    }
}

/// Validates untrusted hypotheses (`{"hypotheses": [{"statement", "observationIds"}]}`).
/// A hypothesis that cites no observation, or any ID not in `known_ids`, is dropped.
pub fn validate_hypotheses(raw: &Value, known_ids: &HashSet<&str>) -> Vec<Hypothesis> {
    let Some(items) = raw.get("hypotheses").and_then(Value::as_array) else {
        return vec![];
    };
    let mut out = Vec::new();
    for item in items {
        let statement = item
            .get("statement")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let ids: Option<Vec<String>> = item
            .get("observationIds")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .map(|v| v.as_str().map(str::to_string))
                    .collect::<Option<Vec<_>>>()
            })
            .unwrap_or(None);
        let (Some(statement), Some(mut ids)) = (statement, ids) else {
            tracing::warn!(hypothesis = %item, "live evidence: dropping malformed hypothesis");
            continue;
        };
        let mut seen = HashSet::new();
        ids.retain(|id| seen.insert(id.clone()));
        if ids.is_empty() {
            tracing::warn!(
                statement,
                "live evidence: dropping hypothesis citing no observation"
            );
            continue;
        }
        if let Some(unknown) = ids.iter().find(|id| !known_ids.contains(id.as_str())) {
            tracing::warn!(statement, unknown = %unknown, "live evidence: dropping hypothesis citing unknown observation");
            continue;
        }
        out.push(Hypothesis {
            statement: statement.chars().take(MAX_STATEMENT_CHARS).collect(),
            observation_ids: ids,
        });
        if out.len() == MAX_HYPOTHESES {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn hypotheses_must_cite_known_observations() {
        let known: HashSet<&str> = ["obs-1", "obs-2"].into_iter().collect();
        let raw = json!({"hypotheses": [
            {"statement": "DB saturation caused latency", "observationIds": ["obs-1", "obs-2"]},
            {"statement": "A deploy broke auth", "observationIds": ["obs-9"]},
            {"statement": "Cosmic rays", "observationIds": []},
            {"statement": "No citations at all"},
            {"statement": "Mixed", "observationIds": ["obs-1", "obs-3"]},
            {"statement": "  ", "observationIds": ["obs-1"]},
            {"statement": "Non-string id", "observationIds": [1]},
        ]});
        let kept = validate_hypotheses(&raw, &known);
        assert_eq!(
            kept,
            vec![Hypothesis {
                statement: "DB saturation caused latency".into(),
                observation_ids: vec!["obs-1".into(), "obs-2".into()],
            }]
        );
        assert!(validate_hypotheses(&json!({"other": 1}), &known).is_empty());
    }

    #[test]
    fn skipped_timeline_records_missing_evidence_unless_not_diagnostic() {
        let t = Timeline::skipped(SkipReason::NotConfigured, "no Datadog credentials");
        let v = serde_json::to_value(&t).unwrap();
        assert_eq!(v["status"], "skipped");
        assert_eq!(v["skipReason"], "not_configured");
        assert_eq!(v["missingEvidence"][0]["reason"], "skipped");
        assert!(
            t.prompt_context()
                .unwrap()
                .contains("no Datadog credentials")
        );

        let t = Timeline::skipped(SkipReason::NotDiagnostic, "not diagnostic");
        assert!(t.missing_evidence.is_empty());
        assert!(t.prompt_context().is_none());
    }
}
