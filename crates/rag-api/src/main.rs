use anyhow::Result;
use axum::{Json, Router, extract::State, routing::post};
use rag_core::{
    domain::Hit, openai::OpenAiClient, planner, qdrant::Qdrant, rag_service::answer_question,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::net::SocketAddr;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Clone)]
struct AppState {
    oa: OpenAiClient,
    qd: Qdrant,
}

#[derive(Deserialize)]
struct PlanReq {
    question: String,
}

#[derive(Serialize)]
struct PlanResp {
    plan: planner::QueryPlan,
}

#[derive(Deserialize)]
struct AskReq {
    question: String,
    env: Option<String>,
    service: Option<String>,
    #[allow(dead_code)]
    from_utc: Option<String>,
    #[allow(dead_code)]
    to_utc: Option<String>,
    #[allow(dead_code)]
    filters: Option<Vec<String>>,
    rewritten_query: Option<String>,
}

#[derive(Debug, Clone)]
struct TopKConfig {
    fixed: Option<usize>,
    default: usize,
    max: usize,
}

impl TopKConfig {
    fn from_env() -> Self {
        Self {
            fixed: std::env::var("RAG_TOPK_FIXED")
                .ok()
                .and_then(|v| v.parse().ok()),
            default: std::env::var("RAG_TOPK_DEFAULT")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(16),
            max: std::env::var("RAG_TOPK_MAX")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(32),
        }
    }

    #[cfg(test)]
    fn new(fixed: Option<usize>, default: usize, max: usize) -> Self {
        Self {
            fixed,
            default,
            max,
        }
    }
}

fn choose_topk_with_config(q: &str, config: &TopKConfig) -> usize {
    // Optional fixed override
    if let Some(n) = config.fixed {
        return n.clamp(1, 64);
    }

    let q_lc = q.to_lowercase();
    let is_rca = q_lc.contains("why") || q_lc.contains("root cause") || q_lc.contains("rca");
    let is_range = q_lc.contains("yesterday")
        || q_lc.contains(":")
        || q_lc.contains(" from ")
        || q_lc.contains(" to ");
    let is_inc = q_lc.contains("incident") || q_lc.contains("sev") || q_lc.contains("severity");

    let mut k = config.default;
    if is_rca {
        k = (k + 6).min(config.max);
    } // more breadth for RCA
    if is_range {
        k = (k + 2).min(config.max);
    } // a bit more for explicit windows
    if is_inc {
        k = (k + 2).min(config.max);
    } // incidents often need more signals
    k
}

fn choose_topk(q: &str) -> usize {
    choose_topk_with_config(q, &TopKConfig::from_env())
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .with(tracing_subscriber::fmt::layer())
        .init();

    let state = AppState {
        oa: OpenAiClient::new_from_env()?,
        qd: Qdrant::new_from_env()?,
    };

    let app = Router::new()
        .route("/ask/plan", post(plan))
        .route("/ask", post(ask))
        .with_state(state);

    let addr: SocketAddr = "0.0.0.0:5191".parse().unwrap();
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("listening on {}", addr);
    axum::serve(listener, app).await?;
    Ok(())
}

async fn plan(State(st): State<AppState>, Json(req): Json<PlanReq>) -> Json<PlanResp> {
    let p = planner::plan_query(&st.oa, &req.question)
        .await
        .unwrap_or_else(|_| planner::QueryPlan {
            intent: planner::Intent::Unknown,
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
        });
    Json(PlanResp { plan: p })
}

async fn ask(State(st): State<AppState>, Json(req): Json<AskReq>) -> Json<serde_json::Value> {
    // Build Qdrant filter (basic: service/env)
    let mut must = vec![];
    if let Some(env) = &req.env {
        must.push(json!({"key":"Environment","match":{"value":env}}));
    }
    if let Some(svc) = &req.service {
        must.push(json!({"key":"Service","match":{"value":svc}}));
    }
    let filter = if must.is_empty() {
        None
    } else {
        Some(json!({ "must": must }))
    };

    // Embed rewritten or original question
    let query = req
        .rewritten_query
        .as_ref()
        .unwrap_or(&req.question)
        .clone();
    let vec = st.oa.embed(&query).await.unwrap_or_default();

    // Retrieve generously; cut by server-side topK after rerank
    let search_limit = std::env::var("RAG_SEARCH_CANDIDATES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(64);
    let top_k = choose_topk(&req.question);

    let candidates = st
        .qd
        .search(vec, search_limit, filter)
        .await
        .unwrap_or_else(|_| Vec::<Hit>::new());

    let answer = answer_question(&st.oa, candidates, top_k, &req.question)
        .await
        .unwrap_or_else(|e| format!("Error: {e}"));

    Json(json!({ "answer": answer }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_choose_topk_default() {
        let config = TopKConfig::new(None, 16, 32);
        let k = choose_topk_with_config("simple query", &config);
        assert_eq!(k, 16); // default
    }

    #[test]
    fn test_choose_topk_rca_query() {
        let config = TopKConfig::new(None, 16, 32);

        let k = choose_topk_with_config("why did the service fail?", &config);
        assert_eq!(k, 22); // default 16 + 6 for RCA

        let k = choose_topk_with_config("what is the root cause?", &config);
        assert_eq!(k, 22);

        let k = choose_topk_with_config("RCA analysis needed", &config);
        assert_eq!(k, 22);
    }

    #[test]
    fn test_choose_topk_time_range_query() {
        let config = TopKConfig::new(None, 16, 32);

        let k = choose_topk_with_config("errors yesterday", &config);
        assert_eq!(k, 18); // default 16 + 2 for range

        let k = choose_topk_with_config("logs from 10:00 to 11:00", &config);
        assert_eq!(k, 18);
    }

    #[test]
    fn test_choose_topk_incident_query() {
        let config = TopKConfig::new(None, 16, 32);

        let k = choose_topk_with_config("show me the incident", &config);
        assert_eq!(k, 18); // default 16 + 2 for incident

        let k = choose_topk_with_config("sev1 issues", &config);
        assert_eq!(k, 18);

        let k = choose_topk_with_config("severity 2", &config);
        assert_eq!(k, 18);
    }

    #[test]
    fn test_choose_topk_combined_patterns() {
        let config = TopKConfig::new(None, 16, 32);

        // RCA + time range + incident should add all bonuses
        let k = choose_topk_with_config("why did the incident happen yesterday?", &config);
        // default 16 + 6 (RCA) + 2 (range) + 2 (incident) = 26
        assert_eq!(k, 26);
    }

    #[test]
    fn test_choose_topk_respects_max() {
        let config = TopKConfig::new(None, 16, 20);

        // RCA would add 6, but max is 20
        let k = choose_topk_with_config("why did this fail?", &config);
        assert_eq!(k, 20);
    }

    #[test]
    fn test_choose_topk_custom_default() {
        let config = TopKConfig::new(None, 10, 32);

        let k = choose_topk_with_config("simple query", &config);
        assert_eq!(k, 10);
    }

    #[test]
    fn test_choose_topk_fixed_override() {
        let config = TopKConfig::new(Some(25), 16, 32);

        // Fixed should override all patterns
        let k = choose_topk_with_config("why did the incident happen yesterday?", &config);
        assert_eq!(k, 25);
    }

    #[test]
    fn test_choose_topk_fixed_clamped() {
        let config = TopKConfig::new(Some(100), 16, 32);

        let k = choose_topk_with_config("any query", &config);
        assert_eq!(k, 64); // clamped to max 64

        let config = TopKConfig::new(Some(0), 16, 32);
        let k = choose_topk_with_config("any query", &config);
        assert_eq!(k, 1); // clamped to min 1
    }

    #[test]
    fn test_choose_topk_case_insensitive() {
        let config = TopKConfig::new(None, 16, 32);

        let k = choose_topk_with_config("WHY DID THIS FAIL?", &config);
        assert_eq!(k, 22); // RCA pattern should still match

        let k = choose_topk_with_config("INCIDENT YESTERDAY", &config);
        assert_eq!(k, 20); // incident + range
    }
}
