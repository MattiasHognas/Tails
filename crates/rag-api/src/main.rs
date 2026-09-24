use anyhow::Result;
use axum::{
    Json, Router,
    extract::{State, rejection::JsonRejection},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
};
use rag_core::{
    domain::SourceKind,
    error::{RagError, Stage},
    openai::OpenAiClient,
    planner::{self, Clock, PlanContext, QueryPlan, SystemClock, Window},
    qdrant::Qdrant,
    rag_service::{StageTimeouts, retrieve_and_answer},
    resilience::{env_duration_ms, run_stage},
    retrieval::{ExplicitScope, RetrievalScope, normalize_filters},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Clone)]
struct AppState {
    oa: OpenAiClient,
    qd: Qdrant,
    limits: Limits,
    clock: Arc<dyn Clock>,
}

/// Request deadlines. Retry budgets live on the clients (`RetryPolicy`).
#[derive(Debug, Clone, Copy)]
struct Limits {
    /// Overall deadline for one `/ask` request (`RAG_ASK_DEADLINE_MS`).
    ask_deadline: Duration,
    stages: StageTimeouts,
}

impl Limits {
    fn from_env() -> Self {
        Self {
            ask_deadline: env_duration_ms("RAG_ASK_DEADLINE_MS", Duration::from_secs(90)),
            stages: StageTimeouts::from_env(),
        }
    }
}

/// Maps [`RagError`] to an HTTP status and a stable JSON body:
/// `{"error": {"code", "message", "stage", "retryable"}}`.
struct ApiError(RagError);

impl From<RagError> for ApiError {
    fn from(e: RagError) -> Self {
        Self(e)
    }
}

impl ApiError {
    fn status(&self) -> StatusCode {
        match &self.0 {
            RagError::InvalidRequest(_) => StatusCode::BAD_REQUEST,
            RagError::Timeout { .. } => StatusCode::GATEWAY_TIMEOUT,
            RagError::UpstreamUnavailable { .. } => StatusCode::SERVICE_UNAVAILABLE,
            RagError::EmbeddingFailed { .. }
            | RagError::RetrievalFailed { .. }
            | RagError::GenerationFailed { .. }
            | RagError::PlanningFailed { .. }
            | RagError::IndexingFailed { .. } => StatusCode::BAD_GATEWAY,
            RagError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let e = &self.0;
        let status = self.status();
        // Full cause chain goes to logs only; the body carries the sanitized
        // Display (no upstream bodies, URLs or credentials).
        let mut chain = String::new();
        let mut src = std::error::Error::source(e);
        while let Some(s) = src {
            chain.push_str(" <- ");
            chain.push_str(&s.to_string());
            src = s.source();
        }
        if status.is_server_error() {
            tracing::error!(code = e.code(), status = status.as_u16(), error = %e, cause = %chain, "request failed");
        } else {
            tracing::warn!(code = e.code(), status = status.as_u16(), error = %e, "request rejected");
        }
        let body = json!({
            "error": {
                "code": e.code(),
                "message": e.to_string(),
                "stage": e.stage(),
                "retryable": e.retryable(),
            }
        });
        (status, Json(body)).into_response()
    }
}

fn invalid_json(rejection: JsonRejection) -> ApiError {
    ApiError(RagError::InvalidRequest(rejection.body_text()))
}

fn require_question(q: &str) -> Result<(), ApiError> {
    if q.trim().is_empty() {
        return Err(ApiError(RagError::InvalidRequest(
            "question must not be empty".into(),
        )));
    }
    Ok(())
}

#[derive(Deserialize)]
struct PlanReq {
    question: String,
    /// IANA timezone used to resolve relative times; defaults to UTC.
    timezone: Option<String>,
}

#[derive(Serialize)]
struct PlanResp {
    plan: QueryPlan,
}

/// Explicit fields always win over values the planner infers. See README "/ask".
#[derive(Deserialize)]
struct AskReq {
    question: String,
    env: Option<String>,
    service: Option<String>,
    from_utc: Option<String>,
    to_utc: Option<String>,
    filters: Option<Vec<String>>,
    /// Restrict evidence to these source kinds (e.g. `logs`, `monitor`).
    kinds: Option<Vec<String>>,
    /// IANA timezone used to resolve relative times; defaults to UTC.
    timezone: Option<String>,
    rewritten_query: Option<String>,
    /// A plan from `/ask/plan`. When present the planner is not called again; it is
    /// still validated like planner output.
    plan: Option<Value>,
}

fn bad_request(msg: impl std::fmt::Display) -> ApiError {
    ApiError(RagError::InvalidRequest(msg.to_string()))
}

fn non_empty(s: &Option<String>) -> Option<String> {
    s.as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn plan_context(st: &AppState, timezone: Option<&str>) -> Result<PlanContext, ApiError> {
    let tz = planner::parse_timezone(timezone).map_err(bad_request)?;
    Ok(PlanContext::new(st.clock.now(), tz))
}

/// Validates caller-supplied constraints. Unlike planner output, invalid explicit
/// input is rejected with 400 instead of being silently dropped.
fn explicit_scope(req: &AskReq) -> Result<ExplicitScope, ApiError> {
    let bound = |name: &str, v: &Option<String>| {
        non_empty(v)
            .map(|s| {
                planner::parse_utc(&s)
                    .ok_or_else(|| bad_request(format!("{name} is not an RFC 3339 timestamp")))
            })
            .transpose()
    };
    let from = bound("from_utc", &req.from_utc)?;
    let to = bound("to_utc", &req.to_utc)?;
    if let (Some(f), Some(t)) = (from, to)
        && f >= t
    {
        return Err(bad_request("from_utc must be before to_utc"));
    }
    let kinds = req
        .kinds
        .as_ref()
        .map(|ks| {
            ks.iter()
                .map(|k| {
                    SourceKind::parse_lenient(k)
                        .ok_or_else(|| bad_request(format!("unknown source kind {k:?}")))
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?;
    Ok(ExplicitScope {
        service: non_empty(&req.service),
        environment: non_empty(&req.env),
        window: (from.is_some() || to.is_some()).then_some(Window { from, to }),
        kinds,
        filters: normalize_filters(req.filters.as_deref().unwrap_or_default()),
    })
}

/// Runs the planner within the planning stage timeout. Planner output is untrusted
/// and sanitized; a failed or timed-out planner call is an error, not a default plan.
async fn run_planner(
    st: &AppState,
    question: &str,
    ctx: &PlanContext,
) -> Result<QueryPlan, ApiError> {
    Ok(run_stage(
        Stage::Planning,
        st.limits.stages.planning,
        planner::plan_query(&st.oa, question, ctx),
    )
    .await?)
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

fn app(state: AppState) -> Router {
    Router::new()
        .route("/ask/plan", post(plan))
        .route("/ask", post(ask))
        .with_state(state)
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
        limits: Limits::from_env(),
        clock: Arc::new(SystemClock),
    };

    let addr: SocketAddr = "0.0.0.0:5191".parse().unwrap();
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("listening on {}", addr);
    axum::serve(listener, app(state)).await?;
    Ok(())
}

async fn plan(
    State(st): State<AppState>,
    req: Result<Json<PlanReq>, JsonRejection>,
) -> Result<Json<PlanResp>, ApiError> {
    let Json(req) = req.map_err(invalid_json)?;
    require_question(&req.question)?;
    let ctx = plan_context(&st, req.timezone.as_deref())?;
    let plan = run_planner(&st, &req.question, &ctx).await?;
    Ok(Json(PlanResp { plan }))
}

async fn ask(
    State(st): State<AppState>,
    req: Result<Json<AskReq>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    let Json(req) = req.map_err(invalid_json)?;
    require_question(&req.question)?;
    let ctx = plan_context(&st, req.timezone.as_deref())?;
    let explicit = explicit_scope(&req)?;

    // Planning, embedding, retrieval and generation share one overall deadline.
    let deadline = st.limits.ask_deadline;
    let result = run_stage(Stage::Request, deadline, async {
        let plan = match &req.plan {
            Some(raw) => planner::sanitize_plan(raw, &req.question, &ctx),
            None => run_planner(&st, &req.question, &ctx)
                .await
                .map_err(|e| e.0)?,
        };
        let scope = RetrievalScope::resolve(&explicit, &plan);
        tracing::info!(?scope, "retrieval scope");

        // Embed rewritten or original question
        let query = non_empty(&req.rewritten_query)
            .or_else(|| plan.rewritten_query.clone())
            .unwrap_or_else(|| req.question.clone());

        // Retrieve generously; cut by server-side topK after rerank
        let search_limit = std::env::var("RAG_SEARCH_CANDIDATES")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(64);
        let top_k = choose_topk(&req.question);

        // Embedding/retrieval failures are errors, never empty evidence; the LLM
        // is only called when retrieval succeeded with at least one hit.
        let outcome = retrieve_and_answer(
            &st.oa,
            &st.qd,
            &query,
            &req.question,
            scope.to_qdrant_filter(),
            search_limit,
            top_k,
            &st.limits.stages,
        )
        .await?;
        Ok((outcome, plan, scope))
    })
    .await;
    let (outcome, plan, scope) = result?;

    Ok(Json(json!({
        "answer": outcome.answer,
        "evidence": outcome.evidence,
        "plan": plan,
        "scope": scope.to_json(),
    })))
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

    mod e2e {
        use super::super::*;
        use chrono::{DateTime, Utc};
        use rag_core::planner::FixedClock;
        use wiremock::matchers::{body_partial_json, method, path};
        use wiremock::{Mock, MockServer, Request, ResponseTemplate};

        struct Harness {
            base: String,
            openai: MockServer,
            qdrant: MockServer,
        }

        /// Starts the API against mocked OpenAI and Qdrant with a fixed clock.
        async fn harness(now: &str, llm_plan: Value) -> Harness {
            let openai = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/v1/chat/completions"))
                .and(body_partial_json(
                    json!({"response_format": {"type": "json_object"}}),
                ))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "choices": [{"message": {"content": llm_plan.to_string()}}]
                })))
                .with_priority(1)
                .mount(&openai)
                .await;
            Mock::given(method("POST"))
                .and(path("/v1/chat/completions"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "choices": [{"message": {"content": "the answer"}}]
                })))
                .mount(&openai)
                .await;
            Mock::given(method("POST"))
                .and(path("/v1/embeddings"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(json!({"data": [{"embedding": [0.1, 0.2]}]})),
                )
                .mount(&openai)
                .await;

            let qdrant = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/collections/test/points/search"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"result": [{
                    "id": 1,
                    "score": 0.9,
                    "payload": {
                        "id": "incident_1#c0",
                        "Title": "auth-api 5xx spike",
                        "Text": "Error rate above 5%",
                        "SourceUri": "https://app.datadoghq.eu/incidents/1",
                        "Kind": "incident",
                        "Timestamp": "2026-09-23T10:00:00Z",
                        "Service": "auth-api",
                        "Environment": "prod"
                    }
                }]})))
                .mount(&qdrant)
                .await;

            let now: DateTime<Utc> = now.parse().unwrap();
            let state = AppState {
                oa: OpenAiClient::new("k".into(), openai.uri(), "e".into(), "c".into()),
                qd: Qdrant::new(qdrant.uri(), "test".into()),
                limits: Limits {
                    ask_deadline: Duration::from_secs(10),
                    stages: StageTimeouts::default(),
                },
                clock: Arc::new(FixedClock(now)),
            };
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            tokio::spawn(async move { axum::serve(listener, app(state)).await.unwrap() });
            Harness {
                base: format!("http://{addr}"),
                openai,
                qdrant,
            }
        }

        fn body(r: &Request) -> Value {
            serde_json::from_slice(&r.body).unwrap()
        }

        async fn post(base: &str, route: &str, req: Value) -> (StatusCode, Value) {
            let r = reqwest::Client::new()
                .post(format!("{base}{route}"))
                .json(&req)
                .send()
                .await
                .unwrap();
            (r.status(), r.json().await.unwrap())
        }

        async fn planner_calls(h: &Harness) -> usize {
            h.openai
                .received_requests()
                .await
                .unwrap()
                .iter()
                .filter(|r| body(r).get("response_format").is_some())
                .count()
        }

        #[tokio::test]
        async fn ask_plans_server_side_and_filters_qdrant_to_local_yesterday() {
            // The LLM infers entities but gets "yesterday" wrong (UTC day); the
            // server must use the Stockholm day instead.
            let h = harness(
                "2026-09-24T08:00:00Z",
                json!({
                    "intent": "rootCauseWindow",
                    "service": "auth-api",
                    "environment": "prod",
                    "window": {"fromUtc": "2026-09-23T00:00:00Z", "toUtc": "2026-09-24T00:00:00Z"},
                    "filters": ["kind:logs", "kind:incident"],
                    "rewrittenQuery": "auth-api prod errors"
                }),
            )
            .await;

            let (status, resp) = post(
                &h.base,
                "/ask",
                json!({"question": "why did auth-api fail yesterday?", "timezone": "Europe/Stockholm"}),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(resp["answer"], "the answer");
            assert_eq!(resp["scope"]["fromUtc"], "2026-09-22T22:00:00Z");
            assert_eq!(resp["scope"]["toUtc"], "2026-09-23T22:00:00Z");

            let searches = h.qdrant.received_requests().await.unwrap();
            assert_eq!(searches.len(), 1);
            assert_eq!(
                body(&searches[0])["filter"],
                json!({"must": [
                    {"key": "Service", "match": {"value": "auth-api"}},
                    {"key": "Environment", "match": {"value": "prod"}},
                    {"key": "Kind", "match": {"any": ["logs", "incident"]}},
                    {"should": [
                        {"key": "Timestamp", "range": {
                            "gte": "2026-09-22T22:00:00Z",
                            "lt": "2026-09-23T22:00:00Z"
                        }},
                        {"is_empty": {"key": "Timestamp"}},
                        {"key": "Kind", "match": {"any": ["monitor", "dashboard", "sLO"]}}
                    ]}
                ]})
            );

            // The planner prompt carried the user's local time and zone.
            let reqs = h.openai.received_requests().await.unwrap();
            let plan_req = reqs
                .iter()
                .find(|r| body(r).get("response_format").is_some())
                .unwrap();
            let system = body(plan_req)["messages"][0]["content"]
                .as_str()
                .unwrap()
                .to_string();
            assert!(system.contains("2026-09-24T10:00:00+02:00"));
            assert!(system.contains("Europe/Stockholm"));
            // The rewritten query was embedded.
            let embed = reqs
                .iter()
                .find(|r| r.url.path() == "/v1/embeddings")
                .unwrap();
            assert_eq!(body(embed)["input"], "auth-api prod errors");
        }

        #[tokio::test]
        async fn explicit_fields_override_inferred_ones() {
            let h = harness(
                "2026-09-24T08:00:00Z",
                json!({"service": "auth-api", "environment": "prod", "filters": ["kind:logs"]}),
            )
            .await;
            let (status, _) = post(
                &h.base,
                "/ask",
                json!({
                    "question": "auth-api errors yesterday",
                    "service": "payments",
                    "from_utc": "2026-09-20T00:00:00Z",
                    "to_utc": "2026-09-21T00:00:00Z",
                    "kinds": ["monitor"]
                }),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(planner_calls(&h).await, 1);
            let searches = h.qdrant.received_requests().await.unwrap();
            let must = body(&searches[0])["filter"]["must"].clone();
            assert_eq!(
                must[0],
                json!({"key": "Service", "match": {"value": "payments"}})
            );
            // Environment was not given explicitly, so the inferred value applies.
            assert_eq!(
                must[1],
                json!({"key": "Environment", "match": {"value": "prod"}})
            );
            assert_eq!(
                must[2],
                json!({"key": "Kind", "match": {"any": ["monitor"]}})
            );
            assert_eq!(
                must[3]["should"][0]["range"],
                json!({"gte": "2026-09-20T00:00:00Z", "lt": "2026-09-21T00:00:00Z"})
            );
        }

        #[tokio::test]
        async fn supplied_plan_skips_the_planner_but_is_still_validated() {
            let h = harness("2026-09-24T08:00:00Z", json!({})).await;
            let (status, resp) = post(
                &h.base,
                "/ask",
                json!({
                    "question": "checkout latency",
                    "plan": {"service": "checkout", "environment": "none",
                             "window": {"fromUtc": "garbage"}}
                }),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(planner_calls(&h).await, 0);
            assert_eq!(resp["scope"]["environment"], Value::Null);
            let searches = h.qdrant.received_requests().await.unwrap();
            assert_eq!(
                body(&searches[0])["filter"],
                json!({"must": [{"key": "Service", "match": {"value": "checkout"}}]})
            );
        }

        #[tokio::test]
        async fn invalid_explicit_input_is_rejected() {
            let h = harness("2026-09-24T08:00:00Z", json!({})).await;
            for req in [
                json!({"question": "q", "timezone": "Mars/Olympus"}),
                json!({"question": "q", "from_utc": "yesterday"}),
                json!({"question": "q", "from_utc": "2026-09-24T00:00:00Z", "to_utc": "2026-09-23T00:00:00Z"}),
                json!({"question": "q", "kinds": ["tweets"]}),
            ] {
                let (status, resp) = post(&h.base, "/ask", req.clone()).await;
                assert_eq!(status, StatusCode::BAD_REQUEST, "{req}");
                assert_eq!(resp["error"]["code"], "invalid_request");
            }
            assert!(h.qdrant.received_requests().await.unwrap().is_empty());
            let (status, _) = post(
                &h.base,
                "/ask/plan",
                json!({"question": "q", "timezone": "Nowhere/City"}),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
        }

        #[tokio::test]
        async fn plan_endpoint_returns_validated_plan() {
            let h = harness(
                "2026-09-24T08:00:00Z",
                json!({"intent": "bogus", "service": "Auth-API", "filters": ["status:5xx"]}),
            )
            .await;
            let (status, resp) = post(
                &h.base,
                "/ask/plan",
                json!({"question": "auth-api yesterday", "timezone": "Europe/Stockholm"}),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            let plan = &resp["plan"];
            assert_eq!(plan["intent"], "unknown");
            assert_eq!(plan["service"], "auth-api");
            assert_eq!(plan["filters"], json!([]));
            assert_eq!(plan["window"]["fromUtc"], "2026-09-22T22:00:00Z");
        }
    }
}

#[cfg(test)]
mod http_tests {
    use super::*;
    use rag_core::resilience::RetryPolicy;
    use std::time::Instant;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const EMBED: &str = "/v1/embeddings";
    const CHAT: &str = "/v1/chat/completions";
    const SEARCH: &str = "/collections/test/points/search";

    fn fast_retry(max_attempts: u32) -> RetryPolicy {
        RetryPolicy {
            max_attempts,
            base_delay: Duration::from_millis(5),
            max_delay: Duration::from_millis(200),
        }
    }

    fn limits(deadline: Duration) -> Limits {
        Limits {
            ask_deadline: deadline,
            stages: StageTimeouts {
                planning: Duration::from_secs(2),
                embedding: Duration::from_secs(2),
                retrieval: Duration::from_secs(2),
                generation: Duration::from_secs(2),
            },
        }
    }

    /// Serve the API on an ephemeral port against mocked OpenAI and Qdrant.
    async fn spawn_api(upstream: &MockServer, retry: RetryPolicy, limits: Limits) -> String {
        let mut oa = OpenAiClient::new(
            "sk-test-secret".into(),
            upstream.uri(),
            "embed-model".into(),
            "chat-model".into(),
        );
        oa.retry = retry;
        let mut qd = Qdrant::new(upstream.uri(), "test".into());
        qd.retry = retry;
        let state = AppState {
            oa,
            qd,
            limits,
            clock: Arc::new(SystemClock),
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app(state)).await.unwrap() });
        format!("http://{addr}")
    }

    async fn post(base: &str, route: &str, body: serde_json::Value) -> (u16, serde_json::Value) {
        let r = reqwest::Client::new()
            .post(format!("{base}{route}"))
            .json(&body)
            .send()
            .await
            .unwrap();
        let status = r.status().as_u16();
        (status, r.json().await.unwrap())
    }

    /// Supplies an empty plan so the planner is skipped and each test exercises
    /// the embedding, retrieval or generation stage it targets.
    async fn ask(base: &str) -> (u16, serde_json::Value) {
        post(
            base,
            "/ask",
            json!({"question": "why did auth-api fail?", "plan": {}}),
        )
        .await
    }

    fn embedding_ok() -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_json(json!({"data": [{"embedding": [0.1, 0.2, 0.3]}]}))
    }

    fn one_hit() -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_json(json!({"result": [{
            "id": 1,
            "score": 0.9,
            "payload": {
                "id": "incident_1#c0",
                "Title": "auth-api 5xx spike",
                "Text": "Error rate above 5%",
                "SourceUri": "https://app.datadoghq.eu/incidents/1",
                "Kind": "incident",
                "Timestamp": "2025-01-01T00:00:00Z",
                "Service": "auth-api",
                "Environment": "prod"
            }
        }]}))
    }

    fn chat_ok(text: &str) -> ResponseTemplate {
        ResponseTemplate::new(200)
            .set_body_json(json!({"choices": [{"message": {"content": text}}]}))
    }

    async fn mount(server: &MockServer, p: &str, resp: ResponseTemplate, times: u64) {
        Mock::given(method("POST"))
            .and(path(p))
            .respond_with(resp)
            .expect(times)
            .mount(server)
            .await;
    }

    fn assert_error(body: &serde_json::Value, code: &str, stage: &str, retryable: bool) {
        let e = &body["error"];
        assert_eq!(e["code"], code, "body: {body}");
        assert_eq!(e["stage"], stage, "body: {body}");
        assert_eq!(e["retryable"], retryable, "body: {body}");
        assert!(e["message"].is_string());
        assert!(
            body.get("answer").is_none(),
            "errors must not carry an answer"
        );
        assert!(!body.to_string().contains("sk-test-secret"));
    }

    #[tokio::test]
    async fn embedding_5xx_is_typed_error_and_llm_is_never_called() {
        let up = MockServer::start().await;
        mount(&up, EMBED, ResponseTemplate::new(500), 2).await;
        mount(&up, SEARCH, one_hit(), 0).await;
        mount(&up, CHAT, chat_ok("made up"), 0).await;
        let base = spawn_api(&up, fast_retry(2), limits(Duration::from_secs(5))).await;

        let (status, body) = ask(&base).await;
        assert_eq!(status, 503);
        assert_error(&body, "upstream_unavailable", "embedding", true);
    }

    #[tokio::test]
    async fn embedding_4xx_is_not_retried_and_maps_to_502() {
        let up = MockServer::start().await;
        let rejected = ResponseTemplate::new(400)
            .set_body_string(r#"{"error":"Incorrect API key provided: sk-test-secret"}"#);
        mount(&up, EMBED, rejected, 1).await;
        mount(&up, CHAT, chat_ok("made up"), 0).await;
        let base = spawn_api(&up, fast_retry(4), limits(Duration::from_secs(5))).await;

        let (status, body) = ask(&base).await;
        assert_eq!(status, 502);
        assert_error(&body, "embedding_failed", "embedding", false);
    }

    #[tokio::test]
    async fn search_failure_is_typed_error_and_stops_after_max_attempts() {
        let up = MockServer::start().await;
        mount(&up, EMBED, embedding_ok(), 1).await;
        mount(&up, SEARCH, ResponseTemplate::new(502), 3).await;
        mount(&up, CHAT, chat_ok("no incidents found"), 0).await;
        let base = spawn_api(&up, fast_retry(3), limits(Duration::from_secs(5))).await;

        let (status, body) = ask(&base).await;
        assert_eq!(status, 503);
        assert_error(&body, "upstream_unavailable", "retrieval", true);
    }

    #[tokio::test]
    async fn missing_collection_is_retrieval_failed() {
        let up = MockServer::start().await;
        mount(&up, EMBED, embedding_ok(), 1).await;
        mount(&up, SEARCH, ResponseTemplate::new(404), 1).await;
        mount(&up, CHAT, chat_ok("no incidents found"), 0).await;
        let base = spawn_api(&up, fast_retry(3), limits(Duration::from_secs(5))).await;

        let (status, body) = ask(&base).await;
        assert_eq!(status, 502);
        assert_error(&body, "retrieval_failed", "retrieval", false);
    }

    #[tokio::test]
    async fn zero_hits_is_200_with_no_evidence_marker_and_no_llm_call() {
        let up = MockServer::start().await;
        mount(&up, EMBED, embedding_ok(), 1).await;
        let empty = ResponseTemplate::new(200).set_body_json(json!({"result": []}));
        mount(&up, SEARCH, empty, 1).await;
        mount(&up, CHAT, chat_ok("made up"), 0).await;
        let base = spawn_api(&up, fast_retry(3), limits(Duration::from_secs(5))).await;

        let (status, body) = ask(&base).await;
        assert_eq!(status, 200);
        assert_eq!(body["evidence"], "none");
        assert_eq!(body["answer"], rag_core::rag_service::NO_EVIDENCE_ANSWER);
    }

    #[tokio::test]
    async fn rate_limited_embedding_is_retried_after_retry_after() {
        let up = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(EMBED))
            .respond_with(ResponseTemplate::new(429).insert_header("retry-after-ms", "150"))
            .up_to_n_times(1)
            .expect(1)
            .mount(&up)
            .await;
        mount(&up, EMBED, embedding_ok(), 1).await;
        mount(&up, SEARCH, one_hit(), 1).await;
        mount(&up, CHAT, chat_ok("auth-api had a 5xx spike"), 1).await;
        let base = spawn_api(&up, fast_retry(3), limits(Duration::from_secs(5))).await;

        let started = Instant::now();
        let (status, body) = ask(&base).await;
        assert_eq!(status, 200, "body: {body}");
        assert_eq!(body["evidence"], "found");
        assert_eq!(body["answer"], "auth-api had a 5xx spike");
        assert!(
            started.elapsed() >= Duration::from_millis(150),
            "Retry-After must be honored"
        );
    }

    #[tokio::test]
    async fn slow_upstream_hits_deadline_with_504() {
        let up = MockServer::start().await;
        let slow = embedding_ok().set_delay(Duration::from_secs(3));
        mount(&up, EMBED, slow, 1).await;
        mount(&up, CHAT, chat_ok("made up"), 0).await;
        let base = spawn_api(&up, fast_retry(3), limits(Duration::from_millis(300))).await;

        let started = Instant::now();
        let (status, body) = ask(&base).await;
        assert!(started.elapsed() < Duration::from_millis(1500));
        assert_eq!(status, 504);
        assert_error(&body, "timeout", "embedding", true);
    }

    #[tokio::test]
    async fn generation_failure_is_typed_error_not_an_answer() {
        let up = MockServer::start().await;
        mount(&up, EMBED, embedding_ok(), 1).await;
        mount(&up, SEARCH, one_hit(), 1).await;
        mount(&up, CHAT, ResponseTemplate::new(401), 1).await;
        let base = spawn_api(&up, fast_retry(3), limits(Duration::from_secs(5))).await;

        let (status, body) = ask(&base).await;
        assert_eq!(status, 502);
        assert_error(&body, "generation_failed", "generation", false);
    }

    #[tokio::test]
    async fn planning_failure_is_typed_error() {
        let up = MockServer::start().await;
        mount(&up, CHAT, ResponseTemplate::new(500), 2).await;
        let base = spawn_api(&up, fast_retry(2), limits(Duration::from_secs(5))).await;

        let (status, body) = post(&base, "/ask/plan", json!({"question": "what broke?"})).await;
        assert_eq!(status, 503);
        assert_error(&body, "upstream_unavailable", "planning", true);
    }

    #[tokio::test]
    async fn ask_planning_failure_stops_before_retrieval() {
        let up = MockServer::start().await;
        mount(&up, CHAT, ResponseTemplate::new(500), 2).await;
        mount(&up, EMBED, ResponseTemplate::new(200), 0).await;
        mount(&up, SEARCH, one_hit(), 0).await;
        let base = spawn_api(&up, fast_retry(2), limits(Duration::from_secs(5))).await;

        let (status, body) = post(&base, "/ask", json!({"question": "what broke?"})).await;
        assert_eq!(status, 503);
        assert_error(&body, "upstream_unavailable", "planning", true);
    }

    #[tokio::test]
    async fn invalid_requests_are_400() {
        let up = MockServer::start().await;
        let base = spawn_api(&up, fast_retry(1), limits(Duration::from_secs(5))).await;

        let (status, body) = post(&base, "/ask", json!({"env": "prod"})).await;
        assert_eq!(status, 400);
        assert_eq!(body["error"]["code"], "invalid_request");
        assert_eq!(body["error"]["retryable"], false);
        assert!(body["error"]["stage"].is_null());

        let (status, body) = post(&base, "/ask", json!({"question": "  "})).await;
        assert_eq!(status, 400);
        assert_eq!(body["error"]["code"], "invalid_request");
    }
}
