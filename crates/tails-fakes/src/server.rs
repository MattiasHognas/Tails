//! The fakes as one standalone HTTP server, for the end-to-end run of the built
//! binaries (`tails-fakes serve`).
//!
//! One address serves, by path:
//! - `/api/...`: the fake Datadog API over the question set's corpus. The indexing
//!   endpoints are [`IndexApi`]; `/api/v1/query` and log searches with any query other
//!   than the indexer's are the live evidence of the current question ([`LiveApi`]).
//! - `/v1/chat/completions`: the fake OpenAI chat model ([`FakeOpenAi`]), scripted from
//!   `questions.json`. A request is matched to its question by the question text: the
//!   planner's user message is the question, the live-evidence hypotheses and the
//!   answer prompt start with it. Each question gets its `plan`, `hypotheses` and an
//!   answer citing `answer.citeDocuments` (resolved to the source URIs the adapters
//!   give those documents), `citeObservations` and `extra`. An unknown question gets
//!   an empty plan and an answer without citations.
//! - `/v1/embeddings`: 404 unless `embeddings` is set (the hashed bag-of-words
//!   embeddings, a local stand-in when no embedding model is available).
//! - `GET /_fakes/health`: 200 once serving.
//! - `GET /_fakes/prompt?question=<text>`: the last answer prompt for that question
//!   (404 if none), so the checker can verify numbering and evidence.
//!
//! Live evidence is per question, but a live query does not say which question it is
//! for. The planner request of each `/ask` selects the current question, whose `live`
//! data then answers live queries until the next planner request. `/ask` requests must
//! therefore not run concurrently (the end-to-end run asks one question at a time).

use crate::datadog::{self, Corpus, IndexApi, LOG_INDEX_QUERY, Live, LiveApi};
use crate::openai::{AnswerScript, FakeOpenAi};
use crate::questions::{self, Dataset};
use anyhow::{Result, bail};
use chrono::{Duration, Utc};
use rag_core::datadog::Datadog;
use rag_core::domain::RagDocument;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

#[derive(Clone)]
pub struct Fakes {
    index: IndexApi,
    openai: FakeOpenAi,
    /// Live data per question text.
    live: Arc<HashMap<String, LiveApi>>,
    current: Arc<Mutex<LiveApi>>,
}

/// Every document the adapters produce from `corpus` with `site` (so the source URIs
/// match what an indexer with `DD_SITE=site` stores), fetched from a private fake
/// Datadog server over a window covering the whole corpus.
pub async fn documents(corpus: Corpus, site: &str) -> Result<Vec<RagDocument>> {
    let (server, _) = datadog::serve(corpus).await;
    let mut dd = Datadog::new("api-key".into(), "app-key".into(), site.into());
    dd.api_base = server.uri();
    dd.retry = rag_core::resilience::RetryPolicy::none();
    let to = Utc::now();
    let from = to - Duration::days(50 * 365);
    let (from, to) = (from.to_rfc3339(), to.to_rfc3339());
    let mut docs = vec![];
    docs.extend(dd.get_monitors().await?);
    docs.extend(dd.list_dashboards().await?);
    docs.extend(dd.list_slos().await?);
    docs.extend(dd.list_metrics(&from, &to).await?);
    docs.extend(dd.get_incidents(&from, &to).await?);
    docs.extend(dd.search_logs(&from, &to).await?);
    docs.extend(dd.list_service_definitions().await?);
    docs.extend(
        dd.search_change_events(&from, &to, rag_core::change_events::DEFAULT_QUERY)
            .await?,
    );
    Ok(docs)
}

impl Fakes {
    /// Fakes for `dataset` over `corpus` (see the module docs). Fails when a question
    /// names a document the corpus does not produce.
    pub async fn new(
        mut dataset: Dataset,
        corpus: Corpus,
        site: &str,
        embeddings: bool,
    ) -> Result<Self> {
        let docs = documents(corpus.clone(), site).await?;
        let parents: BTreeMap<&str, &RagDocument> =
            docs.iter().map(|d| (d.id.as_str(), d)).collect();
        let unknown = questions::unknown_ids(&dataset, &parents);
        if !unknown.is_empty() {
            bail!("questions name unknown documents: {unknown:?}");
        }
        let groups = questions::source_groups(&parents);
        questions::to_groups(&mut dataset, &groups);

        let openai = FakeOpenAi {
            no_embeddings: !embeddings,
            ..FakeOpenAi::default()
        };
        let mut live = HashMap::new();
        let no_live = questions::LiveSpec::default();
        {
            let mut script = openai.script.lock().unwrap();
            for q in &dataset.questions {
                script.plans.insert(q.question.clone(), q.plan.clone());
                if let Some(h) = &q.hypotheses {
                    script
                        .hypotheses
                        .insert(q.question.clone(), json!({"hypotheses": h}));
                }
                script.answers.insert(
                    q.question.clone(),
                    AnswerScript {
                        cite_uris: questions::cite_uris(q, &parents, &groups),
                        cite_observations: q.answer.cite_observations.clone(),
                        extra: q.answer.extra.clone(),
                    },
                );
                let spec = q.live.as_ref().unwrap_or(&no_live);
                live.insert(
                    q.question.clone(),
                    LiveApi::new(questions::live_data(spec, q.now(&dataset.defaults))),
                );
            }
        }
        Ok(Self {
            index: IndexApi::new(corpus),
            openai,
            live: Arc::new(live),
            current: Arc::new(Mutex::new(LiveApi::new(Live::default()))),
        })
    }

    /// Serves the fakes on `listener` until the returned server is dropped.
    pub async fn serve(self, listener: std::net::TcpListener) -> MockServer {
        let server = MockServer::builder()
            .listener(listener)
            .disable_request_recording()
            .start()
            .await;
        Mock::given(wiremock::matchers::any())
            .respond_with(self)
            .mount(&server)
            .await;
        server
    }

    fn live_api(&self) -> LiveApi {
        self.current.lock().unwrap().clone()
    }
}

impl Respond for Fakes {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let path = req.url.path();
        match (req.method.as_str(), path) {
            ("GET", "/_fakes/health") => ResponseTemplate::new(200).set_body_string("ok"),
            ("GET", "/_fakes/prompt") => {
                let question = req
                    .url
                    .query_pairs()
                    .find(|(k, _)| k == "question")
                    .map(|(_, v)| v.to_string())
                    .unwrap_or_default();
                match self.openai.prompt(&question) {
                    Some(p) => ResponseTemplate::new(200).set_body_string(p),
                    None => ResponseTemplate::new(404),
                }
            }
            (_, p) if p.starts_with("/v1/") => {
                if let Some(question) = FakeOpenAi::planner_question(req) {
                    let live = self
                        .live
                        .get(&question)
                        .cloned()
                        .unwrap_or_else(|| LiveApi::new(Live::default()));
                    *self.current.lock().unwrap() = live;
                }
                self.openai.handle(req)
            }
            ("GET", "/api/v1/query") => self.live_api().respond(req),
            ("POST", "/api/v2/logs/events/search") => {
                let body: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
                if body["filter"]["query"] == LOG_INDEX_QUERY {
                    self.index.respond(req)
                } else {
                    self.live_api().respond(req)
                }
            }
            _ => self.index.respond(req),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn start(embeddings: bool) -> (MockServer, String) {
        let (dataset, corpus) = questions::load_dir(&questions::data_dir());
        let fakes = Fakes::new(dataset, corpus, "datadoghq.eu", embeddings)
            .await
            .unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let server = fakes.serve(listener).await;
        let uri = server.uri();
        (server, uri)
    }

    fn chat(system: &str, user: &str, json_mode: bool) -> Value {
        let mut body = json!({"model": "m", "messages": [
            {"role": "system", "content": system}, {"role": "user", "content": user}]});
        if json_mode {
            body["response_format"] = json!({"type": "json_object"});
        }
        body
    }

    #[tokio::test]
    async fn serves_plans_answers_and_live_data_per_question() {
        let (_server, uri) = start(false).await;
        let http = reqwest::Client::new();
        let post = |path: &str, body: Value| {
            let http = http.clone();
            let url = format!("{uri}{path}");
            async move {
                let r = http
                    .post(url)
                    .header("DD-API-KEY", "k")
                    .header("DD-APPLICATION-KEY", "k")
                    .json(&body)
                    .send()
                    .await
                    .unwrap();
                let status = r.status().as_u16();
                (status, r.json::<Value>().await.unwrap_or(Value::Null))
            }
        };
        let health = http
            .get(format!("{uri}/_fakes/health"))
            .send()
            .await
            .unwrap();
        assert_eq!(health.status(), 200);

        // Embeddings come from a real model in the end-to-end run.
        let (status, _) = post("/v1/embeddings", json!({"input": "x", "model": "m"})).await;
        assert_eq!(status, 404);

        // The planner reply is the question's plan, and selects its live data.
        let q01 = "Why was checkout slow yesterday?";
        let (_, plan) = post(
            "/v1/chat/completions",
            chat("You are a planning assistant for an SRE RAG", q01, true),
        )
        .await;
        let plan: Value =
            serde_json::from_str(plan["choices"][0]["message"]["content"].as_str().unwrap())
                .unwrap();
        assert_eq!(plan["service"], "checkout");
        assert_eq!(plan["intent"], "rootCauseWindow");

        let from = questions::at("2026-03-11T00:00:00Z").timestamp();
        let series = http
            .get(format!("{uri}/api/v1/query"))
            .query(&[
                (
                    "query",
                    "avg:trace.http.request.duration{service:checkout,env:prod}",
                ),
                ("from", &from.to_string()),
                ("to", &(from + 86_400).to_string()),
            ])
            .send()
            .await
            .unwrap()
            .json::<Value>()
            .await
            .unwrap();
        assert_eq!(
            series["series"][0]["pointlist"].as_array().unwrap().len(),
            25
        );
        let (_, logs) = post(
            "/api/v2/logs/events/search",
            json!({"filter": {"query": "service:checkout env:prod status:(error OR warn)"}}),
        )
        .await;
        assert_eq!(logs["data"].as_array().unwrap().len(), 8);

        // The indexer's log search still reaches the corpus.
        let (status, logs) = post(
            "/api/v2/logs/events/search",
            json!({"filter": {"query": LOG_INDEX_QUERY, "from": "2026-03-11T00:00:00Z",
                              "to": "2026-03-12T00:00:00Z"}, "page": {"limit": 1000}}),
        )
        .await;
        assert_eq!(status, 200);
        assert!(!logs["data"].as_array().unwrap().is_empty());

        // Another question's planner request switches the live data.
        post(
            "/v1/chat/completions",
            chat(
                "You are a planning assistant",
                "Which incidents hit auth-api yesterday?",
                true,
            ),
        )
        .await;
        let (_, logs) = post(
            "/api/v2/logs/events/search",
            json!({"filter": {"query": "service:checkout env:prod status:(error OR warn)"}}),
        )
        .await;
        assert!(logs["data"].as_array().unwrap().is_empty());

        // The answer cites the question's documents by the prompt's numbering.
        let prompt = "Question:\nWhich incidents hit auth-api yesterday?\n\nContext:\n\
                      [DOC #1] Something else (Monitor)\nSource: https://example.com/x\n\
                      [DOC #2] Login failures (Incident)\n\
                      Source: https://app.datadoghq.eu/incidents/42\n";
        let (_, answer) = post("/v1/chat/completions", chat("You answer", prompt, false)).await;
        let answer = answer["choices"][0]["message"]["content"].as_str().unwrap();
        assert!(answer.starts_with("Answer to: Which incidents hit auth-api yesterday?"));
        let recorded = http
            .get(format!("{uri}/_fakes/prompt"))
            .query(&[("question", "Which incidents hit auth-api yesterday?")])
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert_eq!(recorded, prompt);
    }

    #[tokio::test]
    async fn serves_stand_in_embeddings_when_asked() {
        let (_server, uri) = start(true).await;
        let v: Value = reqwest::Client::new()
            .post(format!("{uri}/v1/embeddings"))
            .json(&json!({"input": ["a b", "c"], "model": "m"}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(v["data"][1]["index"], 1);
        assert_eq!(
            v["data"][0]["embedding"].as_array().unwrap().len(),
            crate::openai::DIM
        );
    }
}
