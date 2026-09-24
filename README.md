# Tails

Datadog RAG, built in Rust.

Includes a **one-shot Datadog indexer**, **RAG API**, and **CLI** with automatic inference of
`service` and `environment`. The server dynamically selects `topK` based on question intent.

---

## Quickstart

```bash
# 1. Index Datadog data into Qdrant
cd crates/rag-indexer
cargo run

# 2. Start the RAG API
cd ../rag-api
cargo run

# 3. Ask a question via CLI
cd ../rag-cli
RAG_API_BASE=http://localhost:5191 cargo run -- ask "why did auth-api spike yesterday?"
```

The planner infers `service`/`environment` automatically.  
The server chooses `K` dynamically — no `--k` flag needed.

---

## Crates Overview

| Crate | Description |
|-------|--------------|
| `rag-core` | Domain models, OpenAI, Qdrant (search + upsert), Datadog client (monitors, incidents, logs, dashboards, metrics, SLOs), chunker, planner, reranker, RAG service. |
| `rag-api` | Axum REST API — `/ask/plan` (intent + inferred filters) and `/ask` (retrieval + answer). |
| `rag-cli` | CLI that calls the API. Planner infers service/env; server decides top-K. |
| `rag-indexer` | One-shot, resumable indexer for Datadog → Qdrant with per-source checkpoints. Perfect for Kubernetes CronJob. |

---

## Development

### Prerequisites

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

# Retrieval tuning (optional)
RAG_TOPK_DEFAULT=16
RAG_TOPK_MAX=32
RAG_SEARCH_CANDIDATES=64
```

---

## Running

### API
```
cd crates/rag-api
cargo run
```

### CLI

**Quick Install** (recommended):

Linux/macOS:
```bash
curl -fsSL https://raw.githubusercontent.com/MattiasHognas/tails/main/install.sh | bash
```

Windows (PowerShell):
```powershell
irm https://raw.githubusercontent.com/MattiasHognas/tails/main/install.ps1 | iex
```

The installer will:
- Detect your platform automatically
- Download the latest release
- Update existing installation if found
- Add to PATH (on Windows)

**Advanced Installation Options**:

Install to a custom directory (Linux/macOS):
```bash
INSTALL_DIR=/usr/local/bin VERSION=v1.0.0 bash install.sh
```

Install to a custom directory (Windows):
```powershell
.\install.ps1 -InstallDir "C:\Tools\rag-cli" -Version "v1.0.0"
```

Default installation locations:
- Linux/macOS: `$HOME/.local/bin/rag-cli`
- Windows: `%LOCALAPPDATA%\rag-cli\rag-cli.exe`

**Manual Download**:
- Linux (x86_64): `rag-cli-linux-x86_64`
- Linux (aarch64): `rag-cli-linux-aarch64`
- macOS (Intel): `rag-cli-macos-x86_64`
- macOS (Apple Silicon): `rag-cli-macos-aarch64`
- Windows (x86_64): `rag-cli-windows-x86_64.exe`

Binaries are automatically built and available as [GitHub Release](https://github.com/MattiasHognas/tails/releases) artifacts.

**Build from source** (development)
```
cd crates/rag-cli
RAG_API_BASE=http://localhost:5191 cargo run -- ask "why did auth-api spike yesterday?"
```

> No `--k` flag — server dynamically chooses K.  
> No `--env` or `--service` needed — planner infers them automatically (e.g., “auth-api prod”).

Manual override if desired:
```
cargo run -- ask "auth-api latency spikes" --env prod --service auth-api
```

### Indexer (manual run)
```
cd crates/rag-indexer
INDEXER_WATERMARK=./watermark.json DD_API_KEY=... DD_APP_KEY=... DD_SITE=datadoghq.eu OPENAI_API_KEY=... QDRANT_ENDPOINT=http://localhost:6333 QDRANT_COLLECTION=datadog_rag cargo run
```

### How the indexer resumes

Each run indexes every source (monitors, dashboards, SLOs, metrics, incidents, logs)
independently and records its progress in the checkpoint file at `INDEXER_WATERMARK`:

```json
{ "sources": { "logs": { "indexed_until": "2025-01-01T12:00:00Z" }, "incidents": { "indexed_until": "2025-01-01T11:45:00Z" } } }
```

- **Complete fetches:** every Datadog list is paginated to the end (logs via the
  `meta.page.after` cursor, 1000 per page, oldest first).
- **Per-source checkpoints:** a source's checkpoint advances to the run's start time only
  after all of its documents are embedded and upserted, and the file is rewritten
  atomically (temp file + rename). A failing source is logged and retried from its old
  checkpoint on the next run while the others advance; the run then exits non-zero.
- **Windows:** logs, incidents and metrics are fetched from their checkpoint minus
  `INDEXER_OVERLAP_MINUTES` (default 10) until now, so records that reach Datadog late
  are picked up by the next run. A source without a checkpoint starts
  `INDEXER_LOOKBACK_MINUTES` (default 90) back. After an outage, the whole gap since the
  checkpoint is fetched. Monitors, dashboards and SLOs are re-synced in full each run.
- **No duplicates:** documents are deduplicated by ID within a run, and Qdrant point IDs
  are derived from document IDs, so re-indexing overlapping records overwrites them.
- **Upgrades:** a legacy watermark file holding a single timestamp is used as the
  checkpoint for every source and rewritten in the new format. An unreadable checkpoint
  file stops the run instead of silently re-starting from the lookback; delete it to
  start over.

---

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

---

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
---
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

---

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

### Qdrant storage contract

Ingestion and retrieval share a typed payload in `rag-core::qdrant`. Stored keys
are `Title`, `Text`, `SourceUri`, `Kind`, `Timestamp`, `Service`, `Environment`,
`Metadata`, and lowercase `id`. The payload `id` remains the logical chunk ID
(for example, `monitor_123#c0`); `Metadata.chunk_of` preserves the parent ID.
Qdrant point IDs are deterministic UUIDv5 values derived from the logical chunk ID
in a fixed Tails namespace, so retries and content updates replace the same point.

The `Qdrant contract` CI workflow runs an upsert/search test against real Qdrant.
To run it locally, start an isolated Qdrant instance, then run:

```bash
QDRANT_TEST_ENDPOINT=http://localhost:6333 cargo test --locked -p rag-core --test qdrant_roundtrip -- --ignored
```

The test creates and deletes its own uniquely named collection and checks chunk
identity, full payload recovery, filtering, and idempotent upserts.

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

For detailed mutation testing results and recommendations, see [MUTATION_TESTING_REPORT.md](MUTATION_TESTING_REPORT.md).

**Current Test Coverage:**
- **141 unit tests** covering core functionality, API logic, and indexer
- **Mutation testing:** 43.1% caught (62/144 mutants) - **+24.7% improvement!**
- Strong coverage of OpenAI client (100%), Qdrant client (80%), and choose_topk logic (76.5%)
- See report for areas needing additional test coverage

---

## Notes

- The planner (`/ask/plan`) extracts **intent**, **time window**, **service/env**, and **clarifying questions**.
- The API embeds the rewritten query, searches Qdrant, and reranks results using hybrid heuristics.
- The CLI automatically displays clarifying questions if planner uncertainty is high.
- Default values can be tuned via environment variables on the API service.
