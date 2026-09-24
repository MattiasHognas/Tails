//! Checks that the citations in a generated answer resolve.
//!
//! The answer model is told to cite indexed documents as `[DOC #n]`, numbered like
//! the prompt and the `sources` list, and live observations by ID (`[obs-N]`).
//! Models sometimes cite a number that was never in the prompt or an observation
//! that does not exist. [`validate_citations`] finds every citation in an answer
//! and reports those that do not resolve, so callers can flag them instead of
//! showing a reader a link to nothing.

use serde::Serialize;
use std::collections::BTreeSet;

/// Why a citation could not be resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CitationProblem {
    /// `[DOC #n]` where `n` is 0 or larger than the number of sources.
    UnknownDocument,
    /// `obs-N` that is not an observation in the timeline.
    UnknownObservation,
}

/// One citation in the answer that does not resolve.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CitationWarning {
    /// The citation as normalized text, e.g. `DOC #7` or `obs-12`.
    pub citation: String,
    pub reason: CitationProblem,
}

/// Every citation found in an answer, and the ones that do not resolve.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CitationReport {
    /// Distinct `[DOC #n]` numbers cited, ascending.
    pub documents: Vec<usize>,
    /// Distinct observation IDs cited (`obs-N`), in order of first appearance.
    pub observations: Vec<String>,
    /// Citations that resolve to no source or observation, in order of appearance.
    pub warnings: Vec<CitationWarning>,
}

impl CitationReport {
    /// Total distinct citations (documents and observations).
    pub fn cited(&self) -> usize {
        self.documents.len() + self.observations.len()
    }

    /// Sources (1-based numbers up to `source_count`) that the answer never cites.
    pub fn uncited_sources(&self, source_count: usize) -> Vec<usize> {
        (1..=source_count)
            .filter(|n| !self.documents.contains(n))
            .collect()
    }
}

/// Finds `DOC #n` citations (bracketed like `[DOC #2]`, grouped like
/// `[DOC #1, DOC #3]`, or bare) and observation IDs `obs-N` in `answer`, and checks
/// them against `source_count` (sources are numbered `1..=source_count`) and the
/// timeline's `observation_ids`.
pub fn validate_citations<S: AsRef<str>>(
    answer: &str,
    source_count: usize,
    observation_ids: &[S],
) -> CitationReport {
    let mut documents = BTreeSet::new();
    let mut observations: Vec<String> = Vec::new();
    let mut warnings: Vec<CitationWarning> = Vec::new();
    let mut warn = |citation: String, reason| {
        if !warnings.iter().any(|w| w.citation == citation) {
            warnings.push(CitationWarning { citation, reason });
        }
    };

    for (pos, _) in answer.match_indices("DOC #") {
        let digits: String = answer[pos + "DOC #".len()..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        let Ok(n) = digits.parse::<usize>() else {
            continue;
        };
        if (1..=source_count).contains(&n) {
            documents.insert(n);
        } else {
            warn(format!("DOC #{n}"), CitationProblem::UnknownDocument);
        }
    }

    for (pos, _) in answer.match_indices("obs-") {
        // Only a standalone ID: not the tail of a longer word such as `jobs-3`.
        if answer[..pos]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '-')
        {
            continue;
        }
        let digits: String = answer[pos + "obs-".len()..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        if digits.is_empty() {
            continue;
        }
        let id = format!("obs-{digits}");
        if observation_ids.iter().any(|o| o.as_ref() == id) {
            if !observations.contains(&id) {
                observations.push(id);
            }
        } else {
            warn(id, CitationProblem::UnknownObservation);
        }
    }

    CitationReport {
        documents: documents.into_iter().collect(),
        observations,
        warnings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OBS: [&str; 3] = ["obs-1", "obs-2", "obs-10"];

    #[test]
    fn resolves_bracketed_grouped_and_bare_document_citations() {
        let answer = "Latency spiked [DOC #2]. See [DOC #1, DOC #3] and DOC #2 again.";
        let r = validate_citations(answer, 3, &OBS);
        assert_eq!(r.documents, vec![1, 2, 3]);
        assert!(r.warnings.is_empty());
        assert_eq!(r.uncited_sources(4), vec![4]);
    }

    #[test]
    fn flags_documents_outside_the_sources_list() {
        let answer = "Per [DOC #0] and [DOC #4] and [DOC #4] (twice), plus [DOC #1].";
        let r = validate_citations(answer, 3, &OBS);
        assert_eq!(r.documents, vec![1]);
        assert_eq!(
            r.warnings,
            vec![
                CitationWarning {
                    citation: "DOC #0".into(),
                    reason: CitationProblem::UnknownDocument
                },
                CitationWarning {
                    citation: "DOC #4".into(),
                    reason: CitationProblem::UnknownDocument
                },
            ]
        );
        // With no sources at all, every document citation is unknown.
        assert_eq!(validate_citations("[DOC #1]", 0, &OBS).warnings.len(), 1);
    }

    #[test]
    fn resolves_observations_and_flags_unknown_ones() {
        let answer = "Observed: p95 spike [obs-2], burst [obs-10]; hypothesis [obs-1, obs-7]. \
                      Not a citation: jobs-3, obs-, prod-obs-2.";
        let r = validate_citations(answer, 0, &OBS);
        assert_eq!(r.observations, vec!["obs-2", "obs-10", "obs-1"]);
        assert_eq!(
            r.warnings,
            vec![CitationWarning {
                citation: "obs-7".into(),
                reason: CitationProblem::UnknownObservation
            }]
        );
        assert_eq!(r.cited(), 3);
    }

    #[test]
    fn multibyte_text_around_citations_is_handled() {
        let answer = "Betalningar misslyckades 💳 [DOC #1]；決済エラー[obs-1]。Åter DOC #9é";
        let r = validate_citations(answer, 1, &OBS);
        assert_eq!(r.documents, vec![1]);
        assert_eq!(r.observations, vec!["obs-1"]);
        assert_eq!(r.warnings[0].citation, "DOC #9");
    }
}
