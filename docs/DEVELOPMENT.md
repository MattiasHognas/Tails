# Development

How to build, configure, run, deploy and test Tails. For how the components fit
together, see [ARCHITECTURE.md](ARCHITECTURE.md).

## Prerequisites

- Rust (latest stable)
- Docker (for containerization)
- ~~NASM (for cryptographic aws_lc_rs feature)~~ - NO LONGER REQUIRED, using native-tls instead, leaving this here in case I decide to move back to rustls in the future for better performance
- ~~CMake (for cryptographic aws_lc_rs feature)~~ - NO LONGER REQUIRED, using native-tls instead, leaving this here in case I decide to move back to rustls in the future for better performance

## Environment Variables

```
# OpenAI
OPENAI_API_KEY=...
OPENAI_EMBEDDING_MODEL=text-embedding-3-small
OPENAI_CHAT_MODEL=o4-mini
OPENAI_BASE_URL=https://api.openai.com   # any OpenAI-compatible server
# Embeddings from another OpenAI-compatible server, e.g. self-hosted
# text-embeddings-inference (POST /v1/embeddings). Unset or blank: OPENAI_BASE_URL and
# OPENAI_API_KEY. Indexer and API must use the same model; the collection is created
# with its dimension. TEI accepts at most 32 inputs per request by default
# (--max-client-batch-size), so keep INDEXER_EMBED_BATCH_SIZE at or below it.
OPENAI_EMBEDDING_BASE_URL=
OPENAI_EMBEDDING_API_KEY=
# Instruction some embedding models are trained with, put in front of each text as given
# (keep its trailing space). Unset or blank: none, right for OpenAI's models. Examples:
#   bge (v1.5)    query:    "Represent this sentence for searching relevant passages: "
#   e5            query: "query: "            document: "passage: "
#   nomic-embed   query: "search_query: "     document: "search_document: "
# The query prefix is only used by the API; the document prefix only by the indexer, and
# changing it re-embeds everything (it is part of the content hash).
OPENAI_EMBEDDING_QUERY_PREFIX=
OPENAI_EMBEDDING_DOCUMENT_PREFIX=

# Qdrant (the indexer creates the collection: named dense + sparse vectors)
QDRANT_ENDPOINT=http://qdrant:6333
QDRANT_COLLECTION=datadog_rag

# Datadog
DD_API_KEY=...
DD_APP_KEY=...
DD_SITE=datadoghq.eu    # or datadoghq.com
DD_API_BASE_URL=        # optional: API base instead of https://api.$DD_SITE (a proxy, or
                        # the fake Datadog of the end-to-end tests); links still use DD_SITE
# The application key needs read access to what is indexed. The service catalog needs
# `apm_service_catalog_read` and change events `events_read`; disable those sources
# below if the key lacks them.

# Indexer
INDEXER_WATERMARK=/data/watermark.json   # per-source checkpoint file
INDEXER_LOOKBACK_MINUTES=90              # window for a source's first run
INDEXER_OVERLAP_MINUTES=10               # re-read before each checkpoint for late arrivals
INDEXER_EMBED_BATCH_SIZE=128             # texts per embeddings request (1-2048)
INDEXER_EMBED_BATCH_MAX_CHARS=200000     # characters per embeddings request (rough token budget)
INDEXER_EMBED_CONCURRENCY=4              # embedding batches/upserts, lookups or deletes in flight (1-32)
INDEXER_ALLOW_EMPTY_SYNC_DELETE=false    # true: an empty monitor/dashboard/SLO/service catalog fetch deletes all indexed ones
INDEXER_SERVICE_CATALOG_ENABLED=true     # false: skip service definitions (GET /api/v2/services/definitions)
INDEXER_CHANGE_EVENTS_ENABLED=false      # true: index change events (POST /api/v2/events/search); off until the query below fits your deploy tooling
INDEXER_CHANGE_EVENTS_QUERY=             # event search query for deploys/config changes; default:
                                         # @evt.category:change OR source:(argocd OR spinnaker OR jenkins OR gitlab OR github OR launchdarkly OR terraform)

# Retrieval tuning (optional)
RAG_TOPK_DEFAULT=16
RAG_TOPK_MAX=32
RAG_TOPK_FIXED=              # optional: always use this K (1-64)
RAG_SEARCH_CANDIDATES=64
RAG_FUSION=rrf               # rrf|dbsf: how dense and keyword results are fused (docs/ARCHITECTURE.md#retrieval-and-ranking)
RAG_KEYWORD_STOPWORDS=off    # on|off: drop English function words from keyword queries (documents unchanged)

# Timeouts, deadlines and retries (optional; milliseconds)
RAG_HTTP_CONNECT_TIMEOUT_MS=5000     # per connection attempt (OpenAI + Qdrant clients)
RAG_HTTP_REQUEST_TIMEOUT_MS=60000    # per HTTP attempt
RAG_ASK_DEADLINE_MS=90000            # overall deadline for one /ask request
RAG_PLAN_TIMEOUT_MS=30000            # /ask/plan planning stage
RAG_EMBED_TIMEOUT_MS=15000           # embedding stage (incl. retries)
RAG_SEARCH_TIMEOUT_MS=15000          # Qdrant hybrid search stage (incl. retries)
RAG_GENERATE_TIMEOUT_MS=60000        # answer generation stage (incl. retries)
RAG_RETRY_MAX_ATTEMPTS=3             # total attempts per upstream call (1 = no retries)
RAG_RETRY_BASE_DELAY_MS=200          # first backoff; doubles per retry, with jitter
RAG_RETRY_MAX_DELAY_MS=10000         # backoff cap; a longer Retry-After fails fast

# Live evidence (optional; API). Uses DD_API_KEY/DD_APP_KEY/DD_SITE above; without
# them live evidence is disabled and reported per request, and the API still starts.
RAG_LIVE_EVIDENCE=on                 # on|off: query live Datadog data for diagnostic questions
RAG_LIVE_EVIDENCE_TIMEOUT_MS=20000   # budget for all live queries of one question (incl. retries)
RAG_LIVE_MAX_SERVICES=3              # services queried per question (logs + metric scope)
RAG_LIVE_MAX_METRICS=5               # metric queries per question
RAG_LIVE_MAX_LOG_EVENTS=1000         # error/warn logs fetched per service
RAG_LIVE_MAX_WINDOW_HOURS=168        # longer windows are not queried live

# Where the API listens, as host:port (default 0.0.0.0:5191)
RAG_API_ADDR=0.0.0.0:5191

# Test-only (API): fixes "now" at this RFC 3339 time, so the end-to-end tests can ask
# "yesterday" questions about a recorded corpus. Never set it in a deployment.
RAG_TEST_FIXED_NOW=
```

For live evidence the API's Datadog application key needs the `timeseries_query`
(`GET /api/v1/query`) and `logs_read_data` (`POST /api/v2/logs/events/search`)
permissions/scopes. The indexer additionally needs read access to monitors,
dashboards, SLOs, incidents and metrics.

## Running locally

### API
```
cd crates/rag-api
cargo run
```

### CLI (from source)

```
cd crates/rag-cli
RAG_API_BASE=http://localhost:5191 cargo run -- ask "why did auth-api spike yesterday?"
```

> No `--k` flag — server dynamically chooses K.  
> No `--env` or `--service` needed — planner infers them automatically (e.g., “auth-api prod”).

Manual override if desired (explicit flags always win over inferred values):
```
cargo run -- ask "auth-api latency spikes" --env prod --service auth-api --kind logs --tz Europe/Stockholm
```

`--tz` defaults to `TZ` (when it is an IANA name) or the system timezone.

The CLI prints a readable report on stdout; for diagnostic questions it includes the
live-evidence timeline (observed facts with links, hypotheses citing them, and what
couldn't be checked). `--no-live-evidence` skips the live queries and `--json` prints
the raw API response. See [Using the CLI](../README.md#using-the-cli).

Installers and prebuilt binaries are described in the [README](../README.md#installing-the-cli).

### Indexer (manual run)
```
cd crates/rag-indexer
INDEXER_WATERMARK=./watermark.json DD_API_KEY=... DD_APP_KEY=... DD_SITE=datadoghq.eu OPENAI_API_KEY=... QDRANT_ENDPOINT=http://localhost:6333 QDRANT_COLLECTION=datadog_rag cargo run
```

The first run creates the collection (`QDRANT_COLLECTION`) with a `dense` and a `sparse`
vector per point ([Qdrant storage](ARCHITECTURE.md#qdrant-storage)). A collection
written before hybrid search (one unnamed vector) is refused: point `QDRANT_COLLECTION`
at a new name, or delete the old collection, and start with a new checkpoint file
(`INDEXER_WATERMARK`) so that the first run fetches the full lookback.

How the indexer windows, checkpoints and deduplicates is described in
[ARCHITECTURE.md](ARCHITECTURE.md#how-the-indexer-resumes). Unchanged documents are not
re-embedded, and obsolete chunks and deleted monitors, dashboards and SLOs are removed
([Incremental indexing](ARCHITECTURE.md#incremental-indexing)).

## Docker

### Building Images

Build server services from pre-compiled binaries:

```bash
# Build release binaries first
cargo build --release

# Build Docker images
docker build -f crates/rag-indexer/Dockerfile -t rag-indexer:latest .
docker build -f crates/rag-api/Dockerfile -t rag-api:latest .
```

### Running with Docker

**Indexer:**
```bash
docker run --rm \
  -e OPENAI_API_KEY=... \
  -e QDRANT_ENDPOINT=http://qdrant:6333 \
  -e QDRANT_COLLECTION=datadog_rag \
  -e DD_API_KEY=... \
  -e DD_APP_KEY=... \
  -e DD_SITE=datadoghq.eu \
  rag-indexer:latest
```

**API:**
```bash
docker run --rm -p 5191:5191 \
  -e OPENAI_API_KEY=... \
  -e QDRANT_ENDPOINT=http://qdrant:6333 \
  -e QDRANT_COLLECTION=datadog_rag \
  -e DD_API_KEY=... \
  -e DD_APP_KEY=... \
  -e DD_SITE=datadoghq.eu \
  rag-api:latest
```

## Example CronJob (Kubernetes)

```yaml
apiVersion: batch/v1
kind: CronJob
metadata:
  name: rag-indexer
spec:
  schedule: "*/15 * * * *"
  # Runs share the checkpoint file, so never let two overlap.
  concurrencyPolicy: Forbid
  jobTemplate:
    spec:
      template:
        spec:
          restartPolicy: OnFailure
          securityContext:
            fsGroup: 1000   # the image runs as UID 1000; make the volume writable
          containers:
          - name: indexer
            image: ghcr.io/yourorg/rag-indexer:latest
            env:
            - name: OPENAI_API_KEY
              valueFrom: { secretKeyRef: { name: openai, key: apiKey } }
            - name: QDRANT_ENDPOINT
              value: http://qdrant:6333
            - name: QDRANT_COLLECTION
              value: datadog_rag
            - name: DD_API_KEY
              valueFrom: { secretKeyRef: { name: datadog, key: apiKey } }
            - name: DD_APP_KEY
              valueFrom: { secretKeyRef: { name: datadog, key: appKey } }
            - name: DD_SITE
              value: datadoghq.eu
            - name: INDEXER_WATERMARK
              value: /data/watermark.json
            - name: INDEXER_LOOKBACK_MINUTES
              value: "90"
            - name: INDEXER_OVERLAP_MINUTES
              value: "10"
            volumeMounts:
            - name: data
              mountPath: /data
          volumes:
          - name: data
            persistentVolumeClaim:
              claimName: rag-indexer-data
# Checkpoints must survive between runs; with emptyDir every run would start over.
apiVersion: v1
kind: PersistentVolumeClaim
metadata:
  name: rag-indexer-data
spec:
  accessModes:
  - ReadWriteOnce
  resources:
    requests:
      storage: 1Gi
```

## Testing

### Running Tests

```bash
# Run all tests
cargo test

# Run tests for a specific crate
cargo test -p rag-core

# Run tests with output
cargo test -- --nocapture
```

### Qdrant contract test

The `Build` CI workflow starts a real Qdrant (v1.19.1) as a service container and,
after the regular test run, runs the ignored `qdrant_roundtrip` upsert/search test
and the real-Qdrant pipeline tests against it. Both runs are instrumented, so the coverage report includes the Qdrant
client paths. To run it locally, start an isolated Qdrant instance, then run:

```bash
QDRANT_TEST_ENDPOINT=http://localhost:6333 cargo test --locked -p rag-core --test qdrant_roundtrip -- --ignored
```

The test creates its own uniquely named collection with `create_collection` (after
`check_collection` reports it missing, and checks that a collection with the old unnamed
vector is refused), deletes it afterwards, and checks chunk identity, full payload
recovery, filtering (including the time window), idempotent upserts, hybrid search (an
exact identifier found by keywords although its dense vector ranks second, both
questions fused in one query, normalized scores), and the incremental-indexing calls:
retrieving bookkeeping by point ID, `set_payload`, and counting and deleting by the
shrink and stale-document filters.

Recommended payload indexes for large collections are listed under
[Qdrant storage](ARCHITECTURE.md#qdrant-storage).

### Pipeline tests (writer against reader)

Unit tests check each side against hand-written JSON, so they keep passing when the
indexer's writer and the API's reader drift apart. The pipeline tests in
`crates/rag-indexer/src/pipeline_tests/` run both for real on one store:

1. a fake Datadog API serves the recorded fixtures (`crates/rag-core/tests/fixtures/datadog`)
   and a crafted corpus, with the pagination the adapters follow, plus the per-object
   endpoints (dashboard definitions, incident timelines and attachments, notebooks);
2. the indexer's own `index_sources` + `IncrementalSink` (chunking, content hashes,
   batched embeddings, upserts, cleanup) writes to the store;
3. the API's router (`rag_api::app`, served with a fixed clock) answers `/ask`: planner
   reply, `RetrievalScope` filter, search, rerank, prompt, `sources`, `citationWarnings`.

Embeddings and chat come from a deterministic fake OpenAI server: a signed, hashed
bag-of-words vector per text (1024 dimensions, so texts sharing words are close), a
canned planner reply per question, and an answer model that cites the documents it is
told to by reading its prompt. The store is an in-memory fake Qdrant that stores what
the writer sent and evaluates the filter subset Tails uses (`must`/`should`/`must_not`,
`match` value/any/except, numeric and datetime `range`, `is_empty`, `is_null`, `has_id`)
and the hybrid query: a collection of named cosine dense and IDF sparse vectors, and
`POST /points/query` with filtered dense and sparse prefetches fused by reciprocal rank
fusion or distribution-based score fusion, scored the way Qdrant 1.19 scores them
(sparse: `Σ query value · idf · stored value` over shared indices,
`idf = ln(1 + (N − n + 0.5)/(n + 0.5))` over the collection; RRF: `Σ 1/(k + rank)`;
DBSF: per list `(s − μ + 3σ) / 6σ` with Welford's mean and sample σ in f32, unclipped,
0.5 for one point or equal scores, summed; equal scores, which Qdrant orders
arbitrarily, by point ID). `support::qdrant::tests` checks these scores against
hand-computed values (including ties, one-point lists, points missing from a list and
an outlier beyond 3σ), on the fake and (ignored by default, `query_api_matches_real_qdrant`
and `dbsf_matches_real_qdrant`) on a real Qdrant. Anything else, such as the old unnamed
vector, a plain `points/search` or another fusion, it rejects and the test fails. Each test also has a `*_real_qdrant`
variant, ignored by default, that uses a fresh collection on `QDRANT_TEST_ENDPOINT`.

| Test | Checks |
|------|--------|
| `pipeline_tests::contract` | Every stored point decodes (reader's `QdrantPayload`) to the chunk the adapters produced, under the UUIDv5 of its ID, and has exactly the `dense` and `sparse` vectors the reader queries, the sparse one built from the chunk's embedding input; the collection passes the indexer's layout check; `Kind` equals the filter's value; `Timestamp` is RFC 3339; `Service`/`Environment` are lowercase; `ContentHash`/`ChunkCount`/`SyncId` never reach `sources`. `/ask` returns the right documents for service (`Auth-API` from Datadog vs `AUTH-API` from the planner), environment, kind and time filters (half-open window, timeless kinds, a log pattern day whose first and last log lie outside a short window it logged in, a day that logged only around the window left out); logs are grouped by pattern and UTC day with their count; service catalog entries (kind `catalog`) are stored undated with no environment and pass any window, change events (kind `change`) are filtered by event time, service and environment from their tags; a multi-chunk log pattern and the days of one pattern are one source, counted per day in the asker's timezone in the prompt; `sources` are stored documents numbered like the prompt; unknown citations appear in `citationWarnings`; a second run rewrites nothing. |
| `pipeline_tests::unicode` | See [Unicode policy](#unicode-policy). |
| `pipeline_tests::quality` | The [incident question set](#incident-question-set). |

```bash
# In-memory store: part of the normal test run.
cargo test -p rag-indexer pipeline_tests

# Real Qdrant (CI runs these in the Build workflow):
QDRANT_TEST_ENDPOINT=http://localhost:6333 cargo test -p rag-indexer --bin rag-indexer -- \
  --ignored --exact pipeline_tests::contract::pipeline_contract_real_qdrant \
  pipeline_tests::unicode::unicode_round_trip_real_qdrant \
  pipeline_tests::quality::incident_questions_real_qdrant \
  pipeline_tests::support::qdrant::tests::query_api_matches_real_qdrant \
  pipeline_tests::support::qdrant::tests::dbsf_matches_real_qdrant
```

The tests live in the indexer's binary crate (a `#[cfg(test)]` module), because that is
where the writer is; `rag-api` is a dev-dependency there, exposed as a library
(`AppState`, `app`, `serve`) with `main.rs` only reading the environment and serving.
The fake Datadog API, the fake OpenAI server and the question set's model and scoring
are in the `tails-fakes` crate (also a dev-dependency), which the
[end-to-end tests](#end-to-end-tests) run as a standalone server; the fake Qdrant stays
in `pipeline_tests::support`.

### Unicode policy

Datadog text routinely contains Swedish `åäö`, `é`, emoji, CJK and combining marks, so
no code may cut text at a byte offset that is not a char boundary:

- Byte-limited cuts go through `rag_core::text::truncate_bytes` (the last char boundary
  at or below the limit, never separating a character from a following combining mark,
  variation selector, skin-tone modifier or zero-width-joiner sequence) or
  `truncate_with_marker`. Prompt excerpts (`EXCERPT_MAX_BYTES`, 1500 bytes, then
  ` …[truncated]`), log pattern sample messages (4000 bytes), change event messages
  (4000 bytes), service catalog descriptions (2000 bytes) and values (300 bytes),
  embedding header values
  (300 bytes for the title, 100 for other fields) and logged upstream error bodies (512
  bytes) use them.
- Character-limited cuts use `chars()`: the chunker (1800 chars, 200 overlap), log
  message patterns (160 chars, shared by indexing and live evidence) and CLI snippets.
- Never `String::truncate(n)`, `&s[..n]` or `split_at(n)` with `n` computed from a
  length, unless `n` comes from `find`/`char_indices` on the same string or only ASCII
  was skipped.

The tests put multibyte characters exactly at 1500 bytes (±5) and at chunk size ±3,
sweep every limit on mixed samples (`text.rs`), and check through the whole pipeline
(`pipeline_tests::unicode`) that nothing panics, excerpts are valid prefixes, chunks
reassemble to the stored text (for logs, a pattern document holding the original message)
and every payload survives the write/read round trip unchanged.

### Incident question set

`crates/rag-indexer/tests/incident_questions/` holds a versioned, human-readable
evaluation set: `questions.json` (36 incident questions) and `corpus.json` (monitors,
incidents with their timelines and postmortem notebooks, SLOs, logs, dashboards with
their definitions, metrics, service definitions (`serviceDefinitions`) and change events
(`events`) in Datadog response shape, with
distractors: a similarly named service, another environment, events outside the window,
a burst of 300 near-identical logs, patterns logged on other days of the week, error
codes and metric names that differ from the asked one in a single word, and events of
different ages next to timeless monitors and dashboards). `logBursts` in the corpus are expanded by the
harness into individual logs (`crates/tails-fakes/src/datadog.rs`, `expand_burst`).
The recorded fixtures are indexed alongside. Each question has a fixed `now` and
timezone, the canned planner reply (`plan`), optional explicit request fields, the
expected `scope`, `mustRetrieve`/`mayRetrieve`/`mustNotRetrieve` document IDs, expected
timeline observations with the live data that produces them, the `evidence` the prompt
must state, and what the answer cites.

Run it and print the report:

```bash
cargo test -p rag-indexer incident_questions_in_memory -- --nocapture
```

```
incident questions v4 | store: in-memory fake Qdrant | fusion=rrf stopwords=off | models: ...
question                     recall prec@R margin exclude scope   cites  intent   obs  evid srcs  notes
q01-checkout-slow-yesterday    1.00   1.00      -     8/8    ok     5/5     5/5   2/2   0/0    6
q09-checkout-march-2           1.00   1.00  1.284     4/4    ok     1/1     1/1   0/0   0/0    4
...
aggregate over 36 questions (known gaps excluded):
  recall@k                   1.000 (threshold 0.95)
```

- `recall`: share of `mustRetrieve` among `sources` (k = every source given to the
  answer model). `prec@R`: share of the first R sources that are must/may documents,
  R = number of `mustRetrieve`. `margin` (not thresholded): the lowest reranked score of
  a retrieved `mustRetrieve` source over the highest score of a source that is neither
  must nor may, from the prompt's `Score:` lines; above 1 the answer ranks first, and
  the closer to 1 the more fragile that is (`-` when there is nothing to compare).
  `exclude`: `mustNotRetrieve` kept out of `sources`.
  `scope`: the response's `scope` equals the expected keys.
- `cites`: citations in the answer that resolve / all citations (deliberate negative
  controls excluded). `intent`: intended citations that point at the intended document
  (`[DOC #n]` is `sources[n-1]`), and intended observations cited. `obs`: expected
  observations present with their ID, kind and service. `evid` (evidence accuracy):
  `evidence` entries met, each naming a document that must appear as exactly one
  `[DOC #n]` block of the answer prompt containing the given texts, such as a log
  pattern's count in the question's window. `srcs`: number of sources.
- `notes` lists misses, leaked distractors, scope differences, the top sources when
  precision drops, and the observations actually collected.

Every question also asserts, regardless of thresholds, that `sources` are stored
documents numbered like the prompt, that no document or log pattern is listed twice, and
that `citationWarnings` equals
`validate_citations` on the answer. The test fails when an aggregate drops below
`thresholds` in `questions.json`.

**Adding a question:** add documents to `corpus.json` if needed (IDs become
`monitor_<id>`, `incident_<id>`, `slo_<id>`, `dashboard_<id>`,
`metric_<name with dots as underscores>`, `catalog_<service>`, `change_<event id>`; logs
are indexed as
[pattern documents](ARCHITECTURE.md#log-patterns) per UTC day, and `log_<id>` in a
question names the log pattern holding log `<id>`, which is one source whichever of its
days represents it), then an entry to `questions`. Unknown IDs fail the
run. The fake embeddings are a hashed bag of words, so a question tests ranking only when
the words it shares with the relevant and the distracting documents decide the order. For live evidence add `live` (hourly series with `spikes`, log `bursts`); the
observation IDs are assigned chronologically, so run once and read the collected
observations in `notes` before writing `expect.timeline`. A question documenting a
known limitation gets `"knownGap": "<why>"`: it is reported but not counted, and the
report says when it starts passing.

**Comparing retrieval configurations:** the harness's API reads `RAG_FUSION`,
`RAG_RRF_K`, `RAG_RRF_WEIGHTS` and `RAG_KEYWORD_STOPWORDS` like `rag-api` (the report's
first line names them), so run it once per configuration and compare the aggregates and
the per-question `margin`:

```bash
for f in rrf dbsf; do for s in off on; do
  RAG_FUSION=$f RAG_KEYWORD_STOPWORDS=$s cargo test -p rag-indexer --bin rag-indexer \
    incident_questions_in_memory -- --nocapture > "report-$f-$s.txt"
done; done
```

With real embeddings, use `E2E_COMPARE` of the [end-to-end run](#end-to-end-tests).

**Changing thresholds:** thresholds are the committed floor, not the current score. Raise
one when a change improves the aggregate for good. Lower one only together with the
change that justifies it, and say why in the pull request; bump `version` when questions
or expectations change meaning.

**Real models (opt-in, never in CI):** `incident_questions_openai` uses
`OPENAI_API_KEY` (and optionally `OPENAI_BASE_URL`, `OPENAI_EMBEDDING_MODEL`,
`OPENAI_CHAT_MODEL`) for embeddings and answers, sends the canned plans with the request,
and uses real Qdrant when `QDRANT_TEST_ENDPOINT` is set. It enforces the retrieval and
citation-validity thresholds; citation accuracy and negative controls only apply to the
canned answer model.

```bash
OPENAI_API_KEY=... cargo test -p rag-indexer incident_questions_openai -- --ignored --nocapture
```

### End-to-end tests

The pipeline tests call the indexer and the API in-process. The end-to-end run
(`scripts/e2e.sh`, the `e2e` job of the `Build` workflow) runs the **built release
binaries as separate processes** instead, configured only through their environment,
and asks the incident question set through `rag-cli`:

| Component | In the e2e run |
|-----------|----------------|
| `rag-indexer`, `rag-api`, `rag-cli` | Real release binaries |
| Qdrant | Real (`qdrant/qdrant:v1.19.1`), a fresh collection per run |
| Embeddings | Real: [text-embeddings-inference](https://github.com/huggingface/text-embeddings-inference) (`cpu-1.9.4`) serving `BAAI/bge-small-en-v1.5` (384 dimensions) on its OpenAI-compatible `POST /v1/embeddings`, reached through `OPENAI_EMBEDDING_BASE_URL` |
| Datadog API | Fake: `tails-fakes serve`, the question set's corpus plus the recorded fixtures, and each question's live data |
| Chat model | Fake: `tails-fakes serve`, the canned plan, hypotheses and answer of each question |

What it covers beyond the pipeline tests: the binaries' environment handling
(`OPENAI_*`, `QDRANT_*`, `DD_*`, `INDEXER_*`), the indexer creating the collection from a
probe embedding, a real embedding model's batching and dimension, `rag-cli`'s request and
`--json` output, and retrieval quality with real embeddings.

`tails-fakes serve` matches each chat request to its question by the question text: the
planner's user message is the question, and the hypotheses and answer prompts start with
it. The planner request also selects which question's live data answers the following
live Datadog queries, so questions are asked one at a time. The answer model cites the
documents in `answer.citeDocuments` by the source URIs the adapters give them (computed
at startup from the same corpus and `--site`, which must equal the indexer's `DD_SITE`).
It does not serve embeddings unless started with `--embeddings`.

The script builds the binaries, starts `tails-fakes`, runs `rag-indexer` once (a first run
over the whole corpus, with change events and the service catalog on) and requires exit 0
and a watermark file. `tails-e2e` then probes the embedding endpoint for its dimension,
requires the collection to exist with that dimension, reads every stored point back, and
for each distinct `now` in the question set starts `rag-api` with the clock fixed at it
(`RAG_TEST_FIXED_NOW`, which only the tests set: most questions say "yesterday" about a
recorded corpus) and asks that `now`'s questions with
`rag-cli ask <question> --tz <timezone> [--service/--env/--kind] --json`. Processes,
the collection and the work directory are removed at the end.

Checks, with the harness's own scoring (`tails_fakes::questions`):

- **Hard (must pass):** the indexer exits 0; the collection's dimension equals the
  model's; every `rag-cli ask --json` exits 0 with a well-formed response; `sources` are
  stored documents, numbered like the prompt, none listed twice; `citationWarnings` equals
  `validate_citations` on the answer; negative controls are flagged; the scope (service,
  environment, window, kinds) equals the expected one.
- **Quality (thresholded):** the aggregates of the [incident question set](#incident-question-set)
  against `tests/incident_questions/e2e_thresholds.json`, separate from the fake-embedding
  thresholds in `questions.json`. They were measured with `BAAI/bge-small-en-v1.5` and set
  slightly below the measured values; questions that miss with real embeddings are listed
  in the report's notes, and the fake-embedding expectations are not changed for them.
  Change the e2e thresholds like the others: raise them when retrieval improves for good,
  lower them only with the change that justifies it. Changing the model means measuring
  again.

Run it locally (needs Docker, or Qdrant and TEI already running):

```bash
# Start Qdrant and TEI in docker (the model is cached in ~/.cache/tails-e2e/tei)
E2E_DOCKER=qdrant,tei scripts/e2e.sh

# Against running services
QDRANT_ENDPOINT=http://localhost:6333 TEI_URL=http://localhost:8080 scripts/e2e.sh

# No embedding model: tails-fakes' hashed bag-of-words embeddings. Checks the processes
# and the plumbing; the scores then match the in-process harness.
E2E_EMBEDDINGS=fake QDRANT_ENDPOINT=http://localhost:6333 scripts/e2e.sh

# OpenAI's embeddings (OPENAI_EMBEDDING_MODEL, default text-embedding-3-small, at
# OPENAI_EMBEDDING_BASE_URL, default https://api.openai.com); chat stays on the fakes
E2E_EMBEDDINGS=openai OPENAI_EMBEDDING_API_KEY=... QDRANT_ENDPOINT=http://localhost:6333 scripts/e2e.sh
```

Other variables: `TEI_IMAGE`, `TEI_MODEL`, `TEI_MODEL_REVISION`, `TEI_QUERY_PREFIX` (the
model's query instruction, bge's by default; empty for a model without one),
`TEI_DOCUMENT_PREFIX` (the model's document prefix, such as e5's `passage: `; none by
default, as bge wants), `TEI_DATA` (model
cache), `FAKES_ADDR` (default `127.0.0.1:8900`), `API_ADDR` (where the started `rag-api`
listens via `RAG_API_ADDR`, default `127.0.0.1:5191`), `E2E_SKIP_BUILD=1`, `E2E_KEEP=1`
(keep the collection and the logs), `E2E_OUT` (a directory to copy every run's
`--summary` JSON into). Both addresses must be free.

**Comparing query-side configurations:** fusion (`RAG_FUSION`) and keyword stopwords
(`RAG_KEYWORD_STOPWORDS`) only change how `rag-api` queries, so one index serves them
all. `E2E_COMPARE` lists further configurations as `fusion:stopwords`; after the main
run (the configuration in the environment, else the defaults), `tails-e2e` asks every
question again under each, and `scripts/e2e_compare.py` prints their aggregates side by
side and every question whose recall, precision@R, excluded distractors or `margin`
differs. Only the main run decides the exit status. The CI job compares the defaults
with the other three combinations:

```bash
E2E_COMPARE="dbsf:off rrf:on dbsf:on" E2E_DOCKER=qdrant,tei scripts/e2e.sh
```

An `rrf` entry can add RRF's parameters as `:k=<RAG_RRF_K>` and
`:w=<dense>,<keyword>` (`RAG_RRF_WEIGHTS`), e.g.
`E2E_COMPARE="rrf:off:k=60 rrf:off:w=1,2 rrf:on:k=10:w=2,1"`.

For a question whose must-retrieve documents are not all ranked first, `tails-e2e`
also prints ("explain") the top documents of dense and keyword search alone, the fused
hybrid search under the run's configuration, and the reranked scores.

The model is pinned to a Hugging Face commit (`TEI_MODEL_REVISION`, passed to TEI as
`REVISION`), because the e2e thresholds were measured with it and a new upload of the same
model name could change the embeddings. The script reads the revision TEI actually loaded
from its `/info` endpoint and stops if it differs. To move to a new revision, update
`TEI_MODEL_REVISION` in `build.yml` (and the script's default), run the job, and set the
thresholds in `e2e_thresholds.json` from its report. Set `TEI_MODEL_REVISION=` (empty)
to run a local model without the check. In CI, the job starts TEI with
`docker run` after restoring the model cache (a service container would start before the
cache is restored); indexing runs one embedding batch of at most 32 texts at a time, so
TEI on CPU embeds the same inputs the same way on every run.

### Mutation Testing

The project uses [cargo-mutants](https://mutants.rs/) for mutation testing to identify missing test coverage:

```bash
# Install cargo-mutants
cargo install cargo-mutants

# Run mutation tests
cargo mutants

# Run on specific package
cargo mutants --package rag-core
```

For detailed mutation testing results and recommendations, see [MUTATION_TESTING_REPORT.md](../MUTATION_TESTING_REPORT.md).

**Current Test Coverage:**
- **141 unit tests** covering core functionality, API logic, and indexer
- **Mutation testing:** 43.1% caught (62/144 mutants) - **+24.7% improvement!**
- Strong coverage of OpenAI client (100%), Qdrant client (80%), and choose_topk logic (76.5%)
- See report for areas needing additional test coverage

### Releases

CLI release builds and release notes are described in
[RELEASE_DOCUMENTATION.md](RELEASE_DOCUMENTATION.md).
