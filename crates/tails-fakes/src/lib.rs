//! Test fakes for Tails and the incident question set.
//!
//! - [`datadog`]: a fake Datadog API serving a corpus in the shape of real responses
//!   (indexing endpoints with pagination, and the live-evidence queries of `/ask`).
//! - [`openai`]: a deterministic stand-in for the OpenAI API: hashed bag-of-words
//!   embeddings, canned planner replies and an answer model that cites what it is told.
//! - [`questions`]: the incident question set's data model and scoring.
//!
//! The in-process pipeline tests (`rag-indexer`'s `pipeline_tests`, a dev-dependency)
//! mount these on ephemeral wiremock servers. The `tails-fakes` binary serves the same
//! fakes as a standalone process for the end-to-end run of the built binaries, and
//! `tails-e2e` asks the question set through `rag-cli` and scores the answers. See
//! docs/DEVELOPMENT.md#end-to-end-tests.

pub mod datadog;
pub mod openai;
pub mod questions;
pub mod server;
