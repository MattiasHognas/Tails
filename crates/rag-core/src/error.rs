//! Typed errors for the RAG pipeline.
//!
//! Infrastructure failures (embedding, retrieval, generation, planning) must
//! surface as errors, never as empty vectors or empty result sets that the LLM
//! would then turn into a confident "nothing found" answer.

use serde::Serialize;
use std::fmt;
use std::time::Duration;

/// Pipeline stage a failure happened in. Serialized in API error bodies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Planning,
    Embedding,
    Retrieval,
    Generation,
    Indexing,
    /// Live Datadog queries for diagnostic questions. Failures here are
    /// reported as missing evidence in the timeline, not as request errors.
    LiveEvidence,
    /// The overall per-request deadline, not attributable to one stage.
    Request,
}

impl Stage {
    pub fn as_str(self) -> &'static str {
        match self {
            Stage::Planning => "planning",
            Stage::Embedding => "embedding",
            Stage::Retrieval => "retrieval",
            Stage::Generation => "generation",
            Stage::Indexing => "indexing",
            Stage::LiveEvidence => "live_evidence",
            Stage::Request => "request",
        }
    }
}

impl fmt::Display for Stage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A failed call to an upstream HTTP dependency (OpenAI, Qdrant, Datadog).
///
/// `Display` never contains upstream response bodies or request URLs, so it is
/// safe to return to API clients; details are logged server-side instead.
#[derive(Debug, thiserror::Error)]
pub enum UpstreamError {
    #[error("could not connect to upstream")]
    Connect(#[source] reqwest::Error),
    #[error("upstream request timed out")]
    Timeout,
    #[error("upstream returned HTTP {status}")]
    Status {
        status: u16,
        retry_after: Option<Duration>,
    },
    #[error("upstream transport error")]
    Transport(#[source] reqwest::Error),
    #[error("invalid upstream response: {0}")]
    InvalidResponse(String),
}

impl UpstreamError {
    /// Classify a reqwest error, stripping the URL so it never reaches clients.
    pub fn from_reqwest(e: reqwest::Error) -> Self {
        if e.is_timeout() {
            UpstreamError::Timeout
        } else if e.is_connect() {
            UpstreamError::Connect(e.without_url())
        } else if e.is_decode() {
            UpstreamError::InvalidResponse("could not decode response body".into())
        } else {
            UpstreamError::Transport(e.without_url())
        }
    }

    /// Transient failures are worth retrying: connect errors, timeouts,
    /// transport errors, HTTP 429 and 5xx. Other 4xx and malformed responses
    /// are not.
    pub fn is_transient(&self) -> bool {
        match self {
            UpstreamError::Connect(_) | UpstreamError::Timeout | UpstreamError::Transport(_) => {
                true
            }
            UpstreamError::Status { status, .. } => *status == 429 || *status >= 500,
            UpstreamError::InvalidResponse(_) => false,
        }
    }

    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            UpstreamError::Status { retry_after, .. } => *retry_after,
            _ => None,
        }
    }
}

/// Outcome of a (possibly retried) upstream call that did not succeed.
#[derive(Debug)]
pub struct UpstreamFailure {
    pub error: UpstreamError,
    /// Number of attempts made (>= 1).
    pub attempts: u32,
    /// True when the error was transient but no further attempt was allowed
    /// (attempt budget or deadline exhausted).
    pub exhausted: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum RagError {
    #[error("embedding failed: {source}")]
    EmbeddingFailed {
        #[source]
        source: UpstreamError,
    },
    #[error("retrieval failed: {source}")]
    RetrievalFailed {
        #[source]
        source: UpstreamError,
    },
    #[error("answer generation failed: {source}")]
    GenerationFailed {
        #[source]
        source: UpstreamError,
    },
    #[error("query planning failed: {source}")]
    PlanningFailed {
        #[source]
        source: UpstreamError,
    },
    #[error("indexing failed: {source}")]
    IndexingFailed {
        #[source]
        source: UpstreamError,
    },
    #[error("live evidence query failed: {source}")]
    LiveEvidenceFailed {
        #[source]
        source: UpstreamError,
    },
    #[error("{stage} timed out")]
    Timeout { stage: Stage },
    #[error("{stage} upstream unavailable after {attempts} attempt(s): {source}")]
    UpstreamUnavailable {
        stage: Stage,
        attempts: u32,
        #[source]
        source: UpstreamError,
    },
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    #[error("internal error: {0}")]
    Internal(String),
}

impl RagError {
    /// Build the stage-specific error for an upstream failure.
    pub fn upstream(stage: Stage, failure: UpstreamFailure) -> Self {
        let UpstreamFailure {
            error,
            attempts,
            exhausted,
        } = failure;
        if matches!(error, UpstreamError::Timeout) {
            return RagError::Timeout { stage };
        }
        if exhausted {
            return RagError::UpstreamUnavailable {
                stage,
                attempts,
                source: error,
            };
        }
        Self::failed(stage, error)
    }

    /// Stage-specific "failed" variant for a non-retried upstream error.
    pub fn failed(stage: Stage, source: UpstreamError) -> Self {
        match stage {
            Stage::Embedding => RagError::EmbeddingFailed { source },
            Stage::Retrieval => RagError::RetrievalFailed { source },
            Stage::Generation => RagError::GenerationFailed { source },
            Stage::Planning => RagError::PlanningFailed { source },
            Stage::Indexing => RagError::IndexingFailed { source },
            Stage::LiveEvidence => RagError::LiveEvidenceFailed { source },
            Stage::Request => RagError::Internal(source.to_string()),
        }
    }

    /// Stable machine-readable code for API error bodies.
    pub fn code(&self) -> &'static str {
        match self {
            RagError::EmbeddingFailed { .. } => "embedding_failed",
            RagError::RetrievalFailed { .. } => "retrieval_failed",
            RagError::GenerationFailed { .. } => "generation_failed",
            RagError::PlanningFailed { .. } => "planning_failed",
            RagError::IndexingFailed { .. } => "indexing_failed",
            RagError::LiveEvidenceFailed { .. } => "live_evidence_failed",
            RagError::Timeout { .. } => "timeout",
            RagError::UpstreamUnavailable { .. } => "upstream_unavailable",
            RagError::InvalidRequest(_) => "invalid_request",
            RagError::Internal(_) => "internal",
        }
    }

    pub fn stage(&self) -> Option<Stage> {
        match self {
            RagError::EmbeddingFailed { .. } => Some(Stage::Embedding),
            RagError::RetrievalFailed { .. } => Some(Stage::Retrieval),
            RagError::GenerationFailed { .. } => Some(Stage::Generation),
            RagError::PlanningFailed { .. } => Some(Stage::Planning),
            RagError::IndexingFailed { .. } => Some(Stage::Indexing),
            RagError::LiveEvidenceFailed { .. } => Some(Stage::LiveEvidence),
            RagError::Timeout { stage } | RagError::UpstreamUnavailable { stage, .. } => {
                Some(*stage)
            }
            RagError::InvalidRequest(_) | RagError::Internal(_) => None,
        }
    }

    /// Whether the client may reasonably retry the same request later.
    pub fn retryable(&self) -> bool {
        match self {
            RagError::EmbeddingFailed { source }
            | RagError::RetrievalFailed { source }
            | RagError::GenerationFailed { source }
            | RagError::PlanningFailed { source }
            | RagError::IndexingFailed { source }
            | RagError::LiveEvidenceFailed { source } => source.is_transient(),
            RagError::Timeout { .. } | RagError::UpstreamUnavailable { .. } => true,
            RagError::InvalidRequest(_) | RagError::Internal(_) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(code: u16) -> UpstreamError {
        UpstreamError::Status {
            status: code,
            retry_after: None,
        }
    }

    #[test]
    fn transient_classification() {
        assert!(status(429).is_transient());
        assert!(status(500).is_transient());
        assert!(status(503).is_transient());
        assert!(!status(400).is_transient());
        assert!(!status(401).is_transient());
        assert!(!status(404).is_transient());
        assert!(UpstreamError::Timeout.is_transient());
        assert!(!UpstreamError::InvalidResponse("x".into()).is_transient());
    }

    #[test]
    fn upstream_failure_mapping() {
        let e = RagError::upstream(
            Stage::Embedding,
            UpstreamFailure {
                error: status(500),
                attempts: 3,
                exhausted: true,
            },
        );
        assert_eq!(e.code(), "upstream_unavailable");
        assert_eq!(e.stage(), Some(Stage::Embedding));
        assert!(e.retryable());

        let e = RagError::upstream(
            Stage::Retrieval,
            UpstreamFailure {
                error: status(400),
                attempts: 1,
                exhausted: false,
            },
        );
        assert_eq!(e.code(), "retrieval_failed");
        assert!(!e.retryable());

        let e = RagError::upstream(
            Stage::Generation,
            UpstreamFailure {
                error: UpstreamError::Timeout,
                attempts: 2,
                exhausted: true,
            },
        );
        assert_eq!(e.code(), "timeout");
        assert_eq!(e.stage(), Some(Stage::Generation));
    }

    #[test]
    fn display_does_not_include_bodies() {
        let e = RagError::failed(Stage::Retrieval, status(401));
        assert_eq!(
            e.to_string(),
            "retrieval failed: upstream returned HTTP 401"
        );
    }
}
