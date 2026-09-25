//! `tails-e2e`: asks the incident question set through the built `rag-cli` and
//! `rag-api` binaries after `rag-indexer` has indexed the corpus, and scores the
//! answers. Driven by `scripts/e2e.sh`; see docs/DEVELOPMENT.md#end-to-end-tests.
//!
//! It reads the same environment as the binaries (`OPENAI_*`, `QDRANT_*`, `DD_*`,
//! `RAG_FUSION`, `RAG_KEYWORD_STOPWORDS`), so the dimension probe goes to the embedding
//! endpoint the indexer used and `rag-api`, which it starts itself, inherits the
//! configuration.
//!
//! 1. Hard: the collection exists with the embedding model's dimension and has points.
//! 2. Reads every stored point back with the reader's payload type.
//! 3. For each distinct `now` of the question set, starts `rag-api` with the clock
//!    fixed at it (`RAG_TEST_FIXED_NOW`, test-only), waits for it, asks that `now`'s
//!    questions one at a time with `rag-cli ask … --json`, and stops it.
//! 4. Scores each response with [`tails_fakes::questions::score`], the harness's own
//!    scoring. Hard (must pass): `rag-cli` exits 0 with a well-formed response,
//!    sources are stored documents numbered like the prompt with none listed twice,
//!    `citationWarnings` equals `validate_citations`, negative controls are flagged and
//!    the scope matches. Quality: the aggregates against the committed e2e thresholds.
//!
//! 5. Explains every question whose must-retrieve documents aren't all ranked first:
//!    the top documents of dense search and of keyword search alone (per query text,
//!    with the question's scope, keyword queries as the API builds them), the fused
//!    hybrid search, and the reranked sources with their scores from the answer
//!    prompt, so a ranking difference can be traced to its search.
//! 6. With `--summary`, writes the configuration, aggregates and per-question results
//!    as JSON, for `scripts/e2e.sh` to compare configurations.
//!
//! Exits 1 when a hard check fails or an aggregate is below its threshold.

use anyhow::{Context, Result, anyhow, bail};
use clap::Parser;
use rag_core::domain::RagDocument;
use rag_core::openai::OpenAiClient;
use rag_core::qdrant::{HybridConfig, Qdrant, QdrantPayload, SearchQuery};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tails_fakes::questions::{self, Answers, Question, Row, Thresholds};
use tokio::process::{Child, Command};

#[derive(Parser)]
#[command(
    name = "tails-e2e",
    about = "Ask the incident question set through rag-cli"
)]
struct Args {
    /// The rag-api binary.
    #[arg(long)]
    api_bin: PathBuf,
    /// The rag-cli binary.
    #[arg(long)]
    cli_bin: PathBuf,
    /// Where the started rag-api listens: it is started with `RAG_API_ADDR` set to this
    /// URL's host and port.
    #[arg(long, default_value = "http://127.0.0.1:5191")]
    api_base: String,
    /// The tails-fakes server, for the answer prompts it recorded.
    #[arg(long, default_value = "http://127.0.0.1:8900")]
    fakes: String,
    /// The question set directory (questions.json, corpus.json).
    #[arg(long)]
    questions: Option<PathBuf>,
    /// The e2e thresholds (default: e2e_thresholds.json in the question set).
    #[arg(long)]
    thresholds: Option<PathBuf>,
    /// rag-api's output is appended here.
    #[arg(long, default_value = "rag-api.log")]
    api_log: PathBuf,
    /// Write the configuration, aggregates and per-question results here as JSON.
    #[arg(long)]
    summary: Option<PathBuf>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct E2eThresholds {
    #[allow(dead_code)]
    description: String,
    /// The embedding model the thresholds were measured with.
    embedding_model: String,
    thresholds: Thresholds,
}

fn env(name: &str) -> Result<String> {
    std::env::var(name).with_context(|| format!("{name} is not set"))
}

/// The collection's dense vector size and point count.
async fn collection(http: &reqwest::Client, qdrant: &str, name: &str) -> Result<(u64, u64)> {
    let v: Value = http
        .get(format!("{qdrant}/collections/{name}"))
        .send()
        .await?
        .error_for_status()
        .with_context(|| format!("collection {name}"))?
        .json()
        .await?;
    let r = &v["result"];
    let size = r["config"]["params"]["vectors"]["dense"]["size"]
        .as_u64()
        .ok_or_else(|| anyhow!("collection {name} has no dense vector: {v}"))?;
    Ok((size, r["points_count"].as_u64().unwrap_or(0)))
}

/// Every stored point's payload, decoded by the reader's own type.
async fn stored(http: &reqwest::Client, qdrant: &str, name: &str) -> Result<Vec<RagDocument>> {
    let mut out = vec![];
    let mut offset = Value::Null;
    loop {
        let v: Value = http
            .post(format!("{qdrant}/collections/{name}/points/scroll"))
            .json(
                &json!({"limit": 256, "with_payload": true, "with_vector": false,
                          "offset": offset}),
            )
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        for p in v["result"]["points"].as_array().into_iter().flatten() {
            let payload: QdrantPayload = serde_json::from_value(p["payload"].clone())
                .with_context(|| format!("the reader cannot decode {}", p["payload"]))?;
            out.push(RagDocument::from(payload));
        }
        offset = v["result"]["next_page_offset"].clone();
        if offset.is_null() {
            return Ok(out);
        }
    }
}

/// Starts rag-api with the clock fixed at `now` and waits until it accepts requests.
async fn start_api(args: &Args, now: &str) -> Result<Child> {
    let addr = args
        .api_base
        .trim_start_matches("http://")
        .trim_end_matches('/')
        .to_string();
    if tokio::net::TcpStream::connect(&addr).await.is_ok() {
        bail!("something already listens on {addr}; stop it before the end-to-end run");
    }
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&args.api_log)?;
    let mut child = Command::new(&args.api_bin)
        .env("RAG_TEST_FIXED_NOW", now)
        .env("RAG_API_ADDR", &addr)
        .stdout(log.try_clone()?)
        .stderr(log)
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("starting {:?}", args.api_bin))?;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(status) = child.try_wait()? {
            bail!("rag-api exited with {status} (see {:?})", args.api_log);
        }
        if tokio::net::TcpStream::connect(&addr).await.is_ok() {
            return Ok(child);
        }
        if Instant::now() > deadline {
            bail!("rag-api did not listen on {addr} within 30s");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// `rag-cli ask … --json` for `q`: its exit status and parsed stdout.
async fn ask(args: &Args, q: &Question, tz: &str) -> Result<Value> {
    let mut cmd = Command::new(&args.cli_bin);
    cmd.arg("ask")
        .arg(&q.question)
        .args(["--tz", tz, "--json"])
        .env("RAG_API_BASE", &args.api_base)
        .env("NO_COLOR", "1");
    for (k, v) in q
        .request
        .as_ref()
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
    {
        let flag = match k.as_str() {
            "service" => "--service",
            "env" => "--env",
            "kinds" => {
                for kind in v.as_array().into_iter().flatten() {
                    cmd.arg("--kind").arg(kind.as_str().unwrap_or_default());
                }
                continue;
            }
            other => bail!("request field {other} has no rag-cli flag"),
        };
        cmd.arg(flag).arg(v.as_str().unwrap_or_default());
    }
    let out = tokio::time::timeout(Duration::from_secs(180), cmd.output())
        .await
        .map_err(|_| anyhow!("rag-cli timed out"))??;
    let stdout = String::from_utf8_lossy(&out.stdout);
    if !out.status.success() {
        bail!(
            "rag-cli exited with {}: {} {}",
            out.status,
            stdout.trim(),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let v: Value = serde_json::from_str(&stdout)
        .with_context(|| format!("rag-cli printed invalid JSON: {stdout}"))?;
    let well_formed = v["answer"].is_string()
        && v["sources"].is_array()
        && v["scope"].is_object()
        && v["citationWarnings"].is_array();
    if !well_formed {
        bail!("malformed response (answer, sources, scope, citationWarnings): {v}");
    }
    Ok(v)
}

/// The Qdrant filter for a question's expected scope (as `questions.json` writes it).
fn scope_filter(scope: &Value) -> Option<Value> {
    let text = |k: &str| scope[k].as_str().map(str::to_string);
    let time = |k: &str| scope[k].as_str().and_then(rag_core::planner::parse_utc);
    rag_core::retrieval::RetrievalScope {
        service: text("service"),
        environment: text("environment"),
        from_utc: time("fromUtc"),
        to_utc: time("toUtc"),
        kinds: scope["kinds"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|k| {
                k.as_str()
                    .and_then(rag_core::domain::SourceKind::parse_lenient)
            })
            .collect(),
    }
    .to_qdrant_filter()
}

/// The top `limit` points of one named vector search, as `(score, parent id, title)`.
async fn top_points(
    http: &reqwest::Client,
    qdrant: &str,
    collection: &str,
    query: Value,
    using: &str,
    filter: &Option<Value>,
    limit: usize,
) -> Result<Vec<(f64, String, String)>> {
    let mut body = json!({"query": query, "using": using, "limit": limit, "with_payload": true});
    if let Some(f) = filter {
        body["filter"] = f.clone();
    }
    let v: Value = http
        .post(format!("{qdrant}/collections/{collection}/points/query"))
        .json(&body)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(v["result"]["points"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| {
            let doc: Option<RagDocument> =
                serde_json::from_value::<QdrantPayload>(p["payload"].clone())
                    .ok()
                    .map(Into::into);
            let (id, title) = doc
                .map(|d| (d.parent_id().to_string(), d.title))
                .unwrap_or_default();
            (p["score"].as_f64().unwrap_or(0.0), id, title)
        })
        .collect())
}

/// Prints why `q` ranked as it did: dense and keyword search alone for each query text
/// the API searches with (the question, and the plan's rewrite when it differs), and
/// the reranked sources with their scores from the answer prompt.
async fn explain(
    http: &reqwest::Client,
    oa: &OpenAiClient,
    qd: &Qdrant,
    q: &Question,
    prompt: Option<&str>,
) -> Result<()> {
    let (qdrant, collection) = (qd.endpoint.as_str(), qd.collection.as_str());
    println!(
        "\nexplain {} (must retrieve {:?}): {:?}",
        q.id, q.expect.must_retrieve, q.question
    );
    let filter = scope_filter(&q.expect.scope);
    let mut texts = vec![q.question.clone()];
    if let Some(rw) = q.plan["rewrittenQuery"].as_str()
        && !rw.trim().eq_ignore_ascii_case(q.question.trim())
    {
        texts.push(rw.to_string());
    }
    let dense = oa.embed_queries(&texts).await?;
    let searches: Vec<SearchQuery> = texts
        .iter()
        .zip(&dense)
        .map(|(text, dense)| SearchQuery {
            dense: dense.clone(),
            sparse: qd.hybrid.keyword_query(text),
        })
        .collect();
    for (text, search) in texts.iter().zip(&searches) {
        let lists = [
            ("dense", json!(search.dense)),
            ("keyword", json!(search.sparse)),
        ];
        for (name, query) in lists {
            let using = if name == "dense" { "dense" } else { "sparse" };
            println!("  {name} search for {text:?}:");
            for (rank, (score, id, title)) in
                top_points(http, qdrant, collection, query, using, &filter, 5)
                    .await?
                    .iter()
                    .enumerate()
            {
                println!("    {}. {score:.4}  {id}  {title}", rank + 1);
            }
        }
    }
    // As many candidates as the API retrieves: DBSF normalizes over the whole lists.
    let candidates = std::env::var("RAG_SEARCH_CANDIDATES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(64);
    println!(
        "  hybrid search ({}, {candidates} candidates, normalized score):",
        qd.hybrid.describe()
    );
    for (rank, hit) in qd
        .hybrid_search(&searches, candidates, filter.clone())
        .await?
        .iter()
        .take(5)
        .enumerate()
    {
        println!(
            "    {}. {:.4}  {}  {}",
            rank + 1,
            hit.score,
            hit.doc.parent_id(),
            hit.doc.title
        );
    }
    println!("  reranked sources (score after kind prior and recency weight):");
    let mut title = None;
    for line in prompt.unwrap_or_default().lines() {
        if line.starts_with("[DOC #") {
            title = Some(line.to_string());
        } else if let (Some(t), Some(score)) = (&title, line.strip_prefix("Score: ")) {
            println!("    {t}  score {score}");
            title = None;
        }
    }
    Ok(())
}

async fn prompt(http: &reqwest::Client, fakes: &str, question: &str) -> Option<String> {
    let r = http
        .get(format!("{fakes}/_fakes/prompt"))
        .query(&[("question", question)])
        .send()
        .await
        .ok()?;
    if !r.status().is_success() {
        return None;
    }
    r.text().await.ok()
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let http = reqwest::Client::new();
    let dir = args.questions.clone().unwrap_or_else(questions::data_dir);
    let thresholds_path = args
        .thresholds
        .clone()
        .unwrap_or_else(|| dir.join("e2e_thresholds.json"));
    let e2e: E2eThresholds = serde_json::from_str(
        &std::fs::read_to_string(&thresholds_path)
            .with_context(|| format!("{thresholds_path:?}"))?,
    )
    .with_context(|| format!("{thresholds_path:?}"))?;
    let qdrant = env("QDRANT_ENDPOINT")?.trim_end_matches('/').to_string();
    let collection_name = env("QDRANT_COLLECTION")?;
    // The API's retrieval settings: rag-api inherits the same environment.
    let hybrid = HybridConfig::from_env()?;
    let mut qd = Qdrant::new(qdrant.clone(), collection_name.clone());
    qd.hybrid = hybrid;
    let mut hard: Vec<String> = vec![];

    // The embedding model's dimension, from the endpoint the indexer used.
    let oa = OpenAiClient::new_from_env()?;
    let dim = oa
        .embed("collection dimension probe")
        .await
        .context("embedding probe")?
        .len() as u64;
    let (size, points) = collection(&http, &qdrant, &collection_name).await?;
    println!(
        "collection {collection_name}: {points} points, dense size {size}; \
         embedding model {} at {}: dimension {dim}",
        oa.embedding_model, oa.embedding_base_url
    );
    if size != dim {
        hard.push(format!(
            "collection dimension {size} != embedding dimension {dim}"
        ));
    }
    if points == 0 {
        hard.push("the collection is empty".into());
    }
    if e2e.embedding_model != oa.embedding_model {
        println!(
            "note: thresholds in {thresholds_path:?} were measured with {}, this run embeds \
             with {}",
            e2e.embedding_model, oa.embedding_model
        );
    }

    let docs = stored(&http, &qdrant, &collection_name).await?;
    let parents: BTreeMap<&str, &RagDocument> = docs.iter().map(|d| (d.parent_id(), d)).collect();
    let (mut dataset, _corpus) = questions::load_dir(&dir);
    let unknown = questions::unknown_ids(&dataset, &parents);
    if !unknown.is_empty() {
        bail!("documents the questions name are not stored: {unknown:?}");
    }
    let groups = questions::source_groups(&parents);
    questions::to_groups(&mut dataset, &groups);

    // One rag-api per distinct `now`, questions in dataset order within it.
    let mut by_now: Vec<(String, Vec<usize>)> = vec![];
    for (i, q) in dataset.questions.iter().enumerate() {
        let now = q.now(&dataset.defaults).to_rfc3339();
        match by_now.iter_mut().find(|(n, _)| *n == now) {
            Some((_, idx)) => idx.push(i),
            None => by_now.push((now, vec![i])),
        }
    }
    let mut rows: Vec<Option<Row>> = dataset.questions.iter().map(|_| None).collect();
    // Questions that didn't rank their must-retrieve documents first, with their prompt.
    let mut to_explain: Vec<(usize, Option<String>)> = vec![];
    let started = Instant::now();
    for (now, idx) in &by_now {
        let mut api = start_api(&args, now).await?;
        for &i in idx {
            let q = &dataset.questions[i];
            let tz = q.timezone(&dataset.defaults);
            let row = match ask(&args, q, tz).await {
                Ok(resp) => {
                    let prompt = prompt(&http, &args.fakes, &q.question).await;
                    let mut row = questions::score(
                        q,
                        &resp,
                        prompt.as_deref(),
                        &parents,
                        &groups,
                        Answers::Canned,
                    );
                    if row.recall < 1.0 || row.precision < 1.0 {
                        to_explain.push((i, prompt.clone()));
                    }
                    if row.negatives_flagged < row.negatives_total {
                        row.hard.push(format!(
                            "{} of {} deliberately unknown citations flagged",
                            row.negatives_flagged, row.negatives_total
                        ));
                    }
                    if !row.scope_ok {
                        row.hard.push("scope differs from the expected one".into());
                    }
                    row
                }
                Err(e) => Row {
                    id: q.id.clone(),
                    known_gap: q.known_gap.is_some(),
                    hard: vec![format!("{e:#}")],
                    ..Row::default()
                },
            };
            rows[i] = Some(row);
        }
        api.kill().await.ok();
        api.wait().await.ok();
    }
    let rows: Vec<Row> = rows.into_iter().flatten().collect();

    println!(
        "\nincident questions v{} | real Qdrant | {} | embeddings: {} (dimension {dim}) | \
         canned plans and answers | {} questions in {:.1}s",
        dataset.version,
        hybrid.describe(),
        oa.embedding_model,
        rows.len(),
        started.elapsed().as_secs_f64()
    );
    questions::print_header();
    for row in &rows {
        questions::print_row(row);
    }
    questions::print_aggregate(&rows, &e2e.thresholds);
    for (i, prompt) in &to_explain {
        if let Err(e) = explain(&http, &oa, &qd, &dataset.questions[*i], prompt.as_deref()).await {
            println!("  (could not explain: {e:#})");
        }
    }

    for row in &rows {
        hard.extend(row.hard.iter().map(|h| format!("{}: {h}", row.id)));
    }
    let below = questions::threshold_failures(&rows, &e2e.thresholds, Answers::Canned);
    println!();
    if hard.is_empty() {
        println!("hard checks: all passed");
    } else {
        println!("hard checks FAILED:");
        for h in &hard {
            println!("  {h}");
        }
    }
    if below.is_empty() {
        println!("quality: every aggregate at or above its e2e threshold");
    } else {
        println!("quality: BELOW THRESHOLD: {below:?}");
    }
    if let Some(path) = &args.summary {
        let summary = summary(&hybrid, &rows, &hard, &below);
        std::fs::write(path, serde_json::to_string_pretty(&summary)?)
            .with_context(|| format!("{path:?}"))?;
    }
    if !hard.is_empty() || !below.is_empty() {
        std::process::exit(1);
    }
    Ok(())
}

/// The run as JSON: configuration, aggregates, per-question results and failures.
fn summary(hybrid: &HybridConfig, rows: &[Row], hard: &[String], below: &[String]) -> Value {
    let a = questions::aggregate(rows);
    json!({
        "fusion": hybrid.fusion.name(),
        "stopwords": hybrid.query_stopwords,
        "aggregate": {
            "recallAtK": a.recall_at_k,
            "precisionAtR": a.precision_at_r,
            "distractorExclusion": a.distractor_exclusion,
            "scopeAccuracy": a.scope_accuracy,
            "citationValidity": a.citation_validity,
            "citationAccuracy": a.citation_accuracy,
            "observationRecall": a.observation_recall,
            "negativeControlsFlagged": a.negative_controls_flagged,
            "evidenceAccuracy": a.evidence_accuracy,
        },
        "questions": rows.iter().map(|r| json!({
            "id": r.id,
            "recall": r.recall,
            "precision": r.precision,
            "excluded": r.excluded,
            "distractors": r.distractors,
            "margin": r.margin,
            "problems": r.problems,
        })).collect::<Vec<_>>(),
        "hard": hard,
        "belowThreshold": below,
    })
}
