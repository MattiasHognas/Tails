//! End-to-end tests of the indexer's writer against the API's reader.
//!
//! Unit tests on each side check hand-written JSON, so they keep passing when the
//! writer and the reader drift apart. These tests run both for real on one store:
//!
//! Datadog responses (recorded fixtures and a crafted corpus, served by a fake
//! Datadog API) → the real adapters → `index_sources` + `IncrementalSink` (chunking,
//! content hashes, batched embeddings, upserts, cleanup) → Qdrant → the real `/ask`
//! router (planner, `RetrievalScope` filter, search, rerank, prompt, sources).
//!
//! Embeddings and chat come from a deterministic fake OpenAI server
//! ([`support::openai`]). The store is an in-memory fake Qdrant that evaluates the
//! filter subset Tails uses ([`support::qdrant`]), or, in the `*_real_qdrant` variants
//! (ignored by default), the Qdrant at `QDRANT_TEST_ENDPOINT`.
//!
//! - [`contract`]: payload keys, `Kind` values, timestamps, service/environment casing,
//!   chunk grouping, point IDs, bookkeeping keys and citations agree across the pipeline.
//! - [`unicode`]: multibyte text at chunk and excerpt boundaries survives the round trip.
//! - [`quality`]: the incident question set (`tests/incident_questions/`): retrieval
//!   and citation metrics with committed thresholds.

mod contract;
mod quality;
mod support;
mod unicode;
