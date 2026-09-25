use crate::domain::RagDocument;
use crate::error::{RagError, Stage, UpstreamError};
use crate::resilience::{RetryPolicy, send_with_retry};
use anyhow::Result;
use chrono::{DateTime, Utc};
use urlencoding;

/// Incidents are fetched in every state; the indexing window is applied to `created`.
const INCIDENT_SEARCH_QUERY: &str = "state:(active OR stable OR resolved)";
/// Maximum `page[size]` accepted by the incident search endpoint.
const INCIDENT_PAGE_SIZE: u64 = 100;
/// Only error and warning logs are indexed.
const LOG_SEARCH_QUERY: &str = "status:error OR status:warn";
/// Maximum `page.limit` accepted by the log search endpoint.
const LOG_PAGE_LIMIT: u64 = 1000;
/// Maximum `page_size` accepted by the monitor list endpoint.
const MONITOR_PAGE_SIZE: u64 = 1000;
/// Page size for the dashboard list (`count`); the API default, as no maximum is documented.
const DASHBOARD_PAGE_SIZE: u64 = 100;
/// Page size for the SLO list (`limit`); the API default.
const SLO_PAGE_SIZE: u64 = 1000;

pub struct Datadog {
    pub api_key: String,
    pub app_key: String,
    pub site: String,
    /// Base URL for API requests, `https://api.{site}` unless overridden.
    pub api_base: String,
    pub http: reqwest::Client,
    /// Retry budget for the live-evidence queries ([`Self::query_metrics`],
    /// [`Self::search_log_events`]) and the per-incident and per-dashboard detail
    /// fetches of indexing. The list and search calls of indexing are not retried here.
    pub retry: RetryPolicy,
}

/// One series from `GET /api/v1/query`.
#[derive(Debug, Clone, PartialEq)]
pub struct MetricSeries {
    /// The expression Datadog evaluated, e.g. `avg:trace.http.request.duration{service:auth-api}`.
    pub expression: String,
    pub metric: String,
    /// Comma-separated tags identifying the series.
    pub scope: String,
    /// Seconds between points (the rollup Datadog chose), when reported.
    pub interval_secs: Option<u64>,
    /// `(unix millis, value)`; `None` is a point Datadog returned as `null`.
    pub points: Vec<(i64, Option<f64>)>,
}

/// Error/warning logs matching a live-evidence query.
#[derive(Debug, Clone)]
pub struct LogEvents {
    pub events: Vec<RagDocument>,
    /// More events matched than the cap allowed to fetch.
    pub truncated: bool,
}

impl Datadog {
    pub fn new(api_key: String, app_key: String, site: String) -> Self {
        Self {
            api_base: format!("https://api.{}", site),
            api_key,
            app_key,
            site,
            http: reqwest::Client::new(),
            retry: RetryPolicy::default(),
        }
    }

    pub fn new_from_env() -> Result<Self> {
        Ok(Self::new(
            std::env::var("DD_API_KEY")?,
            std::env::var("DD_APP_KEY")?,
            std::env::var("DD_SITE").unwrap_or_else(|_| "datadoghq.com".into()),
        ))
    }

    /// Fetches all monitors with `GET /api/v1/monitor`, following `page`/`page_size`
    /// pagination until a short page is returned.
    pub async fn get_monitors(&self) -> Result<Vec<RagDocument>> {
        let url = format!("{}/api/v1/monitor", self.api_base);
        let mut docs = Vec::new();
        let mut page: u64 = 0;
        loop {
            let response = self
                .http
                .get(&url)
                .header("DD-API-KEY", &self.api_key)
                .header("DD-APPLICATION-KEY", &self.app_key)
                .query(&[
                    ("page", page.to_string()),
                    ("page_size", MONITOR_PAGE_SIZE.to_string()),
                ])
                .send()
                .await?;

            if !response.status().is_success() {
                anyhow::bail!("Failed to fetch monitors: {}", response.status());
            }

            let monitors: Vec<serde_json::Value> = response.json().await?;
            docs.extend(monitors.iter().map(|m| self.monitor_document(m)));
            if (monitors.len() as u64) < MONITOR_PAGE_SIZE {
                break;
            }
            page += 1;
        }

        Ok(docs)
    }

    fn monitor_document(&self, monitor: &serde_json::Value) -> RagDocument {
        let id = monitor["id"].as_i64().unwrap_or(0).to_string();
        let name = monitor["name"].as_str().unwrap_or("").to_string();
        let message = monitor["message"].as_str().unwrap_or("").to_string();
        let query = monitor["query"].as_str().unwrap_or("").to_string();
        let monitor_type = monitor["type"].as_str().unwrap_or("").to_string();

        let tags = monitor["tags"]
            .as_array()
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
            .unwrap_or_default();

        let service = tag_value(&tags, "service");
        let environment = tag_value(&tags, "env");

        let mut metadata = serde_json::Map::new();
        metadata.insert(
            "monitor_type".to_string(),
            serde_json::Value::String(monitor_type),
        );
        metadata.insert(
            "query".to_string(),
            serde_json::Value::String(query.clone()),
        );
        metadata.insert("tags".to_string(), serde_json::json!(tags));

        let text = format!("{}\n\nQuery: {}\n\nMessage: {}", name, query, message);

        RagDocument {
            id: format!("monitor_{}", id),
            title: name,
            text,
            source_uri: format!("https://app.{}/monitors/{}", self.site, id),
            kind: crate::domain::SourceKind::Monitor,
            timestamp: None,
            service,
            environment,
            metadata,
        }
    }

    /// Fetches incidents created within `[from_iso, to_iso]` using
    /// `GET /api/v2/incidents/search`, newest first, following offset pagination
    /// until the results are older than the window, then each one's timeline and
    /// postmortem (see [`crate::datadog_incidents`]).
    pub async fn get_incidents(&self, from_iso: &str, to_iso: &str) -> Result<Vec<RagDocument>> {
        let url = format!("{}/api/v2/incidents/search", self.api_base);
        let from = chrono::DateTime::parse_from_rfc3339(from_iso)?;
        let to = chrono::DateTime::parse_from_rfc3339(to_iso)?;

        let mut incidents_in_window = Vec::new();
        let mut offset: u64 = 0;
        loop {
            let response = self
                .http
                .get(&url)
                .header("DD-API-KEY", &self.api_key)
                .header("DD-APPLICATION-KEY", &self.app_key)
                .query(&[
                    ("query", INCIDENT_SEARCH_QUERY.to_string()),
                    ("sort", "-created".to_string()),
                    ("page[size]", INCIDENT_PAGE_SIZE.to_string()),
                    ("page[offset]", offset.to_string()),
                ])
                .send()
                .await?;

            if !response.status().is_success() {
                anyhow::bail!("Failed to fetch incidents: {}", response.status());
            }

            let result: serde_json::Value = response.json().await?;
            let incidents = result["data"]["attributes"]["incidents"]
                .as_array()
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "Unexpected incident search response: missing data.attributes.incidents"
                    )
                })?;

            let mut reached_window_start = false;
            for incident in incidents {
                let incident = &incident["data"];
                let Some(created) = incident["attributes"]["created"]
                    .as_str()
                    .and_then(|c| chrono::DateTime::parse_from_rfc3339(c).ok())
                else {
                    continue;
                };
                if created < from {
                    reached_window_start = true;
                    break;
                }
                if created <= to {
                    incidents_in_window.push(incident.clone());
                }
            }

            let total = result["data"]["attributes"]["total"].as_u64();
            let next_offset = result["meta"]["pagination"]["next_offset"]
                .as_u64()
                .unwrap_or(offset + incidents.len() as u64);
            if reached_window_start
                || incidents.is_empty()
                || next_offset <= offset
                || total.is_some_and(|t| next_offset >= t)
            {
                break;
            }
            offset = next_offset;
        }

        Ok(self.incident_documents(incidents_in_window).await)
    }

    /// Fetches every error/warning log in `[from_iso, to_iso]` with
    /// `POST /api/v2/logs/events/search`, oldest first, following the
    /// `meta.page.after` cursor until the last page, and returns one document per
    /// (service, environment, status, message pattern); see [`crate::log_patterns`].
    /// Logs without a parseable timestamp are skipped.
    pub async fn search_logs(&self, from_iso: &str, to_iso: &str) -> Result<Vec<RagDocument>> {
        let url = format!("{}/api/v2/logs/events/search", self.api_base);

        let mut events = Vec::new();
        let mut skipped = 0usize;
        let mut cursor: Option<String> = None;
        loop {
            let query = log_search_body(
                from_iso,
                to_iso,
                LOG_SEARCH_QUERY,
                LOG_PAGE_LIMIT,
                cursor.as_deref(),
            );

            let response = self
                .http
                .post(&url)
                .header("DD-API-KEY", &self.api_key)
                .header("DD-APPLICATION-KEY", &self.app_key)
                .json(&query)
                .send()
                .await?;

            if !response.status().is_success() {
                anyhow::bail!("Failed to search logs: {}", response.status());
            }

            let result: serde_json::Value = response.json().await?;
            let logs = result["data"]
                .as_array()
                .ok_or_else(|| anyhow::anyhow!("Unexpected log search response: missing data"))?;
            for log in logs {
                match log_event(log) {
                    Some(e) => events.push(e),
                    None => skipped += 1,
                }
            }

            // `meta.page.after` is absent on the last page.
            match result["meta"]["page"]["after"].as_str() {
                Some(after) if !logs.is_empty() => {
                    if cursor.as_deref() == Some(after) {
                        anyhow::bail!("Log search returned the same cursor twice");
                    }
                    cursor = Some(after.to_string());
                }
                _ => break,
            }
        }

        if skipped > 0 {
            tracing::warn!(skipped, "skipped logs without a parseable timestamp");
        }
        let app_base = format!("https://app.{}", self.site);
        Ok(crate::log_patterns::group(&events)
            .iter()
            .map(|p| p.to_document(&app_base))
            .collect())
    }

    /// One log as its own document; live evidence analyses these one by one.
    fn log_document(&self, log: &serde_json::Value) -> RagDocument {
        let id = log["id"].as_str().unwrap_or("").to_string();
        let attrs = &log["attributes"];
        let message = attrs["message"].as_str().unwrap_or("").to_string();
        let status = attrs["status"].as_str().unwrap_or("").to_string();
        let timestamp = attrs["timestamp"].as_str().map(|s| s.to_string());

        let tags = attr_tags(attrs);
        let (service, environment) = log_scope(attrs, &tags);

        let mut metadata = serde_json::Map::new();
        metadata.insert(
            "status".to_string(),
            serde_json::Value::String(status.clone()),
        );
        metadata.insert("tags".to_string(), serde_json::json!(tags));

        let title = format!("Log: {} - {}", service, status);

        RagDocument {
            id: format!("log_{}", id),
            title,
            text: message,
            source_uri: format!("https://app.{}/logs?query=id:{}", self.site, id),
            kind: crate::domain::SourceKind::Logs,
            timestamp,
            service,
            environment,
            metadata,
        }
    }

    /// Fetches up to `max_events` logs matching `query` in `[from, to]` with
    /// `POST /api/v2/logs/events/search`, oldest first, following the cursor like
    /// [`Self::search_logs`]. Transient failures are retried per `self.retry`;
    /// failures are attributed to [`Stage::LiveEvidence`].
    pub async fn search_log_events(
        &self,
        query: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        max_events: usize,
    ) -> Result<LogEvents, RagError> {
        let url = format!("{}/api/v2/logs/events/search", self.api_base);
        let (from_iso, to_iso) = (
            crate::planner::format_utc(from),
            crate::planner::format_utc(to),
        );
        let mut events = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let limit = (max_events.saturating_sub(events.len()) as u64).clamp(1, LOG_PAGE_LIMIT);
            let body = log_search_body(&from_iso, &to_iso, query, limit, cursor.as_deref());
            let r = send_with_retry(&self.retry, "datadog logs search", || {
                self.http
                    .post(&url)
                    .header("DD-API-KEY", &self.api_key)
                    .header("DD-APPLICATION-KEY", &self.app_key)
                    .json(&body)
            })
            .await
            .map_err(|f| RagError::upstream(Stage::LiveEvidence, f))?;
            let result: serde_json::Value = r.json().await.map_err(|e| {
                RagError::failed(Stage::LiveEvidence, UpstreamError::from_reqwest(e))
            })?;
            let logs = result["data"]
                .as_array()
                .ok_or_else(|| invalid("log search response has no data"))?;
            events.extend(logs.iter().map(|log| self.log_document(log)));

            let next = result["meta"]["page"]["after"]
                .as_str()
                .filter(|_| !logs.is_empty());
            match next {
                Some(_) if events.len() >= max_events => {
                    return Ok(LogEvents {
                        events,
                        truncated: true,
                    });
                }
                Some(after) => {
                    if cursor.as_deref() == Some(after) {
                        return Err(invalid("log search returned the same cursor twice"));
                    }
                    cursor = Some(after.to_string());
                }
                None => {
                    return Ok(LogEvents {
                        events,
                        truncated: false,
                    });
                }
            }
        }
    }

    /// Queries timeseries points with `GET /api/v1/query` (`from`/`to` in Unix
    /// seconds). Transient failures are retried per `self.retry`; failures are
    /// attributed to [`Stage::LiveEvidence`]. A 200 response whose `status` is
    /// not `ok` is an invalid response, not an empty result.
    pub async fn query_metrics(
        &self,
        query: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<MetricSeries>, RagError> {
        let url = format!("{}/api/v1/query", self.api_base);
        let params = [
            ("from", from.timestamp().to_string()),
            ("to", to.timestamp().to_string()),
            ("query", query.to_string()),
        ];
        let r = send_with_retry(&self.retry, "datadog metrics query", || {
            self.http
                .get(&url)
                .header("DD-API-KEY", &self.api_key)
                .header("DD-APPLICATION-KEY", &self.app_key)
                .query(&params)
        })
        .await
        .map_err(|f| RagError::upstream(Stage::LiveEvidence, f))?;
        let result: serde_json::Value = r
            .json()
            .await
            .map_err(|e| RagError::failed(Stage::LiveEvidence, UpstreamError::from_reqwest(e)))?;
        if let Some(status) = result["status"].as_str()
            && status != "ok"
        {
            tracing::warn!(status, error = %result["error"], "datadog metrics query returned an error status");
            return Err(invalid("metrics query status is not ok"));
        }
        let series = result["series"]
            .as_array()
            .ok_or_else(|| invalid("metrics query response has no series"))?;
        Ok(series
            .iter()
            .map(|s| MetricSeries {
                expression: s["expression"].as_str().unwrap_or(query).to_string(),
                metric: s["metric"].as_str().unwrap_or("").to_string(),
                scope: s["scope"].as_str().unwrap_or("").to_string(),
                interval_secs: s["interval"].as_u64(),
                points: s["pointlist"]
                    .as_array()
                    .map(|pl| {
                        pl.iter()
                            .filter_map(|p| Some((p.get(0)?.as_f64()? as i64, p.get(1)?.as_f64())))
                            .collect()
                    })
                    .unwrap_or_default(),
            })
            .collect())
    }

    /// Fetches every dashboard list entry with `GET /api/v1/dashboard`, following
    /// `start`/`count` pagination until a short page is returned.
    pub async fn list_dashboard_summaries(&self) -> Result<Vec<serde_json::Value>> {
        let url = format!("{}/api/v1/dashboard", self.api_base);
        let mut summaries = Vec::new();
        let mut start: u64 = 0;
        loop {
            let response = self
                .http
                .get(&url)
                .header("DD-API-KEY", &self.api_key)
                .header("DD-APPLICATION-KEY", &self.app_key)
                .query(&[
                    ("start", start.to_string()),
                    ("count", DASHBOARD_PAGE_SIZE.to_string()),
                ])
                .send()
                .await?;

            if !response.status().is_success() {
                anyhow::bail!("Failed to fetch dashboards: {}", response.status());
            }

            let result: serde_json::Value = response.json().await?;
            let dashboards = result["dashboards"].as_array().ok_or_else(|| {
                anyhow::anyhow!("Unexpected dashboard list response: missing dashboards")
            })?;
            summaries.extend(dashboards.iter().cloned());
            if (dashboards.len() as u64) < DASHBOARD_PAGE_SIZE {
                break;
            }
            start += dashboards.len() as u64;
        }

        Ok(summaries)
    }

    /// Every dashboard with its definition fetched (within the per-run budget of
    /// [`crate::datadog_dashboards`]); nothing stored is reused.
    pub async fn list_dashboards(&self) -> Result<Vec<RagDocument>> {
        let summaries = self.list_dashboard_summaries().await?;
        Ok(self
            .dashboard_documents(&summaries, &std::collections::HashMap::new())
            .await)
    }

    pub async fn list_metrics(&self, from_iso: &str, to_iso: &str) -> Result<Vec<RagDocument>> {
        let url = format!("{}/api/v1/metrics", self.api_base);

        // The endpoint lists metrics active from `from` (Unix seconds) until now; it has no `to`.
        let from_ts = chrono::DateTime::parse_from_rfc3339(from_iso).map(|dt| dt.timestamp())?;

        let response = self
            .http
            .get(&url)
            .header("DD-API-KEY", &self.api_key)
            .header("DD-APPLICATION-KEY", &self.app_key)
            .query(&[("from", from_ts.to_string())])
            .send()
            .await?;

        if !response.status().is_success() {
            anyhow::bail!("Failed to fetch metrics: {}", response.status());
        }

        let result: serde_json::Value = response.json().await?;
        let empty_vec = vec![];
        let metrics = result["metrics"].as_array().unwrap_or(&empty_vec);
        let mut docs = Vec::new();

        for metric in metrics {
            let metric_name = metric.as_str().unwrap_or("").to_string();

            // Extract service and env from metric name if possible
            let service = metric_name
                .split('.')
                .next()
                .unwrap_or(&metric_name)
                .to_string();

            let mut metadata = serde_json::Map::new();
            metadata.insert(
                "metric_name".to_string(),
                serde_json::Value::String(metric_name.clone()),
            );

            docs.push(RagDocument {
                id: format!("metric_{}", metric_name.replace('.', "_")),
                title: format!("Metric: {}", metric_name),
                text: format!("Active metric: {}", metric_name),
                source_uri: format!(
                    "https://app.{}/metric/explorer?metric={}",
                    self.site,
                    urlencoding::encode(&metric_name)
                ),
                kind: crate::domain::SourceKind::Metrics,
                timestamp: Some(to_iso.to_string()),
                service,
                environment: String::new(),
                metadata,
            });
        }

        Ok(docs)
    }

    /// Fetches all SLOs with `GET /api/v1/slo`, following `offset`/`limit`
    /// pagination until a short page is returned.
    pub async fn list_slos(&self) -> Result<Vec<RagDocument>> {
        let url = format!("{}/api/v1/slo", self.api_base);
        let mut docs = Vec::new();
        let mut offset: u64 = 0;
        loop {
            let response = self
                .http
                .get(&url)
                .header("DD-API-KEY", &self.api_key)
                .header("DD-APPLICATION-KEY", &self.app_key)
                .query(&[
                    ("offset", offset.to_string()),
                    ("limit", SLO_PAGE_SIZE.to_string()),
                ])
                .send()
                .await?;

            if !response.status().is_success() {
                anyhow::bail!("Failed to fetch SLOs: {}", response.status());
            }

            let result: serde_json::Value = response.json().await?;
            let slos = result["data"]
                .as_array()
                .ok_or_else(|| anyhow::anyhow!("Unexpected SLO list response: missing data"))?;
            docs.extend(slos.iter().map(|slo| self.slo_document(slo)));
            if (slos.len() as u64) < SLO_PAGE_SIZE {
                break;
            }
            offset += slos.len() as u64;
        }

        Ok(docs)
    }

    fn slo_document(&self, slo: &serde_json::Value) -> RagDocument {
        let id = slo["id"].as_str().unwrap_or("").to_string();
        let name = slo["name"].as_str().unwrap_or("").to_string();
        let description = slo["description"].as_str().unwrap_or("").to_string();
        let slo_type = slo["type"].as_str().unwrap_or("").to_string();
        let target = slo["thresholds"]
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|t| t["target"].as_f64())
            .unwrap_or(0.0);

        let tags = slo["tags"]
            .as_array()
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
            .unwrap_or_default();

        let service = tag_value(&tags, "service");
        let environment = tag_value(&tags, "env");

        let mut metadata = serde_json::Map::new();
        metadata.insert(
            "slo_type".to_string(),
            serde_json::Value::String(slo_type.clone()),
        );
        metadata.insert("target".to_string(), serde_json::json!(target));
        metadata.insert("tags".to_string(), serde_json::json!(tags));

        // Thresholds are reported as percentages, e.g. 99.9.
        let text = if description.is_empty() {
            format!("{}\n\nType: {}\nTarget: {}%", name, slo_type, target)
        } else {
            format!(
                "{}\n\n{}\n\nType: {}\nTarget: {}%",
                name, description, slo_type, target
            )
        };

        RagDocument {
            id: format!("slo_{}", id),
            title: name,
            text,
            source_uri: format!("https://app.{}/slo/{}", self.site, id),
            kind: crate::domain::SourceKind::SLO,
            timestamp: None,
            service,
            environment,
            metadata,
        }
    }
}

fn invalid(msg: &str) -> RagError {
    RagError::failed(
        Stage::LiveEvidence,
        UpstreamError::InvalidResponse(msg.into()),
    )
}

/// Body for `POST /api/v2/logs/events/search`. Ascending order makes progress
/// through the window monotonic.
fn log_search_body(
    from_iso: &str,
    to_iso: &str,
    query: &str,
    limit: u64,
    cursor: Option<&str>,
) -> serde_json::Value {
    let mut page = serde_json::json!({ "limit": limit });
    if let Some(cursor) = cursor {
        page["cursor"] = serde_json::Value::String(cursor.to_string());
    }
    serde_json::json!({
        "filter": {
            "from": from_iso,
            "to": to_iso,
            "query": query
        },
        "page": page,
        "sort": "timestamp"
    })
}

/// Service and environment values as stored in the `Service`/`Environment` payload.
/// Qdrant keyword matches are case-sensitive and the retrieval scope lowercases
/// planner and filter values (see [`crate::planner::sanitize_tag_value`]), so the
/// writer lowercases too; otherwise `service:Auth-API` would never match `auth-api`.
pub fn normalize_scope_value(value: &str) -> String {
    value.trim().to_lowercase()
}

/// The value of the first `key:value` tag, normalized like [`normalize_scope_value`];
/// empty when there is none.
fn tag_value(tags: &[&str], key: &str) -> String {
    tags.iter()
        .find_map(|t| t.strip_prefix(key)?.strip_prefix(':'))
        .map(normalize_scope_value)
        .unwrap_or_default()
}

fn attr_tags(attrs: &serde_json::Value) -> Vec<&str> {
    attrs["tags"]
        .as_array()
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
        .unwrap_or_default()
}

/// A log's service and environment, normalized like the payload. Logs carry the
/// reserved `service` attribute; tags are only a fallback.
fn log_scope(attrs: &serde_json::Value, tags: &[&str]) -> (String, String) {
    let service = attrs["service"]
        .as_str()
        .map(normalize_scope_value)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| tag_value(tags, "service"));
    (service, tag_value(tags, "env"))
}

/// A log from the log search response as the pattern grouping reads it; `None` without
/// a parseable timestamp.
pub fn log_event(log: &serde_json::Value) -> Option<crate::log_patterns::LogEvent> {
    let attrs = &log["attributes"];
    let timestamp = DateTime::parse_from_rfc3339(attrs["timestamp"].as_str()?)
        .ok()?
        .with_timezone(&Utc);
    let (service, environment) = log_scope(attrs, &attr_tags(attrs));
    Some(crate::log_patterns::LogEvent {
        id: log["id"].as_str().unwrap_or("").to_string(),
        timestamp,
        service,
        environment,
        status: attrs["status"].as_str().unwrap_or("").to_string(),
        message: attrs["message"].as_str().unwrap_or("").to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_datadog_struct_creation() {
        let dd = Datadog::new(
            "test_api_key".to_string(),
            "test_app_key".to_string(),
            "datadoghq.com".to_string(),
        );

        assert_eq!(dd.api_key, "test_api_key");
        assert_eq!(dd.app_key, "test_app_key");
        assert_eq!(dd.site, "datadoghq.com");
    }

    #[test]
    fn test_datadog_url_formatting() {
        let dd = Datadog::new(
            "test_api_key".to_string(),
            "test_app_key".to_string(),
            "datadoghq.eu".to_string(),
        );

        // Test monitor URL
        let monitor_url = format!("{}/api/v1/monitor", dd.api_base);
        assert_eq!(monitor_url, "https://api.datadoghq.eu/api/v1/monitor");

        // Test dashboard URL
        let dashboard_url = format!("{}/api/v1/dashboard", dd.api_base);
        assert_eq!(dashboard_url, "https://api.datadoghq.eu/api/v1/dashboard");

        // Test SLO URL
        let slo_url = format!("{}/api/v1/slo", dd.api_base);
        assert_eq!(slo_url, "https://api.datadoghq.eu/api/v1/slo");
    }

    #[test]
    fn test_datadog_new_from_env_default_site() {
        let dd = Datadog::new(
            "test_api".to_string(),
            "test_app".to_string(),
            "datadoghq.com".to_string(),
        );
        assert_eq!(dd.site, "datadoghq.com");
        assert_eq!(dd.api_key, "test_api");
        assert_eq!(dd.app_key, "test_app");
    }

    #[test]
    fn test_datadog_new_from_env_custom_site() {
        let dd = Datadog::new(
            "test_api".to_string(),
            "test_app".to_string(),
            "datadoghq.eu".to_string(),
        );
        assert_eq!(dd.site, "datadoghq.eu");
        assert_eq!(dd.api_key, "test_api");
        assert_eq!(dd.app_key, "test_app");
    }

    #[test]
    fn test_datadog_new_direct_constructor() {
        let dd = Datadog::new(
            "key1".to_string(),
            "key2".to_string(),
            "custom.site".to_string(),
        );
        assert_eq!(dd.api_key, "key1");
        assert_eq!(dd.app_key, "key2");
        assert_eq!(dd.site, "custom.site");
    }

    #[test]
    fn test_datadog_app_url_formatting() {
        let dd = Datadog::new(
            "test".to_string(),
            "test".to_string(),
            "datadoghq.com".to_string(),
        );

        // Test app URLs (for source_uri in documents)
        let monitor_app_url = format!("https://app.{}/monitors/{}", dd.site, "12345");
        assert_eq!(monitor_app_url, "https://app.datadoghq.com/monitors/12345");

        let incident_app_url = format!("https://app.{}/incidents/{}", dd.site, "inc-123");
        assert_eq!(
            incident_app_url,
            "https://app.datadoghq.com/incidents/inc-123"
        );

        let dashboard_app_url = format!("https://app.{}/dashboard/{}", dd.site, "dash-456");
        assert_eq!(
            dashboard_app_url,
            "https://app.datadoghq.com/dashboard/dash-456"
        );

        let slo_app_url = format!("https://app.{}/slo/{}", dd.site, "slo-789");
        assert_eq!(slo_app_url, "https://app.datadoghq.com/slo/slo-789");
    }

    #[test]
    fn test_tag_values_are_lowercased_like_the_retrieval_scope() {
        let tags = [
            "environment:test",
            "env:Production",
            "service:Auth-API",
            "version:1.2.3",
        ];
        assert_eq!(tag_value(&tags, "service"), "auth-api");
        // `environment:` is not the `env` tag.
        assert_eq!(tag_value(&tags, "env"), "production");
        assert_eq!(tag_value(&["version:1.0.0"], "service"), "");
        assert_eq!(tag_value(&["service:"], "service"), "");

        // Every value the writer stores must survive the planner's validation
        // unchanged, or a filter on it can never match.
        for raw in ["Auth-API", " checkout ", "PROD", "payments.gateway"] {
            let stored = normalize_scope_value(raw);
            assert_eq!(
                crate::planner::sanitize_tag_value(&serde_json::json!(raw), "service"),
                Some(stored)
            );
        }

        let dd = Datadog::new("a".into(), "b".into(), "datadoghq.eu".into());
        let log = dd.log_document(&serde_json::json!({
            "id": "1",
            "attributes": {"service": "Auth-API", "tags": ["env:PROD"], "message": "åäö"}
        }));
        assert_eq!(
            (log.service.as_str(), log.environment.as_str()),
            ("auth-api", "prod")
        );
        let incident = dd.incident_document(
            &serde_json::json!({
                "id": "x",
                "attributes": {"title": "t", "fields": {
                    "services": {"value": ["Payments"]},
                    "environment": {"value": "Staging"}
                }}
            }),
            &Default::default(),
        );
        assert_eq!(
            (incident.service.as_str(), incident.environment.as_str()),
            ("payments", "staging")
        );
    }

    #[test]
    fn test_incidents_api_url() {
        let dd = Datadog::new(
            "test".to_string(),
            "test".to_string(),
            "datadoghq.eu".to_string(),
        );

        let url = format!("{}/api/v2/incidents/search", dd.api_base);
        assert_eq!(url, "https://api.datadoghq.eu/api/v2/incidents/search");
    }

    #[test]
    fn test_logs_api_url() {
        let dd = Datadog::new(
            "test".to_string(),
            "test".to_string(),
            "datadoghq.com".to_string(),
        );

        let url = format!("{}/api/v2/logs/events/search", dd.api_base);
        assert_eq!(url, "https://api.datadoghq.com/api/v2/logs/events/search");
    }

    #[test]
    fn test_metrics_api_url() {
        let dd = Datadog::new(
            "test".to_string(),
            "test".to_string(),
            "datadoghq.com".to_string(),
        );

        let url = format!("{}/api/v1/metrics", dd.api_base);
        assert_eq!(url, "https://api.datadoghq.com/api/v1/metrics");
    }

    #[test]
    fn test_metric_url_encoding() {
        let dd = Datadog::new(
            "test".to_string(),
            "test".to_string(),
            "datadoghq.com".to_string(),
        );

        let metric_name = "system.cpu.usage";
        let encoded = urlencoding::encode(metric_name);
        let url = format!("https://app.{}/metric/explorer?metric={}", dd.site, encoded);
        assert_eq!(
            url,
            "https://app.datadoghq.com/metric/explorer?metric=system.cpu.usage"
        );

        // Test with special characters
        let metric_name2 = "custom.metric:rate";
        let encoded2 = urlencoding::encode(metric_name2);
        let url2 = format!(
            "https://app.{}/metric/explorer?metric={}",
            dd.site, encoded2
        );
        assert!(url2.contains("custom.metric%3Arate"));
    }

    use wiremock::matchers::{
        body_partial_json, header, method, path, query_param, query_param_is_missing,
    };
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Loads a recorded Datadog response from `tests/fixtures/datadog`.
    fn fixture(name: &str) -> serde_json::Value {
        let path = format!(
            "{}/tests/fixtures/datadog/{}",
            env!("CARGO_MANIFEST_DIR"),
            name
        );
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap()
    }

    fn mock_client(server: &MockServer) -> Datadog {
        let mut dd = Datadog::new(
            "test_api_key".to_string(),
            "test_app_key".to_string(),
            "datadoghq.com".to_string(),
        );
        dd.api_base = server.uri();
        dd
    }

    /// Matches the documented incident search contract: GET with a required `query`.
    fn incident_search(offset: &str) -> wiremock::MockBuilder {
        Mock::given(method("GET"))
            .and(path("/api/v2/incidents/search"))
            .and(header("DD-API-KEY", "test_api_key"))
            .and(header("DD-APPLICATION-KEY", "test_app_key"))
            .and(query_param("query", INCIDENT_SEARCH_QUERY))
            .and(query_param("sort", "-created"))
            .and(query_param("page[size]", "100"))
            .and(query_param("page[offset]", offset))
    }

    #[tokio::test]
    async fn test_get_incidents_follows_pagination() {
        let server = MockServer::start().await;
        incident_search("0")
            .respond_with(
                ResponseTemplate::new(200).set_body_json(fixture("incidents_search_page1.json")),
            )
            .expect(1)
            .mount(&server)
            .await;
        incident_search("2")
            .respond_with(
                ResponseTemplate::new(200).set_body_json(fixture("incidents_search_page2.json")),
            )
            .expect(1)
            .mount(&server)
            .await;
        incident_search("4")
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": {
                    "type": "incidents_search_results",
                    "attributes": {"facets": {}, "incidents": [], "total": 1703}
                },
                "meta": {"pagination": {"offset": 4, "next_offset": 4, "size": 0}}
            })))
            .expect(1)
            .mount(&server)
            .await;

        let docs = mock_client(&server)
            .get_incidents("2023-03-28T00:00:00Z", "2023-03-29T00:00:00Z")
            .await
            .unwrap();

        let ids: Vec<_> = docs.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "incident_aa819dbd-9016-5c31-84c5-48ff15b845cf",
                "incident_6f648ab1-026d-5e82-a49a-6b88e098b018",
                "incident_a262514b-6262-5be0-a72e-f0afd52b71c7",
            ]
        );

        let first = &docs[0];
        assert!(first.title.starts_with("Test-Go-"));
        assert_eq!(
            first.source_uri,
            "https://app.datadoghq.com/incidents/128961"
        );
        assert_eq!(
            first.timestamp.as_deref(),
            Some("2023-03-28T00:12:55+00:00")
        );
        assert_eq!(first.metadata["severity"], "UNKNOWN");
        assert_eq!(first.metadata["state"], "active");
        assert!(matches!(first.kind, crate::domain::SourceKind::Incident));
    }

    #[tokio::test]
    async fn test_get_incidents_stops_at_window_start() {
        let server = MockServer::start().await;
        incident_search("0")
            .respond_with(
                ResponseTemplate::new(200).set_body_json(fixture("incidents_search_page1.json")),
            )
            .expect(1)
            .mount(&server)
            .await;
        incident_search("2")
            .respond_with(
                ResponseTemplate::new(200).set_body_json(fixture("incidents_search_page2.json")),
            )
            .expect(1)
            .mount(&server)
            .await;
        // Page 2 is already older than the window, so no further page is requested.
        incident_search("4")
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;

        // Excludes the newest incident (after `to`) and the oldest (before `from`).
        let docs = mock_client(&server)
            .get_incidents("2023-03-28T00:12:50Z", "2023-03-28T00:12:53Z")
            .await
            .unwrap();

        let ids: Vec<_> = docs.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(ids, ["incident_6f648ab1-026d-5e82-a49a-6b88e098b018"]);
    }

    #[tokio::test]
    async fn test_get_incidents_rejects_unexpected_shape() {
        let server = MockServer::start().await;
        incident_search("0")
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"data": []})))
            .mount(&server)
            .await;

        let result = mock_client(&server)
            .get_incidents("2023-03-28T00:00:00Z", "2023-03-29T00:00:00Z")
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_get_incidents_error_status() {
        let server = MockServer::start().await;
        incident_search("0")
            .respond_with(ResponseTemplate::new(400))
            .mount(&server)
            .await;

        let result = mock_client(&server)
            .get_incidents("2023-03-28T00:00:00Z", "2023-03-29T00:00:00Z")
            .await;
        assert!(result.is_err());
    }

    #[test]
    fn test_incident_document_reads_field_values() {
        let dd = Datadog::new(
            "test".to_string(),
            "test".to_string(),
            "datadoghq.eu".to_string(),
        );
        let mut incident =
            fixture("incidents_search_page1.json")["data"]["attributes"]["incidents"][0]["data"]
                .clone();
        let fields = &mut incident["attributes"]["fields"];
        fields["services"]["value"] = serde_json::json!(["checkout", "payments"]);
        fields["env"] = serde_json::json!({"type": "dropdown", "value": "prod"});
        incident["attributes"]["customer_impact_scope"] = serde_json::json!("EU checkout");

        let doc = dd.incident_document(&incident, &Default::default());
        assert_eq!(doc.service, "checkout");
        assert_eq!(doc.environment, "prod");
        assert_eq!(doc.metadata["customer_impact"], "EU checkout");
        assert!(doc.text.contains("Customer Impact: EU checkout"));
        assert_eq!(doc.source_uri, "https://app.datadoghq.eu/incidents/128961");
    }

    #[tokio::test]
    async fn test_get_monitors_from_fixture() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/monitor"))
            .and(header("DD-API-KEY", "test_api_key"))
            .and(header("DD-APPLICATION-KEY", "test_app_key"))
            .and(query_param("page", "0"))
            .and(query_param("page_size", "1000"))
            .respond_with(ResponseTemplate::new(200).set_body_json(fixture("monitors.json")))
            .expect(1)
            .mount(&server)
            .await;

        let docs = mock_client(&server).get_monitors().await.unwrap();

        assert_eq!(docs.len(), 2);
        assert_eq!(docs[0].id, "monitor_34822915");
        assert_eq!(docs[0].title, "SLO Monitor: aws_alb_latency_p95 for splunk");
        assert_eq!(
            docs[0].source_uri,
            "https://app.datadoghq.com/monitors/34822915"
        );
    }

    /// A full page of `n` synthetic entries built by `entry(i)`.
    fn full_page(n: u64, entry: impl Fn(u64) -> serde_json::Value) -> serde_json::Value {
        serde_json::Value::Array((0..n).map(entry).collect())
    }

    #[tokio::test]
    async fn test_get_monitors_follows_pagination() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/monitor"))
            .and(query_param("page", "0"))
            .and(query_param("page_size", "1000"))
            .respond_with(ResponseTemplate::new(200).set_body_json(full_page(
                MONITOR_PAGE_SIZE,
                |i| serde_json::json!({"id": i + 1, "name": format!("monitor {}", i + 1)}),
            )))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/monitor"))
            .and(query_param("page", "1"))
            .and(query_param("page_size", "1000"))
            .respond_with(ResponseTemplate::new(200).set_body_json(fixture("monitors.json")))
            .expect(1)
            .mount(&server)
            .await;

        let docs = mock_client(&server).get_monitors().await.unwrap();

        assert_eq!(docs.len(), 1002);
        assert_eq!(docs[0].id, "monitor_1");
        assert_eq!(docs[1000].id, "monitor_34822915");
    }

    #[tokio::test]
    async fn test_get_monitors_error_handling() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/monitor"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&server)
            .await;

        assert!(mock_client(&server).get_monitors().await.is_err());
    }

    const LOGS_CURSOR_1: &str = "eyJhZnRlciI6IkFRQUFBWUFaZGJvSjdkR3dOZ0FBQUFCQldVRmFaR0p5TVVGQlEwczRPRU4zWW1aMVVESlJRVUUifQ";
    const LOGS_CURSOR_2: &str = "eyJhZnRlciI6IkFRQUFBWUFaZGJvSzdkR3dPUUFBQUFCQldVRmFaR0p5TVVGQlEwczRPRU4zWW1aMVVESlJRVVEifQ";

    /// Matches a log search for the given window, oldest first, at the maximum page size,
    /// carrying `cursor` (or no cursor for the first page).
    fn log_search(cursor: Option<&'static str>) -> wiremock::MockBuilder {
        Mock::given(method("POST"))
            .and(path("/api/v2/logs/events/search"))
            .and(header("DD-API-KEY", "test_api_key"))
            .and(header("DD-APPLICATION-KEY", "test_app_key"))
            .and(body_partial_json(serde_json::json!({
                "filter": {
                    "from": "2022-04-11T16:00:00Z",
                    "to": "2022-04-11T17:00:00Z",
                    "query": "status:error OR status:warn"
                },
                "page": {"limit": 1000},
                "sort": "timestamp"
            })))
            .and(move |req: &wiremock::Request| {
                let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
                body["page"]["cursor"].as_str() == cursor
            })
    }

    #[tokio::test]
    async fn test_search_logs_follows_cursor_pagination() {
        let server = MockServer::start().await;
        log_search(None)
            .respond_with(
                ResponseTemplate::new(200).set_body_json(fixture("logs_search_page1.json")),
            )
            .expect(1)
            .mount(&server)
            .await;
        log_search(Some(LOGS_CURSOR_1))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(fixture("logs_search_page2.json")),
            )
            .expect(1)
            .mount(&server)
            .await;
        log_search(Some(LOGS_CURSOR_2))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(fixture("logs_search_page3.json")),
            )
            .expect(1)
            .mount(&server)
            .await;

        let docs = mock_client(&server)
            .search_logs("2022-04-11T16:00:00Z", "2022-04-11T17:00:00Z")
            .await
            .unwrap();

        // Three logs from three pages, one per (status, pattern) group.
        assert_eq!(docs.len(), 3);
        let mut ids: Vec<&str> = docs
            .iter()
            .flat_map(|d| d.metadata["sample_log_ids"].as_array().unwrap())
            .map(|v| v.as_str().unwrap())
            .collect();
        ids.sort();
        assert_eq!(
            ids,
            [
                "AQAAAYAZdbh47dGwNwAAAABBWUFaZGJyMUFBQ0s4OEN3YmZ1UDJRQUI",
                "AQAAAYAZdboJ7dGwNgAAAABBWUFaZGJyMUFBQ0s4OEN3YmZ1UDJRQUE",
                "AQAAAYAZdboK7dGwOAAAAABBWUFaZGJyMUFBQ0s4OEN3YmZ1UDJRQUM",
            ]
        );
        for d in &docs {
            assert!(
                d.id.starts_with(crate::log_patterns::DOC_ID_PREFIX),
                "{}",
                d.id
            );
            assert_eq!(d.environment, "integrations-lab");
            assert_eq!(d.metadata["count"], 1);
        }
        let ok = docs.iter().find(|d| d.metadata["status"] == "ok").unwrap();
        assert_eq!(ok.timestamp.as_deref(), Some("2022-04-11T16:29:47.000Z"));
        assert_eq!(
            ok.metadata["pattern"],
            "#.#.#.# - - [#/Apr/#:#:#:# +#] \"GET / HTTP/#.#\" # # #.#"
        );
        assert!(
            ok.source_uri.starts_with(
                "https://app.datadoghq.com/logs?query=env%3Aintegrations-lab%20status%3Aok&from_ts="
            ),
            "{}",
            ok.source_uri
        );
        let last = docs
            .iter()
            .filter_map(|d| d.timestamp.as_deref())
            .max()
            .unwrap();
        assert_eq!(last, "2022-04-11T16:29:47.402Z");
    }

    /// Logs differing only in numbers and IDs become one document with their count.
    #[tokio::test]
    async fn test_search_logs_groups_repeated_messages() {
        let server = MockServer::start().await;
        let log = |id: &str, ts: &str, msg: &str| {
            serde_json::json!({"id": id, "type": "log", "attributes": {
                "service": "Checkout", "status": "error", "timestamp": ts,
                "message": msg, "tags": ["env:prod"]}})
        };
        let mut data: Vec<serde_json::Value> = (0..250)
            .map(|i| {
                log(
                    &format!("id-{i}"),
                    &format!("2022-04-11T16:{:02}:{:02}.000Z", i / 60, i % 60),
                    &format!(
                        "timeout after {}ms for order {:08x}-e29b-41d4-a716-446655440000",
                        900 + i,
                        i
                    ),
                )
            })
            .collect();
        data.push(log(
            "other",
            "2022-04-11T16:30:00.000Z",
            "db pool exhausted",
        ));
        data.push(log("no-ts", "not a time", "db pool exhausted"));
        Mock::given(method("POST"))
            .and(path("/api/v2/logs/events/search"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"data": data, "meta": {"page": {}}})),
            )
            .mount(&server)
            .await;

        let docs = mock_client(&server)
            .search_logs("2022-04-11T16:00:00Z", "2022-04-11T17:00:00Z")
            .await
            .unwrap();
        assert_eq!(docs.len(), 2);
        let timeouts = docs
            .iter()
            .find(|d| d.metadata["pattern"] == "timeout after #ms for order <uuid>")
            .unwrap();
        assert_eq!(timeouts.metadata["count"], 250);
        assert_eq!(timeouts.service, "checkout");
        assert_eq!(timeouts.metadata["first_seen"], "2022-04-11T16:00:00.000Z");
        assert_eq!(
            timeouts.timestamp.as_deref(),
            Some("2022-04-11T16:04:09.000Z")
        );
        assert_eq!(
            timeouts.metadata["samples"].as_array().unwrap().len(),
            crate::log_patterns::MAX_SAMPLES
        );
        let pool = docs.iter().find(|d| d.id != timeouts.id).unwrap();
        assert_eq!(
            pool.metadata["count"], 1,
            "the log without a timestamp is skipped"
        );
    }

    #[tokio::test]
    async fn test_search_logs_stops_without_next_cursor() {
        let server = MockServer::start().await;
        let mut body = fixture("logs_search_page1.json");
        body.as_object_mut().unwrap().remove("meta");
        log_search(None)
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .expect(1)
            .mount(&server)
            .await;

        let docs = mock_client(&server)
            .search_logs("2022-04-11T16:00:00Z", "2022-04-11T17:00:00Z")
            .await
            .unwrap();
        let counted: u64 = docs
            .iter()
            .map(|d| d.metadata["count"].as_u64().unwrap())
            .sum();
        assert_eq!(counted, 2);
    }

    #[tokio::test]
    async fn test_search_logs_rejects_repeated_cursor() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v2/logs/events/search"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(fixture("logs_search_page1.json")),
            )
            .expect(2)
            .mount(&server)
            .await;

        let result = mock_client(&server)
            .search_logs("2022-04-11T16:00:00Z", "2022-04-11T17:00:00Z")
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_search_logs_error_on_later_page() {
        let server = MockServer::start().await;
        log_search(None)
            .respond_with(
                ResponseTemplate::new(200).set_body_json(fixture("logs_search_page1.json")),
            )
            .mount(&server)
            .await;
        log_search(Some(LOGS_CURSOR_1))
            .respond_with(ResponseTemplate::new(429))
            .mount(&server)
            .await;

        // A partial result must not be reported as success.
        let result = mock_client(&server)
            .search_logs("2022-04-11T16:00:00Z", "2022-04-11T17:00:00Z")
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_search_logs_prefers_service_attribute() {
        let server = MockServer::start().await;
        let mut body = fixture("logs_search_page1.json");
        body.as_object_mut().unwrap().remove("meta");
        body["data"][0]["attributes"]["service"] = serde_json::json!("sinatra-app");
        Mock::given(method("POST"))
            .and(path("/api/v2/logs/events/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;

        let docs = mock_client(&server)
            .search_logs("2022-04-11T16:00:00Z", "2022-04-11T17:00:00Z")
            .await
            .unwrap();

        let service_of = |status: &str| {
            docs.iter()
                .find(|d| d.metadata["status"] == status)
                .unwrap()
                .service
                .clone()
        };
        assert_eq!(service_of("ok"), "sinatra-app");
        assert_eq!(service_of("info"), "");
    }

    #[tokio::test]
    async fn test_list_dashboards_from_fixture() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/dashboard"))
            .and(query_param("start", "0"))
            .and(query_param("count", "100"))
            .respond_with(ResponseTemplate::new(200).set_body_json(fixture("dashboards.json")))
            .expect(1)
            .mount(&server)
            .await;
        // Each listed dashboard's definition is fetched (a recorded `GET` body).
        Mock::given(method("GET"))
            .and(path("/api/v1/dashboard/npw-6di-usv"))
            .and(header("DD-API-KEY", "test_api_key"))
            .and(header("DD-APPLICATION-KEY", "test_app_key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(fixture("dashboard_get.json")))
            .expect(1)
            .mount(&server)
            .await;

        let docs = mock_client(&server).list_dashboards().await.unwrap();

        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].id, "dashboard_npw-6di-usv");
        // A null description leaves the title, then the widget's query.
        assert_eq!(
            docs[0].text,
            format!("{}\n\nWidgets:\n- runtime:jvm", docs[0].title)
        );
        assert_eq!(docs[0].metadata["author"], "frog@datadoghq.com");
        assert_eq!(
            docs[0].metadata["modified_at"],
            "2023-02-16T21:47:50.216943+00:00"
        );
        assert_eq!(docs[0].service, "");
    }

    #[tokio::test]
    async fn test_list_metrics_sends_only_from() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/metrics"))
            .and(query_param("from", "1704067200"))
            .and(query_param_is_missing("to"))
            .respond_with(ResponseTemplate::new(200).set_body_json(fixture("metrics.json")))
            .mount(&server)
            .await;

        let docs = mock_client(&server)
            .list_metrics("2024-01-01T00:00:00Z", "2024-01-02T00:00:00Z")
            .await
            .unwrap();

        let ids: Vec<_> = docs.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "metric_system_cpu_idle",
                "metric_system_mem_free",
                "metric_aws_ec2_cpuutilization"
            ]
        );
    }

    #[tokio::test]
    async fn test_list_slos_reports_target_percentage() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/slo"))
            .and(query_param("offset", "0"))
            .and(query_param("limit", "1000"))
            .respond_with(ResponseTemplate::new(200).set_body_json(fixture("slos.json")))
            .expect(1)
            .mount(&server)
            .await;

        let docs = mock_client(&server).list_slos().await.unwrap();

        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].id, "slo_c2ce7fb6030c5c0b8035d1ce94dec12c");
        assert!(docs[0].text.contains("Target: 95%"), "{}", docs[0].text);
        assert_eq!(docs[0].metadata["target"], 95.0);
    }

    #[tokio::test]
    async fn test_list_dashboards_follows_pagination() {
        let server = MockServer::start().await;
        let mut first = fixture("dashboards.json");
        first["dashboards"] = full_page(
            DASHBOARD_PAGE_SIZE,
            |i| serde_json::json!({"id": format!("dash-{}", i), "title": format!("Dashboard {}", i)}),
        );
        Mock::given(method("GET"))
            .and(path("/api/v1/dashboard"))
            .and(query_param("start", "0"))
            .and(query_param("count", "100"))
            .respond_with(ResponseTemplate::new(200).set_body_json(first))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/dashboard"))
            .and(query_param("start", "100"))
            .and(query_param("count", "100"))
            .respond_with(ResponseTemplate::new(200).set_body_json(fixture("dashboards.json")))
            .expect(1)
            .mount(&server)
            .await;

        let docs = mock_client(&server).list_dashboards().await.unwrap();

        assert_eq!(docs.len(), 101);
        assert_eq!(docs[100].id, "dashboard_npw-6di-usv");
    }

    #[tokio::test]
    async fn test_list_slos_follows_pagination() {
        let server = MockServer::start().await;
        let mut first = fixture("slos.json");
        first["data"] = full_page(
            SLO_PAGE_SIZE,
            |i| serde_json::json!({"id": format!("slo{}", i), "name": format!("SLO {}", i)}),
        );
        Mock::given(method("GET"))
            .and(path("/api/v1/slo"))
            .and(query_param("offset", "0"))
            .and(query_param("limit", "1000"))
            .respond_with(ResponseTemplate::new(200).set_body_json(first))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/slo"))
            .and(query_param("offset", "1000"))
            .and(query_param("limit", "1000"))
            .respond_with(ResponseTemplate::new(200).set_body_json(fixture("slos.json")))
            .expect(1)
            .mount(&server)
            .await;

        let docs = mock_client(&server).list_slos().await.unwrap();

        assert_eq!(docs.len(), 1001);
        assert_eq!(docs[1000].id, "slo_c2ce7fb6030c5c0b8035d1ce94dec12c");
    }

    fn utc(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    #[tokio::test]
    async fn test_query_metrics_sends_documented_request_and_parses_fixture() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/query"))
            .and(header("DD-API-KEY", "test_api_key"))
            .and(header("DD-APPLICATION-KEY", "test_app_key"))
            .and(query_param("from", "1641343852"))
            .and(query_param("to", "1641430252"))
            .and(query_param("query", "system.cpu.idle{*}"))
            .respond_with(ResponseTemplate::new(200).set_body_json(fixture("metrics_query.json")))
            .expect(1)
            .mount(&server)
            .await;

        let series = mock_client(&server)
            .query_metrics(
                "system.cpu.idle{*}",
                utc("2022-01-05T00:50:52Z"),
                utc("2022-01-06T00:50:52Z"),
            )
            .await
            .unwrap();
        assert_eq!(series.len(), 1);
        let s = &series[0];
        assert_eq!(s.metric, "system.cpu.idle");
        assert_eq!(s.expression, "system.cpu.idle{*}");
        assert_eq!(s.interval_secs, Some(300));
        assert_eq!(s.points.len(), 288);
        assert_eq!(s.points[0], (1641344100000, Some(91.4583840476142)));
    }

    #[tokio::test]
    async fn test_query_metrics_retries_5xx_and_rejects_error_status() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/query"))
            .respond_with(ResponseTemplate::new(503))
            .up_to_n_times(1)
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/query"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"status": "error", "error": "bad query"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let mut dd = mock_client(&server);
        dd.retry = RetryPolicy {
            max_attempts: 2,
            base_delay: std::time::Duration::from_millis(1),
            max_delay: std::time::Duration::from_millis(10),
        };
        let err = dd
            .query_metrics(
                "x{*}",
                utc("2022-01-05T00:00:00Z"),
                utc("2022-01-06T00:00:00Z"),
            )
            .await
            .unwrap_err();
        assert_eq!(err.code(), "live_evidence_failed");
        assert_eq!(err.stage(), Some(Stage::LiveEvidence));
    }

    #[tokio::test]
    async fn test_search_log_events_sends_query_and_stops_at_cap() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v2/logs/events/search"))
            .and(header("DD-API-KEY", "test_api_key"))
            .and(header("DD-APPLICATION-KEY", "test_app_key"))
            .and(body_partial_json(serde_json::json!({
                "filter": {
                    "from": "2022-04-11T16:00:00Z",
                    "to": "2022-04-11T17:00:00Z",
                    "query": "service:auth-api status:(error OR warn)"
                },
                "page": {"limit": 2},
                "sort": "timestamp"
            })))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(fixture("logs_search_page1.json")),
            )
            .expect(1)
            .mount(&server)
            .await;

        let logs = mock_client(&server)
            .search_log_events(
                "service:auth-api status:(error OR warn)",
                utc("2022-04-11T16:00:00Z"),
                utc("2022-04-11T17:00:00Z"),
                2,
            )
            .await
            .unwrap();
        // The page carried a next cursor, but the cap was reached.
        assert_eq!(logs.events.len(), 2);
        assert!(logs.truncated);
    }

    #[tokio::test]
    async fn test_search_log_events_follows_cursor_until_last_page() {
        let server = MockServer::start().await;
        log_search(None)
            .respond_with(
                ResponseTemplate::new(200).set_body_json(fixture("logs_search_page1.json")),
            )
            .mount(&server)
            .await;
        log_search(Some(LOGS_CURSOR_1))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(fixture("logs_search_page2.json")),
            )
            .mount(&server)
            .await;
        log_search(Some(LOGS_CURSOR_2))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(fixture("logs_search_page3.json")),
            )
            .mount(&server)
            .await;
        let logs = mock_client(&server)
            .search_log_events(
                "status:error OR status:warn",
                utc("2022-04-11T16:00:00Z"),
                utc("2022-04-11T17:00:00Z"),
                5000,
            )
            .await
            .unwrap();
        assert_eq!(logs.events.len(), 3);
        assert!(!logs.truncated);
    }
}
