use crate::domain::RagDocument;
use anyhow::Result;
use urlencoding;

/// Incidents are fetched in every state; the indexing window is applied to `created`.
const INCIDENT_SEARCH_QUERY: &str = "state:(active OR stable OR resolved)";
/// Maximum `page[size]` accepted by the incident search endpoint.
const INCIDENT_PAGE_SIZE: u64 = 100;

pub struct Datadog {
    pub api_key: String,
    pub app_key: String,
    pub site: String,
    /// Base URL for API requests, `https://api.{site}` unless overridden.
    pub api_base: String,
    pub http: reqwest::Client,
}

impl Datadog {
    pub fn new(api_key: String, app_key: String, site: String) -> Self {
        Self {
            api_base: format!("https://api.{}", site),
            api_key,
            app_key,
            site,
            http: reqwest::Client::new(),
        }
    }

    pub fn new_from_env() -> Result<Self> {
        Ok(Self::new(
            std::env::var("DD_API_KEY")?,
            std::env::var("DD_APP_KEY")?,
            std::env::var("DD_SITE").unwrap_or_else(|_| "datadoghq.com".into()),
        ))
    }

    pub async fn get_monitors(&self) -> Result<Vec<RagDocument>> {
        let url = format!("{}/api/v1/monitor", self.api_base);
        let response = self
            .http
            .get(&url)
            .header("DD-API-KEY", &self.api_key)
            .header("DD-APPLICATION-KEY", &self.app_key)
            .send()
            .await?;

        if !response.status().is_success() {
            anyhow::bail!("Failed to fetch monitors: {}", response.status());
        }

        let monitors: Vec<serde_json::Value> = response.json().await?;
        let mut docs = Vec::new();

        for monitor in monitors {
            let id = monitor["id"].as_i64().unwrap_or(0).to_string();
            let name = monitor["name"].as_str().unwrap_or("").to_string();
            let message = monitor["message"].as_str().unwrap_or("").to_string();
            let query = monitor["query"].as_str().unwrap_or("").to_string();
            let monitor_type = monitor["type"].as_str().unwrap_or("").to_string();

            let tags = monitor["tags"]
                .as_array()
                .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
                .unwrap_or_default();

            let service = tags
                .iter()
                .find(|t| t.starts_with("service:"))
                .and_then(|t| t.strip_prefix("service:"))
                .unwrap_or("")
                .to_string();

            let environment = tags
                .iter()
                .find(|t| t.starts_with("env:"))
                .and_then(|t| t.strip_prefix("env:"))
                .unwrap_or("")
                .to_string();

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

            docs.push(RagDocument {
                id: format!("monitor_{}", id),
                title: name,
                text,
                source_uri: format!("https://app.{}/monitors/{}", self.site, id),
                kind: crate::domain::SourceKind::Monitor,
                timestamp: None,
                service,
                environment,
                metadata,
            });
        }

        Ok(docs)
    }

    /// Fetches incidents created within `[from_iso, to_iso]` using
    /// `GET /api/v2/incidents/search`, newest first, following offset pagination
    /// until the results are older than the window.
    pub async fn get_incidents(&self, from_iso: &str, to_iso: &str) -> Result<Vec<RagDocument>> {
        let url = format!("{}/api/v2/incidents/search", self.api_base);
        let from = chrono::DateTime::parse_from_rfc3339(from_iso)?;
        let to = chrono::DateTime::parse_from_rfc3339(to_iso)?;

        let mut docs = Vec::new();
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
                    docs.push(self.incident_document(incident));
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

        Ok(docs)
    }

    fn incident_document(&self, incident: &serde_json::Value) -> RagDocument {
        let id = incident["id"].as_str().unwrap_or("").to_string();
        let attrs = &incident["attributes"];
        let fields = &attrs["fields"];
        let title = attrs["title"].as_str().unwrap_or("").to_string();
        let customer_impact = attrs["customer_impact_scope"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let severity = attrs["severity"]
            .as_str()
            .map(str::to_string)
            .or_else(|| incident_field(fields, "severity"))
            .unwrap_or_else(|| "UNKNOWN".to_string());
        let state = attrs["state"]
            .as_str()
            .map(str::to_string)
            .or_else(|| incident_field(fields, "state"))
            .unwrap_or_default();
        let created = attrs["created"].as_str().map(|s| s.to_string());
        let service = incident_field(fields, "services").unwrap_or_default();
        let environment = incident_field(fields, "env")
            .or_else(|| incident_field(fields, "environment"))
            .unwrap_or_default();

        let mut metadata = serde_json::Map::new();
        metadata.insert(
            "severity".to_string(),
            serde_json::Value::String(severity.clone()),
        );
        metadata.insert("state".to_string(), serde_json::Value::String(state));
        metadata.insert(
            "customer_impact".to_string(),
            serde_json::Value::String(customer_impact.clone()),
        );

        let text = format!(
            "{}\n\nSeverity: {}\n\nCustomer Impact: {}",
            title, severity, customer_impact
        );

        // The web UI addresses incidents by their numeric public ID.
        let app_id = attrs["public_id"]
            .as_u64()
            .map(|n| n.to_string())
            .unwrap_or_else(|| id.clone());

        RagDocument {
            id: format!("incident_{}", id),
            title,
            text,
            source_uri: format!("https://app.{}/incidents/{}", self.site, app_id),
            kind: crate::domain::SourceKind::Incident,
            timestamp: created,
            service,
            environment,
            metadata,
        }
    }

    pub async fn search_logs(&self, from_iso: &str, to_iso: &str) -> Result<Vec<RagDocument>> {
        let url = format!("{}/api/v2/logs/events/search", self.api_base);

        let query = serde_json::json!({
            "filter": {
                "from": from_iso,
                "to": to_iso,
                "query": "status:error OR status:warn"
            },
            "page": {
                "limit": 100
            },
            "sort": "-timestamp"
        });

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
        let empty_vec = vec![];
        let logs = result["data"].as_array().unwrap_or(&empty_vec);
        let mut docs = Vec::new();

        for log in logs {
            let id = log["id"].as_str().unwrap_or("").to_string();
            let attrs = &log["attributes"];
            let message = attrs["message"].as_str().unwrap_or("").to_string();
            let status = attrs["status"].as_str().unwrap_or("").to_string();
            let timestamp = attrs["timestamp"].as_str().map(|s| s.to_string());

            let tags = attrs["tags"]
                .as_array()
                .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
                .unwrap_or_default();

            // Logs carry the reserved `service` attribute; tags are only a fallback.
            let service = attrs["service"]
                .as_str()
                .filter(|s| !s.is_empty())
                .or_else(|| {
                    tags.iter()
                        .find(|t| t.starts_with("service:"))
                        .and_then(|t| t.strip_prefix("service:"))
                })
                .unwrap_or("")
                .to_string();

            let environment = tags
                .iter()
                .find(|t| t.starts_with("env:"))
                .and_then(|t| t.strip_prefix("env:"))
                .unwrap_or("")
                .to_string();

            let mut metadata = serde_json::Map::new();
            metadata.insert(
                "status".to_string(),
                serde_json::Value::String(status.clone()),
            );
            metadata.insert("tags".to_string(), serde_json::json!(tags));

            let title = format!("Log: {} - {}", service, status);

            docs.push(RagDocument {
                id: format!("log_{}", id),
                title,
                text: message,
                source_uri: format!("https://app.{}/logs?query=id:{}", self.site, id),
                kind: crate::domain::SourceKind::Logs,
                timestamp,
                service,
                environment,
                metadata,
            });
        }

        Ok(docs)
    }

    pub async fn list_dashboards(&self) -> Result<Vec<RagDocument>> {
        let url = format!("{}/api/v1/dashboard", self.api_base);
        let response = self
            .http
            .get(&url)
            .header("DD-API-KEY", &self.api_key)
            .header("DD-APPLICATION-KEY", &self.app_key)
            .send()
            .await?;

        if !response.status().is_success() {
            anyhow::bail!("Failed to fetch dashboards: {}", response.status());
        }

        let result: serde_json::Value = response.json().await?;
        let empty_vec = vec![];
        let dashboards = result["dashboards"].as_array().unwrap_or(&empty_vec);
        let mut docs = Vec::new();

        for dashboard in dashboards {
            let id = dashboard["id"].as_str().unwrap_or("").to_string();
            let title = dashboard["title"].as_str().unwrap_or("").to_string();
            let description = dashboard["description"].as_str().unwrap_or("").to_string();
            let author_handle = dashboard["author_handle"]
                .as_str()
                .unwrap_or("")
                .to_string();
            let created = dashboard["created_at"].as_str().map(|s| s.to_string());

            let mut metadata = serde_json::Map::new();
            metadata.insert(
                "author".to_string(),
                serde_json::Value::String(author_handle),
            );

            let text = if description.is_empty() {
                title.clone()
            } else {
                format!("{}\n\n{}", title, description)
            };

            docs.push(RagDocument {
                id: format!("dashboard_{}", id),
                title,
                text,
                source_uri: format!("https://app.{}/dashboard/{}", self.site, id),
                kind: crate::domain::SourceKind::Dashboard,
                timestamp: created,
                service: String::new(),
                environment: String::new(),
                metadata,
            });
        }

        Ok(docs)
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

    pub async fn list_slos(&self) -> Result<Vec<RagDocument>> {
        let url = format!("{}/api/v1/slo", self.api_base);
        let response = self
            .http
            .get(&url)
            .header("DD-API-KEY", &self.api_key)
            .header("DD-APPLICATION-KEY", &self.app_key)
            .send()
            .await?;

        if !response.status().is_success() {
            anyhow::bail!("Failed to fetch SLOs: {}", response.status());
        }

        let result: serde_json::Value = response.json().await?;
        let empty_vec = vec![];
        let slos = result["data"].as_array().unwrap_or(&empty_vec);
        let mut docs = Vec::new();

        for slo in slos {
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

            let service = tags
                .iter()
                .find(|t| t.starts_with("service:"))
                .and_then(|t| t.strip_prefix("service:"))
                .unwrap_or("")
                .to_string();

            let environment = tags
                .iter()
                .find(|t| t.starts_with("env:"))
                .and_then(|t| t.strip_prefix("env:"))
                .unwrap_or("")
                .to_string();

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

            docs.push(RagDocument {
                id: format!("slo_{}", id),
                title: name,
                text,
                source_uri: format!("https://app.{}/slo/{}", self.site, id),
                kind: crate::domain::SourceKind::SLO,
                timestamp: None,
                service,
                environment,
                metadata,
            });
        }

        Ok(docs)
    }
}

/// Reads an incident field (`{"type": ..., "value": ...}`) as a string, taking the
/// first entry of multi-value fields such as `services`.
fn incident_field(fields: &serde_json::Value, name: &str) -> Option<String> {
    let value = &fields[name]["value"];
    value
        .as_str()
        .or_else(|| value.as_array()?.first()?.as_str())
        .filter(|v| !v.is_empty())
        .map(str::to_string)
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
    fn test_tag_extraction_service() {
        let tags = ["env:production", "service:auth-api", "version:1.2.3"];

        let service = tags
            .iter()
            .find(|t| t.starts_with("service:"))
            .and_then(|t| t.strip_prefix("service:"))
            .unwrap_or("")
            .to_string();

        assert_eq!(service, "auth-api");
    }

    #[test]
    fn test_tag_extraction_environment() {
        let tags = ["service:api-service", "env:staging", "region:us-west-2"];

        let environment = tags
            .iter()
            .find(|t| t.starts_with("env:"))
            .and_then(|t| t.strip_prefix("env:"))
            .unwrap_or("")
            .to_string();

        assert_eq!(environment, "staging");
    }

    #[test]
    fn test_tag_extraction_missing_tags() {
        let tags = ["version:1.0.0", "region:eu-west-1"];

        let service = tags
            .iter()
            .find(|t| t.starts_with("service:"))
            .and_then(|t| t.strip_prefix("service:"))
            .unwrap_or("")
            .to_string();

        let environment = tags
            .iter()
            .find(|t| t.starts_with("env:"))
            .and_then(|t| t.strip_prefix("env:"))
            .unwrap_or("")
            .to_string();

        assert_eq!(service, "");
        assert_eq!(environment, "");
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

        let doc = dd.incident_document(&incident);
        assert_eq!(doc.service, "checkout");
        assert_eq!(doc.environment, "prod");
        assert_eq!(doc.metadata["customer_impact"], "EU checkout");
        assert!(doc.text.contains("Customer Impact: EU checkout"));
        assert_eq!(doc.source_uri, "https://app.datadoghq.eu/incidents/128961");
    }

    #[test]
    fn test_incident_field_handles_null_values() {
        let fields = &fixture("incidents_search_page1.json")["data"]["attributes"]["incidents"][0]
            ["data"]["attributes"]["fields"];
        assert_eq!(incident_field(fields, "services"), None);
        assert_eq!(incident_field(fields, "state").as_deref(), Some("active"));
        assert_eq!(incident_field(fields, "missing"), None);
    }

    #[tokio::test]
    async fn test_get_monitors_from_fixture() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/monitor"))
            .and(header("DD-API-KEY", "test_api_key"))
            .and(header("DD-APPLICATION-KEY", "test_app_key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(fixture("monitors.json")))
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

    #[tokio::test]
    async fn test_search_logs_from_fixture() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v2/logs/events/search"))
            .and(body_partial_json(serde_json::json!({
                "filter": {
                    "from": "2022-04-11T16:00:00Z",
                    "to": "2022-04-11T17:00:00Z",
                    "query": "status:error OR status:warn"
                },
                "page": {"limit": 100},
                "sort": "-timestamp"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(fixture("logs_search.json")))
            .mount(&server)
            .await;

        let docs = mock_client(&server)
            .search_logs("2022-04-11T16:00:00Z", "2022-04-11T17:00:00Z")
            .await
            .unwrap();

        assert_eq!(docs.len(), 2);
        assert_eq!(
            docs[0].id,
            "log_AQAAAYAZdbh47dGwNwAAAABBWUFaZGJyMUFBQ0s4OEN3YmZ1UDJRQUI"
        );
        assert_eq!(docs[0].environment, "integrations-lab");
        assert_eq!(docs[0].metadata["status"], "ok");
        assert_eq!(
            docs[0].timestamp.as_deref(),
            Some("2022-04-11T16:29:47.000Z")
        );
    }

    #[tokio::test]
    async fn test_search_logs_prefers_service_attribute() {
        let server = MockServer::start().await;
        let mut body = fixture("logs_search.json");
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

        assert_eq!(docs[0].service, "sinatra-app");
        assert_eq!(docs[1].service, "");
    }

    #[tokio::test]
    async fn test_list_dashboards_from_fixture() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/dashboard"))
            .respond_with(ResponseTemplate::new(200).set_body_json(fixture("dashboards.json")))
            .mount(&server)
            .await;

        let docs = mock_client(&server).list_dashboards().await.unwrap();

        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].id, "dashboard_npw-6di-usv");
        // A null description leaves only the title.
        assert_eq!(docs[0].text, docs[0].title);
        assert_eq!(docs[0].metadata["author"], "frog@datadoghq.com");
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
            .respond_with(ResponseTemplate::new(200).set_body_json(fixture("slos.json")))
            .mount(&server)
            .await;

        let docs = mock_client(&server).list_slos().await.unwrap();

        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].id, "slo_c2ce7fb6030c5c0b8035d1ce94dec12c");
        assert!(docs[0].text.contains("Target: 95%"), "{}", docs[0].text);
        assert_eq!(docs[0].metadata["target"], 95.0);
    }
}
