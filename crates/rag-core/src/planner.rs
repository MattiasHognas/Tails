use crate::openai::OpenAiClient;
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Intent {
    RootCauseWindow,
    IncidentSummary,
    MonitorExplanation,
    SemanticLogSearch,
    MetricQuestion,
    DashboardLookup,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeRange {
    pub from_utc: Option<String>,
    pub to_utc: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

pub async fn plan_query(oa: &OpenAiClient, user_query: &str) -> Result<QueryPlan> {
    let sys = r#"
You are a planning assistant for an SRE RAG over Datadog.
Return strictly valid JSON with:
  intent, service, environment, monitorId, incidentId, metric, sloId,
  window { fromUtc, toUtc } in UTC ISO-8601 if inferred,
  filters[], missingFields[], clarifyingQuestions[], rewrittenQuery.

Inference rules:
- Try to INFER `service` and `environment` from the user text and common Datadog tag patterns
  (e.g., "auth-api", "payment", "env:prod", "prod", "staging", "dev").
- If you can infer them confidently, fill `service` and/or `environment`.
- If not confident, add them to `missingFields` and include precise `clarifyingQuestions`.
- If a concrete time is mentioned (e.g., "yesterday 14:00-15:00 CET"),
  convert to UTC and set `window.fromUtc` and `window.toUtc`.
- Set `rewrittenQuery` to a crisp, search-friendly paraphrase (include inferred service/env words).
"#;
    let plan: QueryPlan = oa.chat_json(sys, user_query).await?;
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;

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
