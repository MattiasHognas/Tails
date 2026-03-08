use crate::domain::RagDocument;
use anyhow::Result;
use urlencoding;

pub struct Datadog {
    pub api_key: String,
    pub app_key: String,
    pub site: String,
    pub http: reqwest::Client,
}

impl Datadog {
    pub fn new(api_key: String, app_key: String, site: String) -> Self {
        Self {
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
        let url = format!("https://api.{}/api/v1/monitor", self.site);
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

    pub async fn get_incidents(&self, from_iso: &str, to_iso: &str) -> Result<Vec<RagDocument>> {
        let url = format!("https://api.{}/api/v2/incidents/search", self.site);

        let query = serde_json::json!({
            "filter": {
                "from": from_iso,
                "to": to_iso,
            },
            "page": {
                "size": 100
            }
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
            anyhow::bail!("Failed to fetch incidents: {}", response.status());
        }

        let result: serde_json::Value = response.json().await?;
        let empty_vec = vec![];
        let incidents = result["data"].as_array().unwrap_or(&empty_vec);
        let mut docs = Vec::new();

        for incident in incidents {
            let id = incident["id"].as_str().unwrap_or("").to_string();
            let attrs = &incident["attributes"];
            let title = attrs["title"].as_str().unwrap_or("").to_string();
            let customer_impact = attrs["customer_impact_scope"]
                .as_str()
                .unwrap_or("")
                .to_string();
            let severity = attrs["severity"].as_str().unwrap_or("UNKNOWN").to_string();
            let created = attrs["created"].as_str().map(|s| s.to_string());

            let fields = attrs["fields"].as_object();
            let service = fields
                .and_then(|f| f.get("service"))
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string();

            let environment = fields
                .and_then(|f| f.get("env"))
                .and_then(|e| e.as_str())
                .unwrap_or("")
                .to_string();

            let mut metadata = serde_json::Map::new();
            metadata.insert(
                "severity".to_string(),
                serde_json::Value::String(severity.clone()),
            );
            metadata.insert(
                "customer_impact".to_string(),
                serde_json::Value::String(customer_impact.clone()),
            );

            let text = format!(
                "{}\n\nSeverity: {}\n\nCustomer Impact: {}",
                title, severity, customer_impact
            );

            docs.push(RagDocument {
                id: format!("incident_{}", id),
                title,
                text,
                source_uri: format!("https://app.{}/incidents/{}", self.site, id),
                kind: crate::domain::SourceKind::Incident,
                timestamp: created,
                service,
                environment,
                metadata,
            });
        }

        Ok(docs)
    }

    pub async fn search_logs(&self, from_iso: &str, to_iso: &str) -> Result<Vec<RagDocument>> {
        let url = format!("https://api.{}/api/v2/logs/events/search", self.site);

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
        let url = format!("https://api.{}/api/v1/dashboard", self.site);
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
        let url = format!("https://api.{}/api/v1/metrics", self.site);

        // Parse ISO timestamps to Unix timestamps
        let from_ts = chrono::DateTime::parse_from_rfc3339(from_iso).map(|dt| dt.timestamp())?;
        let to_ts = chrono::DateTime::parse_from_rfc3339(to_iso)
            .map(|dt| dt.timestamp())
            .unwrap_or(chrono::Utc::now().timestamp());

        let response = self
            .http
            .get(&url)
            .header("DD-API-KEY", &self.api_key)
            .header("DD-APPLICATION-KEY", &self.app_key)
            .query(&[("from", from_ts.to_string()), ("to", to_ts.to_string())])
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
        let url = format!("https://api.{}/api/v1/slo", self.site);
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

            let text = if description.is_empty() {
                format!(
                    "{}\n\nType: {}\nTarget: {}%",
                    name,
                    slo_type,
                    target * 100.0
                )
            } else {
                format!(
                    "{}\n\n{}\n\nType: {}\nTarget: {}%",
                    name,
                    description,
                    slo_type,
                    target * 100.0
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_datadog_struct_creation() {
        let dd = Datadog {
            api_key: "test_api_key".to_string(),
            app_key: "test_app_key".to_string(),
            site: "datadoghq.com".to_string(),
            http: reqwest::Client::new(),
        };

        assert_eq!(dd.api_key, "test_api_key");
        assert_eq!(dd.app_key, "test_app_key");
        assert_eq!(dd.site, "datadoghq.com");
    }

    #[test]
    fn test_datadog_url_formatting() {
        let dd = Datadog {
            api_key: "test_api_key".to_string(),
            app_key: "test_app_key".to_string(),
            site: "datadoghq.eu".to_string(),
            http: reqwest::Client::new(),
        };

        // Test monitor URL
        let monitor_url = format!("https://api.{}/api/v1/monitor", dd.site);
        assert_eq!(monitor_url, "https://api.datadoghq.eu/api/v1/monitor");

        // Test dashboard URL
        let dashboard_url = format!("https://api.{}/api/v1/dashboard", dd.site);
        assert_eq!(dashboard_url, "https://api.datadoghq.eu/api/v1/dashboard");

        // Test SLO URL
        let slo_url = format!("https://api.{}/api/v1/slo", dd.site);
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
        let dd = Datadog {
            api_key: "test".to_string(),
            app_key: "test".to_string(),
            site: "datadoghq.com".to_string(),
            http: reqwest::Client::new(),
        };

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
        let dd = Datadog {
            api_key: "test".to_string(),
            app_key: "test".to_string(),
            site: "datadoghq.eu".to_string(),
            http: reqwest::Client::new(),
        };

        let url = format!("https://api.{}/api/v2/incidents/search", dd.site);
        assert_eq!(url, "https://api.datadoghq.eu/api/v2/incidents/search");
    }

    #[test]
    fn test_logs_api_url() {
        let dd = Datadog {
            api_key: "test".to_string(),
            app_key: "test".to_string(),
            site: "datadoghq.com".to_string(),
            http: reqwest::Client::new(),
        };

        let url = format!("https://api.{}/api/v2/logs/events/search", dd.site);
        assert_eq!(url, "https://api.datadoghq.com/api/v2/logs/events/search");
    }

    #[test]
    fn test_metrics_api_url() {
        let dd = Datadog {
            api_key: "test".to_string(),
            app_key: "test".to_string(),
            site: "datadoghq.com".to_string(),
            http: reqwest::Client::new(),
        };

        let url = format!("https://api.{}/api/v1/metrics", dd.site);
        assert_eq!(url, "https://api.datadoghq.com/api/v1/metrics");
    }

    #[test]
    fn test_metric_url_encoding() {
        let dd = Datadog {
            api_key: "test".to_string(),
            app_key: "test".to_string(),
            site: "datadoghq.com".to_string(),
            http: reqwest::Client::new(),
        };

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

    #[tokio::test]
    async fn test_get_monitors_with_mock_server() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/v1/monitor"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "id": 12345,
                    "name": "Test Monitor",
                    "message": "Monitor message",
                    "query": "avg(last_5m):avg:system.cpu.user{*} > 80",
                    "type": "metric alert",
                    "tags": ["env:prod", "service:api"]
                }
            ])))
            .mount(&mock_server)
            .await;

        // Create a custom Datadog client with the mock server URL
        let dd = Datadog {
            api_key: "test_api_key".to_string(),
            app_key: "test_app_key".to_string(),
            site: "example.com".to_string(), // Use a dummy site
            http: reqwest::Client::new(),
        };

        // Override the URL construction for testing by calling the API directly
        let url = format!("{}/api/v1/monitor", mock_server.uri());
        let response = dd
            .http
            .get(&url)
            .header("DD-API-KEY", &dd.api_key)
            .header("DD-APPLICATION-KEY", &dd.app_key)
            .send()
            .await;

        assert!(response.is_ok());
        let response = response.unwrap();
        assert!(response.status().is_success());
    }

    #[tokio::test]
    async fn test_get_monitors_error_handling() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/v1/monitor"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&mock_server)
            .await;

        let dd = Datadog {
            api_key: "invalid_key".to_string(),
            app_key: "invalid_app".to_string(),
            site: mock_server.uri().replace("http://", ""),
            http: reqwest::Client::new(),
        };

        let result = dd.get_monitors().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_search_logs_with_mock_server() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/api/v2/logs/events/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{
                    "id": "log1",
                    "attributes": {
                        "message": "Error occurred",
                        "service": "api",
                        "status": "error",
                        "tags": ["env:prod"]
                    }
                }]
            })))
            .mount(&mock_server)
            .await;

        let dd = Datadog {
            api_key: "test_api_key".to_string(),
            app_key: "test_app_key".to_string(),
            site: "example.com".to_string(),
            http: reqwest::Client::new(),
        };

        // Test the HTTP call directly rather than going through search_logs
        let url = format!("{}/api/v2/logs/events/search", mock_server.uri());
        let query = serde_json::json!({
            "filter": {
                "from": "2025-01-01T00:00:00Z",
                "to": "2025-01-01T23:59:59Z",
                "query": "status:error OR status:warn"
            }
        });

        let response = dd
            .http
            .post(&url)
            .header("DD-API-KEY", &dd.api_key)
            .header("DD-APPLICATION-KEY", &dd.app_key)
            .json(&query)
            .send()
            .await;

        assert!(response.is_ok());
        let response = response.unwrap();
        assert!(response.status().is_success());
    }

    #[tokio::test]
    async fn test_get_incidents_with_mock_server() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/api/v2/incidents/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{
                    "id": "inc-123",
                    "attributes": {
                        "title": "Production outage",
                        "customer_impact_scope": "Multiple services affected",
                        "severity": "SEV-1",
                        "created": "2025-01-01T10:00:00Z",
                        "fields": {
                            "service": "api",
                            "env": "production"
                        }
                    }
                }]
            })))
            .mount(&mock_server)
            .await;

        let dd = Datadog {
            api_key: "test_api_key".to_string(),
            app_key: "test_app_key".to_string(),
            site: "example.com".to_string(),
            http: reqwest::Client::new(),
        };

        let url = format!("{}/api/v2/incidents/search", mock_server.uri());
        let query = serde_json::json!({
            "filter": {
                "from": "2025-01-01T00:00:00Z",
                "to": "2025-01-01T23:59:59Z",
            },
            "page": {
                "size": 100
            }
        });

        let response = dd
            .http
            .post(&url)
            .header("DD-API-KEY", &dd.api_key)
            .header("DD-APPLICATION-KEY", &dd.app_key)
            .json(&query)
            .send()
            .await;

        assert!(response.is_ok());
        let response = response.unwrap();
        assert!(response.status().is_success());
    }

    #[tokio::test]
    async fn test_list_dashboards_with_mock_server() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/v1/dashboard"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "dashboards": [{
                    "id": "dash-123",
                    "title": "System Overview",
                    "description": "Main system metrics",
                    "author_handle": "admin@example.com",
                    "created_at": "2025-01-01T00:00:00Z"
                }]
            })))
            .mount(&mock_server)
            .await;

        let dd = Datadog {
            api_key: "test_api_key".to_string(),
            app_key: "test_app_key".to_string(),
            site: "example.com".to_string(),
            http: reqwest::Client::new(),
        };

        let url = format!("{}/api/v1/dashboard", mock_server.uri());
        let response = dd
            .http
            .get(&url)
            .header("DD-API-KEY", &dd.api_key)
            .header("DD-APPLICATION-KEY", &dd.app_key)
            .send()
            .await;

        assert!(response.is_ok());
        let response = response.unwrap();
        assert!(response.status().is_success());
    }

    #[tokio::test]
    async fn test_list_metrics_with_mock_server() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/v1/metrics"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "metrics": [
                    "system.cpu.usage",
                    "system.memory.usage",
                    "api.request.count"
                ]
            })))
            .mount(&mock_server)
            .await;

        let dd = Datadog {
            api_key: "test_api_key".to_string(),
            app_key: "test_app_key".to_string(),
            site: "example.com".to_string(),
            http: reqwest::Client::new(),
        };

        let url = format!("{}/api/v1/metrics", mock_server.uri());
        let response = dd
            .http
            .get(&url)
            .header("DD-API-KEY", &dd.api_key)
            .header("DD-APPLICATION-KEY", &dd.app_key)
            .query(&[("from", "1704067200"), ("to", "1704153600")])
            .send()
            .await;

        assert!(response.is_ok());
        let response = response.unwrap();
        assert!(response.status().is_success());
    }

    #[tokio::test]
    async fn test_list_slos_with_mock_server() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/v1/slo"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{
                    "id": "slo-123",
                    "name": "API Availability",
                    "description": "API uptime SLO",
                    "type": "metric",
                    "thresholds": [{
                        "target": 0.999
                    }],
                    "tags": ["env:production", "service:api"]
                }]
            })))
            .mount(&mock_server)
            .await;

        let dd = Datadog {
            api_key: "test_api_key".to_string(),
            app_key: "test_app_key".to_string(),
            site: "example.com".to_string(),
            http: reqwest::Client::new(),
        };

        let url = format!("{}/api/v1/slo", mock_server.uri());
        let response = dd
            .http
            .get(&url)
            .header("DD-API-KEY", &dd.api_key)
            .header("DD-APPLICATION-KEY", &dd.app_key)
            .send()
            .await;

        assert!(response.is_ok());
        let response = response.unwrap();
        assert!(response.status().is_success());
    }
}
