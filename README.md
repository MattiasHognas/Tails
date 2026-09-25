# Tails

Datadog RAG, built in Rust.

Includes a **one-shot Datadog indexer**, **RAG API**, and **CLI** with automatic inference of
`service` and `environment`. The server dynamically selects `topK` based on question intent.

For how the pieces fit together, see [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).
To build, configure, run, deploy or test Tails, see [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md).

**Testing:** `cargo test` runs the unit tests and the in-process pipeline tests (fake
Datadog, fake OpenAI, in-memory Qdrant). CI also runs them against a real Qdrant, and an
[end-to-end run](docs/DEVELOPMENT.md#end-to-end-tests) (`scripts/e2e.sh`) of the built
indexer, API and CLI against real Qdrant and real embeddings from
text-embeddings-inference, with Datadog and the chat model faked.

**Self-hosted embeddings:** set `OPENAI_EMBEDDING_BASE_URL` (and, if needed,
`OPENAI_EMBEDDING_API_KEY`) to send embeddings to another OpenAI-compatible server, such as
text-embeddings-inference, while chat stays on `OPENAI_BASE_URL`.

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

The server plans each question: it infers `service`/`environment`, resolves time
phrases like "yesterday" in your timezone, and applies them as Qdrant filters.  
The server chooses `K` dynamically — no `--k` flag needed.

Configuration (OpenAI, Qdrant and Datadog credentials) is described in
[docs/DEVELOPMENT.md](docs/DEVELOPMENT.md#environment-variables).

---

## Using the CLI

Once the API is running (see [Quickstart](#quickstart)), install `rag-cli` (see
[Installing the CLI](#installing-the-cli)) and point it at the API:

```bash
export RAG_API_BASE=http://localhost:5191   # default
export RAG_API_TOKEN=...                    # optional bearer token

# Ask in plain language. The server infers service, environment and time window.
rag-cli ask "why did auth-api return 5xx errors yesterday?"

# Pin what you already know. Explicit flags always win over inferred values.
rag-cli ask "latency spikes after the deploy" --service auth-api --env prod

# Only use certain evidence (repeatable): logs, metrics, monitor, incident, dashboard, slo, git,
# catalog (service catalog: owners, on-call, runbooks, dependencies), change (deploys and config changes)
rag-cli ask "what alerted overnight?" --kind monitor --kind incident

# Ownership and runbooks come from the Datadog Software Catalog
rag-cli ask "who owns checkout and where is its runbook?"

# Deploys and configuration changes are indexed as change events
# (off by default; set INDEXER_CHANGE_EVENTS_ENABLED=true on the indexer)
rag-cli ask "what changed in checkout right before the errors started at 14:02?" --env prod

# Resolve "yesterday", "last 2 hours", ... in a specific timezone (default: your system zone)
rag-cli ask "errors in payments since yesterday" --tz Europe/Stockholm

# See how the server interprets a question, without retrieving or answering
rag-cli plan "why did checkout fail in staging last night?"
```

`ask` prints a readable report on stdout:

```text
$ rag-cli ask "why did auth-api fail yesterday?" --tz Europe/Stockholm
Answer
  auth-api returned 5xx between 12:05 and 12:40 [1]. Latency spiked at the same
  time [obs-2], and error logs show the database connection pool was exhausted
  [obs-3].

  Top signals:
  - Error rate above 5% on auth-api in prod [1]
  - Latency monitor fired [2]

  Next steps:
  1. Check the connection pool size and recent deploys to auth-api.

Scope
  auth-api · prod · 2026-09-23 00:00–24:00 Europe/Stockholm

Evidence
  2 indexed documents · 2 live observations

Live evidence
  Window: 2026-09-23 00:00–24:00 Europe/Stockholm
  Observed:
    [obs-2] 2026-09-23 12:00–13:00 trace.http.request.duration: 2 point(s) above
      the baseline band (0.3); extreme 1.5
      https://app.datadoghq.eu/metric/explorer?exp_metric=trace.http.request.duration
    [obs-3] 2026-09-23 12:10–12:20 6 error logs: db connection pool exhausted
      after #ms
      https://app.datadoghq.eu/logs?query=service%3Aauth-api
  Hypotheses (unverified):
    - Connection pool exhaustion slowed requests [obs-2, obs-3]
  Not checked:
    - metrics for payments (no_metrics_discovered): no metric documents or
      monitor queries mention payments

Sources
  [1] auth-api 5xx spike (incident, 2026-09-23 12:02) ·
      https://app.datadoghq.eu/incidents/1
  [2] auth-api latency (monitor) · https://app.datadoghq.eu/monitors/7
```

- **Answer**: the model's answer. Its `[DOC #n]` citations are shown as `[n]` and listed
  under **Sources**; `[obs-N]` cites a live observation.
- **Scope**: the service, environment, time window (in your timezone) and source kinds
  actually used for retrieval.
- **Evidence**: how many indexed documents and live observations the answer is based on.
  With no evidence, the answer says so and suggests widening the window, dropping
  `--service`/`--env` or checking that the data has been indexed.
- **Live evidence**: for diagnostic questions (for example "why did …", "did latency spike
  …") with a time window, the API also queries Datadog live for that window: observed
  facts with Datadog links, hypotheses that cite them, and what couldn't be checked. It is
  hidden for non-diagnostic questions and when you pass `--no-live-evidence`, which skips
  the live queries.
- **Need more info**: the planner's clarifying questions, when it has any.

`plan` prints the interpreted intent, service, environment, window and other fields as
key/value lines.

Colors and bold are used only when stdout is a terminal and `NO_COLOR` is not set; text
is then wrapped to the terminal width. When stdout is piped or redirected, the output is
plain text with no ANSI codes and is not wrapped.

**`--json`** (on `ask` and `plan`) prints the raw API response instead, for scripts:

```bash
rag-cli ask --json "why did auth-api fail yesterday?" | jq -r '.sources[] | "\(.n) \(.uri)"'
```

**Output streams and exit codes** (both modes):

| Outcome | stdout | stderr | Exit code |
|---------|--------|--------|-----------|
| Answer (including `"evidence": "none"`) or plan | the result (text, or the API JSON with `--json`) | nothing | 0 |
| API error, unreachable API, timeout, invalid response | nothing | the error (one readable line plus a hint, or one line of JSON with `--json`) | 1 |
| Invalid arguments | nothing | clap's usage message | 2 |

Without `--json`, a failed step (planning, embedding, search or generation) prints the
typed error, for example
`error [upstream_unavailable] at stage 'retrieval' (HTTP 503): ...`, and client-side
failures print, for example,
`error [api_unreachable] (http://localhost:5191): connection refused` with a hint to check
`RAG_API_BASE`. See [API errors and evidence](docs/ARCHITECTURE.md#api-errors-and-evidence).

With `--json`, stderr gets a single line of JSON with the same shape as the API's typed
errors, so `jq -r .error.code` works on it:

```json
{"error":{"code":"api_unreachable","message":"could not connect to http://localhost:5191: connection refused","stage":null,"retryable":true}}
```

A typed API error body is passed through unchanged. Other failures use these codes:
`api_unreachable` (connection failed, retryable), `timeout` (retryable),
`request_failed`, `invalid_response` (a 2xx body that is not JSON), and
`upstream_http_error` for a non-2xx body that isn't a typed error, such as a proxy's HTML
page (retryable for 5xx and 429). `stage` is `null` for these.

```bash
if out=$(rag-cli ask --json "why did auth-api fail yesterday?" 2>err.json); then
  jq -r .answer <<<"$out"
else
  jq -r .error.code err.json
fi
```

> **Upgrading:** `rag-cli ask` and `rag-cli plan` used to print the raw API JSON on stdout
> (with the live-evidence timeline and clarifying questions on stderr). They now print
> readable text by default. Scripts that parse the output should add `--json`, which
> prints the API response exactly as before, and should read errors as JSON from stderr.

---

## Installing the CLI

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

---

## API

### `POST /ask`

```json
{
  "question": "why did auth-api fail yesterday?",
  "timezone": "Europe/Stockholm",
  "service": null,
  "env": null,
  "from_utc": null,
  "to_utc": null,
  "kinds": null,
  "filters": null,
  "rewritten_query": null,
  "plan": null,
  "live_evidence": null
}
```

Only `question` is required. The response is
`{"answer": "...", "evidence": "found" | "none", "sources": [...], "citationWarnings": [...], "plan": {...}, "scope": {"service", "environment", "fromUtc", "toUtc", "kinds"}, "timeline": {...}}`,
where `sources` lists the indexed documents given to the answer model, numbered like
the answer's `[DOC #n]` citations (`id` is the indexed document's ID):

```json
"sources": [{"n": 1, "id": "incident_1f0c…", "title": "auth-api 5xx spike", "kind": "incident",
             "timestamp": "2026-09-23T10:02:00Z", "service": "auth-api", "environment": "prod",
             "uri": "https://app.datadoghq.eu/incidents/1"}]
```

`sources` is empty when no answer model was called (`"evidence": "none"`).
`citationWarnings` lists citations in the answer that resolve to nothing, e.g.
`[{"citation": "DOC #7", "reason": "unknown_document"}]` or `"unknown_observation"` for
an `obs-N` not in the timeline; it is empty when every citation resolves (see
[Citation checks](docs/ARCHITECTURE.md#citation-checks)). `plan` is the
validated plan, `scope` is what was actually applied to retrieval and
`timeline` is the live Datadog evidence for diagnostic questions (see
[Live evidence](docs/ARCHITECTURE.md#live-evidence-timeline)). `"live_evidence": false`
turns the live queries off for one request.

The full request rules (precedence of explicit and inferred values, timezone handling,
validation and retrieval filters) are described in
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md#question-pipeline).

### `POST /ask/plan`

`{"question": "...", "timezone": "Europe/Stockholm"}` → `{"plan": {...}}`: the
validated plan, without retrieval.

Errors use a typed JSON body with `400`, `502`, `503`, `504` or `500`; see
[API errors and evidence](docs/ARCHITECTURE.md#api-errors-and-evidence).

---

## Documentation

- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md): components, flowchart, question pipeline, errors, indexing and storage.
- [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md): prerequisites, environment variables, running locally, Docker, Kubernetes and testing.
- [docs/RELEASE_DOCUMENTATION.md](docs/RELEASE_DOCUMENTATION.md): how CLI releases and their notes are produced.
