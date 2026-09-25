//! The `/ask` and `/ask/plan` HTTP API. `main.rs` builds [`AppState::from_env`]
//! and serves [`app`]; tests build the state directly (fixed clock, local
//! upstreams) and serve the same router.

use anyhow::Result;
use axum::{
    Json, Router,
    extract::{State, rejection::JsonRejection},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
};
use rag_core::{
    citations::validate_citations,
    datadog::{Datadog, normalize_scope_value},
    domain::{Hit, SourceKind},
    error::{RagError, Stage},
    live_evidence::{self, GateInput, LiveEvidenceConfig, LiveEvidenceRequest, Timeline},
    openai::OpenAiClient,
    planner::{self, Clock, PlanContext, QueryPlan, SystemClock, Window},
    qdrant::Qdrant,
    rag_service::{
        AskWindow, StageTimeouts, answer_candidates, logged_in_window, retrieve, search_queries,
    },
    resilience::{HttpConfig, RetryPolicy, env_duration_ms, run_stage},
    retrieval::{ExplicitScope, RetrievalScope, normalize_filters},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

/// Everything a request handler needs. Built from the environment by
/// [`AppState::from_env`]; tests construct it directly.
#[derive(Clone)]
pub struct AppState {
    pub oa: OpenAiClient,
    pub qd: Qdrant,
    pub limits: Limits,
    pub clock: Arc<dyn Clock>,
    /// Datadog client for live evidence; `None` without `DD_API_KEY`/`DD_APP_KEY`.
    pub dd: Option<Arc<Datadog>>,
    pub live: LiveEvidenceConfig,
}

impl AppState {
    /// OpenAI, Qdrant, deadlines, live evidence and Datadog from the environment,
    /// with the system clock. Fails only without `OPENAI_API_KEY`.
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            oa: OpenAiClient::new_from_env()?,
            qd: Qdrant::new_from_env()?,
            limits: Limits::from_env(),
            clock: Arc::new(SystemClock),
            dd: datadog_from_env(),
            live: LiveEvidenceConfig::from_env(),
        })
    }
}

/// Request deadlines. Retry budgets live on the clients (`RetryPolicy`).
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Overall deadline for one `/ask` request (`RAG_ASK_DEADLINE_MS`).
    pub ask_deadline: Duration,
    pub stages: StageTimeouts,
}

impl Limits {
    pub fn from_env() -> Self {
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
            | RagError::IndexingFailed { .. }
            | RagError::LiveEvidenceFailed { .. } => StatusCode::BAD_GATEWAY,
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

/// Explicit fields always win over values the planner infers. See docs/ARCHITECTURE.md "Question pipeline".
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
    /// Live Datadog evidence: `false` disables it, `true` runs it even for a
    /// question not classified as diagnostic. Omitted: diagnostic questions only.
    live_evidence: Option<bool>,
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
    // Stored `Service`/`Environment` values are lowercase and Qdrant matches are
    // case-sensitive, so `--service Auth-API` must become `auth-api` like planner values.
    let scope_value = |v: &Option<String>| non_empty(v).map(|s| normalize_scope_value(&s));
    Ok(ExplicitScope {
        service: scope_value(&req.service),
        environment: scope_value(&req.env),
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

/// Datadog client for live evidence, if credentials are configured. Missing
/// credentials disable live evidence (reported per request); they never fail startup.
fn datadog_from_env() -> Option<Arc<Datadog>> {
    match Datadog::new_from_env() {
        Ok(mut dd) => {
            dd.http = HttpConfig::from_env().build_client();
            dd.retry = RetryPolicy::from_env();
            Some(Arc::new(dd))
        }
        Err(_) => {
            tracing::warn!("DD_API_KEY/DD_APP_KEY not set; live evidence is disabled");
            None
        }
    }
}

/// Live Datadog evidence for the question, from the resolved scope and the
/// retrieved hits. Never fails the request: problems are in `missingEvidence`.
async fn live_timeline(
    st: &AppState,
    req: &AskReq,
    plan: &QueryPlan,
    scope: &RetrievalScope,
    hits: &[Hit],
) -> Timeline {
    let window = match live_evidence::gate(GateInput {
        config: &st.live,
        configured: st.dd.is_some(),
        requested: req.live_evidence,
        diagnostic: live_evidence::is_diagnostic(&plan.intent, &req.question),
        from: scope.from_utc,
        to: scope.to_utc,
        now: st.clock.now(),
    }) {
        Ok(window) => window,
        Err(skipped) => {
            tracing::info!(reason = ?skipped.skip_reason, "live evidence skipped");
            return *skipped;
        }
    };
    // `gate` already skipped when no client is configured.
    let Some(dd) = st.dd.as_deref() else {
        return Timeline::skipped(
            live_evidence::SkipReason::NotConfigured,
            "no Datadog client",
        );
    };
    let found = live_evidence::discovery::discover(
        hits,
        scope.service.as_deref(),
        plan.metric.as_deref(),
        st.live.caps,
    );
    if found.services.is_empty() && found.metrics.is_empty() {
        return Timeline::skipped(
            live_evidence::SkipReason::NothingToQuery,
            "no service or metric could be identified from the question or the retrieved documents",
        );
    }
    let request = LiveEvidenceRequest {
        question: req.question.clone(),
        services: found.services,
        environment: scope.environment.clone(),
        metrics: found.metrics,
        window,
        missing: found.missing,
    };
    let timeline = live_evidence::collect_timeline(
        dd,
        Some(&st.oa),
        request,
        &st.live,
        st.limits.stages.live_evidence,
    )
    .await;
    tracing::info!(
        observations = timeline.observations.len(),
        hypotheses = timeline.hypotheses.len(),
        missing = timeline.missing_evidence.len(),
        "live evidence collected"
    );
    timeline
}

/// The API router: `POST /ask/plan` and `POST /ask`.
pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/ask/plan", post(plan))
        .route("/ask", post(ask))
        .with_state(state)
}

/// Serves [`app`] on `listener` until the server stops.
pub async fn serve(listener: tokio::net::TcpListener, state: AppState) -> std::io::Result<()> {
    axum::serve(listener, app(state)).await
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

        // Search with the question and, when it differs, the rewrite (explicit, then
        // the planner's), so a detail the rewrite dropped is still found.
        let rewrite = non_empty(&req.rewritten_query).or_else(|| plan.rewritten_query.clone());
        let queries = search_queries(&req.question, rewrite.as_deref());
        tracing::info!(searches = queries.len(), "search queries");

        // Retrieve generously; cut by server-side topK after rerank
        let search_limit = std::env::var("RAG_SEARCH_CANDIDATES")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(64);
        let top_k = choose_topk(&req.question);

        // Embedding/retrieval failures are errors, never empty evidence.
        let hits = retrieve(
            &st.oa,
            &st.qd,
            &queries,
            scope.to_qdrant_filter(),
            search_limit,
            &st.limits.stages,
        )
        .await?;
        // Log pattern days are counted in the asked window, in the asker's timezone;
        // recency is weighted against the window, or without one against the clock.
        let window = AskWindow {
            from: scope.from_utc,
            to: scope.to_utc,
            now: ctx.now,
            tz: ctx.tz,
        };
        let hits = logged_in_window(hits, &window);

        let timeline = live_timeline(&st, &req, &plan, &scope, &hits).await;
        // The LLM is only called with at least one hit or live observation.
        let live_context = timeline
            .prompt_context()
            .filter(|_| !hits.is_empty() || timeline.has_observations());
        let outcome = answer_candidates(
            &st.oa,
            hits,
            top_k,
            &req.question,
            live_context.as_deref(),
            &window,
            &st.limits.stages,
        )
        .await?;
        Ok((outcome, plan, scope, timeline))
    })
    .await;
    let (outcome, plan, scope, timeline) = result?;

    // Citations that resolve to no source or observation are reported, not removed:
    // the answer text is returned as generated.
    let observation_ids: Vec<&str> = timeline
        .observations
        .iter()
        .map(|o| o.id.as_str())
        .collect();
    let citations = validate_citations(&outcome.answer, outcome.sources.len(), &observation_ids);
    if !citations.warnings.is_empty() {
        tracing::warn!(warnings = ?citations.warnings, "answer cites unknown documents or observations");
    }

    Ok(Json(json!({
        "answer": outcome.answer,
        "evidence": outcome.evidence,
        "sources": outcome.sources,
        "citationWarnings": citations.warnings,
        "plan": plan,
        "scope": scope.to_json(),
        "timeline": timeline,
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
        use wiremock::matchers::{body_partial_json, method, path, query_param};
        use wiremock::{Mock, MockServer, Request, ResponseTemplate};

        struct Harness {
            base: String,
            openai: MockServer,
            qdrant: MockServer,
        }

        /// Starts the API against mocked OpenAI and Qdrant with a fixed clock.
        async fn harness(now: &str, llm_plan: Value) -> Harness {
            harness_with(now, llm_plan, vec![], None, LiveEvidenceConfig::default()).await
        }

        /// Like [`harness`], with extra Qdrant hits and optionally a mocked Datadog.
        async fn harness_with(
            now: &str,
            llm_plan: Value,
            extra_hits: Vec<Value>,
            datadog: Option<&MockServer>,
            live: LiveEvidenceConfig,
        ) -> Harness {
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
                .respond_with(EmbedEach)
                .mount(&openai)
                .await;

            let qdrant = MockServer::start().await;
            let mut hits = vec![json!({
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
            })];
            hits.extend(extra_hits);
            Mock::given(method("POST"))
                .and(path("/collections/test/points/query"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(json!({"result": {"points": hits}})),
                )
                .mount(&qdrant)
                .await;

            let now: DateTime<Utc> = now.parse().unwrap();
            let state = AppState {
                oa: OpenAiClient::new("k".into(), openai.uri(), "e".into(), "c".into()),
                qd: Qdrant::new(qdrant.uri(), "test".into()),
                limits: Limits {
                    ask_deadline: Duration::from_secs(10),
                    stages: StageTimeouts {
                        live_evidence: Duration::from_millis(500),
                        ..StageTimeouts::default()
                    },
                },
                clock: Arc::new(FixedClock(now)),
                dd: datadog.map(|server| {
                    let mut dd = Datadog::new("api".into(), "app".into(), "datadoghq.eu".into());
                    dd.api_base = server.uri();
                    dd.retry = RetryPolicy::none();
                    Arc::new(dd)
                }),
                live,
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

        /// One embedding per input, for a single string or a batch.
        struct EmbedEach;

        impl wiremock::Respond for EmbedEach {
            fn respond(&self, r: &Request) -> ResponseTemplate {
                let n = body(r)["input"].as_array().map_or(1, Vec::len);
                let data: Vec<Value> = (0..n)
                    .map(|i| json!({"index": i, "embedding": [0.5, 1.0 + i as f32]}))
                    .collect();
                ResponseTemplate::new(200).set_body_json(json!({ "data": data }))
            }
        }

        /// The scope filter of a hybrid query, which every prefetch must carry.
        fn search_filter(r: &Request) -> Value {
            let b = body(r);
            let prefetch = b["prefetch"].as_array().unwrap();
            assert!(!prefetch.is_empty());
            for p in prefetch {
                assert_eq!(p["filter"], prefetch[0]["filter"], "{b}");
            }
            assert!(b.get("filter").is_none());
            prefetch[0]["filter"].clone()
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
                search_filter(&searches[0]),
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
                        {"key": "Kind", "match": {"any": ["metrics", "monitor", "dashboard", "sLO", "serviceCatalog"]}},
                        {"must": [
                            {"key": "Kind", "match": {"value": "logs"}},
                            {"key": "Timestamp", "range": {"gte": "2026-09-22T22:00:00Z"}},
                            {"key": "Metadata.first_seen", "range": {"lt": "2026-09-23T22:00:00Z"}}
                        ]}
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
            // The question and the rewritten query were embedded in one request, and
            // each is searched densely and by keywords.
            let embeds: Vec<&Request> = reqs
                .iter()
                .filter(|r| r.url.path() == "/v1/embeddings")
                .collect();
            assert_eq!(embeds.len(), 1);
            assert_eq!(
                body(embeds[0])["input"],
                json!(["why did auth-api fail yesterday?", "auth-api prod errors"])
            );
            let prefetch = body(&searches[0])["prefetch"].clone();
            let using: Vec<&str> = prefetch
                .as_array()
                .unwrap()
                .iter()
                .map(|p| p["using"].as_str().unwrap())
                .collect();
            assert_eq!(using, ["dense", "sparse", "dense", "sparse"]);
            assert_eq!(prefetch[2]["query"], json!([0.5, 2.0]));
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
                    "service": " Payments ",
                    "from_utc": "2026-09-20T00:00:00Z",
                    "to_utc": "2026-09-21T00:00:00Z",
                    "kinds": ["monitor"]
                }),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(planner_calls(&h).await, 1);
            let searches = h.qdrant.received_requests().await.unwrap();
            let must = search_filter(&searches[0])["must"].clone();
            assert_eq!(
                must[0],
                // Explicit values are trimmed and lowercased like stored payload values.
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
                search_filter(&searches[0]),
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

        /// Datadog mock for "yesterday" in Stockholm asked at 2026-09-24T08:00Z:
        /// window 2026-09-22T22:00Z..2026-09-23T22:00Z, baseline the day before.
        async fn datadog_with_spike() -> MockServer {
            let dd = MockServer::start().await;
            // Hourly points: flat 0.2s, 1.5s at 10:00 and 11:00 UTC on the 23rd.
            let start = 1_790_028_000_000_i64; // 2026-09-21T22:00:00Z
            let points: Vec<Value> = (0..48)
                .map(|i| {
                    let t = start + i * 3_600_000;
                    let v = if i == 36 || i == 37 { 1.5 } else { 0.2 };
                    json!([t, v])
                })
                .collect();
            Mock::given(method("GET"))
                .and(path("/api/v1/query"))
                .and(query_param("from", "1790028000"))
                .and(query_param("to", "1790200800"))
                .and(query_param(
                    "query",
                    "avg:trace.http.request.duration{service:auth-api,env:prod}",
                ))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "status": "ok",
                    "series": [{
                        "metric": "trace.http.request.duration",
                        "expression": "avg:trace.http.request.duration{env:prod,service:auth-api}",
                        "scope": "env:prod,service:auth-api",
                        "interval": 3600,
                        "pointlist": points
                    }]
                })))
                .expect(1)
                .mount(&dd)
                .await;
            let logs: Vec<Value> = (0..6)
                .map(|i| {
                    json!({"id": format!("log{i}"), "attributes": {
                        "service": "auth-api",
                        "status": "error",
                        "timestamp": format!("2026-09-23T10:1{i}:00.000Z"),
                        "message": format!("db connection pool exhausted after {i}00ms"),
                        "tags": ["env:prod"]
                    }})
                })
                .collect();
            Mock::given(method("POST"))
                .and(path("/api/v2/logs/events/search"))
                .and(body_partial_json(json!({"filter": {
                    "from": "2026-09-22T22:00:00Z",
                    "to": "2026-09-23T22:00:00Z",
                    "query": "service:auth-api env:prod status:(error OR warn)"
                }})))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(json!({"data": logs, "meta": {"page": {}}})),
                )
                .expect(1)
                .mount(&dd)
                .await;
            dd
        }

        fn monitor_hit() -> Value {
            json!({
                "id": 2,
                "score": 0.8,
                "payload": {
                    "id": "monitor_7#c0",
                    "Title": "auth-api latency",
                    "Text": "auth-api latency\n\nQuery: avg(last_5m):avg:trace.http.request.duration{service:auth-api} > 1",
                    "SourceUri": "https://app.datadoghq.eu/monitors/7",
                    "Kind": "monitor",
                    "Timestamp": null,
                    "Service": "auth-api",
                    "Environment": "prod",
                    "Metadata": {"query": "avg(last_5m):avg:trace.http.request.duration{service:auth-api} > 1"}
                }
            })
        }

        /// A plan supplied with the request, so the only JSON-mode LLM call is
        /// hypothesis generation.
        fn diagnostic_ask() -> Value {
            json!({
                "question": "why did auth-api fail yesterday?",
                "timezone": "Europe/Stockholm",
                "plan": {"intent": "rootCauseWindow", "service": "auth-api", "environment": "prod"}
            })
        }

        #[tokio::test]
        async fn diagnostic_ask_returns_live_timeline_and_grounds_the_answer() {
            let dd = datadog_with_spike().await;
            let hypotheses = json!({"hypotheses": [
                {"statement": "Pool exhaustion slowed requests", "observationIds": ["obs-2", "obs-3"]},
                {"statement": "Unrelated deploy", "observationIds": ["obs-99"]}
            ]});
            let h = harness_with(
                "2026-09-24T08:00:00Z",
                hypotheses,
                vec![monitor_hit()],
                Some(&dd),
                LiveEvidenceConfig::default(),
            )
            .await;

            let (status, resp) = post(&h.base, "/ask", diagnostic_ask()).await;
            assert_eq!(status, StatusCode::OK, "{resp}");
            assert_eq!(resp["answer"], "the answer");
            assert!(resp["plan"].is_object() && resp["scope"].is_object());

            let t = &resp["timeline"];
            assert_eq!(t["status"], "collected");
            assert_eq!(
                t["window"],
                json!({"fromUtc": "2026-09-22T22:00:00Z", "toUtc": "2026-09-23T22:00:00Z"})
            );
            assert_eq!(
                t["baseline"],
                json!({"fromUtc": "2026-09-21T22:00:00Z", "toUtc": "2026-09-22T22:00:00Z"})
            );
            let obs = t["observations"].as_array().unwrap();
            let kinds: Vec<(&str, &str)> = obs
                .iter()
                .map(|o| (o["id"].as_str().unwrap(), o["kind"].as_str().unwrap()))
                .collect();
            assert_eq!(
                kinds,
                vec![
                    ("obs-1", "seriesSummary"),
                    ("obs-2", "spike"),
                    ("obs-3", "logBurst"),
                    ("obs-4", "logSummary"),
                ]
            );
            let spike = &obs[1];
            for key in [
                "id", "kind", "source", "service", "query", "startUtc", "endUtc", "summary",
                "values", "link",
            ] {
                assert!(spike.get(key).is_some(), "observation lacks {key}: {spike}");
            }
            assert_eq!(spike["source"], "metricQuery");
            assert_eq!(spike["service"], "auth-api");
            assert_eq!(spike["startUtc"], "2026-09-23T10:00:00Z");
            assert_eq!(spike["endUtc"], "2026-09-23T11:00:00Z");
            assert_eq!(spike["values"]["peak"], 1.5);
            assert!(
                spike["link"]
                    .as_str()
                    .unwrap()
                    .starts_with("https://app.datadoghq.eu/metric/explorer?")
            );
            assert_eq!(obs[2]["source"], "logQuery");
            assert_eq!(obs[2]["values"]["count"], 6);
            assert_eq!(
                t["hypotheses"],
                json!([{"statement": "Pool exhaustion slowed requests", "observationIds": ["obs-2", "obs-3"]}])
            );
            assert_eq!(t["missingEvidence"], json!([]));

            // The final answer prompt carries the timeline and the instruction to
            // keep observed facts apart from hypotheses.
            let reqs = h.openai.received_requests().await.unwrap();
            let answer_req = reqs
                .iter()
                .find(|r| {
                    r.url.path() == "/v1/chat/completions"
                        && body(r).get("response_format").is_none()
                })
                .unwrap();
            let user = body(answer_req)["messages"][1]["content"]
                .as_str()
                .unwrap()
                .to_string();
            assert!(user.contains("Live evidence timeline"));
            assert!(user.contains("[obs-2]"));
            assert!(user.contains("Pool exhaustion slowed requests [obs-2, obs-3]"));
            assert!(!user.contains("Unrelated deploy"));
            assert!(user.contains("separate from 'Hypotheses'"));
        }

        #[tokio::test]
        async fn sources_list_the_documents_cited_in_the_answer_prompt() {
            let h = harness_with(
                "2026-09-24T08:00:00Z",
                json!({}),
                vec![monitor_hit()],
                None,
                LiveEvidenceConfig::default(),
            )
            .await;
            let (status, resp) = post(
                &h.base,
                "/ask",
                json!({"question": "list auth-api dashboards", "plan": {"intent": "dashboardLookup"}}),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{resp}");
            let sources = resp["sources"].as_array().unwrap();
            // "the answer" cites nothing, so nothing is unresolved.
            assert_eq!(resp["citationWarnings"], json!([]));

            let reqs = h.openai.received_requests().await.unwrap();
            let answer_req = reqs
                .iter()
                .find(|r| {
                    r.url.path() == "/v1/chat/completions"
                        && body(r).get("response_format").is_none()
                })
                .unwrap();
            let user = body(answer_req)["messages"][1]["content"]
                .as_str()
                .unwrap()
                .to_string();
            let cited: Vec<&str> = user.lines().filter(|l| l.starts_with("[DOC #")).collect();
            assert_eq!(cited.len(), 2);
            assert_eq!(sources.len(), cited.len());
            for (i, (line, src)) in cited.iter().zip(sources).enumerate() {
                assert_eq!(src["n"], i + 1);
                let title = src["title"].as_str().unwrap();
                assert!(
                    line.starts_with(&format!("[DOC #{}] {title} (", i + 1)),
                    "{line} vs {src}"
                );
                let uri = src["uri"].as_str().unwrap();
                assert!(user.contains(&format!("Source: {uri}")));
            }
            let incident = sources
                .iter()
                .find(|s| s["kind"] == "incident")
                .expect("incident source");
            assert_eq!(
                incident,
                &json!({"n": incident["n"], "id": "incident_1#c0",
                    "title": "auth-api 5xx spike", "kind": "incident",
                    "timestamp": "2026-09-23T10:00:00Z", "service": "auth-api",
                    "environment": "prod", "uri": "https://app.datadoghq.eu/incidents/1"})
            );
        }

        #[tokio::test]
        async fn datadog_failure_is_missing_evidence_and_ask_still_succeeds() {
            let dd = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/api/v1/query"))
                .respond_with(ResponseTemplate::new(503))
                .expect(1)
                .mount(&dd)
                .await;
            Mock::given(method("POST"))
                .and(path("/api/v2/logs/events/search"))
                .respond_with(ResponseTemplate::new(500))
                .expect(1)
                .mount(&dd)
                .await;
            let h = harness_with(
                "2026-09-24T08:00:00Z",
                json!({}),
                vec![monitor_hit()],
                Some(&dd),
                LiveEvidenceConfig::default(),
            )
            .await;
            let (status, resp) = post(&h.base, "/ask", diagnostic_ask()).await;
            assert_eq!(status, StatusCode::OK, "{resp}");
            assert_eq!(resp["answer"], "the answer");
            let t = &resp["timeline"];
            assert_eq!(t["status"], "collected");
            assert_eq!(t["observations"], json!([]));
            assert_eq!(t["hypotheses"], json!([]));
            let missing = t["missingEvidence"].as_array().unwrap();
            assert_eq!(missing.len(), 2);
            assert!(missing.iter().all(|m| m["reason"] == "query_failed"));
            assert!(missing[0]["detail"].as_str().unwrap().contains("HTTP 503"));
            // No observations, so no hypothesis call was made.
            assert_eq!(planner_calls(&h).await, 0);
        }

        #[tokio::test]
        async fn datadog_timeout_is_missing_evidence_within_the_stage_budget() {
            let dd = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/api/v1/query"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(json!({"status": "ok", "series": []}))
                        .set_delay(Duration::from_secs(5)),
                )
                .mount(&dd)
                .await;
            Mock::given(method("POST"))
                .and(path("/api/v2/logs/events/search"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(json!({"data": []}))
                        .set_delay(Duration::from_secs(5)),
                )
                .mount(&dd)
                .await;
            let h = harness_with(
                "2026-09-24T08:00:00Z",
                json!({}),
                vec![monitor_hit()],
                Some(&dd),
                LiveEvidenceConfig::default(),
            )
            .await;
            // The harness gives live evidence 500ms; the answer still comes back.
            let started = std::time::Instant::now();
            let (status, resp) = post(&h.base, "/ask", diagnostic_ask()).await;
            assert_eq!(status, StatusCode::OK, "{resp}");
            assert!(started.elapsed() < Duration::from_secs(3));
            assert_eq!(resp["answer"], "the answer");
            let reasons: Vec<&str> = resp["timeline"]["missingEvidence"]
                .as_array()
                .unwrap()
                .iter()
                .map(|m| m["reason"].as_str().unwrap())
                .collect();
            assert_eq!(reasons, vec!["timed_out", "timed_out"]);
        }

        #[tokio::test]
        async fn live_evidence_is_skipped_without_window_or_when_disabled() {
            let dd = MockServer::start().await;
            Mock::given(wiremock::matchers::any())
                .respond_with(ResponseTemplate::new(500))
                .expect(0)
                .mount(&dd)
                .await;
            let h = harness_with(
                "2026-09-24T08:00:00Z",
                json!({}),
                vec![monitor_hit()],
                Some(&dd),
                LiveEvidenceConfig::default(),
            )
            .await;

            // No window: the question has no relative time and none is supplied.
            let (status, resp) = post(
                &h.base,
                "/ask",
                json!({"question": "why does auth-api fail?", "plan": {"intent": "rootCauseWindow"}}),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(resp["timeline"]["status"], "skipped");
            assert_eq!(resp["timeline"]["skipReason"], "window_not_specified");
            assert_eq!(resp["timeline"]["missingEvidence"][0]["reason"], "skipped");

            // Disabled by the request.
            let mut req = diagnostic_ask();
            req["live_evidence"] = json!(false);
            let (_, resp) = post(&h.base, "/ask", req).await;
            assert_eq!(resp["timeline"]["skipReason"], "disabled_by_request");

            // Not diagnostic.
            let (_, resp) = post(
                &h.base,
                "/ask",
                json!({"question": "list dashboards yesterday", "timezone": "Europe/Stockholm",
                       "plan": {"intent": "dashboardLookup"}}),
            )
            .await;
            assert_eq!(resp["timeline"]["skipReason"], "not_diagnostic");
            assert_eq!(resp["timeline"]["missingEvidence"], json!([]));

            // Disabled by configuration (RAG_LIVE_EVIDENCE=off).
            let off = harness_with(
                "2026-09-24T08:00:00Z",
                json!({}),
                vec![],
                Some(&dd),
                LiveEvidenceConfig {
                    enabled: false,
                    ..LiveEvidenceConfig::default()
                },
            )
            .await;
            let (_, resp) = post(&off.base, "/ask", diagnostic_ask()).await;
            assert_eq!(resp["timeline"]["skipReason"], "disabled");
        }

        #[tokio::test]
        async fn missing_datadog_credentials_are_reported_not_fatal() {
            let h = harness("2026-09-24T08:00:00Z", json!({})).await;
            let (status, resp) = post(&h.base, "/ask", diagnostic_ask()).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(resp["timeline"]["skipReason"], "not_configured");
            assert_eq!(
                resp["timeline"]["missingEvidence"][0]["subject"],
                "live Datadog data"
            );
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
    const SEARCH: &str = "/collections/test/points/query";

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
                live_evidence: Duration::from_secs(2),
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
            dd: None,
            live: LiveEvidenceConfig::default(),
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
        ResponseTemplate::new(200).set_body_json(json!({"result": {"points": [{
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
        }]}}))
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
        let empty = ResponseTemplate::new(200).set_body_json(json!({"result": {"points": []}}));
        mount(&up, SEARCH, empty, 1).await;
        mount(&up, CHAT, chat_ok("made up"), 0).await;
        let base = spawn_api(&up, fast_retry(3), limits(Duration::from_secs(5))).await;

        let (status, body) = ask(&base).await;
        assert_eq!(status, 200);
        assert_eq!(body["evidence"], "none");
        assert_eq!(body["answer"], rag_core::rag_service::NO_EVIDENCE_ANSWER);
        assert_eq!(body["sources"], json!([]));
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
