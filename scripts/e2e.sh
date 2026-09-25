#!/usr/bin/env bash
# End-to-end run of the built binaries: rag-indexer indexes the incident question set's
# corpus from a fake Datadog into a real Qdrant with real embeddings, then tails-e2e asks
# every question through rag-cli and rag-api and checks the answers.
#
#   Real:   Qdrant, embeddings (text-embeddings-inference, OpenAI-compatible
#           /v1/embeddings), the rag-indexer, rag-api and rag-cli binaries.
#   Faked:  Datadog API and the chat model (planner reply and answer per question),
#           served by `tails-fakes serve`.
#
# Usage (from anywhere in the checkout):
#   scripts/e2e.sh                         # Qdrant and TEI already running
#   E2E_DOCKER=qdrant,tei scripts/e2e.sh   # start both with docker, stop them after
#   E2E_EMBEDDINGS=fake scripts/e2e.sh     # no model: tails-fakes' hashed bag-of-words
#                                          # embeddings (checks the plumbing only)
#   E2E_EMBEDDINGS=openai OPENAI_EMBEDDING_API_KEY=... scripts/e2e.sh
#                                          # OpenAI's embeddings (text-embedding-3-small);
#                                          # chat stays on the fakes
#
# Environment (defaults in brackets):
#   E2E_EMBEDDINGS   tei | openai | fake [tei]. openai: OPENAI_EMBEDDING_BASE_URL
#                    [https://api.openai.com], OPENAI_EMBEDDING_MODEL
#                    [text-embedding-3-small], OPENAI_EMBEDDING_API_KEY [OPENAI_API_KEY],
#                    OPENAI_EMBEDDING_QUERY_PREFIX and _DOCUMENT_PREFIX [none]
#   E2E_DOCKER       containers to start: qdrant, tei, or both comma-separated [none]
#   QDRANT_ENDPOINT  [http://127.0.0.1:6333]
#   TEI_URL          [http://127.0.0.1:8080]
#   TEI_IMAGE        [ghcr.io/huggingface/text-embeddings-inference:cpu-1.9.4]
#   TEI_MODEL        [BAAI/bge-small-en-v1.5]
#   TEI_QUERY_PREFIX the model's query instruction (OPENAI_EMBEDDING_QUERY_PREFIX)
#                    [bge's "Represent this sentence for searching relevant passages: "]
#   TEI_DOCUMENT_PREFIX  the model's document prefix (OPENAI_EMBEDDING_DOCUMENT_PREFIX),
#                    e.g. e5's "passage: " [none, as bge wants]
#   TEI_MODEL_REVISION  the model's Hugging Face commit; TEI must report it [5c38ec7c405ec4b44b94cc5a9bb96e735b38267a]
#   TEI_DATA         model cache mounted into the TEI container [~/.cache/tails-e2e/tei]
#   FAKES_ADDR       [127.0.0.1:8900]
#   API_ADDR         where rag-api listens (RAG_API_ADDR) [127.0.0.1:5191]
#   E2E_SKIP_BUILD   1: use the existing release binaries
#   E2E_KEEP         1: keep the collection and the work directory
#   E2E_OUT          directory to copy every run's tails-e2e --summary JSON into [none]
#   E2E_COMPARE      further query-side configurations to ask the questions with after
#                    the main run, on the same index, as fusion:stopwords separated by
#                    spaces (e.g. "dbsf:off rrf:on dbsf:on"); a table compares them. An
#                    rrf entry can add :k=<RAG_RRF_K> and :w=<dense>,<keyword>
#                    (RAG_RRF_WEIGHTS), e.g. "rrf:off:k=60 rrf:off:w=1,2".
#                    Informational: only the main run (RAG_FUSION, RAG_RRF_K,
#                    RAG_RRF_WEIGHTS and RAG_KEYWORD_STOPWORDS as set, else the
#                    defaults) decides the exit status [none]
# API_ADDR and FAKES_ADDR must be free.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET="${CARGO_TARGET_DIR:-$ROOT/target}/release"
E2E_EMBEDDINGS="${E2E_EMBEDDINGS:-tei}"
E2E_DOCKER="${E2E_DOCKER:-}"
QDRANT_URL="${QDRANT_ENDPOINT:-http://127.0.0.1:6333}"
TEI_URL="${TEI_URL:-http://127.0.0.1:8080}"
TEI_IMAGE="${TEI_IMAGE:-ghcr.io/huggingface/text-embeddings-inference:cpu-1.9.4}"
TEI_MODEL="${TEI_MODEL:-BAAI/bge-small-en-v1.5}"
TEI_QUERY_PREFIX="${TEI_QUERY_PREFIX-Represent this sentence for searching relevant passages: }"
TEI_DOCUMENT_PREFIX="${TEI_DOCUMENT_PREFIX:-}"
TEI_MODEL_REVISION="${TEI_MODEL_REVISION-5c38ec7c405ec4b44b94cc5a9bb96e735b38267a}"
TEI_DATA="${TEI_DATA:-$HOME/.cache/tails-e2e/tei}"
FAKES_ADDR="${FAKES_ADDR:-127.0.0.1:8900}"
FAKES_URL="http://$FAKES_ADDR"
API_ADDR="${API_ADDR:-127.0.0.1:5191}"
E2E_COMPARE="${E2E_COMPARE:-}"
COLLECTION="tails_e2e_$(date +%s)_$$"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/tails-e2e.XXXXXX")"
PIDS=()
CONTAINERS=()
STATUS=1

log() { printf '\n==> %s\n' "$*"; }

cleanup() {
  set +e
  for pid in "${PIDS[@]}"; do
    kill "$pid" 2>/dev/null
    wait "$pid" 2>/dev/null
  done
  if [[ "$STATUS" -ne 0 ]]; then
    for f in "$WORK"/*.log; do
      [[ -f "$f" ]] || continue
      printf '\n--- last lines of %s ---\n' "$(basename "$f")"
      tail -n 40 "$f"
    done
  fi
  if [[ "${E2E_KEEP:-0}" == 1 ]]; then
    echo "kept collection $COLLECTION and $WORK"
  else
    curl -s -X DELETE "$QDRANT_URL/collections/$COLLECTION" >/dev/null
    rm -rf "$WORK"
  fi
  for c in "${CONTAINERS[@]}"; do
    docker rm -f "$c" >/dev/null 2>&1
  done
}
trap cleanup EXIT

# Polls `url` until it answers 2xx, for at most `seconds`.
wait_for() {
  local what="$1" url="$2" seconds="$3" start=$SECONDS
  until curl -sf -o /dev/null "$url"; do
    if (( SECONDS - start >= seconds )); then
      echo "$what did not become ready at $url within ${seconds}s" >&2
      return 1
    fi
    sleep 1
  done
  echo "$what ready after $(( SECONDS - start ))s"
}

port_of() { local u="${1#*://}"; u="${u%%/*}"; echo "${u##*:}"; }

if [[ "${E2E_SKIP_BUILD:-0}" != 1 ]]; then
  log "Building release binaries"
  cargo build --release --locked --workspace --manifest-path "$ROOT/Cargo.toml"
fi
for bin in rag-indexer rag-api rag-cli tails-fakes tails-e2e; do
  [[ -x "$TARGET/$bin" ]] || { echo "missing $TARGET/$bin" >&2; exit 1; }
done

if [[ ",$E2E_DOCKER," == *",qdrant,"* ]]; then
  log "Starting Qdrant (docker)"
  docker run -d --name "tails-e2e-qdrant-$$" -p "$(port_of "$QDRANT_URL"):6333" \
    qdrant/qdrant:v1.19.1 >/dev/null
  CONTAINERS+=("tails-e2e-qdrant-$$")
fi
if [[ "$E2E_EMBEDDINGS" == tei && ",$E2E_DOCKER," == *",tei,"* ]]; then
  log "Starting text-embeddings-inference (docker, $TEI_MODEL)"
  mkdir -p "$TEI_DATA"
  docker run -d --name "tails-e2e-tei-$$" -p "$(port_of "$TEI_URL"):80" \
    -v "$TEI_DATA:/data" -e MODEL_ID="$TEI_MODEL" -e REVISION="$TEI_MODEL_REVISION" \
    "$TEI_IMAGE" >/dev/null
  CONTAINERS+=("tails-e2e-tei-$$")
fi

wait_for Qdrant "$QDRANT_URL/readyz" 60
case "$E2E_EMBEDDINGS" in
  tei)
    # The first start downloads the model into TEI_DATA.
    wait_for "text-embeddings-inference" "$TEI_URL/health" 600
    # The thresholds were measured with one model revision; refuse to run on another.
    TEI_INFO="$(curl -fsS "$TEI_URL/info")"
    TEI_SHA="$(python3 -c 'import json,sys; print(json.load(sys.stdin).get("model_sha") or "")' <<<"$TEI_INFO")"
    echo "text-embeddings-inference serves $TEI_MODEL at revision ${TEI_SHA:-unknown}"
    if [[ -n "$TEI_MODEL_REVISION" && "$TEI_SHA" != "$TEI_MODEL_REVISION" ]]; then
      echo "expected revision $TEI_MODEL_REVISION (TEI_MODEL_REVISION); set it to the new commit and re-measure the thresholds" >&2
      exit 1
    fi
    EMBEDDING_URL="$TEI_URL"
    EMBEDDING_MODEL="$TEI_MODEL"
    QUERY_PREFIX="$TEI_QUERY_PREFIX"
    DOCUMENT_PREFIX="$TEI_DOCUMENT_PREFIX"
    FAKES_FLAGS=()
    ;;
  openai)
    # A hosted OpenAI-compatible /v1/embeddings; the chat model stays faked, so the key
    # only goes to the embeddings endpoint.
    EMBEDDING_URL="${OPENAI_EMBEDDING_BASE_URL:-https://api.openai.com}"
    EMBEDDING_MODEL="${OPENAI_EMBEDDING_MODEL:-text-embedding-3-small}"
    export OPENAI_EMBEDDING_API_KEY="${OPENAI_EMBEDDING_API_KEY:-${OPENAI_API_KEY:-}}"
    if [[ -z "$OPENAI_EMBEDDING_API_KEY" ]]; then
      echo "E2E_EMBEDDINGS=openai needs OPENAI_EMBEDDING_API_KEY (or OPENAI_API_KEY)" >&2
      exit 1
    fi
    QUERY_PREFIX="${OPENAI_EMBEDDING_QUERY_PREFIX:-}"
    DOCUMENT_PREFIX="${OPENAI_EMBEDDING_DOCUMENT_PREFIX:-}"
    FAKES_FLAGS=()
    ;;
  fake)
    EMBEDDING_URL="$FAKES_URL"
    EMBEDDING_MODEL="fake-bow-1024"
    QUERY_PREFIX=""
    DOCUMENT_PREFIX=""
    FAKES_FLAGS=(--embeddings)
    ;;
  *) echo "E2E_EMBEDDINGS must be tei, openai or fake" >&2; exit 1 ;;
esac

log "Starting tails-fakes on $FAKES_ADDR"
"$TARGET/tails-fakes" serve --addr "$FAKES_ADDR" --site datadoghq.eu "${FAKES_FLAGS[@]}" \
  >"$WORK/tails-fakes.log" 2>&1 &
PIDS+=($!)
wait_for tails-fakes "$FAKES_URL/_fakes/health" 60

# Configuration shared by rag-indexer, rag-api (started by tails-e2e) and tails-e2e.
export OPENAI_API_KEY=sk-e2e-fake
export OPENAI_BASE_URL="$FAKES_URL"
export OPENAI_CHAT_MODEL=fake-chat
export OPENAI_EMBEDDING_BASE_URL="$EMBEDDING_URL"
export OPENAI_EMBEDDING_MODEL="$EMBEDDING_MODEL"
export OPENAI_EMBEDDING_QUERY_PREFIX="$QUERY_PREFIX"
export OPENAI_EMBEDDING_DOCUMENT_PREFIX="$DOCUMENT_PREFIX"
export QDRANT_ENDPOINT="$QDRANT_URL"
export QDRANT_COLLECTION="$COLLECTION"
export DD_API_KEY=e2e DD_APP_KEY=e2e DD_SITE=datadoghq.eu
export DD_API_BASE_URL="$FAKES_URL"
export RUST_LOG="${RUST_LOG:-info}" NO_COLOR=1
# One first run reaching back over the whole corpus and the recorded fixtures.
export INDEXER_WATERMARK="$WORK/watermark.json"
export INDEXER_LOOKBACK_MINUTES=$(( 20 * 365 * 24 * 60 ))
export INDEXER_SERVICE_CATALOG_ENABLED=true
export INDEXER_CHANGE_EVENTS_ENABLED=true
# Within TEI's default --max-client-batch-size (32); one batch at a time, so TEI never
# batches requests together and every run embeds exactly the same inputs.
export INDEXER_EMBED_BATCH_SIZE=32
export INDEXER_EMBED_CONCURRENCY=1

log "Indexing into collection $COLLECTION (embeddings: $EMBEDDING_MODEL at $EMBEDDING_URL)"
start=$SECONDS
if ! "$TARGET/rag-indexer" >"$WORK/rag-indexer.log" 2>&1; then
  echo "rag-indexer failed" >&2
  exit 1
fi
echo "rag-indexer exited 0 after $(( SECONDS - start ))s"
grep -E "Indexed |created Qdrant collection" "$WORK/rag-indexer.log" | sed -E 's/^.*(INFO|WARN) [^ ]+: //' || true
[[ -s "$WORK/watermark.json" ]] || { echo "rag-indexer wrote no watermark" >&2; exit 1; }

# Asks every question with the query-side configuration `$1` (a label), writing its
# summary for the comparison table.
ask_all() {
  "$TARGET/tails-e2e" --api-bin "$TARGET/rag-api" --cli-bin "$TARGET/rag-cli" \
    --api-base "http://$API_ADDR" --fakes "$FAKES_URL" --api-log "$WORK/rag-api-$1.log" \
    --summary "$WORK/summary-$1.json"
}

log "Asking the incident questions through rag-cli (RAG_FUSION=${RAG_FUSION:-default}, RAG_RRF_K=${RAG_RRF_K:-default}, RAG_RRF_WEIGHTS=${RAG_RRF_WEIGHTS:-default}, RAG_KEYWORD_STOPWORDS=${RAG_KEYWORD_STOPWORDS:-default})"
GATE=0
ask_all main || GATE=$?
SUMMARIES=("$WORK/summary-main.json")

# Fusion and keyword stopwords only change how rag-api queries, so the other
# configurations reuse the index. Informational: they never fail the run.
for cfg in $E2E_COMPARE; do
  IFS=: read -r fusion stopwords extra <<<"$cfg"
  k="" weights=""
  IFS=: read -ra params <<<"$extra"
  for p in "${params[@]}"; do
    case "$p" in
      k=*) k="${p#k=}" ;;
      w=*) weights="${p#w=}" ;;
      *) echo "E2E_COMPARE: unknown parameter $p in $cfg (expected k=<n> or w=<dense>,<keyword>)" >&2; exit 1 ;;
    esac
  done
  name="$(tr -c 'A-Za-z0-9\n' '-' <<<"$cfg")"
  log "Comparison run (informational): RAG_FUSION=$fusion RAG_KEYWORD_STOPWORDS=$stopwords${k:+ RAG_RRF_K=$k}${weights:+ RAG_RRF_WEIGHTS=$weights}"
  if ! RAG_FUSION="$fusion" RAG_KEYWORD_STOPWORDS="$stopwords" RAG_RRF_K="$k" RAG_RRF_WEIGHTS="$weights" \
    ask_all "$name"; then
    echo "($cfg is below a threshold or failed a hard check; informational only)"
  fi
  [[ -f "$WORK/summary-$name.json" ]] && SUMMARIES+=("$WORK/summary-$name.json")
done
if [[ -n "${E2E_OUT:-}" ]]; then
  mkdir -p "$E2E_OUT"
  for f in "${SUMMARIES[@]}"; do
    [[ -f "$f" ]] && cp "$f" "$E2E_OUT/"
  done
fi
if [[ -n "$E2E_COMPARE" && -f "${SUMMARIES[0]}" ]]; then
  log "Configurations compared (the first one decides the run)"
  python3 "$ROOT/scripts/e2e_compare.py" "${SUMMARIES[@]}"
fi
[[ "$GATE" -eq 0 ]] || exit "$GATE"
STATUS=0
