use crate::domain::{Hit, SourceKind};
use itertools::Itertools;

/// Rerank retrieved chunks and keep at most `take` sources.
///
/// Hits are grouped by the source they are listed as
/// ([`crate::domain::RagDocument::group_id`]): the chunks of one document
/// (`metadata.chunk_of`), and the days of one log pattern (`metadata.pattern_id`).
/// Each group is collapsed to its best hit after the kind prior and recency decay
/// (ties broken by ID), also when there are fewer candidates than `take`, so a
/// document or pattern is never listed (and numbered as a `[DOC #n]`) twice.
pub fn rerank_mmr_signals(candidates: &[Hit], take: usize) -> Vec<Hit> {
    // Adjust score by priors + simple recency decay (if timestamp present)
    fn prior(kind: &SourceKind) -> f32 {
        match kind {
            SourceKind::Incident => 1.10,
            SourceKind::Monitor => 1.05,
            SourceKind::SLO => 1.03,
            SourceKind::Dashboard => 1.00,
            SourceKind::Metrics => 1.00,
            SourceKind::Logs => 0.98,
            SourceKind::Git => 1.0,
            // A deploy or config change is a frequent root-cause lead, so slightly above
            // neutral; below incidents and monitors, which state the problem itself.
            SourceKind::Change => 1.02,
            // Reference data (owners, runbooks, dependencies). Undated, so never
            // decayed: neutral, or it would top every question about its service.
            SourceKind::ServiceCatalog => 1.00,
        }
    }
    let now = time::OffsetDateTime::now_utc();
    let adjusted = candidates.iter().cloned().map(|mut h| {
        let mut adj = h.score * prior(&h.doc.kind);
        if let Some(ts) = &h.doc.timestamp
            && let Ok(t) =
                time::OffsetDateTime::parse(ts, &time::format_description::well_known::Rfc3339)
        {
            let age = (now - t).abs().whole_seconds() as f32;
            let half_life = 24.0 * 3600.0; // 24h
            let decay = (0.5f32).powf(age / half_life);
            adj *= decay.max(0.5);
        }
        h.score = adj;
        h
    });
    // Ties (and group order, which comes from a hash map) are broken by ID so the
    // result is deterministic.
    let best_first = |a: &Hit, b: &Hit| b.score.total_cmp(&a.score).then(a.doc.id.cmp(&b.doc.id));
    let mut by_parent: Vec<Hit> = adjusted
        .into_group_map_by(|h| h.doc.group_id().to_string())
        .into_values()
        .map(|mut v| {
            v.sort_by(best_first);
            v.swap_remove(0)
        })
        .collect();
    by_parent.sort_by(best_first);

    // Greedy MMR with token Jaccard-ish on text
    let mut selected: Vec<Hit> = Vec::new();
    let mut remaining = by_parent;
    let lambda = 0.75f32;

    fn sim(a: &str, b: &str) -> f32 {
        let ta: std::collections::HashSet<_> =
            a.split_whitespace().map(|s| s.to_lowercase()).collect();
        let tb: std::collections::HashSet<_> =
            b.split_whitespace().map(|s| s.to_lowercase()).collect();
        let inter = ta.intersection(&tb).count() as f32;
        let denom = ((ta.len() * tb.len()) as f32).sqrt().max(1.0);
        inter / denom
    }

    while !remaining.is_empty() && selected.len() < take {
        let mut best_idx = 0usize;
        let mut best_val = f32::NEG_INFINITY;
        for (i, cand) in remaining.iter().enumerate() {
            let max_sim = selected
                .iter()
                .map(|s| sim(&cand.doc.text, &s.doc.text))
                .fold(0.0f32, f32::max);
            let val = lambda * cand.score + (1.0 - lambda) * (1.0 - max_sim);
            if val > best_val {
                best_val = val;
                best_idx = i;
            }
        }
        selected.push(remaining.remove(best_idx));
    }
    selected
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{RagDocument, SourceKind};

    /// An undated hit (no recency decay), so its adjusted score is score × prior.
    fn hit(id: &str, text: &str, score: f32, kind: SourceKind) -> Hit {
        Hit {
            doc: RagDocument {
                id: id.to_string(),
                title: format!("Title {id}"),
                text: text.to_string(),
                source_uri: format!("http://example.com/{id}"),
                kind,
                timestamp: None,
                service: "test-service".to_string(),
                environment: "test".to_string(),
                metadata: serde_json::Map::new(),
            },
            score,
        }
    }

    fn chunk_of(mut h: Hit, parent: &str) -> Hit {
        h.doc
            .metadata
            .insert("chunk_of".into(), serde_json::json!(parent));
        h
    }

    fn ids(hits: &[Hit]) -> Vec<&str> {
        hits.iter().map(|h| h.doc.id.as_str()).collect()
    }

    #[test]
    fn empty_input_and_take_zero_select_nothing() {
        assert!(rerank_mmr_signals(&[], 5).is_empty());
        let one = [hit("a", "alpha", 0.9, SourceKind::Logs)];
        assert!(rerank_mmr_signals(&one, 0).is_empty());
    }

    /// Regression: with fewer candidates than `take`, chunks of one document used to
    /// be returned unchanged, so `sources` listed the same document several times.
    #[test]
    fn chunks_of_one_document_are_collapsed_even_below_take() {
        let candidates = vec![
            chunk_of(hit("doc1#c0", "first part", 0.7, SourceKind::Logs), "doc1"),
            chunk_of(hit("doc1#c1", "second part", 0.9, SourceKind::Logs), "doc1"),
            chunk_of(hit("doc1#c2", "third part", 0.8, SourceKind::Logs), "doc1"),
            chunk_of(
                hit("doc2#c0", "other document", 0.6, SourceKind::Logs),
                "doc2",
            ),
        ];
        let out = rerank_mmr_signals(&candidates, 16);
        assert_eq!(ids(&out), ["doc1#c1", "doc2#c0"]);
        assert!((out[0].score - 0.9 * 0.98).abs() < 1e-6);
    }

    /// The days of one log pattern are one source, represented by the day with the best
    /// score after recency decay (ties broken by ID); another pattern stays separate.
    #[test]
    fn days_of_one_log_pattern_are_one_source() {
        let day = |id: &str, pattern: &str, score: f32, ts: &str| {
            let mut h = chunk_of(hit(&format!("{id}#c0"), id, score, SourceKind::Logs), id);
            h.doc.timestamp = Some(ts.into());
            h.doc
                .metadata
                .insert("pattern_id".into(), serde_json::json!(pattern));
            h
        };
        let now = chrono::Utc::now();
        let recent = (now - chrono::Duration::hours(1)).to_rfc3339();
        let candidates = vec![
            day("p_2020-01-01", "p", 0.9, "2020-01-01T10:00:00Z"),
            day("p_today", "p", 0.8, &recent),
            day("p_2020-01-02", "p", 0.9, "2020-01-02T10:00:00Z"),
            day("q_2020-01-01", "q", 0.5, "2020-01-01T10:00:00Z"),
            day("q_2020-01-02", "q", 0.5, "2020-01-02T10:00:00Z"),
        ];
        let out = rerank_mmr_signals(&candidates, 16);
        // 0.8 barely decayed beats 0.9 at the 0.5 floor; equal days pick the lower ID.
        assert_eq!(ids(&out), ["p_today#c0", "q_2020-01-01#c0"]);
        let reversed: Vec<Hit> = candidates.into_iter().rev().collect();
        assert_eq!(ids(&rerank_mmr_signals(&reversed, 16)), ids(&out));
    }

    #[test]
    fn source_kind_priors_order_equally_scored_hits() {
        let candidates = vec![
            hit("logs", "l l", 1.0, SourceKind::Logs),
            hit("dashboard", "d d", 1.0, SourceKind::Dashboard),
            hit("slo", "s s", 1.0, SourceKind::SLO),
            hit("incident", "i i", 1.0, SourceKind::Incident),
            hit("monitor", "m m", 1.0, SourceKind::Monitor),
            hit("change", "c c", 1.0, SourceKind::Change),
            hit("catalog", "k k", 1.0, SourceKind::ServiceCatalog),
        ];
        let out = rerank_mmr_signals(&candidates, 7);
        assert_eq!(
            ids(&out),
            [
                "incident",
                "monitor",
                "slo",
                "change",
                "catalog",
                "dashboard",
                "logs"
            ]
        );
        let scores: Vec<f32> = out.iter().map(|h| h.score).collect();
        for (got, want) in scores
            .iter()
            .zip([1.10, 1.05, 1.03, 1.02, 1.00, 1.00, 0.98])
        {
            assert!((got - want).abs() < 1e-6, "{scores:?}");
        }
    }

    #[test]
    fn recency_decay_halves_old_documents_at_most() {
        let mut old = hit("old", "old text", 1.0, SourceKind::Monitor);
        old.doc.timestamp = Some("2020-01-01T00:00:00Z".into());
        let mut fresh = hit("fresh", "fresh text", 0.6, SourceKind::Monitor);
        fresh.doc.timestamp = Some(chrono::Utc::now().to_rfc3339());
        let mut unparsable = hit("unparsable", "other words", 0.55, SourceKind::Monitor);
        unparsable.doc.timestamp = Some("yesterday".into());

        let out = rerank_mmr_signals(&[old, fresh, unparsable], 3);
        assert_eq!(ids(&out), ["fresh", "unparsable", "old"]);
        // Years old: the decay floor of 0.5 applies.
        assert!((out[2].score - 1.05 * 0.5).abs() < 1e-6);
        // Just now: practically no decay.
        assert!((out[0].score - 0.6 * 1.05).abs() < 1e-3);
        // An unparsable timestamp is not decayed.
        assert!((out[1].score - 0.55 * 1.05).abs() < 1e-6);
    }

    #[test]
    fn mmr_skips_near_duplicates_in_favour_of_diverse_evidence() {
        let candidates = vec![
            hit(
                "a",
                "error service auth-api production",
                0.90,
                SourceKind::Logs,
            ),
            // Same words, different case: similarity is case-insensitive.
            hit(
                "b",
                "ERROR Service Auth-API Production",
                0.89,
                SourceKind::Logs,
            ),
            hit(
                "c",
                "completely different unique text",
                0.80,
                SourceKind::Logs,
            ),
        ];
        let out = rerank_mmr_signals(&candidates, 2);
        // b: 0.75·0.872 + 0.25·(1 − 1) = 0.654; c: 0.75·0.784 + 0.25·1 = 0.838.
        assert_eq!(ids(&out), ["a", "c"]);
        // With room for all three, the duplicate comes last.
        assert_eq!(ids(&rerank_mmr_signals(&candidates, 3)), ["a", "c", "b"]);
    }

    #[test]
    fn relevance_still_wins_over_small_diversity_gains() {
        let candidates = vec![
            hit("1", "alpha beta", 0.9, SourceKind::Monitor),
            hit("2", "gamma delta", 0.8, SourceKind::Monitor),
            hit("3", "epsilon zeta", 0.2, SourceKind::Monitor),
        ];
        let out = rerank_mmr_signals(&candidates, 2);
        assert_eq!(ids(&out), ["1", "2"]);
        // Fields other than the score pass through untouched.
        assert_eq!(out[0].doc.title, "Title 1");
        assert_eq!(out[0].doc.source_uri, "http://example.com/1");
    }
}
