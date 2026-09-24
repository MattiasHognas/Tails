//! Chooses which services and metrics to query live, from the indexed
//! documents already retrieved for the question plus the resolved scope.
//!
//! Ranking: an explicit/planned service or metric always comes first. Other
//! services are ranked by the summed retrieval score of hits that name them
//! (metric documents excluded: their `service` is only a name prefix). Metrics
//! are ranked by the summed score of the metric and monitor documents that
//! mention them. Ties keep first-seen order.

use super::timeline::{MissingEvidence, MissingReason};
use crate::domain::{Hit, SourceKind};
use std::collections::HashMap;

/// Aggregations accepted from monitor queries; anything else becomes `avg`.
const AGGREGATIONS: [&str; 4] = ["avg", "sum", "min", "max"];
const MAX_METRIC_NAME: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiscoveryCaps {
    pub max_services: usize,
    pub max_metrics: usize,
}

/// A metric to query, optionally tied to the service it was discovered for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetricTarget {
    pub metric: String,
    /// Space aggregation (`avg`, `sum`, `min`, `max`).
    pub aggregation: String,
    pub service: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Discovery {
    pub services: Vec<String>,
    pub metrics: Vec<MetricTarget>,
    /// What discovery had to leave out (caps, services without metrics).
    pub missing: Vec<MissingEvidence>,
}

/// Whether `s` is safe to splice into a Datadog tag filter (`service:<s>`).
pub fn is_tag_value(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 200
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/' | ':'))
}

fn is_metric_name(s: &str) -> bool {
    s.len() <= MAX_METRIC_NAME
        && s.starts_with(|c: char| c.is_ascii_alphabetic())
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
}

/// Metric names (with their space aggregation, if written) in a Datadog
/// metric query such as a monitor's
/// `avg(last_5m):avg:trace.http.request.duration{service:auth-api} > 2`.
/// A metric name is the identifier right before a `{...}` scope; group-by
/// braces (`by {host}`) and log/event queries have none.
pub fn metric_names_in_query(query: &str) -> Vec<(Option<String>, String, Option<String>)> {
    let mut out: Vec<(Option<String>, String, Option<String>)> = Vec::new();
    let bytes = query.as_bytes();
    for (i, _) in query.match_indices('{') {
        let mut start = i;
        while start > 0 {
            let c = bytes[start - 1] as char;
            if c.is_ascii_alphanumeric() || c == '_' || c == '.' {
                start -= 1;
            } else {
                break;
            }
        }
        let name = &query[start..i];
        if !is_metric_name(name) || !name.contains('.') {
            continue;
        }
        let aggregation = query[..start]
            .strip_suffix(':')
            .and_then(|p| p.rsplit(|c: char| !c.is_ascii_alphabetic()).next())
            .filter(|a| AGGREGATIONS.contains(a))
            .map(str::to_string);
        let scope_end = query[i..].find('}').map(|e| i + e);
        let service = scope_end.and_then(|e| {
            query[i + 1..e]
                .split(',')
                .find_map(|t| t.trim().strip_prefix("service:"))
                .filter(|s| is_tag_value(s))
                .map(str::to_string)
        });
        if !out.iter().any(|(_, n, _)| n == name) {
            out.push((aggregation, name.to_string(), service));
        }
    }
    out
}

fn monitor_query(hit: &Hit) -> Option<String> {
    hit.doc
        .metadata
        .get("query")
        .and_then(|q| q.as_str())
        .map(str::to_string)
        .or_else(|| {
            hit.doc
                .text
                .lines()
                .find_map(|l| l.strip_prefix("Query: "))
                .map(str::to_string)
        })
}

fn metric_doc_name(hit: &Hit) -> Option<String> {
    hit.doc
        .metadata
        .get("metric_name")
        .and_then(|m| m.as_str())
        .map(str::to_string)
        .or_else(|| hit.doc.title.strip_prefix("Metric: ").map(str::to_string))
}

/// Ranked, deduplicated keys; `pinned` keys come first in the given order.
struct Ranking<T> {
    order: Vec<T>,
    score: HashMap<usize, f32>,
}

impl<T: PartialEq + Clone> Ranking<T> {
    fn new() -> Self {
        Self {
            order: vec![],
            score: HashMap::new(),
        }
    }

    fn add(&mut self, key: T, score: f32) -> usize {
        let idx = match self.order.iter().position(|k| *k == key) {
            Some(i) => i,
            None => {
                self.order.push(key);
                self.order.len() - 1
            }
        };
        *self.score.entry(idx).or_default() += score;
        idx
    }

    fn ranked(&self) -> Vec<T> {
        let mut idx: Vec<usize> = (0..self.order.len()).collect();
        // Stable sort keeps first-seen order for equal scores.
        idx.sort_by(|a, b| self.score[b].total_cmp(&self.score[a]));
        idx.into_iter().map(|i| self.order[i].clone()).collect()
    }
}

/// Picks the services and metrics to query. `scope_service` (explicit or
/// planned) and `plan_metric` are always included first.
pub fn discover(
    hits: &[Hit],
    scope_service: Option<&str>,
    plan_metric: Option<&str>,
    caps: DiscoveryCaps,
) -> Discovery {
    const PINNED: f32 = 1e6;
    let mut services: Ranking<String> = Ranking::new();
    let mut metrics: Ranking<String> = Ranking::new();
    let mut metric_info: HashMap<String, (Option<String>, Option<String>)> = HashMap::new();

    if let Some(s) = scope_service.filter(|s| is_tag_value(s)) {
        services.add(s.to_string(), PINNED);
    }
    if let Some(m) = plan_metric.filter(|m| is_metric_name(m)) {
        metrics.add(m.to_string(), PINNED);
        metric_info.insert(m.to_string(), (None, scope_service.map(str::to_string)));
    }

    for hit in hits {
        let doc = &hit.doc;
        let score = hit.score.max(0.0);
        if doc.kind != SourceKind::Metrics && is_tag_value(&doc.service) {
            services.add(doc.service.clone(), score);
        }
        let found: Vec<(Option<String>, String, Option<String>)> = match doc.kind {
            SourceKind::Metrics => metric_doc_name(hit)
                .filter(|m| is_metric_name(m))
                .map(|m| vec![(None, m, None)])
                .unwrap_or_default(),
            SourceKind::Monitor => monitor_query(hit)
                .map(|q| metric_names_in_query(&q))
                .unwrap_or_default()
                .into_iter()
                .map(|(agg, name, svc)| {
                    let svc = svc.or_else(|| Some(doc.service.clone()).filter(|s| is_tag_value(s)));
                    (agg, name, svc)
                })
                .collect(),
            _ => vec![],
        };
        for (agg, name, svc) in found {
            metrics.add(name.clone(), score);
            let info = metric_info.entry(name).or_insert((None, None));
            info.0 = info.0.take().or(agg);
            info.1 = info.1.take().or(svc);
        }
    }

    let mut missing = Vec::new();
    let mut ranked_services = services.ranked();
    if ranked_services.len() > caps.max_services {
        let dropped = ranked_services.split_off(caps.max_services);
        missing.push(MissingEvidence::new(
            dropped.join(", "),
            MissingReason::Capped,
            format!(
                "not queried: only the top {} services are checked (RAG_LIVE_MAX_SERVICES)",
                caps.max_services
            ),
        ));
    }

    let mut targets: Vec<MetricTarget> = metrics
        .ranked()
        .into_iter()
        .map(|metric| {
            let (agg, svc) = metric_info.get(&metric).cloned().unwrap_or_default();
            // A metric's own service is used only if that service is queried;
            // otherwise it is scoped to the top service (if any).
            let service = svc
                .filter(|s| ranked_services.contains(s))
                .or_else(|| ranked_services.first().cloned());
            MetricTarget {
                metric,
                aggregation: agg.unwrap_or_else(|| "avg".into()),
                service,
            }
        })
        .collect();
    if targets.len() > caps.max_metrics {
        let dropped = targets.split_off(caps.max_metrics);
        missing.push(MissingEvidence::new(
            dropped
                .iter()
                .map(|t| t.metric.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            MissingReason::Capped,
            format!(
                "not queried: only the top {} metrics are checked (RAG_LIVE_MAX_METRICS)",
                caps.max_metrics
            ),
        ));
    }
    for s in &ranked_services {
        if !targets.iter().any(|t| t.service.as_deref() == Some(s)) {
            missing.push(MissingEvidence::new(
                format!("metrics for {s}"),
                MissingReason::NoMetricsDiscovered,
                "no metric or monitor document for this service was retrieved; only its logs were checked",
            ));
        }
    }

    Discovery {
        services: ranked_services,
        metrics: targets,
        missing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::RagDocument;
    use serde_json::json;

    fn hit(kind: SourceKind, service: &str, score: f32, metadata: serde_json::Value) -> Hit {
        Hit {
            doc: RagDocument {
                id: "x".into(),
                title: "t".into(),
                text: String::new(),
                source_uri: String::new(),
                kind,
                timestamp: None,
                service: service.into(),
                environment: "prod".into(),
                metadata: metadata.as_object().cloned().unwrap_or_default(),
            },
            score,
        }
    }

    #[test]
    fn extracts_metric_names_from_monitor_queries() {
        assert_eq!(
            metric_names_in_query(
                "avg(last_5m):avg:trace.http.request.duration{service:auth-api,env:prod} > 2"
            ),
            vec![(
                Some("avg".into()),
                "trace.http.request.duration".into(),
                Some("auth-api".into())
            )]
        );
        assert_eq!(
            metric_names_in_query(
                "sum(last_5m):sum:trace.http.request.errors{service:auth-api}.as_count() / sum:trace.http.request.hits{service:auth-api}.as_count() > 0.05"
            )
            .into_iter()
            .map(|(a, n, _)| (a, n))
            .collect::<Vec<_>>(),
            vec![
                (Some("sum".into()), "trace.http.request.errors".into()),
                (Some("sum".into()), "trace.http.request.hits".into()),
            ]
        );
        // Recorded monitor (fixtures/datadog/monitors.json): group-by braces are ignored.
        assert_eq!(
            metric_names_in_query(
                "avg(last_1m):avg:aws.applicationelb.target_response_time.p95{systemid:splunk,aws_account_type:production} by {region} > 0.2"
            ),
            vec![(
                Some("avg".into()),
                "aws.applicationelb.target_response_time.p95".into(),
                None
            )]
        );
        assert!(metric_names_in_query(r#"logs("service:auth-api status:error").index("*").rollup("count").last("5m") > 10"#).is_empty());
        assert_eq!(
            metric_names_in_query("p99:trace.x.duration{*}")[0].0,
            None,
            "unknown aggregations are not trusted"
        );
    }

    #[test]
    fn discovers_and_ranks_services_and_metrics_from_hits() {
        let monitors: Vec<serde_json::Value> = serde_json::from_str(
            &std::fs::read_to_string(format!(
                "{}/tests/fixtures/datadog/monitors.json",
                env!("CARGO_MANIFEST_DIR")
            ))
            .unwrap(),
        )
        .unwrap();
        let hits = vec![
            hit(SourceKind::Logs, "payments", 0.5, json!({})),
            hit(
                SourceKind::Monitor,
                "auth-api",
                0.9,
                json!({"query": "avg(last_5m):avg:trace.http.request.duration{service:auth-api} > 2"}),
            ),
            hit(
                SourceKind::Metrics,
                "system",
                0.4,
                json!({"metric_name": "system.cpu.user"}),
            ),
            hit(SourceKind::Incident, "auth-api", 0.8, json!({})),
            hit(SourceKind::Logs, "billing", 0.1, json!({})),
            hit(
                SourceKind::Monitor,
                "",
                0.3,
                json!({"query": monitors[0]["query"]}),
            ),
        ];
        let d = discover(
            &hits,
            Some("checkout"),
            Some("checkout.latency"),
            DiscoveryCaps {
                max_services: 3,
                max_metrics: 3,
            },
        );
        // Pinned scope service first, then by summed score; metric docs' prefix
        // ("system") is not a service.
        assert_eq!(d.services, vec!["checkout", "auth-api", "payments"]);
        assert_eq!(
            d.metrics,
            vec![
                MetricTarget {
                    metric: "checkout.latency".into(),
                    aggregation: "avg".into(),
                    service: Some("checkout".into()),
                },
                MetricTarget {
                    metric: "trace.http.request.duration".into(),
                    aggregation: "avg".into(),
                    service: Some("auth-api".into()),
                },
                MetricTarget {
                    metric: "system.cpu.user".into(),
                    aggregation: "avg".into(),
                    service: Some("checkout".into()),
                },
            ]
        );
        let reasons: Vec<_> = d
            .missing
            .iter()
            .map(|m| (m.reason, m.subject.as_str()))
            .collect();
        assert_eq!(
            reasons,
            vec![
                (MissingReason::Capped, "billing"),
                (
                    MissingReason::Capped,
                    "aws.applicationelb.target_response_time.p95"
                ),
                (MissingReason::NoMetricsDiscovered, "metrics for payments"),
            ]
        );
    }

    #[test]
    fn unsafe_values_are_ignored() {
        let hits = vec![hit(SourceKind::Logs, "a b} OR *", 1.0, json!({}))];
        let d = discover(
            &hits,
            Some("x\")"),
            Some("drop table"),
            DiscoveryCaps {
                max_services: 3,
                max_metrics: 3,
            },
        );
        assert!(d.services.is_empty());
        assert!(d.metrics.is_empty());
    }
}
