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

# Qdrant
QDRANT_ENDPOINT=http://qdrant:6333
QDRANT_COLLECTION=datadog_rag

# Datadog
DD_API_KEY=...
DD_APP_KEY=...
DD_SITE=datadoghq.eu    # or datadoghq.com

# Indexer
INDEXER_WATERMARK=/data/watermark.json   # per-source checkpoint file
INDEXER_LOOKBACK_MINUTES=90              # window for a source's first run
INDEXER_OVERLAP_MINUTES=10               # re-read before each checkpoint for late arrivals
INDEXER_EMBED_BATCH_SIZE=128             # texts per embeddings request (1-2048)
INDEXER_EMBED_BATCH_MAX_CHARS=200000     # characters per embeddings request (rough token budget)
INDEXER_EMBED_CONCURRENCY=4              # embedding batches/upserts, lookups or deletes in flight (1-32)
INDEXER_ALLOW_EMPTY_SYNC_DELETE=false    # true: an empty monitor/dashboard/SLO fetch deletes all indexed ones

# Retrieval tuning (optional)
RAG_TOPK_DEFAULT=16
RAG_TOPK_MAX=32
RAG_TOPK_FIXED=              # optional: always use this K (1-64)
RAG_SEARCH_CANDIDATES=64

# Timeouts, deadlines and retries (optional; milliseconds)
RAG_HTTP_CONNECT_TIMEOUT_MS=5000     # per connection attempt (OpenAI + Qdrant clients)
RAG_HTTP_REQUEST_TIMEOUT_MS=60000    # per HTTP attempt
RAG_ASK_DEADLINE_MS=90000            # overall deadline for one /ask request
RAG_PLAN_TIMEOUT_MS=30000            # /ask/plan planning stage
RAG_EMBED_TIMEOUT_MS=15000           # embedding stage (incl. retries)
RAG_SEARCH_TIMEOUT_MS=15000          # Qdrant search stage (incl. retries)
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

How the indexer windows, checkpoints and deduplicates is described in
[ARCHITECTURE.md](ARCHITECTURE.md#how-the-indexer-resumes). Unchanged documents are not
re-embedded, and obsolete chunks and deleted monitors, dashboards and SLOs are removed
([Incremental indexing](ARCHITECTURE.md#incremental-indexing)). The first run after
upgrading to incremental indexing re-embeds everything once, because existing points
have no content hash yet.

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
against it. Both runs are instrumented, so the coverage report includes the Qdrant
client paths. To run it locally, start an isolated Qdrant instance, then run:

```bash
QDRANT_TEST_ENDPOINT=http://localhost:6333 cargo test --locked -p rag-core --test qdrant_roundtrip -- --ignored
```

The test creates and deletes its own uniquely named collection and checks chunk
identity, full payload recovery, filtering (including the time window), idempotent
upserts, and the incremental-indexing calls: retrieving bookkeeping by point ID,
`set_payload`, and counting and deleting by the shrink and stale-document filters.

Recommended payload indexes for large collections are listed under
[Qdrant storage](ARCHITECTURE.md#qdrant-storage).

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
