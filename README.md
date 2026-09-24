# Tails

Datadog RAG, built in Rust.

Includes a **one-shot Datadog indexer**, **RAG API**, and **CLI** with automatic inference of
`service` and `environment`. The server dynamically selects `topK` based on question intent.

For how the pieces fit together, see [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).
To build, configure, run, deploy or test Tails, see [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md).

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

# Only use certain evidence (repeatable): logs, metrics, monitor, incident, dashboard, slo, git
rag-cli ask "what alerted overnight?" --kind monitor --kind incident

# Resolve "yesterday", "last 2 hours", ... in a specific timezone (default: your system zone)
rag-cli ask "errors in payments since yesterday" --tz Europe/Stockholm

# See how the server interprets a question, without retrieving or answering
rag-cli plan "why did checkout fail in staging last night?"
```

`ask` prints the API response as JSON: the `answer`, whether it is backed by indexed
documents (`"evidence": "found"` or `"none"`), the validated `plan`, and the `scope`
actually applied to retrieval:

```json
{
  "answer": "auth-api returned 5xx between 14:05 and 14:40 UTC ... [DOC #1] ...",
  "evidence": "found",
  "plan": { "intent": "rootCauseWindow", "service": "auth-api", "environment": "prod", "...": "..." },
  "scope": {
    "service": "auth-api",
    "environment": "prod",
    "fromUtc": "2026-09-22T22:00:00Z",
    "toUtc": "2026-09-23T22:00:00Z",
    "kinds": []
  }
}
```

If the planner is unsure about something, its clarifying questions are printed to
stderr under `Need more info:`. If a step fails (planning, embedding, search or
generation), no answer is printed. The CLI prints the typed error, for example
`error [upstream_unavailable] at stage 'retrieval' (HTTP 503): ...`, and exits with
status 1. See [API errors and evidence](docs/ARCHITECTURE.md#api-errors-and-evidence).

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
  "plan": null
}
```

Only `question` is required. The response is
`{"answer": "...", "evidence": "found" | "none", "plan": {...}, "scope": {"service", "environment", "fromUtc", "toUtc", "kinds"}}`,
where `plan` is the validated plan and `scope` is what was actually applied to retrieval.

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
