# Hybrid fusion against real Qdrant and six embedding models

Follow-up to pull request #56, which made fusion (`RAG_FUSION=rrf|dbsf`) and query
stopwords (`RAG_KEYWORD_STOPWORDS`) configurable and kept RRF without stopwords. This
experiment runs the comparison with real Qdrant containers and six real embedding
models instead of one, adds RRF's `k` and per-list weights to the grid, and applies the
decision rule below.

## Decision

**The defaults stay: `RAG_FUSION=rrf` with `k` = 2 and equal weights,
`RAG_KEYWORD_STOPWORDS=off`.** No configuration meets the rule:

- **Rule (a), fake embeddings, every question must keep passing** (thresholds 1.00):
  DBSF lets a distractor into q12 (distractor exclusion 0.985), and every `k` other than
  2 regresses at least one question (k = 1: q09; k = 5: q41; k = 10: q40, q41; k = 60:
  q40, q41, q50). Weighted RRF at `k` = 2 passes with some weights and not others
  (1:1.5 and 3:1 with stopwords regress q17).
- **Rule (b), real embeddings, at least as good for every model and better for most:**
  DBSF is better for 1 of 6 models (bge-small, where it fixes q40) and equal for the
  other 5; it is never worse on pass/fail, but "better for most" fails. No RRF `k` or
  weight is better for any model; `k` = 10 and 60 are worse for all six models, `k` = 5
  for three and `k` = 1 for two, and the weights that were never worse (1:1.5, 1:2, 3:1)
  change no question for any model.
- Stopwords change no question's result for any model or fusion (they only move
  margins, mostly q42's up).

DBSF's advantage in pull request #56 is specific to bge-small. What does hold across
all six models is that the remaining misses are not fusion problems: q17 is a scoping
problem (missed by every model and every fusion), and every other miss (q40 with
bge-small, q41 with bge-large, e5-small and e5-base) is an **exact fusion tie** between
two incidents that the reranker's recency weight resolves in favour of the newer, wrong
one. See [Next steps](#next-steps).

## Setup

| Component | Version |
|---|---|
| Qdrant | `qdrant/qdrant:v1.19.1` (commit `6ab21cac18eb`, image `sha256:12364fe851b9f17356fc88189fc06d1b521262e04659ec7345975b00c9246a10`), the image CI uses. `qdrant/qdrant:latest` is the same image. Also tested: `v1.18.3` (commit `db8fa43fcb6a`) and `dev` (1.19.2-dev, commit `ed1ce7dc37e3`). |
| Embeddings server | `ghcr.io/huggingface/text-embeddings-inference:cpu-1.9.4` (`sha256:2538ea1c9640d3763b15af668039d24172d063b42337b0c27796fc2be180c78d`), on CPU |
| Tails | this branch: `scripts/e2e.sh`, release binaries, fake Datadog and fake chat from `tails-fakes serve` |
| Question set | `crates/rag-indexer/tests/incident_questions/`, v4, 36 questions |
| Machine | 4 vCPU (Intel Xeon @ 2.10 GHz), 15 GiB RAM, no GPU |

Models, each pinned to a Hugging Face commit (`TEI_MODEL_REVISION`, which the script
checks against TEI's `/info`), with the prefixes from its model card at that revision:

| Model | Revision | Dim | Query prefix (`TEI_QUERY_PREFIX`) | Document prefix (`TEI_DOCUMENT_PREFIX`) | Model card says |
|---|---|---|---|---|---|
| `BAAI/bge-small-en-v1.5` | `5c38ec7c405ec4b44b94cc5a9bb96e735b38267a` | 384 | `Represent this sentence for searching relevant passages: ` | none | query instruction for retrieval in the model table; "no instruction needs to be added to passages" |
| `BAAI/bge-base-en-v1.5` | `a5beb1e3e68b9ab74eb54cfd186867f64f240e1a` | 768 | same | none | same card |
| `BAAI/bge-large-en-v1.5` | `d4aa6901d3a41ba39fb536a557fa166f842b0e09` | 1024 | same | none | same card |
| `intfloat/e5-small-v2` | `ffb93f3bd4047442299a41ebb6fa998a38507c52` | 384 | `query: ` | `passage: ` | "Each input text should start with "query: " or "passage: "" |
| `intfloat/e5-base-v2` | `f52bf8ec8c7124536f0efb74aca902b2995e5bcd` | 768 | `query: ` | `passage: ` | same |
| `nomic-ai/nomic-embed-text-v1.5` | `e9b6763023c676ca8431644204f50c2b100d9aab` | 768 | `search_query: ` | `search_document: ` | "embed your documents as `search_document: <text here>` and embed your user queries as `search_query: <text here>`" |

TEI applies each model's own pooling and normalization (CLS for bge, mean for e5 and
nomic); nomic is served at its full 768 dimensions (no Matryoshka truncation). TEI runs
bge and e5 on its ONNX Runtime backend; for nomic that backend fails to parse the
model's `config.json` ("duplicate field `hidden_size`") and TEI falls back to its Candle
`NomicBert` implementation, also on CPU.
`text-embedding-3-small` was **not** run: no `OPENAI_API_KEY` was available in this
environment. `scripts/e2e.sh` gained `E2E_EMBEDDINGS=openai` for it (embeddings from
`OPENAI_EMBEDDING_BASE_URL`, default `https://api.openai.com`, with
`OPENAI_EMBEDDING_API_KEY` or `OPENAI_API_KEY`; chat stays on the fakes); its plumbing
was checked with TEI standing in for OpenAI's endpoint (see [Reproduction](#reproduction)).

Grid per model: one index, then the 36 questions asked under 24 query-side
configurations (fusion × stopwords): RRF with `k` ∈ {1, 2, 5, 10, 60}, RRF at `k` = 2
with dense:keyword weights ∈ {1:1.5, 1:2, 1:3, 1.5:1, 2:1, 3:1}, and DBSF; each with
stopwords off and on. The whole grid was run twice for every model. Both runs gave
identical summaries (every aggregate, pass/fail and margin) except one margin in one of
the 144 model × configuration cells (e5-base, `rrf(k=2,w=3:1) stop`, q50: 2.199 and
2.193, passing both times), most likely from equal fused scores, which Qdrant orders
arbitrarily. The tables are from the second run.

## 1. Reproducing the CI numbers

`E2E_DOCKER=qdrant,tei E2E_COMPARE="dbsf:off rrf:on dbsf:on" scripts/e2e.sh` with the
committed images and revision reproduced CI exactly:

| | `rrf nostop` (default) | `dbsf nostop` | `rrf stop` | `dbsf stop` |
|---|---|---|---|---|
| precision@R | 0.944 (misses q17, q40) | 0.972 (misses q17) | 0.944 | 0.972 |
| every other aggregate | 1.000 | 1.000 | 1.000 | 1.000 |
| q12 / q30 / q40 margin | 3.207 / 1.714 / 0.854 | 1.532 / 1.242 / 1.040 | 3.207 / 1.714 / 0.854 | 1.532 / 1.242 / 1.040 |

One environment-specific step was needed: this sandbox intercepts HTTPS with its own
certificate authority, so the TEI container could not download the model ("self-signed
certificate in certificate chain") until the proxy's CA bundle was mounted at
`/etc/ssl/certs/ca-certificates.crt`. That only affects the download, not the model or
the embeddings (TEI's `/info` reported the pinned revision). The later runs started TEI
that way and pointed the script at it (`TEI_URL`), with Qdrant 1.19.1 already running
(`QDRANT_ENDPOINT`).

## 2. RRF and DBSF on real Qdrant, across versions

`qdrant_roundtrip`, `query_api_matches_real_qdrant` and `dbsf_matches_real_qdrant` (with
the new `k` and weight cases below) pass against all three servers:

| Server | `qdrant_roundtrip` | `query_api_matches_real_qdrant` | `dbsf_matches_real_qdrant` |
|---|---|---|---|
| `qdrant/qdrant:v1.19.1` (= `latest`) | pass | pass | pass |
| `v1.18.3` | pass | pass | pass |
| `dev` (1.19.2-dev, `ed1ce7dc37e3`) | pass | pass | pass |

The latest release is v1.19.1, the version CI already pins: `qdrant/qdrant:latest`
resolves to the same image digest. So there is no newer release to differ from; the
previous release line and the unreleased `dev` build were tested instead. **No change in
RRF or DBSF behaviour or request syntax between them:**

- The fusion code is the same: `lib/segment/src/common/score_fusion.rs` (DBSF) is
  byte-identical in v1.18.3, v1.19.1 and `dev`;
  `lib/segment/src/common/reciprocal_rank_fusion.rs` differs only in one test assertion
  (`assert!(a == b)` → `assert_eq!(a, b)`).
- The same live requests give the same scores on all three (two points per list,
  `k` = 2 unless stated):

  | `query` | Result on 1.18.3, 1.19.1 and 1.19.2-dev |
  |---|---|
  | `{"rrf": {"k": 2}}` | 3: 0.7, 4: 0.583333, 1: 0.5, 2: 0.333333 |
  | `{"rrf": {"k": 10, "weights": [3, 1]}}` | 3: 0.196774, 4: 0.190909, 1: 0.107143, 2: 0.103448 |
  | `{"fusion": "dbsf"}` | 4: 0.893436, 3: 0.87863, 1: 0.61505, 2: 0.612884 |
  | `{"fusion": "dbsf", "weights": [1, 5]}` | **the same as without weights**: the unknown key is silently ignored |
  | `{"rrf": {"weights": [1]}}` (two prefetches) | 400 "RRF weights length (1) does not match number of prefetches (2)" |
  | `{"rrf": {"weights": [1, -1]}}` | accepted; a weight of 0 or below makes that list add 0 |

DBSF as the fake implements it is confirmed again: per list `(s − (μ − 3σ)) / 6σ` with
the sample variance from Welford's method in f32, no clipping, 0.5 for a list of one
point or of equal scores, summed. It takes no parameters, and a `weights` key next to
`"fusion": "dbsf"` is ignored rather than refused, which is why Tails refuses
`RAG_RRF_K`/`RAG_RRF_WEIGHTS` with `RAG_FUSION=dbsf` instead of sending them.

## 3. Other fusion options: RRF `k` and weights

**What Qdrant supports** (docs, source and live requests agree):

- `{"rrf": {"k": n}}`: "Setting RRF Constant k — Available as of v1.16.0"; the REST
  schema (`lib/api/src/rest/schema.rs`, `struct Rrf`) validates `k` ≥ 1, and `k: 0` is
  refused with 400. Tails already sent `k` = 2 explicitly.
- `{"rrf": {"k": n, "weights": [w₁, …]}}`: "Weighted RRF — Available as of v1.17.0";
  one weight per prefetch, in prefetch order, and the count must match.
- DBSF: `{"fusion": "dbsf"}` only ("Available as of v1.11.0"); no parameters.
- (Not tested here: the `formula` query, which could combine raw prefetch scores
  arbitrarily, and relevance feedback. They are not fusion presets.)

**Qdrant's weighted formula is not "multiply the list's RRF scores by w".** From
`reciprocal_rank_fusion.rs` (v1.19.1), a point at 0-based rank `r` of a list of weight
`w` scores `1 / ((r + 1) / w + k − 1)`: the weight divides the 1-based rank. At weight
3 the third point of a list scores what the first scores at weight 1 (Qdrant's docs: "for
each 3 results of first prefetch, have one result of second"). Two consequences for
tuning:

- The larger `k`, the less a weight does at the top (at `k` = 60 the first point scores
  1/59.33 at weight 3 against 1/60 at weight 1).
- A higher weight raises every point of that list but **flattens** the gap between its
  first and second point: at `k` = 2 that gap is `w / ((1 + w)(2 + w))`, 0.167 at w = 1,
  0.171 at its maximum (w = √2), 0.167 at w = 2 and 0.150 at w = 3. So a heavier keyword
  list does not make a decisive keyword match (q40: keyword 24.6 against 1.8) count
  more against a dense near-tie; it barely moves a tie between "first in dense, second
  in keyword" and the reverse, which is exactly what q40 and q41 are.

**Added in this branch:**

- `RAG_RRF_K` (integer ≥ 1, default 2) and `RAG_RRF_WEIGHTS=dense,keyword` (two numbers
  above 0, default unset: no `weights` sent). `rag_core::qdrant::Fusion::Rrf` now carries
  `Rrf { k, weights }`; `hybrid_query_body` returns the kind of each prefetch list, so a
  weight is sent per list (dense and keyword for the question, and again for the
  planner's rewrite) and the normalization divides by the best possible weighted score,
  `Σ 1 / (1 / w + k − 1)` over the lists. With `dbsf` either variable is an error.
- The fake Qdrant implements `k` and weights with Qdrant's f32 formula, refuses what
  Tails never sends (weights ≤ 0, empty, or not one per prefetch), and the real-server
  checks cover them: `query_api_matches_real_qdrant` gained three weighted/`k` cases,
  `qdrant_roundtrip` checks `hybrid_search`'s normalized scores for `k` = 10, 1:2 and
  2:1. In addition, the fake-embedding question set run against the real Qdrant
  (`incident_questions_real_qdrant`) printed exactly the same report (every row and
  margin) as against the fake for `rrf(k=10)`, `rrf(k=60)`, weights 1:2, 2:1, 1:3 and
  `dbsf`.
- `scripts/e2e.sh`: `E2E_COMPARE` entries take `:k=<n>` and `:w=<dense>,<keyword>`
  (e.g. `rrf:off:k=60 rrf:on:w=1,2`); `TEI_DOCUMENT_PREFIX` for models with a document
  prefix (e5, nomic); `E2E_OUT` keeps every run's summary JSON. `tails-e2e --summary`
  records the fusion's label (`rrf(k=10)`, `rrf(k=2,w=1:2)`), which the comparison table
  shows.

## 4. Results

### 4.1 Fake embeddings (rule (a))

The in-process harness (`incident_questions_in_memory`, fake Qdrant, hashed
bag-of-words embeddings, thresholds from `questions.json`), per configuration.
`gate`: every aggregate at or above its threshold. Margins as in docs/DEVELOPMENT.md
(the lowest must-retrieve score over the highest unrelated one; above 1 ranks first).
Besides the real-embedding grid, the fake set was also run with `k` = 10 and weights
1:2 and 2:1.

| configuration | rec@k | prec@R | excl | scope | cit.val | cit.acc | obs | neg | evid | gate | questions regressed against `rrf nostop` | q12 | q17 | q30 | q40 | q42 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| `rrf nostop` | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | – | 2.707 | 1.126 | 1.191 | 1.463 | 1.802 |
| `rrf stop` | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | – | 2.707 | 1.000 | 1.191 | 1.463 | 3.308 |
| `dbsf nostop` | 1.000 | 1.000 | 0.985 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | FAIL | q12 (distractors ["dashboard_dash-checkout"]) | 1.810 | 1.389 | 1.081 | 1.468 | 1.793 |
| `dbsf stop` | 1.000 | 1.000 | 0.985 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | FAIL | q12 (distractors ["dashboard_dash-checkout"]) | 1.810 | 1.344 | 1.081 | 1.468 | 3.101 |
| `rrf(k=1) nostop` | 1.000 | 0.972 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q09 (top ["dashboard_dash-checkout"]) | 3.621 | 1.249 | 1.201 | 2.047 | 2.521 |
| `rrf(k=1) stop` | 1.000 | 0.972 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q09 (top ["dashboard_dash-checkout"]) | 3.621 | 1.000 | 1.201 | 2.047 | 4.399 |
| `rrf(k=5) nostop` | 1.000 | 0.972 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q41 (top ["incident_inc-payments-card"]) | 2.167 | 1.028 | 1.127 | 1.103 | 1.358 |
| `rrf(k=5) stop` | 1.000 | 0.972 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q41 (top ["incident_inc-payments-card"]) | 2.167 | 1.000 | 1.127 | 1.103 | 2.651 |
| `rrf(k=10) nostop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q40 (top ["incident_inc-checkout-latency"]); q41 (top ["incident_inc-payments-card"]) | 1.989 | 1.008 | 1.078 | 0.980 | 1.206 |
| `rrf(k=10) stop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q40 (top ["incident_inc-checkout-latency"]); q41 (top ["incident_inc-payments-card"]) | 1.989 | 1.000 | 1.078 | 0.980 | 2.425 |
| `rrf(k=60) nostop` | 1.000 | 0.917 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q40 (top ["incident_inc-checkout-latency"]); q41 (top ["incident_inc-payments-card"]); q50 (top ["incident_inc-checkout-latency"]) | 1.837 | 1.000 | 1.017 | 0.875 | 1.077 |
| `rrf(k=60) stop` | 1.000 | 0.917 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q40 (top ["incident_inc-checkout-latency"]); q41 (top ["incident_inc-payments-card"]); q50 (top ["incident_inc-checkout-latency"]) | 1.837 | 1.000 | 1.017 | 0.875 | 2.171 |
| `rrf(k=2,w=1:1.5) nostop` | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | – | 2.986 | 1.093 | 1.208 | 1.384 | 1.702 |
| `rrf(k=2,w=1:1.5) stop` | 1.000 | 0.972 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17 (top ["logpattern_12a088b81c8ca50729a64fc94640b0e1"]) | 2.986 | 0.994 | 1.208 | 1.384 | 3.643 |
| `rrf(k=2,w=1:2) nostop` | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | – | 3.160 | 1.080 | 1.200 | 1.326 | 1.634 |
| `rrf(k=2,w=1:2) stop` | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | – | 3.160 | 1.000 | 1.200 | 1.326 | 3.864 |
| `rrf(k=2,w=1:3) nostop` | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | – | 3.395 | 1.071 | 1.167 | 1.256 | 1.546 |
| `rrf(k=2,w=1:3) stop` | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | – | 3.395 | 1.015 | 1.167 | 1.256 | 4.135 |
| `rrf(k=2,w=1.5:1) nostop` | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | – | 2.317 | 1.115 | 1.160 | 1.408 | 1.734 |
| `rrf(k=2,w=1.5:1) stop` | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | – | 2.317 | 1.006 | 1.160 | 1.408 | 2.832 |
| `rrf(k=2,w=2:1) nostop` | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | – | 2.109 | 1.100 | 1.155 | 1.357 | 1.671 |
| `rrf(k=2,w=2:1) stop` | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | – | 2.109 | 1.000 | 1.155 | 1.357 | 2.576 |
| `rrf(k=2,w=3:1) nostop` | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | – | 1.881 | 1.071 | 1.159 | 1.279 | 1.575 |
| `rrf(k=2,w=3:1) stop` | 1.000 | 0.972 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17 (top ["logpattern_12a088b81c8ca50729a64fc94640b0e1"]) | 1.881 | 0.985 | 1.159 | 1.279 | 2.297 |
| `rrf(k=10,w=1:2) nostop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q40 (top ["incident_inc-checkout-latency"]); q41 (top ["incident_inc-payments-card"]) | 2.040 | 1.023 | 1.027 | 0.955 | 1.176 |
| `rrf(k=10,w=1:2) stop` | 1.000 | 0.917 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q40 (top ["incident_inc-checkout-latency"]); q41 (top ["incident_inc-payments-card"]); q50 (top ["incident_inc-checkout-latency"]) | 2.040 | 1.019 | 1.027 | 0.955 | 2.491 |
| `rrf(k=10,w=2:1) nostop` | 1.000 | 0.889 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | FAIL | q17 (top ["logpattern_12a088b81c8ca50729a64fc94640b0e1"]); q40 (top ["incident_inc-checkout-latency"]); q41 (top ["incident_inc-payments-card"]); q50 (top ["incident_inc-checkout-latency"]) | 1.853 | 0.988 | 1.075 | 0.941 | 1.158 |
| `rrf(k=10,w=2:1) stop` | 1.000 | 0.889 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | FAIL | q17 (top ["logpattern_12a088b81c8ca50729a64fc94640b0e1"]); q40 (top ["incident_inc-checkout-latency"]); q41 (top ["incident_inc-payments-card"]); q50 (top ["incident_inc-checkout-latency"]) | 1.853 | 0.981 | 1.075 | 0.941 | 2.266 |

### 4.2 Real embeddings, all models (rule (b))

Precision@R per model and configuration; `+f/−r`: questions fixed / regressed against
the same model's `rrf nostop`. A question passes when recall and precision@R are 1 and
every distractor is excluded. `≥ default on every model`: no aggregate lower and no
question regressed for any model. `better on`: models with a higher aggregate or a
fixed question and none regressed. Every other aggregate (recall@k, distractor
exclusion, scope accuracy, citation validity and accuracy, observation recall, negative
controls, evidence accuracy) is 1.000 in every cell; the per-model tables list them.

| configuration | bge-small | bge-base | bge-large | e5-small | e5-base | nomic | ≥ default on every model | better on |
|---|---|---|---|---|---|---|---|---|
| `rrf nostop` | 0.944 | 0.972 | 0.944 | 0.944 | 0.944 | 0.972 | yes | 0/6 |
| `rrf stop` | 0.944 | 0.972 | 0.944 | 0.944 | 0.944 | 0.972 | yes | 0/6 |
| `dbsf nostop` | 0.972 +1/−0 | 0.972 | 0.944 | 0.944 | 0.944 | 0.972 | yes | 1/6 |
| `dbsf stop` | 0.972 +1/−0 | 0.972 | 0.944 | 0.944 | 0.944 | 0.972 | yes | 1/6 |
| `rrf(k=1) nostop` | 0.917 +0/−1 | 0.972 | 0.944 | 0.944 | 0.917 +0/−1 | 0.972 | no | 0/6 |
| `rrf(k=1) stop` | 0.917 +0/−1 | 0.972 | 0.944 | 0.944 | 0.917 +0/−1 | 0.972 | no | 0/6 |
| `rrf(k=5) nostop` | 0.917 +0/−1 | 0.944 +0/−1 | 0.944 | 0.944 | 0.944 | 0.944 +0/−1 | no | 0/6 |
| `rrf(k=5) stop` | 0.917 +0/−1 | 0.944 +0/−1 | 0.944 | 0.944 | 0.944 | 0.944 +0/−1 | no | 0/6 |
| `rrf(k=10) nostop` | 0.917 +0/−1 | 0.917 +0/−2 | 0.917 +0/−1 | 0.917 +0/−1 | 0.917 +0/−1 | 0.917 +0/−2 | no | 0/6 |
| `rrf(k=10) stop` | 0.917 +0/−1 | 0.917 +0/−2 | 0.917 +0/−1 | 0.917 +0/−1 | 0.917 +0/−1 | 0.917 +0/−2 | no | 0/6 |
| `rrf(k=60) nostop` | 0.889 +0/−2 | 0.889 +0/−3 | 0.889 +0/−2 | 0.889 +0/−2 | 0.889 +0/−2 | 0.889 +0/−3 | no | 0/6 |
| `rrf(k=60) stop` | 0.889 +0/−2 | 0.889 +0/−3 | 0.889 +0/−2 | 0.889 +0/−2 | 0.889 +0/−2 | 0.889 +0/−3 | no | 0/6 |
| `rrf(k=2,w=1:1.5) nostop` | 0.944 | 0.972 | 0.944 | 0.944 | 0.944 | 0.972 | yes | 0/6 |
| `rrf(k=2,w=1:1.5) stop` | 0.944 | 0.972 | 0.944 | 0.944 | 0.944 | 0.972 | yes | 0/6 |
| `rrf(k=2,w=1:2) nostop` | 0.944 | 0.972 | 0.944 | 0.944 | 0.944 | 0.972 | yes | 0/6 |
| `rrf(k=2,w=1:2) stop` | 0.944 | 0.972 | 0.944 | 0.944 | 0.944 | 0.972 | yes | 0/6 |
| `rrf(k=2,w=1:3) nostop` | 0.944 | 0.944 +0/−1 | 0.944 | 0.944 | 0.944 | 0.972 | no | 0/6 |
| `rrf(k=2,w=1:3) stop` | 0.944 | 0.944 +0/−1 | 0.944 | 0.944 | 0.944 | 0.972 | no | 0/6 |
| `rrf(k=2,w=1.5:1) nostop` | 0.944 | 0.944 +0/−1 | 0.944 | 0.917 +0/−1 | 0.944 | 0.944 +0/−1 | no | 0/6 |
| `rrf(k=2,w=1.5:1) stop` | 0.944 | 0.944 +0/−1 | 0.944 | 0.917 +0/−1 | 0.944 | 0.944 +0/−1 | no | 0/6 |
| `rrf(k=2,w=2:1) nostop` | 0.944 | 0.972 | 0.944 | 0.917 +0/−1 | 0.944 | 0.944 +0/−1 | no | 0/6 |
| `rrf(k=2,w=2:1) stop` | 0.944 | 0.972 | 0.944 | 0.917 +0/−1 | 0.944 | 0.944 +0/−1 | no | 0/6 |
| `rrf(k=2,w=3:1) nostop` | 0.944 | 0.972 | 0.944 | 0.944 | 0.944 | 0.972 | yes | 0/6 |
| `rrf(k=2,w=3:1) stop` | 0.944 | 0.972 | 0.944 | 0.944 | 0.944 | 0.972 | yes | 0/6 |

### 4.3 Per model

Every aggregate, the gate (e2e thresholds), the questions missed, and the margins of
q12, q17, q30, q40 and q42; then every question whose pass/fail differs from the model's
`rrf nostop`, with recall/precision@R/distractors excluded/margin before → after.
Explain output (dense, keyword, fused and reranked top lists) for **every** miss of
every model and configuration is in
[fusion-containers-explain.md](fusion-containers-explain.md).

#### BAAI/bge-small-en-v1.5

| configuration | rec@k | prec@R | excl | scope | cit.val | cit.acc | obs | neg | evid | gate | misses | q12 | q17 | q30 | q40 | q42 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| `rrf nostop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q40 | 3.207 | 0.700 | 1.714 | 0.854 | 1.802 |
| `rrf stop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q40 | 3.207 | 0.666 | 1.714 | 0.854 | 3.308 |
| `dbsf nostop` | 1.000 | 0.972 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17 | 1.532 | 0.922 | 1.242 | 1.040 | 1.807 |
| `dbsf stop` | 1.000 | 0.972 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17 | 1.532 | 0.922 | 1.242 | 1.040 | 2.958 |
| `rrf(k=1) nostop` | 1.000 | 0.917 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q25, q40 | 4.622 | 0.555 | 2.401 | 0.854 | 2.521 |
| `rrf(k=1) stop` | 1.000 | 0.917 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q25, q40 | 4.622 | 0.500 | 2.401 | 0.854 | 4.399 |
| `rrf(k=5) nostop` | 1.000 | 0.917 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q40, q41 | 2.240 | 0.844 | 1.293 | 0.853 | 1.358 |
| `rrf(k=5) stop` | 1.000 | 0.917 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q40, q41 | 2.240 | 0.834 | 1.293 | 0.853 | 2.651 |
| `rrf(k=10) nostop` | 1.000 | 0.917 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q40, q41 | 1.923 | 0.913 | 1.148 | 0.854 | 1.206 |
| `rrf(k=10) stop` | 1.000 | 0.917 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q40, q41 | 1.923 | 0.909 | 1.148 | 0.854 | 2.425 |
| `rrf(k=60) nostop` | 1.000 | 0.889 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | FAIL | q17, q40, q41, q50 | 1.655 | 0.984 | 1.025 | 0.853 | 1.077 |
| `rrf(k=60) stop` | 1.000 | 0.889 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | FAIL | q17, q40, q41, q50 | 1.655 | 0.984 | 1.025 | 0.853 | 2.171 |
| `rrf(k=2,w=1:1.5) nostop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q40 | 3.518 | 0.718 | 1.652 | 0.858 | 1.702 |
| `rrf(k=2,w=1:1.5) stop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q40 | 3.518 | 0.693 | 1.652 | 0.858 | 3.643 |
| `rrf(k=2,w=1:2) nostop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q40 | 3.730 | 0.733 | 1.592 | 0.853 | 1.634 |
| `rrf(k=2,w=1:2) stop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q40 | 3.730 | 0.714 | 1.592 | 0.853 | 3.864 |
| `rrf(k=2,w=1:3) nostop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q40 | 4.005 | 0.758 | 1.501 | 0.840 | 1.546 |
| `rrf(k=2,w=1:3) stop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q40 | 4.005 | 0.747 | 1.501 | 0.840 | 4.135 |
| `rrf(k=2,w=1.5:1) nostop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q40 | 2.641 | 0.727 | 1.621 | 0.849 | 1.734 |
| `rrf(k=2,w=1.5:1) stop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q40 | 2.641 | 0.693 | 1.621 | 0.849 | 2.832 |
| `rrf(k=2,w=2:1) nostop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q40 | 2.336 | 0.750 | 1.557 | 0.853 | 1.671 |
| `rrf(k=2,w=2:1) stop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q40 | 2.336 | 0.714 | 1.557 | 0.853 | 2.576 |
| `rrf(k=2,w=3:1) nostop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q40 | 2.002 | 0.784 | 1.470 | 0.866 | 1.575 |
| `rrf(k=2,w=3:1) stop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q40 | 2.002 | 0.747 | 1.470 | 0.866 | 2.297 |

Per-question pass/fail changes against `rrf nostop` (recall/precision@R/distractors excluded/margin):

- `dbsf nostop`: q40-checkout-outage-root-cause fixed (1.00/0.00/2/0.854 → 1.00/1.00/2/1.040)
- `dbsf stop`: q40-checkout-outage-root-cause fixed (1.00/0.00/2/0.854 → 1.00/1.00/2/1.040)
- `rrf(k=1) nostop`: q25-failing-right-now REGRESSED (1.00/1.00/0/1.076 → 1.00/0.00/0/0.940; top ["logpattern_4bf138fdadd8c61de9cdd7e162c6b0a1"])
- `rrf(k=1) stop`: q25-failing-right-now REGRESSED (1.00/1.00/0/1.076 → 1.00/0.00/0/0.940; top ["logpattern_4bf138fdadd8c61de9cdd7e162c6b0a1"])
- `rrf(k=5) nostop`: q41-gateway-postmortem-actions REGRESSED (1.00/1.00/1/1.140 → 1.00/0.00/1/0.912; top ["incident_inc-payments-card"])
- `rrf(k=5) stop`: q41-gateway-postmortem-actions REGRESSED (1.00/1.00/1/1.140 → 1.00/0.00/1/0.912; top ["incident_inc-payments-card"])
- `rrf(k=10) nostop`: q41-gateway-postmortem-actions REGRESSED (1.00/1.00/1/1.140 → 1.00/0.00/1/0.836; top ["incident_inc-payments-card"])
- `rrf(k=10) stop`: q41-gateway-postmortem-actions REGRESSED (1.00/1.00/1/1.140 → 1.00/0.00/1/0.836; top ["incident_inc-payments-card"])
- `rrf(k=60) nostop`: q41-gateway-postmortem-actions REGRESSED (1.00/1.00/1/1.140 → 1.00/0.00/1/0.772; top ["incident_inc-payments-card"]); q50-checkout-owner-runbook REGRESSED (1.00/1.00/2/1.778 → 1.00/0.00/2/0.910; top ["incident_inc-checkout-latency"])
- `rrf(k=60) stop`: q41-gateway-postmortem-actions REGRESSED (1.00/1.00/1/1.140 → 1.00/0.00/1/0.772; top ["incident_inc-payments-card"]); q50-checkout-owner-runbook REGRESSED (1.00/1.00/2/1.778 → 1.00/0.00/2/0.903; top ["incident_inc-checkout-latency"])

#### BAAI/bge-base-en-v1.5

| configuration | rec@k | prec@R | excl | scope | cit.val | cit.acc | obs | neg | evid | gate | misses | q12 | q17 | q30 | q40 | q42 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| `rrf nostop` | 1.000 | 0.972 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17 | 2.401 | 0.700 | 1.000 | 1.279 | 1.575 |
| `rrf stop` | 1.000 | 0.972 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17 | 2.401 | 0.666 | 1.000 | 1.279 | 3.156 |
| `dbsf nostop` | 1.000 | 0.972 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17 | 1.373 | 0.944 | 1.003 | 1.407 | 1.760 |
| `dbsf stop` | 1.000 | 0.972 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17 | 1.373 | 0.943 | 1.003 | 1.407 | 2.988 |
| `rrf(k=1) nostop` | 1.000 | 0.972 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17 | 3.207 | 0.555 | 1.002 | 1.708 | 2.099 |
| `rrf(k=1) stop` | 1.000 | 0.972 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17 | 3.207 | 0.500 | 1.002 | 1.708 | 4.198 |
| `rrf(k=5) nostop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q41 | 1.923 | 0.844 | 1.000 | 1.024 | 1.261 |
| `rrf(k=5) stop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q41 | 1.923 | 0.834 | 1.000 | 1.024 | 2.521 |
| `rrf(k=10) nostop` | 1.000 | 0.917 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q40, q41 | 1.759 | 0.913 | 1.000 | 0.939 | 1.155 |
| `rrf(k=10) stop` | 1.000 | 0.917 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q40, q41 | 1.759 | 0.909 | 1.000 | 0.939 | 2.310 |
| `rrf(k=60) nostop` | 1.000 | 0.889 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | FAIL | q17, q40, q41, q50 | 1.627 | 0.984 | 1.001 | 0.868 | 1.067 |
| `rrf(k=60) stop` | 1.000 | 0.889 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | FAIL | q17, q40, q41, q50 | 1.627 | 0.984 | 1.001 | 0.868 | 2.137 |
| `rrf(k=2,w=1:1.5) nostop` | 1.000 | 0.972 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17 | 2.641 | 0.718 | 1.005 | 1.231 | 1.517 |
| `rrf(k=2,w=1:1.5) stop` | 1.000 | 0.972 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17 | 2.641 | 0.693 | 1.005 | 1.231 | 3.460 |
| `rrf(k=2,w=1:2) nostop` | 1.000 | 0.972 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17 | 2.803 | 0.733 | 1.001 | 1.194 | 1.472 |
| `rrf(k=2,w=1:2) stop` | 1.000 | 0.972 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17 | 2.803 | 0.714 | 1.001 | 1.194 | 3.676 |
| `rrf(k=2,w=1:3) nostop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q30 | 2.997 | 0.758 | 0.986 | 1.143 | 1.406 |
| `rrf(k=2,w=1:3) stop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q30 | 2.997 | 0.747 | 0.986 | 1.143 | 3.938 |
| `rrf(k=2,w=1.5:1) nostop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q30 | 2.054 | 0.727 | 0.995 | 1.231 | 1.517 |
| `rrf(k=2,w=1.5:1) stop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q30 | 2.054 | 0.693 | 0.995 | 1.231 | 2.694 |
| `rrf(k=2,w=2:1) nostop` | 1.000 | 0.972 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17 | 1.869 | 0.750 | 1.001 | 1.194 | 1.472 |
| `rrf(k=2,w=2:1) stop` | 1.000 | 0.972 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17 | 1.869 | 0.714 | 1.001 | 1.194 | 2.454 |
| `rrf(k=2,w=3:1) nostop` | 1.000 | 0.972 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17 | 1.668 | 0.784 | 1.016 | 1.143 | 1.406 |
| `rrf(k=2,w=3:1) stop` | 1.000 | 0.972 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17 | 1.668 | 0.747 | 1.016 | 1.143 | 2.188 |

Per-question pass/fail changes against `rrf nostop` (recall/precision@R/distractors excluded/margin):

- `rrf(k=5) nostop`: q41-gateway-postmortem-actions REGRESSED (1.00/1.00/1/1.140 → 1.00/0.00/1/0.912; top ["incident_inc-payments-card"])
- `rrf(k=5) stop`: q41-gateway-postmortem-actions REGRESSED (1.00/1.00/1/1.140 → 1.00/0.00/1/0.912; top ["incident_inc-payments-card"])
- `rrf(k=10) nostop`: q40-checkout-outage-root-cause REGRESSED (1.00/1.00/2/1.279 → 1.00/0.00/2/0.939; top ["incident_inc-checkout-latency"]); q41-gateway-postmortem-actions REGRESSED (1.00/1.00/1/1.140 → 1.00/0.00/1/0.836; top ["incident_inc-payments-card"])
- `rrf(k=10) stop`: q40-checkout-outage-root-cause REGRESSED (1.00/1.00/2/1.279 → 1.00/0.00/2/0.939; top ["incident_inc-checkout-latency"]); q41-gateway-postmortem-actions REGRESSED (1.00/1.00/1/1.140 → 1.00/0.00/1/0.836; top ["incident_inc-payments-card"])
- `rrf(k=60) nostop`: q40-checkout-outage-root-cause REGRESSED (1.00/1.00/2/1.279 → 1.00/0.00/2/0.868; top ["incident_inc-checkout-latency"]); q41-gateway-postmortem-actions REGRESSED (1.00/1.00/1/1.140 → 1.00/0.00/1/0.772; top ["incident_inc-payments-card"]); q50-checkout-owner-runbook REGRESSED (1.00/1.00/2/2.117 → 1.00/0.00/2/0.837; top ["incident_inc-checkout-latency"])
- `rrf(k=60) stop`: q40-checkout-outage-root-cause REGRESSED (1.00/1.00/2/1.279 → 1.00/0.00/2/0.868; top ["incident_inc-checkout-latency"]); q41-gateway-postmortem-actions REGRESSED (1.00/1.00/1/1.140 → 1.00/0.00/1/0.772; top ["incident_inc-payments-card"]); q50-checkout-owner-runbook REGRESSED (1.00/1.00/2/2.117 → 1.00/0.00/2/0.830; top ["incident_inc-checkout-latency"])
- `rrf(k=2,w=1:3) nostop`: q30-exact-error-code REGRESSED (1.00/1.00/0/1.000 → 1.00/0.00/0/0.986; top ["logpattern_a61d00036f005ca3d372d572df6ef8e7"])
- `rrf(k=2,w=1:3) stop`: q30-exact-error-code REGRESSED (1.00/1.00/0/1.000 → 1.00/0.00/0/0.986; top ["logpattern_a61d00036f005ca3d372d572df6ef8e7"])
- `rrf(k=2,w=1.5:1) nostop`: q30-exact-error-code REGRESSED (1.00/1.00/0/1.000 → 1.00/0.00/0/0.995; top ["logpattern_a61d00036f005ca3d372d572df6ef8e7"])
- `rrf(k=2,w=1.5:1) stop`: q30-exact-error-code REGRESSED (1.00/1.00/0/1.000 → 1.00/0.00/0/0.995; top ["logpattern_a61d00036f005ca3d372d572df6ef8e7"])

#### BAAI/bge-large-en-v1.5

| configuration | rec@k | prec@R | excl | scope | cit.val | cit.acc | obs | neg | evid | gate | misses | q12 | q17 | q30 | q40 | q42 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| `rrf nostop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q41 | 2.401 | 0.700 | 1.714 | 1.279 | 1.802 |
| `rrf stop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q41 | 2.401 | 0.666 | 1.714 | 1.279 | 3.308 |
| `dbsf nostop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q41 | 1.471 | 0.929 | 1.193 | 1.372 | 1.798 |
| `dbsf stop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q41 | 1.471 | 0.930 | 1.193 | 1.372 | 2.958 |
| `rrf(k=1) nostop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q41 | 3.207 | 0.555 | 2.401 | 1.708 | 2.521 |
| `rrf(k=1) stop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q41 | 3.207 | 0.500 | 2.401 | 1.708 | 4.399 |
| `rrf(k=5) nostop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q41 | 1.923 | 0.844 | 1.293 | 1.024 | 1.358 |
| `rrf(k=5) stop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q41 | 1.923 | 0.834 | 1.293 | 1.024 | 2.651 |
| `rrf(k=10) nostop` | 1.000 | 0.917 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q40, q41 | 1.759 | 0.913 | 1.148 | 0.939 | 1.206 |
| `rrf(k=10) stop` | 1.000 | 0.917 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q40, q41 | 1.759 | 0.909 | 1.148 | 0.939 | 2.425 |
| `rrf(k=60) nostop` | 1.000 | 0.889 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | FAIL | q17, q40, q41, q50 | 1.627 | 0.984 | 1.025 | 0.868 | 1.077 |
| `rrf(k=60) stop` | 1.000 | 0.889 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | FAIL | q17, q40, q41, q50 | 1.627 | 0.984 | 1.025 | 0.868 | 2.171 |
| `rrf(k=2,w=1:1.5) nostop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q41 | 2.641 | 0.718 | 1.621 | 1.231 | 1.702 |
| `rrf(k=2,w=1:1.5) stop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q41 | 2.641 | 0.693 | 1.621 | 1.231 | 3.643 |
| `rrf(k=2,w=1:2) nostop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q41 | 2.803 | 0.733 | 1.557 | 1.194 | 1.634 |
| `rrf(k=2,w=1:2) stop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q41 | 2.803 | 0.714 | 1.557 | 1.194 | 3.864 |
| `rrf(k=2,w=1:3) nostop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q41 | 2.997 | 0.758 | 1.470 | 1.143 | 1.546 |
| `rrf(k=2,w=1:3) stop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q41 | 2.997 | 0.747 | 1.470 | 1.143 | 4.135 |
| `rrf(k=2,w=1.5:1) nostop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q41 | 2.054 | 0.727 | 1.621 | 1.231 | 1.734 |
| `rrf(k=2,w=1.5:1) stop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q41 | 2.054 | 0.693 | 1.621 | 1.231 | 2.832 |
| `rrf(k=2,w=2:1) nostop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q41 | 1.869 | 0.750 | 1.557 | 1.194 | 1.671 |
| `rrf(k=2,w=2:1) stop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q41 | 1.869 | 0.714 | 1.557 | 1.194 | 2.576 |
| `rrf(k=2,w=3:1) nostop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q41 | 1.668 | 0.784 | 1.470 | 1.143 | 1.575 |
| `rrf(k=2,w=3:1) stop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q41 | 1.668 | 0.747 | 1.470 | 1.143 | 2.297 |

Per-question pass/fail changes against `rrf nostop` (recall/precision@R/distractors excluded/margin):

- `rrf(k=10) nostop`: q40-checkout-outage-root-cause REGRESSED (1.00/1.00/2/1.279 → 1.00/0.00/2/0.939; top ["incident_inc-checkout-latency"])
- `rrf(k=10) stop`: q40-checkout-outage-root-cause REGRESSED (1.00/1.00/2/1.279 → 1.00/0.00/2/0.939; top ["incident_inc-checkout-latency"])
- `rrf(k=60) nostop`: q40-checkout-outage-root-cause REGRESSED (1.00/1.00/2/1.279 → 1.00/0.00/2/0.868; top ["incident_inc-checkout-latency"]); q50-checkout-owner-runbook REGRESSED (1.00/1.00/2/1.927 → 1.00/0.00/2/0.956; top ["incident_inc-checkout-latency"])
- `rrf(k=60) stop`: q40-checkout-outage-root-cause REGRESSED (1.00/1.00/2/1.279 → 1.00/0.00/2/0.868; top ["incident_inc-checkout-latency"]); q50-checkout-owner-runbook REGRESSED (1.00/1.00/2/1.927 → 1.00/0.00/2/0.947; top ["incident_inc-checkout-latency"])

#### intfloat/e5-small-v2

| configuration | rec@k | prec@R | excl | scope | cit.val | cit.acc | obs | neg | evid | gate | misses | q12 | q17 | q30 | q40 | q42 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| `rrf nostop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q41 | 2.401 | 0.700 | 1.110 | 1.279 | 1.575 |
| `rrf stop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q41 | 2.401 | 0.666 | 1.110 | 1.279 | 3.156 |
| `dbsf nostop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q41 | 1.671 | 0.851 | 1.135 | 1.411 | 1.648 |
| `dbsf stop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q41 | 1.671 | 0.856 | 1.135 | 1.411 | 2.652 |
| `rrf(k=1) nostop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q41 | 3.207 | 0.555 | 1.126 | 1.708 | 2.099 |
| `rrf(k=1) stop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q41 | 3.207 | 0.500 | 1.126 | 1.708 | 4.198 |
| `rrf(k=5) nostop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q41 | 1.923 | 0.844 | 1.068 | 1.024 | 1.261 |
| `rrf(k=5) stop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q41 | 1.923 | 0.834 | 1.068 | 1.024 | 2.521 |
| `rrf(k=10) nostop` | 1.000 | 0.917 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q40, q41 | 1.759 | 0.913 | 1.041 | 0.939 | 1.155 |
| `rrf(k=10) stop` | 1.000 | 0.917 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q40, q41 | 1.759 | 0.909 | 1.041 | 0.939 | 2.310 |
| `rrf(k=60) nostop` | 1.000 | 0.889 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | FAIL | q17, q40, q41, q50 | 1.627 | 0.984 | 1.008 | 0.868 | 1.067 |
| `rrf(k=60) stop` | 1.000 | 0.889 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | FAIL | q17, q40, q41, q50 | 1.627 | 0.984 | 1.008 | 0.868 | 2.137 |
| `rrf(k=2,w=1:1.5) nostop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q41 | 2.641 | 0.718 | 1.120 | 1.231 | 1.517 |
| `rrf(k=2,w=1:1.5) stop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q41 | 2.641 | 0.693 | 1.120 | 1.231 | 3.460 |
| `rrf(k=2,w=1:2) nostop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q41 | 2.803 | 0.733 | 1.112 | 1.194 | 1.472 |
| `rrf(k=2,w=1:2) stop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q41 | 2.803 | 0.714 | 1.112 | 1.194 | 3.676 |
| `rrf(k=2,w=1:3) nostop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q41 | 2.997 | 0.758 | 1.083 | 1.143 | 1.406 |
| `rrf(k=2,w=1:3) stop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q41 | 2.997 | 0.747 | 1.083 | 1.143 | 3.938 |
| `rrf(k=2,w=1.5:1) nostop` | 1.000 | 0.917 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q31, q41 | 2.054 | 0.727 | 1.092 | 1.231 | 1.517 |
| `rrf(k=2,w=1.5:1) stop` | 1.000 | 0.917 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q31, q41 | 2.054 | 0.693 | 1.092 | 1.231 | 2.694 |
| `rrf(k=2,w=2:1) nostop` | 1.000 | 0.917 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q31, q41 | 1.869 | 0.750 | 1.091 | 1.194 | 1.472 |
| `rrf(k=2,w=2:1) stop` | 1.000 | 0.917 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q31, q41 | 1.869 | 0.714 | 1.091 | 1.194 | 2.454 |
| `rrf(k=2,w=3:1) nostop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q41 | 1.668 | 0.784 | 1.101 | 1.143 | 1.406 |
| `rrf(k=2,w=3:1) stop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q41 | 1.668 | 0.747 | 1.101 | 1.143 | 2.188 |

Per-question pass/fail changes against `rrf nostop` (recall/precision@R/distractors excluded/margin):

- `rrf(k=10) nostop`: q40-checkout-outage-root-cause REGRESSED (1.00/1.00/2/1.279 → 1.00/0.00/2/0.939; top ["incident_inc-checkout-latency"])
- `rrf(k=10) stop`: q40-checkout-outage-root-cause REGRESSED (1.00/1.00/2/1.279 → 1.00/0.00/2/0.939; top ["incident_inc-checkout-latency"])
- `rrf(k=60) nostop`: q40-checkout-outage-root-cause REGRESSED (1.00/1.00/2/1.279 → 1.00/0.00/2/0.868; top ["incident_inc-checkout-latency"]); q50-checkout-owner-runbook REGRESSED (1.00/1.00/2/2.356 → 1.00/0.00/2/0.960; top ["incident_inc-checkout-latency"])
- `rrf(k=60) stop`: q40-checkout-outage-root-cause REGRESSED (1.00/1.00/2/1.279 → 1.00/0.00/2/0.868; top ["incident_inc-checkout-latency"]); q50-checkout-owner-runbook REGRESSED (1.00/1.00/2/2.356 → 1.00/0.00/2/0.952; top ["incident_inc-checkout-latency"])
- `rrf(k=2,w=1.5:1) nostop`: q31-exact-metric-name REGRESSED (1.00/1.00/0/1.000 → 1.00/0.00/0/0.980; top ["monitor_1102"])
- `rrf(k=2,w=1.5:1) stop`: q31-exact-metric-name REGRESSED (1.00/1.00/0/1.000 → 1.00/0.00/0/0.980; top ["monitor_1102"])
- `rrf(k=2,w=2:1) nostop`: q31-exact-metric-name REGRESSED (1.00/1.00/0/1.000 → 1.00/0.00/0/0.981; top ["monitor_1102"])
- `rrf(k=2,w=2:1) stop`: q31-exact-metric-name REGRESSED (1.00/1.00/0/1.000 → 1.00/0.00/0/0.981; top ["monitor_1102"])

#### intfloat/e5-base-v2

| configuration | rec@k | prec@R | excl | scope | cit.val | cit.acc | obs | neg | evid | gate | misses | q12 | q17 | q30 | q40 | q42 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| `rrf nostop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q41 | 2.401 | 0.700 | 1.110 | 1.279 | 1.575 |
| `rrf stop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q41 | 2.401 | 0.666 | 1.110 | 1.279 | 3.156 |
| `dbsf nostop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q41 | 1.740 | 0.904 | 1.131 | 1.384 | 1.771 |
| `dbsf stop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q41 | 1.740 | 0.905 | 1.131 | 1.384 | 3.025 |
| `rrf(k=1) nostop` | 1.000 | 0.917 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q25, q41 | 3.207 | 0.555 | 1.126 | 1.708 | 2.099 |
| `rrf(k=1) stop` | 1.000 | 0.917 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q25, q41 | 3.207 | 0.500 | 1.126 | 1.708 | 4.198 |
| `rrf(k=5) nostop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q41 | 1.923 | 0.844 | 1.068 | 1.024 | 1.261 |
| `rrf(k=5) stop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q41 | 1.923 | 0.834 | 1.068 | 1.024 | 2.521 |
| `rrf(k=10) nostop` | 1.000 | 0.917 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q40, q41 | 1.759 | 0.913 | 1.041 | 0.939 | 1.155 |
| `rrf(k=10) stop` | 1.000 | 0.917 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q40, q41 | 1.759 | 0.909 | 1.041 | 0.939 | 2.310 |
| `rrf(k=60) nostop` | 1.000 | 0.889 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | FAIL | q17, q40, q41, q50 | 1.627 | 0.984 | 1.008 | 0.868 | 1.067 |
| `rrf(k=60) stop` | 1.000 | 0.889 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | FAIL | q17, q40, q41, q50 | 1.627 | 0.984 | 1.008 | 0.868 | 2.137 |
| `rrf(k=2,w=1:1.5) nostop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q41 | 2.641 | 0.718 | 1.120 | 1.231 | 1.517 |
| `rrf(k=2,w=1:1.5) stop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q41 | 2.641 | 0.693 | 1.120 | 1.231 | 3.460 |
| `rrf(k=2,w=1:2) nostop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q41 | 2.803 | 0.733 | 1.112 | 1.194 | 1.472 |
| `rrf(k=2,w=1:2) stop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q41 | 2.803 | 0.714 | 1.112 | 1.194 | 3.676 |
| `rrf(k=2,w=1:3) nostop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q41 | 2.997 | 0.758 | 1.083 | 1.143 | 1.406 |
| `rrf(k=2,w=1:3) stop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q41 | 2.997 | 0.747 | 1.083 | 1.143 | 3.938 |
| `rrf(k=2,w=1.5:1) nostop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q41 | 2.054 | 0.727 | 1.092 | 1.231 | 1.517 |
| `rrf(k=2,w=1.5:1) stop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q41 | 2.054 | 0.693 | 1.092 | 1.231 | 2.694 |
| `rrf(k=2,w=2:1) nostop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q41 | 1.869 | 0.750 | 1.091 | 1.194 | 1.472 |
| `rrf(k=2,w=2:1) stop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q41 | 1.869 | 0.714 | 1.091 | 1.194 | 2.454 |
| `rrf(k=2,w=3:1) nostop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q41 | 1.668 | 0.784 | 1.101 | 1.143 | 1.406 |
| `rrf(k=2,w=3:1) stop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q41 | 1.668 | 0.747 | 1.101 | 1.143 | 2.188 |

Per-question pass/fail changes against `rrf nostop` (recall/precision@R/distractors excluded/margin):

- `rrf(k=1) nostop`: q25-failing-right-now REGRESSED (1.00/1.00/0/1.076 → 1.00/0.00/0/0.940; top ["logpattern_4bf138fdadd8c61de9cdd7e162c6b0a1"])
- `rrf(k=1) stop`: q25-failing-right-now REGRESSED (1.00/1.00/0/1.076 → 1.00/0.00/0/0.940; top ["logpattern_4bf138fdadd8c61de9cdd7e162c6b0a1"])
- `rrf(k=10) nostop`: q40-checkout-outage-root-cause REGRESSED (1.00/1.00/2/1.279 → 1.00/0.00/2/0.939; top ["incident_inc-checkout-latency"])
- `rrf(k=10) stop`: q40-checkout-outage-root-cause REGRESSED (1.00/1.00/2/1.279 → 1.00/0.00/2/0.939; top ["incident_inc-checkout-latency"])
- `rrf(k=60) nostop`: q40-checkout-outage-root-cause REGRESSED (1.00/1.00/2/1.279 → 1.00/0.00/2/0.868; top ["incident_inc-checkout-latency"]); q50-checkout-owner-runbook REGRESSED (1.00/1.00/2/2.677 → 1.00/0.00/2/0.952; top ["incident_inc-checkout-latency"])
- `rrf(k=60) stop`: q40-checkout-outage-root-cause REGRESSED (1.00/1.00/2/1.279 → 1.00/0.00/2/0.868; top ["incident_inc-checkout-latency"]); q50-checkout-owner-runbook REGRESSED (1.00/1.00/2/2.677 → 1.00/0.00/2/0.942; top ["incident_inc-checkout-latency"])

#### nomic-ai/nomic-embed-text-v1.5

| configuration | rec@k | prec@R | excl | scope | cit.val | cit.acc | obs | neg | evid | gate | misses | q12 | q17 | q30 | q40 | q42 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| `rrf nostop` | 1.000 | 0.972 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17 | 2.401 | 0.700 | 1.110 | 1.463 | 1.575 |
| `rrf stop` | 1.000 | 0.972 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17 | 2.401 | 0.666 | 1.110 | 1.463 | 3.156 |
| `dbsf nostop` | 1.000 | 0.972 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17 | 1.797 | 0.944 | 1.150 | 1.521 | 1.556 |
| `dbsf stop` | 1.000 | 0.972 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17 | 1.797 | 0.943 | 1.150 | 1.521 | 2.417 |
| `rrf(k=1) nostop` | 1.000 | 0.972 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17 | 3.207 | 0.555 | 1.126 | 2.047 | 2.099 |
| `rrf(k=1) stop` | 1.000 | 0.972 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17 | 3.207 | 0.500 | 1.126 | 2.047 | 4.198 |
| `rrf(k=5) nostop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q41 | 1.923 | 0.844 | 1.068 | 1.103 | 1.261 |
| `rrf(k=5) stop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q41 | 1.923 | 0.834 | 1.068 | 1.103 | 2.521 |
| `rrf(k=10) nostop` | 1.000 | 0.917 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q40, q41 | 1.759 | 0.913 | 1.041 | 0.980 | 1.155 |
| `rrf(k=10) stop` | 1.000 | 0.917 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q40, q41 | 1.759 | 0.909 | 1.041 | 0.980 | 2.310 |
| `rrf(k=60) nostop` | 1.000 | 0.889 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | FAIL | q17, q40, q41, q50 | 1.627 | 0.984 | 1.008 | 0.875 | 1.067 |
| `rrf(k=60) stop` | 1.000 | 0.889 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | FAIL | q17, q40, q41, q50 | 1.627 | 0.984 | 1.008 | 0.875 | 2.137 |
| `rrf(k=2,w=1:1.5) nostop` | 1.000 | 0.972 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17 | 2.641 | 0.718 | 1.120 | 1.384 | 1.517 |
| `rrf(k=2,w=1:1.5) stop` | 1.000 | 0.972 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17 | 2.641 | 0.693 | 1.120 | 1.384 | 3.460 |
| `rrf(k=2,w=1:2) nostop` | 1.000 | 0.972 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17 | 2.803 | 0.733 | 1.112 | 1.326 | 1.472 |
| `rrf(k=2,w=1:2) stop` | 1.000 | 0.972 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17 | 2.803 | 0.714 | 1.112 | 1.326 | 3.676 |
| `rrf(k=2,w=1:3) nostop` | 1.000 | 0.972 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17 | 2.997 | 0.758 | 1.083 | 1.256 | 1.406 |
| `rrf(k=2,w=1:3) stop` | 1.000 | 0.972 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17 | 2.997 | 0.747 | 1.083 | 1.256 | 3.938 |
| `rrf(k=2,w=1.5:1) nostop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q31 | 2.054 | 0.727 | 1.092 | 1.408 | 1.517 |
| `rrf(k=2,w=1.5:1) stop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q31 | 2.054 | 0.693 | 1.092 | 1.408 | 2.694 |
| `rrf(k=2,w=2:1) nostop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q31 | 1.869 | 0.750 | 1.091 | 1.357 | 1.472 |
| `rrf(k=2,w=2:1) stop` | 1.000 | 0.944 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17, q31 | 1.869 | 0.714 | 1.091 | 1.357 | 2.454 |
| `rrf(k=2,w=3:1) nostop` | 1.000 | 0.972 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17 | 1.668 | 0.784 | 1.101 | 1.279 | 1.406 |
| `rrf(k=2,w=3:1) stop` | 1.000 | 0.972 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | ok | q17 | 1.668 | 0.747 | 1.101 | 1.279 | 2.188 |

Per-question pass/fail changes against `rrf nostop` (recall/precision@R/distractors excluded/margin):

- `rrf(k=5) nostop`: q41-gateway-postmortem-actions REGRESSED (1.00/1.00/1/1.140 → 1.00/0.00/1/0.912; top ["incident_inc-payments-card"])
- `rrf(k=5) stop`: q41-gateway-postmortem-actions REGRESSED (1.00/1.00/1/1.140 → 1.00/0.00/1/0.912; top ["incident_inc-payments-card"])
- `rrf(k=10) nostop`: q40-checkout-outage-root-cause REGRESSED (1.00/1.00/2/1.463 → 1.00/0.00/2/0.980; top ["incident_inc-checkout-latency"]); q41-gateway-postmortem-actions REGRESSED (1.00/1.00/1/1.140 → 1.00/0.00/1/0.836; top ["incident_inc-payments-card"])
- `rrf(k=10) stop`: q40-checkout-outage-root-cause REGRESSED (1.00/1.00/2/1.463 → 1.00/0.00/2/0.980; top ["incident_inc-checkout-latency"]); q41-gateway-postmortem-actions REGRESSED (1.00/1.00/1/1.140 → 1.00/0.00/1/0.836; top ["incident_inc-payments-card"])
- `rrf(k=60) nostop`: q40-checkout-outage-root-cause REGRESSED (1.00/1.00/2/1.463 → 1.00/0.00/2/0.875; top ["incident_inc-checkout-latency"]); q41-gateway-postmortem-actions REGRESSED (1.00/1.00/1/1.140 → 1.00/0.00/1/0.772; top ["incident_inc-payments-card"]); q50-checkout-owner-runbook REGRESSED (1.00/1.00/2/2.130 → 1.00/0.00/2/0.926; top ["incident_inc-checkout-latency"])
- `rrf(k=60) stop`: q40-checkout-outage-root-cause REGRESSED (1.00/1.00/2/1.463 → 1.00/0.00/2/0.875; top ["incident_inc-checkout-latency"]); q41-gateway-postmortem-actions REGRESSED (1.00/1.00/1/1.140 → 1.00/0.00/1/0.772; top ["incident_inc-payments-card"]); q50-checkout-owner-runbook REGRESSED (1.00/1.00/2/2.130 → 1.00/0.00/2/0.917; top ["incident_inc-checkout-latency"])
- `rrf(k=2,w=1.5:1) nostop`: q31-exact-metric-name REGRESSED (1.00/1.00/0/1.000 → 1.00/0.00/0/0.980; top ["monitor_1102"])
- `rrf(k=2,w=1.5:1) stop`: q31-exact-metric-name REGRESSED (1.00/1.00/0/1.000 → 1.00/0.00/0/0.980; top ["monitor_1102"])
- `rrf(k=2,w=2:1) nostop`: q31-exact-metric-name REGRESSED (1.00/1.00/0/1.000 → 1.00/0.00/0/0.981; top ["monitor_1102"])
- `rrf(k=2,w=2:1) stop`: q31-exact-metric-name REGRESSED (1.00/1.00/0/1.000 → 1.00/0.00/0/0.981; top ["monitor_1102"])

### 4.4 The misses, explained

- **q17** ("Which errors did the inventory service log yesterday?"), missed by every
  model under every configuration: dense search ranks the orders service's log "inventory
  client error: POST /reservations returned #" first (0.84 against 0.81 for bge-small)
  and keyword search matches it too, because it is about inventory; the question
  expects the inventory service's own log. No service filter applies (the plan has
  none), so fusion cannot fix it; DBSF only narrows the gap (margin 0.70 → 0.85–0.94).
- **q40** (bge-small with RRF) and **q41** (bge-large, e5-small, e5-base with RRF *and*
  DBSF): two incidents in scope, each first in one list and second in the other. RRF
  scores them identically (0.8333 each), and the reranker's recency weight then puts the
  newer incident first (q40: checkout latency 0.806 against the outage 0.688; q41: card
  payments 0.905 against the gateway certificate 0.688), though keyword search decisively
  prefers the right one (q40: 24.6 against 1.8; q41: 39.8 against 17.0). DBSF breaks
  q40's tie for bge-small only because three incidents are in scope there; with two
  (q41) any two scores map to 0.5 ± 0.118 per list, so DBSF ties them too (0.5 each).
  With the models where RRF passes q40, the two incidents are not tied (margin
  1.28–1.46).
- **k ≠ 2** regressions: a larger `k` compresses the RRF scores towards each other (at
  `k` = 60 first and second in a list differ by 1.6 %), so the kind priors and the
  recency weight, tuned on `k` = 2's gaps, decide more, and the newer incident wins
  (q40, q41, q50). `k` = 1 does the opposite: it widens the gap between first and second
  place in a list (1 against 0.5, where `k` = 2 gives 0.5 against 0.33), so the fused
  order outweighs the recency weight where the question needs it. In q25 ("What is
  failing in notifications right now?", bge-small and e5-base) an older log is first in
  three of the four lists and the expected, newer one in one; at `k` = 2 recency puts the
  newer one first, at `k` = 1 it no longer can. The fake set's q09 fails at `k` = 1 too.
- **Weight** regressions (bge-base q30 at 1:3 and 1.5:1; e5-small q31 and nomic q31 at
  1.5:1 and 2:1) are all questions that pass the default on a margin of exactly 1.000,
  i.e. on a tie in the reranked score: an exact identifier (an `ERR_…` code, a metric
  name) that keyword search finds. Any weight moves such a tie one way or the other.

## 5. What holds across models, and what depends on one

Across all six models:

- DBSF never makes a question fail that RRF passes, on real embeddings; it raises the
  margins of q17 and q40 and lowers q12's (3.2 or 2.4 → 1.4–1.8 for every model). On
  fake embeddings it fails q12.
- `k` = 10 and `k` = 60 regress questions for every model (k = 60 falls below the
  precision@R threshold of 0.90 for every model); `k` = 5 for three models, `k` = 1 for
  two. `k` = 2 is the best of the tested values for every model.
- Weighted RRF at `k` = 2 never fixes a question. 1:1.5, 1:2 and 3:1 change nothing for
  any model; 1:3, 1.5:1 and 2:1 each regress one question for one to three models.
- Query stopwords never change pass/fail; they raise q42's margin (about 1.6–1.8 → 3.0–3.3
  with RRF) and lower q17's slightly.
- q17 is missed by every model, and the other misses are fusion ties decided by recency.

Specific to one model:

- DBSF's improvement: only bge-small (q40, precision@R 0.944 → 0.972). For bge-base and
  nomic, RRF already gets 0.972; for bge-large, e5-small and e5-base both get 0.944.
- Which tie question fails: q40 for bge-small, q41 for bge-large, e5-small and e5-base,
  neither for bge-base and nomic.
- The size of DBSF's margin changes (e.g. q30: lower for bge-small and bge-large, a
  little higher for the others).

Model quality by itself (default configuration): bge-base and nomic 0.972, the others
0.944 — no larger model is better per se on this set (bge-large misses q41, bge-base
does not).

## 6. Decision under the rule

| Candidate | (a) no fake-set regression | (b) ≥ default for every model | (b) better for most (≥ 4 of 6) | Switch? |
|---|---|---|---|---|
| `dbsf` (either stopwords) | no (q12 distractor) | yes | no (1 of 6) | no |
| `rrf`, `k` = 1, 5, 10 or 60 | no | no | no (0 of 6) | no |
| `rrf`, `k` = 2, weights 1:2 (either stopwords) | yes | yes | no (0 of 6) | no |
| `rrf`, `k` = 2, weights 1:1.5 or 3:1 | only without stopwords | yes | no (0 of 6) | no |
| `rrf`, `k` = 2, weights 1:3, 1.5:1, 2:1 | yes | no | no | no |
| stopwords `on` (with `rrf`) | yes | yes | no (0 of 6) | no |

The defaults are unchanged. `e2e_thresholds.json` is unchanged: the CI model's measured
results are the same as before (precision@R 0.944, everything else 1.000), and no
threshold in `questions.json` was touched.

## Next steps

1. **Break fusion ties before the recency weight does.** q40 and q41 fail on exact RRF
   ties (and q41 on an exact DBSF tie) between two incidents, with the keyword score
   clearly favouring the right one. Options to measure: a tie-break on the raw keyword
   (or dense) score inside the reranker, a smaller recency effect for questions that
   ask about "the root cause"/"postmortem" of a past incident (no window), or a
   `formula` query that adds a small multiple of the normalized keyword score to RRF.
   Each needs the fake Qdrant to follow (for `formula`, checked against the container
   like the fusions here).
2. **Scope q17 by service.** The planner's canned plan has no service for "the inventory
   service"; a service filter (or a service-name boost from the service catalog) would
   fix q17 for every model, whatever the fusion.
3. **Keep measuring with more than one model.** bge-small alone suggested DBSF; five
   other models did not. Any future retrieval change should be run with at least
   bge-small, bge-base and one e5 or nomic model (the whole grid here takes 1.3–4.4
   minutes per model on 4 CPUs, including TEI's start).
4. Run `text-embedding-3-small` when an `OPENAI_API_KEY` is available (command below).
5. Questions that pass the default on a margin of exactly 1.000 pass on a tie in the
   reranked score and flip with any small change: bge-base q30, e5-small q31, nomic q31,
   and the fake set's q17 with stopwords. The tie-break of step 1 should cover them too.

## Timing

Second (clean) run of the grid, nothing else running; TEI on 4 CPUs, models already in
the cache. Per model: starting TEI, indexing the corpus, asking the 36 questions once
per configuration, and the whole `scripts/e2e.sh` run (index plus 24 configurations,
each starting `rag-api` per distinct `now`).

| model | dimension | TEI start (cached model) | index (rag-indexer) | ask 36 questions, per configuration (median, min–max) | whole e2e.sh run (24 configurations) | TEI memory |
|---|---|---|---|---|---|---|
| BAAI/bge-small-en-v1.5 | 384 | 3 s | 5 s | 2.6 s (2.5–2.9) | 76 s | 577.1MiB |
| BAAI/bge-base-en-v1.5 | 768 | 2 s | 14 s | 3.0 s (2.9–3.2) | 94 s | 1.023GiB |
| BAAI/bge-large-en-v1.5 | 1024 | 9 s | 45 s | 5.0 s (4.7–5.2) | 176 s | 2.644GiB |
| intfloat/e5-small-v2 | 384 | 2 s | 4 s | 2.7 s (2.5–3.1) | 77 s | 455.5MiB |
| intfloat/e5-base-v2 | 768 | 2 s | 14 s | 2.9 s (2.8–3.0) | 93 s | 1.075GiB |
| nomic-ai/nomic-embed-text-v1.5 | 768 | 110 s | 46 s | 4.1 s (3.9–4.4) | 152 s | 600.7MiB |

nomic's TEI start is TEI warming up the Candle `NomicBert` model on CPU (111 s from
"Warming up model" to "Ready"; its maximum input is 8192 tokens, against 512 for the
others). The first start of each model, including its download, took 27 s (bge-base,
e5) to 4.3 min (nomic, bge-large).

## Reproduction

Build once, start Qdrant, then run the grid per model (TEI started by the script with
`E2E_DOCKER=tei`, or already running at `TEI_URL`):

```bash
cargo build --release --locked --workspace
docker run -d --name qdrant -p 6333:6333 qdrant/qdrant:v1.19.1

GRID="rrf:on dbsf:off dbsf:on"
for k in 1 5 10 60; do GRID+=" rrf:off:k=$k rrf:on:k=$k"; done
for w in 1,1.5 1,2 1,3 1.5,1 2,1 3,1; do GRID+=" rrf:off:w=$w rrf:on:w=$w"; done

run() {  # model revision query-prefix document-prefix output-dir
  E2E_SKIP_BUILD=1 E2E_DOCKER=tei QDRANT_ENDPOINT=http://127.0.0.1:6333 \
  TEI_IMAGE=ghcr.io/huggingface/text-embeddings-inference:cpu-1.9.4 \
  TEI_MODEL="$1" TEI_MODEL_REVISION="$2" TEI_QUERY_PREFIX="$3" TEI_DOCUMENT_PREFIX="$4" \
  E2E_OUT="$5" E2E_COMPARE="$GRID" scripts/e2e.sh
}
BGE="Represent this sentence for searching relevant passages: "
run BAAI/bge-small-en-v1.5 5c38ec7c405ec4b44b94cc5a9bb96e735b38267a "$BGE" "" out/bge-small
run BAAI/bge-base-en-v1.5 a5beb1e3e68b9ab74eb54cfd186867f64f240e1a "$BGE" "" out/bge-base
run BAAI/bge-large-en-v1.5 d4aa6901d3a41ba39fb536a557fa166f842b0e09 "$BGE" "" out/bge-large
run intfloat/e5-small-v2 ffb93f3bd4047442299a41ebb6fa998a38507c52 "query: " "passage: " out/e5-small
run intfloat/e5-base-v2 f52bf8ec8c7124536f0efb74aca902b2995e5bcd "query: " "passage: " out/e5-base
run nomic-ai/nomic-embed-text-v1.5 e9b6763023c676ca8431644204f50c2b100d9aab \
  "search_query: " "search_document: " out/nomic

# OpenAI embeddings through the same script (chat stays on the fakes):
E2E_EMBEDDINGS=openai OPENAI_EMBEDDING_API_KEY=sk-... OPENAI_EMBEDDING_MODEL=text-embedding-3-small \
  E2E_SKIP_BUILD=1 QDRANT_ENDPOINT=http://127.0.0.1:6333 E2E_OUT=out/openai-3-small \
  E2E_COMPARE="$GRID" scripts/e2e.sh
```

`scripts/e2e_compare.py out/bge-small/summary-*.json` prints one model's comparison
table again. The fake-embedding grid and the real-Qdrant tests:

```bash
for cfg in "rrf::" "rrf:1:" "rrf:5:" "rrf:10:" "rrf:60:" "rrf:2:1,1.5" "rrf:2:1,2" \
           "rrf:2:1,3" "rrf:2:1.5,1" "rrf:2:2,1" "rrf:2:3,1" "rrf:10:1,2" "rrf:10:2,1" \
           "dbsf::"; do
  IFS=: read -r f k w <<<"$cfg"
  for s in off on; do
    RAG_FUSION=$f RAG_RRF_K=$k RAG_RRF_WEIGHTS=$w RAG_KEYWORD_STOPWORDS=$s \
      cargo test -p rag-indexer --bin rag-indexer incident_questions_in_memory -- --nocapture
  done
done

for image in qdrant/qdrant:v1.19.1 qdrant/qdrant:v1.18.3 qdrant/qdrant:dev; do
  docker run -d --name qdrant-test -p 6334:6333 "$image"; sleep 3
  export QDRANT_TEST_ENDPOINT=http://localhost:6334
  cargo test -p rag-core --test qdrant_roundtrip -- --ignored
  cargo test -p rag-indexer --bin rag-indexer -- --ignored --exact \
    pipeline_tests::support::qdrant::tests::query_api_matches_real_qdrant \
    pipeline_tests::support::qdrant::tests::dbsf_matches_real_qdrant
  docker rm -f qdrant-test
done
```

In a sandbox that intercepts HTTPS, TEI needs the proxy's CA to download a model, e.g.
`-v /path/to/ca-bundle.crt:/etc/ssl/certs/ca-certificates.crt:ro` on its `docker run`.
