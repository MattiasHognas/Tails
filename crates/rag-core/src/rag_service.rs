use crate::domain::Hit;
use crate::error::{RagError, Stage};
use crate::log_patterns::{self, LogPattern};
use crate::openai::OpenAiClient;
use crate::qdrant::Qdrant;
use crate::reranker::rerank_mmr_signals;
use crate::resilience::{env_duration_ms, run_stage};
use crate::text::{TRUNCATION_MARKER, truncate_with_marker};
use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use serde::Serialize;
use std::collections::HashSet;
use std::time::Duration;

/// Returned (with `Evidence::None`) when retrieval succeeded but matched
/// nothing. The LLM is not called in that case, so it cannot invent evidence.
pub const NO_EVIDENCE_ANSWER: &str = "No matching evidence was found in the indexed data for this question. \
The search completed successfully but returned no documents; this does not confirm that nothing happened. \
Try widening the time window, removing service/environment filters, or checking that the relevant data has been indexed.";

/// Longest document excerpt given to the answer model, in bytes. Longer texts are
/// cut at a char boundary and marked with [`TRUNCATION_MARKER`].
pub const EXCERPT_MAX_BYTES: usize = 1500;

/// The question's time window (either bound may be open) and the asker's timezone, for
/// counting log patterns ([`log_patterns::occurrences`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AskWindow {
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    pub tz: Tz,
}

impl Default for AskWindow {
    /// No window, days in UTC.
    fn default() -> Self {
        Self {
            from: None,
            to: None,
            tz: Tz::UTC,
        }
    }
}

/// `candidates` without the log pattern days that logged nothing in `window`. The
/// retrieval filter keeps a day whose first..last log overlaps the window; counted by
/// hour ([`LogPattern::hours_in`]), a day with no logs in the window is not evidence
/// for it.
pub fn logged_in_window(candidates: Vec<Hit>, window: &AskWindow) -> Vec<Hit> {
    if window.from.is_none() && window.to.is_none() {
        return candidates;
    }
    candidates
        .into_iter()
        .filter(|h| {
            log_patterns::from_document(&h.doc)
                .is_none_or(|p| p.hours_in(window.from, window.to).next().is_some())
        })
        .collect()
}

/// For each reranked hit that is a log pattern day, how often its pattern occurred on
/// the pattern's days among `candidates` (the hit represents them all), in `window`.
fn pattern_occurrences(
    hits: &[Hit],
    candidates: &[Hit],
    window: &AskWindow,
) -> Vec<Option<String>> {
    hits.iter()
        .map(|h| {
            log_patterns::from_document(&h.doc)?;
            let mut seen = HashSet::new();
            let days: Vec<LogPattern> = candidates
                .iter()
                .filter(|c| c.doc.group_id() == h.doc.group_id() && seen.insert(c.doc.parent_id()))
                .filter_map(|c| log_patterns::from_document(&c.doc))
                .collect();
            Some(log_patterns::occurrences(
                &days,
                window.from,
                window.to,
                window.tz,
            ))
        })
        .collect()
}

/// Reranks `candidates` to at most `top_k` sources and asks the LLM, returning the
/// answer and the sources in `[DOC #n]` order.
async fn rerank_and_generate(
    oa: &OpenAiClient,
    candidates: &[Hit],
    top_k: usize,
    question: &str,
    live_evidence: Option<&str>,
    window: &AskWindow,
) -> Result<(String, Vec<Hit>), RagError> {
    let hits = rerank_mmr_signals(candidates, top_k);
    let occurrences = pattern_occurrences(&hits, candidates, window);
    let answer = generate(oa, &hits, &occurrences, question, live_evidence).await?;
    Ok((answer, hits))
}

/// Generate an answer from retrieved candidates. With no candidates this
/// returns [`NO_EVIDENCE_ANSWER`] without calling the LLM.
pub async fn answer_question(
    oa: &OpenAiClient,
    candidates: Vec<Hit>,
    top_k: usize,
    question: &str,
) -> Result<String, RagError> {
    answer_question_with_live_evidence(oa, candidates, top_k, question, None).await
}

/// Like [`answer_question`], additionally giving the LLM a rendered live-evidence
/// timeline (see [`crate::live_evidence::Timeline::prompt_context`]). The LLM is
/// only skipped when there are neither candidates nor live evidence.
pub async fn answer_question_with_live_evidence(
    oa: &OpenAiClient,
    candidates: Vec<Hit>,
    top_k: usize,
    question: &str,
    live_evidence: Option<&str>,
) -> Result<String, RagError> {
    if candidates.is_empty() && live_evidence.is_none() {
        return Ok(NO_EVIDENCE_ANSWER.to_string());
    }
    let window = AskWindow::default();
    let (answer, _) =
        rerank_and_generate(oa, &candidates, top_k, question, live_evidence, &window).await?;
    Ok(answer)
}

/// Ask the LLM to answer from `hits` (already reranked, cited as `[DOC #n]` in
/// order, with the occurrences line of each log pattern) and the optional
/// live-evidence timeline.
async fn generate(
    oa: &OpenAiClient,
    hits: &[Hit],
    occurrences: &[Option<String>],
    question: &str,
    live_evidence: Option<&str>,
) -> Result<String, RagError> {
    let mut sb = String::new();
    for (i, h) in hits.iter().enumerate() {
        use std::fmt::Write;
        let _ = writeln!(sb, "[DOC #{}] {} ({:?})", i + 1, h.doc.title, h.doc.kind);
        if let Some(ts) = &h.doc.timestamp {
            let _ = writeln!(sb, "Time: {}", ts);
        }
        if !h.doc.service.is_empty() {
            let _ = writeln!(
                sb,
                "Service: {} | Env: {}",
                h.doc.service, h.doc.environment
            );
        }
        let _ = writeln!(sb, "Source: {}", h.doc.source_uri);
        let _ = writeln!(sb, "Score: {:.3}", h.score);
        // selected metadata
        for (k, v) in &h.doc.metadata {
            if [
                "severity",
                "state",
                "dd_incident_id",
                "dd_monitor_id",
                "dd_metric",
                "status",
                "window_from",
                "window_to",
                "type",
            ]
            .contains(&k.as_str())
            {
                let _ = writeln!(sb, "{}: {}", k, v);
            }
        }
        if let Some(Some(line)) = occurrences.get(i) {
            let _ = writeln!(sb, "{line}");
        }
        let body = truncate_with_marker(&h.doc.text, EXCERPT_MAX_BYTES, TRUNCATION_MARKER);
        let _ = writeln!(
            sb,
            "Excerpt:
{}
",
            body
        );
    }

    let mut user = format!(
        "Question:
{}

Context:
{}
",
        question, sb
    );
    if let Some(timeline) = live_evidence {
        use std::fmt::Write;
        let _ = write!(
            user,
            "
Live evidence timeline (measured from Datadog for the question's window):
{}
",
            timeline
        );
    }
    user.push_str(
        "
Instructions:
- Answer concisely.
- If multiple hypotheses exist, list them ordered by likelihood.
- Provide bullet-point 'Top signals' and 'Next steps'.
- Include markdown links to each Source when you cite evidence.
- If the context does not support an answer, say so; never invent evidence.
",
    );
    if occurrences.iter().any(Option::is_some) {
        user.push_str(
            "- A log pattern lists its 'Occurrences' per day; use that line for how often and when \
it occurred (the excerpt only counts one UTC day).
",
        );
    }
    if live_evidence.is_some() {
        user.push_str(
            "- Keep 'Observed' facts (only the timeline's observations, cited by ID such as [obs-1] with their link) \
separate from 'Hypotheses'. Never present a hypothesis or an indexed document as a measured fact.
- Do not compute new numbers from the observations; quote the values given.
- Mention missing evidence that limits confidence.
",
        );
    }
    let system = "You are a helpful SRE assistant. Use only provided context. When unsure, say so. Always cite SourceUri for each claim.";
    let out = oa.chat_complete(system, &user).await?;
    Ok(out)
}

/// Whether an answer is backed by retrieved documents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Evidence {
    Found,
    None,
}

/// An indexed document given to the answer model. `n` is its `[DOC #n]`
/// citation number in the prompt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AnswerSource {
    pub n: usize,
    /// ID of the indexed document (the parent of the retrieved chunk, e.g.
    /// `incident_…` or `monitor_123`), so a source can be traced to the index.
    pub id: String,
    pub title: String,
    /// Source kind as used in filters (`logs`, `monitor`, `slo`, ...).
    pub kind: &'static str,
    pub timestamp: Option<String>,
    pub service: Option<String>,
    pub environment: Option<String>,
    pub uri: String,
}

impl AnswerSource {
    /// Sources for reranked `hits`, numbered exactly like the prompt's `[DOC #n]`.
    pub fn from_hits(hits: &[Hit]) -> Vec<Self> {
        let non_empty = |s: &str| Some(s.to_string()).filter(|s| !s.is_empty());
        hits.iter()
            .enumerate()
            .map(|(i, h)| Self {
                n: i + 1,
                id: h.doc.parent_id().to_string(),
                title: h.doc.title.clone(),
                kind: h.doc.kind.name(),
                timestamp: h.doc.timestamp.clone(),
                service: non_empty(&h.doc.service),
                environment: non_empty(&h.doc.environment),
                uri: h.doc.source_uri.clone(),
            })
            .collect()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AskOutcome {
    pub answer: String,
    pub evidence: Evidence,
    /// Documents passed to the answer model, in `[DOC #n]` order. Empty when
    /// the LLM was not called.
    pub sources: Vec<AnswerSource>,
}

/// Per-stage timeouts for the ask pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StageTimeouts {
    pub planning: Duration,
    pub embedding: Duration,
    pub retrieval: Duration,
    pub generation: Duration,
    /// Budget for all live Datadog queries of one question (run concurrently).
    pub live_evidence: Duration,
}

impl Default for StageTimeouts {
    fn default() -> Self {
        Self {
            planning: Duration::from_secs(30),
            embedding: Duration::from_secs(15),
            retrieval: Duration::from_secs(15),
            generation: Duration::from_secs(60),
            live_evidence: Duration::from_secs(20),
        }
    }
}

impl StageTimeouts {
    /// `RAG_PLAN_TIMEOUT_MS`, `RAG_EMBED_TIMEOUT_MS`, `RAG_SEARCH_TIMEOUT_MS`,
    /// `RAG_GENERATE_TIMEOUT_MS`, `RAG_LIVE_EVIDENCE_TIMEOUT_MS`.
    pub fn from_env() -> Self {
        let d = Self::default();
        Self {
            planning: env_duration_ms("RAG_PLAN_TIMEOUT_MS", d.planning),
            embedding: env_duration_ms("RAG_EMBED_TIMEOUT_MS", d.embedding),
            retrieval: env_duration_ms("RAG_SEARCH_TIMEOUT_MS", d.retrieval),
            generation: env_duration_ms("RAG_GENERATE_TIMEOUT_MS", d.generation),
            live_evidence: env_duration_ms("RAG_LIVE_EVIDENCE_TIMEOUT_MS", d.live_evidence),
        }
    }
}

/// Embed `query`, search Qdrant and answer `question`.
///
/// Any embedding or retrieval failure is returned as an error and the LLM is
/// never called. A successful search with zero hits yields
/// [`Evidence::None`] and [`NO_EVIDENCE_ANSWER`], also without an LLM call.
#[allow(clippy::too_many_arguments)]
pub async fn retrieve_and_answer(
    oa: &OpenAiClient,
    qd: &Qdrant,
    query: &str,
    question: &str,
    filter: Option<serde_json::Value>,
    search_limit: usize,
    top_k: usize,
    timeouts: &StageTimeouts,
) -> Result<AskOutcome, RagError> {
    let candidates = retrieve(oa, qd, query, filter, search_limit, timeouts).await?;
    answer_candidates(
        oa,
        candidates,
        top_k,
        question,
        None,
        &AskWindow::default(),
        timeouts,
    )
    .await
}

/// Embed `query` and search Qdrant. Failures are errors, never empty hits.
pub async fn retrieve(
    oa: &OpenAiClient,
    qd: &Qdrant,
    query: &str,
    filter: Option<serde_json::Value>,
    search_limit: usize,
    timeouts: &StageTimeouts,
) -> Result<Vec<Hit>, RagError> {
    let vector = run_stage(Stage::Embedding, timeouts.embedding, oa.embed(query)).await?;
    run_stage(
        Stage::Retrieval,
        timeouts.retrieval,
        qd.search(vector, search_limit, filter),
    )
    .await
}

/// Answer from retrieved candidates plus an optional rendered live-evidence
/// timeline. With neither, returns [`Evidence::None`] without calling the LLM.
///
/// With a bounded `window`, log pattern days that logged nothing in it (by hour) are
/// dropped first. The days of one pattern are one source, and the prompt gives its
/// count per day in `window` and in `window.tz`, from the retrieved days.
pub async fn answer_candidates(
    oa: &OpenAiClient,
    candidates: Vec<Hit>,
    top_k: usize,
    question: &str,
    live_evidence: Option<&str>,
    window: &AskWindow,
    timeouts: &StageTimeouts,
) -> Result<AskOutcome, RagError> {
    let candidates = logged_in_window(candidates, window);
    if candidates.is_empty() && live_evidence.is_none() {
        tracing::info!("retrieval returned no hits; answering without LLM");
        return Ok(AskOutcome {
            answer: NO_EVIDENCE_ANSWER.to_string(),
            evidence: Evidence::None,
            sources: vec![],
        });
    }
    // Rerank once, so `sources` is numbered exactly like the prompt's `[DOC #n]`.
    let (answer, hits) = run_stage(
        Stage::Generation,
        timeouts.generation,
        rerank_and_generate(oa, &candidates, top_k, question, live_evidence, window),
    )
    .await?;
    Ok(AskOutcome {
        answer,
        evidence: Evidence::Found,
        sources: AnswerSource::from_hits(&hits),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{RagDocument, SourceKind};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn create_test_hit(id: &str, title: &str, text: &str, score: f32) -> Hit {
        Hit {
            doc: RagDocument {
                id: id.to_string(),
                title: title.to_string(),
                text: text.to_string(),
                source_uri: format!("http://example.com/{}", id),
                kind: SourceKind::Monitor,
                timestamp: Some("2025-01-01T00:00:00Z".to_string()),
                service: "test-service".to_string(),
                environment: "production".to_string(),
                metadata: serde_json::Map::new(),
            },
            score,
        }
    }

    async fn chat_server(status: u16, answer: &str) -> (MockServer, OpenAiClient) {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(status).set_body_json(serde_json::json!({
                    "choices": [{"message": {"content": answer}}]
                })),
            )
            .mount(&server)
            .await;
        let mut oa = OpenAiClient::new("k".into(), server.uri(), "e".into(), "c".into());
        oa.retry = crate::resilience::RetryPolicy::none();
        (server, oa)
    }

    /// The user prompt of the most recent chat request.
    async fn last_prompt(server: &MockServer) -> String {
        let reqs = server.received_requests().await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&reqs.last().unwrap().body).unwrap();
        body["messages"][1]["content"].as_str().unwrap().to_string()
    }

    /// The `Excerpt:` of the first document in a prompt.
    fn excerpt(prompt: &str) -> &str {
        let start = prompt.find("Excerpt:\n").unwrap() + "Excerpt:\n".len();
        let len = prompt[start..].find("\n\n").unwrap();
        &prompt[start..start + len]
    }

    #[tokio::test]
    async fn prompt_lists_each_document_with_time_scope_source_and_key_metadata() {
        let (server, oa) = chat_server(200, "Test answer to the question").await;
        let mut incident = create_test_hit("1", "Checkout outage", "Pool exhausted", 0.9);
        incident.doc.kind = SourceKind::Incident;
        for (k, v) in [("severity", "SEV-1"), ("state", "open"), ("owner", "x")] {
            incident.doc.metadata.insert(k.into(), serde_json::json!(v));
        }
        let mut dashboard = create_test_hit("2", "Overview", "Panels", 0.8);
        dashboard.doc.kind = SourceKind::Dashboard;
        dashboard.doc.timestamp = None;
        dashboard.doc.service = String::new();
        dashboard.doc.environment = String::new();

        let answer = answer_question(&oa, vec![incident, dashboard], 2, "What happened?")
            .await
            .unwrap();
        assert_eq!(answer, "Test answer to the question");

        let prompt = last_prompt(&server).await;
        assert!(prompt.starts_with("Question:\nWhat happened?\n\nContext:\n"));
        // Scores are reranked: the incident gets its 1.10 prior and the 0.5 recency
        // floor, so the undated dashboard (0.8) ranks first.
        let first = "[DOC #2] Checkout outage (Incident)\nTime: 2025-01-01T00:00:00Z\n\
            Service: test-service | Env: production\nSource: http://example.com/1\n\
            Score: 0.495\nseverity: \"SEV-1\"\nstate: \"open\"\nExcerpt:\nPool exhausted\n";
        assert!(prompt.contains(first), "{prompt}");
        // Unselected metadata is left out.
        assert!(!prompt.contains("owner"));
        // Absent time and service lines are omitted, not printed empty.
        let second = "[DOC #1] Overview (Dashboard)\nSource: http://example.com/2\n";
        assert!(prompt.contains(second), "{prompt}");
        assert!(!prompt.contains("Live evidence timeline"));
        assert!(!prompt.contains("[obs-1]"));
    }

    #[tokio::test]
    async fn live_evidence_adds_the_timeline_and_fact_hypothesis_instructions() {
        let (server, oa) = chat_server(200, "answer").await;
        let timeline = "Observations (measured facts):\n- [obs-1] spike\n";
        answer_question_with_live_evidence(&oa, vec![], 5, "why?", Some(timeline))
            .await
            .unwrap();
        let prompt = last_prompt(&server).await;
        assert!(prompt.contains(&format!(
            "Live evidence timeline (measured from Datadog for the question's window):\n{timeline}"
        )));
        assert!(prompt.contains("cited by ID such as [obs-1]"));
        assert!(prompt.contains("separate from 'Hypotheses'"));
        assert!(!prompt.contains("[DOC #"));
    }

    /// Excerpts are capped at [`EXCERPT_MAX_BYTES`] without ever splitting a
    /// character. Before the shared helper, a multibyte char straddling byte 1500
    /// panicked the request.
    #[tokio::test]
    async fn excerpts_are_truncated_on_char_boundaries_at_1500_bytes() {
        let (server, oa) = chat_server(200, "ok").await;
        let mut cases: Vec<String> = vec![
            "a".repeat(1499),
            "a".repeat(1500),
            "a".repeat(1501),
            "a".repeat(2000),
        ];
        // Put each multibyte char at every offset around the limit.
        for ch in ["å", "決", "🔥", "e\u{0301}", "👩\u{200D}💻"] {
            for pad in 1490..=1500 {
                cases.push(format!("{}{}{}", "a".repeat(pad), ch.repeat(4), "tail"));
            }
        }
        for text in &cases {
            let hit = create_test_hit("1", "Long", text, 0.9);
            answer_question(&oa, vec![hit], 1, "q").await.unwrap();
            let prompt = last_prompt(&server).await;
            let got = excerpt(&prompt);
            if text.len() <= EXCERPT_MAX_BYTES {
                assert_eq!(got, text);
                continue;
            }
            let head = got
                .strip_suffix(TRUNCATION_MARKER)
                .unwrap_or_else(|| panic!("no marker for {} bytes", text.len()));
            assert!(head.len() <= EXCERPT_MAX_BYTES && head.len() >= EXCERPT_MAX_BYTES - 16);
            assert!(text.starts_with(head));
            // Never a bare base character without its combining mark, or a
            // dangling joiner or half an emoji sequence.
            assert!(!head.ends_with('\u{200D}') && !head.ends_with('👩'));
            if text.contains('\u{0301}') {
                assert!(!head.ends_with('e'), "{:?}", &head[head.len() - 8..]);
            }
        }
    }

    #[tokio::test]
    async fn generation_failures_are_typed_errors() {
        let (_server, oa) = chat_server(401, "unused").await;
        let err = answer_question(&oa, vec![create_test_hit("1", "T", "x", 0.9)], 1, "q")
            .await
            .unwrap_err();
        assert!(matches!(err, RagError::GenerationFailed { .. }), "{err:?}");

        let (_server, oa) = chat_server(500, "unused").await;
        let err = answer_question(&oa, vec![create_test_hit("1", "T", "x", 0.9)], 1, "q")
            .await
            .unwrap_err();
        assert!(
            matches!(
                err,
                RagError::UpstreamUnavailable {
                    stage: Stage::Generation,
                    ..
                }
            ),
            "{err:?}"
        );

        // A 200 with an empty completion is a failure, not an empty answer.
        let (_server, oa) = chat_server(200, "  ").await;
        let err = answer_question(&oa, vec![create_test_hit("1", "T", "x", 0.9)], 1, "q")
            .await
            .unwrap_err();
        assert!(matches!(err, RagError::GenerationFailed { .. }), "{err:?}");
    }

    #[tokio::test]
    async fn empty_candidates_answer_without_calling_the_llm() {
        let (server, oa) = chat_server(200, "No context available").await;
        let result = answer_question(&oa, vec![], 10, "Test question").await;
        assert_eq!(result.unwrap(), NO_EVIDENCE_ANSWER);
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn answer_candidates_sources_match_prompt_numbering() {
        let (server, oa) = chat_server(200, "see [DOC #1]").await;

        let mut slo = create_test_hit("3", "Checkout SLO", "99.9%", 0.5);
        slo.doc.kind = SourceKind::SLO;
        slo.doc.service = String::new();
        slo.doc.environment = String::new();
        slo.doc.timestamp = None;
        let candidates = vec![
            create_test_hit("1", "First", "alpha", 0.9),
            slo,
            create_test_hit("2", "Second", "beta", 0.7),
        ];
        let outcome = answer_candidates(
            &oa,
            candidates,
            2,
            "q",
            None,
            &AskWindow::default(),
            &StageTimeouts::default(),
        )
        .await
        .unwrap();
        assert_eq!(outcome.evidence, Evidence::Found);

        let prompt = last_prompt(&server).await;
        let cited: Vec<&str> = prompt.lines().filter(|l| l.starts_with("[DOC #")).collect();
        // Only the top_k documents given to the model are listed, in prompt order.
        assert_eq!(outcome.sources.len(), 2);
        assert_eq!(cited.len(), outcome.sources.len());
        for (line, src) in cited.iter().zip(&outcome.sources) {
            assert!(
                line.starts_with(&format!("[DOC #{}] {} (", src.n, src.title)),
                "{line} vs {src:?}"
            );
        }
        assert_eq!(
            outcome.sources.iter().map(|s| s.n).collect::<Vec<_>>(),
            vec![1, 2]
        );
        // Numbering follows the reranked order, not the retrieval order: the
        // undated SLO is not decayed and outranks the dated monitors.
        assert_eq!(outcome.sources[0].title, "Checkout SLO");
        let first = &outcome.sources[1];
        assert_eq!(first.title, "First");
        assert_eq!(first.id, "1");
        assert_eq!(first.kind, "monitor");
        assert_eq!(first.uri, "http://example.com/1");
        assert_eq!(first.service.as_deref(), Some("test-service"));
        assert_eq!(first.environment.as_deref(), Some("production"));
        assert_eq!(first.timestamp.as_deref(), Some("2025-01-01T00:00:00Z"));
    }

    #[test]
    fn answer_source_reports_the_parent_document_and_filter_kind_name() {
        let mut hit = create_test_hit("slo_1#c0", "SLO", "x", 0.9);
        hit.doc
            .metadata
            .insert("chunk_of".into(), serde_json::json!("slo_1"));
        hit.doc.kind = SourceKind::SLO;
        hit.doc.service = String::new();
        hit.doc.environment = String::new();
        hit.doc.timestamp = None;
        let s = &AnswerSource::from_hits(&[hit])[0];
        assert_eq!(
            serde_json::to_value(s).unwrap(),
            serde_json::json!({"n": 1, "id": "slo_1", "title": "SLO", "kind": "slo",
                "timestamp": null, "service": null, "environment": null,
                "uri": "http://example.com/slo_1#c0"})
        );
    }

    /// Hits for the day documents (one chunk each) of `logs` (id, time, message).
    fn pattern_days(logs: &[(&str, &str, &str)], score: f32) -> Vec<Hit> {
        let events: Vec<log_patterns::LogEvent> = logs
            .iter()
            .map(|(id, at, message)| log_patterns::LogEvent {
                id: id.to_string(),
                timestamp: at.parse().unwrap(),
                service: "checkout".into(),
                environment: "prod".into(),
                status: "error".into(),
                message: message.to_string(),
            })
            .collect();
        log_patterns::group(&events)
            .iter()
            .map(|p| Hit {
                doc: crate::chunk::chunk(1800, 200, &p.to_document("https://dd")).swap_remove(0),
                score,
            })
            .collect()
    }

    /// The days of one pattern are one source and one `[DOC #n]` with its count per
    /// local day in the window; a day with no log in the window's hours is dropped
    /// before reranking, and a pattern with no day left is not evidence at all.
    #[tokio::test]
    async fn pattern_days_are_one_source_counted_in_the_window() {
        let (server, oa) = chat_server(200, "see [DOC #1]").await;
        let mut candidates = pattern_days(
            &[
                ("mon", "2026-09-21T10:00:00Z", "redis refused 1"),
                ("tue-1", "2026-09-22T21:30:00Z", "redis refused 2"),
                ("tue-2", "2026-09-22T22:30:00Z", "redis refused 3"),
                ("wed", "2026-09-23T08:00:00Z", "redis refused 4"),
            ],
            0.9,
        );
        // Logged only outside the window's hours, although its span overlaps it.
        candidates.extend(pattern_days(
            &[
                ("gap-1", "2026-09-22T08:00:00Z", "disk full"),
                ("gap-2", "2026-09-23T23:30:00Z", "disk full"),
            ],
            0.95,
        ));
        candidates.push(create_test_hit("monitor_1", "Redis", "redis", 0.5));
        // "Wednesday" in Stockholm (UTC+2).
        let window = AskWindow {
            from: Some("2026-09-22T22:00:00Z".parse().unwrap()),
            to: Some("2026-09-23T22:00:00Z".parse().unwrap()),
            tz: "Europe/Stockholm".parse().unwrap(),
        };
        let outcome = answer_candidates(
            &oa,
            candidates.clone(),
            5,
            "q",
            None,
            &window,
            &StageTimeouts::default(),
        )
        .await
        .unwrap();
        let prompt = last_prompt(&server).await;
        let ids: Vec<&str> = outcome.sources.iter().map(|s| s.id.as_str()).collect();
        let tuesday = candidates[1].doc.parent_id();
        let wednesday = candidates[2].doc.parent_id();
        // Tuesday and Wednesday (UTC) score alike; the lower ID represents the pattern.
        assert_eq!(ids.len(), 2, "{ids:?}");
        assert!(ids.contains(&tuesday.min(wednesday)), "{ids:?}");
        assert!(ids.contains(&"monitor_1"), "{ids:?}");
        assert_eq!(prompt.matches("[DOC #").count(), 2, "{prompt}");
        assert_eq!(
            prompt
                .matches("Occurrences in the question's window")
                .count(),
            1
        );
        assert!(prompt.contains(
            "Occurrences in the question's window: 2 (Wed 2026-09-23: 2; \
             days in Europe/Stockholm, hour precision)\nExcerpt:\n"
        ));
        assert!(prompt.contains("use that line for how often and when"));
        assert!(!prompt.contains("disk full"));

        // Without a window: every retrieved day, per day in the asker's zone.
        let outcome = answer_candidates(
            &oa,
            candidates.clone(),
            5,
            "q",
            None,
            &AskWindow {
                from: None,
                to: None,
                ..window
            },
            &StageTimeouts::default(),
        )
        .await
        .unwrap();
        let prompt = last_prompt(&server).await;
        assert_eq!(outcome.sources.len(), 3);
        assert!(prompt.contains(
            "Occurrences on the retrieved days: 4 (Mon 2026-09-21: 1 · Tue 2026-09-22: 1 · \
             Wed 2026-09-23: 2; days in Europe/Stockholm, hour precision)"
        ));

        // Only days outside the window: no evidence, the LLM is not called again.
        let calls = server.received_requests().await.unwrap().len();
        let outcome = answer_candidates(
            &oa,
            candidates[3..5].to_vec(),
            5,
            "q",
            None,
            &window,
            &StageTimeouts::default(),
        )
        .await
        .unwrap();
        assert_eq!(outcome.evidence, Evidence::None);
        assert_eq!(server.received_requests().await.unwrap().len(), calls);
    }

    #[tokio::test]
    async fn no_evidence_outcome_has_no_sources() {
        let oa = OpenAiClient::new(
            "k".into(),
            "http://127.0.0.1:9".into(),
            "e".into(),
            "c".into(),
        );
        let outcome = answer_candidates(
            &oa,
            vec![],
            5,
            "q",
            None,
            &AskWindow::default(),
            &StageTimeouts::default(),
        )
        .await
        .unwrap();
        assert_eq!(outcome.evidence, Evidence::None);
        assert!(outcome.sources.is_empty());
        assert_eq!(outcome.answer, NO_EVIDENCE_ANSWER);
    }
}
