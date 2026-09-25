use crate::domain::{Hit, RagDocument, SourceKind};
use crate::log_patterns::keys::FIRST_SEEN;
use crate::planner::parse_utc;
use crate::retrieval::TIMELESS_KINDS;
use chrono::{DateTime, Utc};
use itertools::Itertools;

/// Recency weight, with a window, of a document whose time is not shown to be in it:
/// [`TIMELESS_KINDS`], undated events, and events far outside the window. An event in
/// the window weighs 1: the user named that time, so being in it is worth 2×, about
/// two ranks of [`crate::qdrant::Qdrant::hybrid_search`]'s fused score (1st in every
/// list scores 1, 3rd 0.5). See [`recency_weight`].
pub const BASELINE_WITH_WINDOW: f32 = 0.5;

/// Recency weight, without a window, of a document with no recent time:
/// [`TIMELESS_KINDS`], undated events, and events much older than
/// [`RECENCY_HALF_LIFE_HOURS`]. An event from just now weighs 1, so recency is worth at
/// most 1.33×: less than one rank at the top of the fused score (2nd in every list
/// scores 0.667 of 1st), so it orders comparable matches but never lifts an event over
/// a clearly better match. See [`recency_weight`].
pub const BASELINE_WITHOUT_WINDOW: f32 = 0.75;

/// Without a window, an event's recency bonus halves every this many hours of age (and,
/// with one, every this many hours outside it).
pub const RECENCY_HALF_LIFE_HOURS: f32 = 24.0;

/// The time a question is about, for [`recency_weight`]: the question's window (either
/// bound may be open) and when it was asked. Passed in, never read from the wall
/// clock, so ranking is reproducible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeFocus {
    pub now: DateTime<Utc>,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
}

impl TimeFocus {
    /// A question without a window, asked at `now`.
    pub fn unbounded(now: DateTime<Utc>) -> Self {
        Self {
            now,
            from: None,
            to: None,
        }
    }

    fn has_window(&self) -> bool {
        self.from.is_some() || self.to.is_some()
    }
}

/// When an event happened: `[Metadata.first_seen, Timestamp]` for a log pattern day
/// (its first and last log), else the instant `Timestamp`. `None` when undated or the
/// timestamp does not parse.
fn event_span(doc: &RagDocument) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
    let end = parse_utc(doc.timestamp.as_deref()?)?;
    let start = doc
        .metadata
        .get(FIRST_SEEN)
        .and_then(|v| v.as_str())
        .and_then(parse_utc)
        .filter(|s| *s <= end)
        .unwrap_or(end);
    Some((start, end))
}

/// How well a document's time fits the time the question is about, in [B, 1] with
/// B = [`BASELINE_WITH_WINDOW`] or [`BASELINE_WITHOUT_WINDOW`]. The reranker multiplies
/// the retrieval score by it.
///
/// Recency is evidence only for events (logs, incidents, change events), whose
/// `Timestamp` is when the thing happened:
/// - [`TIMELESS_KINDS`] (monitors, dashboards, SLOs, metric catalog entries) describe
///   configuration or state; their `Timestamp`, if any, is a creation or last-seen date
///   that says nothing about relevance. They, and undated or unparsable events, weigh B
///   whatever their timestamp.
/// - With a window, an event that overlaps it weighs 1, however long ago the window
///   was: the window already says which time the user cares about. An event outside it
///   decays with its distance `d` from the nearest bound: `B + (1 − B)·2^(−d / 24 h)`.
///   Only candidates the retrieval filter did not restrict can be outside: that filter
///   drops dated events outside the window, and log pattern days that logged nothing in
///   it are dropped before reranking.
/// - Without a window, an event weighs `B + (1 − B)·2^(−age / 24 h)`, age measured from
///   `now` to its end (a pattern day's last log; a future time counts as age 0): just
///   now 1, a day old 0.875, five days 0.758, a week or more ≈ 0.75.
///
/// So recency is a bonus only an event can earn, never a penalty below what a timeless
/// document gets: an old event and a timeless document compare on retrieval score and
/// kind prior alone, so an older event that clearly matches better is not outranked by
/// a monitor or dashboard that matches worse. Age is measured from `now`, not from the
/// newest candidate: relative to the newest candidate, a stale index would give its
/// newest event the full bonus, and a document's weight would depend on which other
/// documents were retrieved.
pub fn recency_weight(doc: &RagDocument, focus: &TimeFocus) -> f32 {
    let baseline = if focus.has_window() {
        BASELINE_WITH_WINDOW
    } else {
        BASELINE_WITHOUT_WINDOW
    };
    if TIMELESS_KINDS.contains(&doc.kind) {
        return baseline;
    }
    let Some((start, end)) = event_span(doc) else {
        return baseline;
    };
    let zero = chrono::Duration::zero();
    let distance = match (focus.from, focus.to) {
        (None, None) => (focus.now - end).max(zero),
        (Some(from), _) if end < from => from - end,
        (_, Some(to)) if start >= to => start - to,
        _ => zero,
    };
    let hours = distance.num_seconds() as f32 / 3600.0;
    baseline + (1.0 - baseline) * 0.5f32.powf(hours / RECENCY_HALF_LIFE_HOURS)
}

/// Kind prior: how likely a document of this kind is to explain an incident.
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
        // Reference data (owners, runbooks, dependencies): neutral, or it would top
        // every question about its service.
        SourceKind::ServiceCatalog => 1.00,
    }
}

/// Rerank retrieved chunks and keep at most `take` sources.
///
/// Each hit's score is multiplied by its kind prior and its [`recency_weight`] for
/// `focus` (the question's window and when it was asked).
///
/// Hits are grouped by the source they are listed as
/// ([`crate::domain::RagDocument::group_id`]): the chunks of one document
/// (`metadata.chunk_of`), and the days of one log pattern (`metadata.pattern_id`).
/// Each group is collapsed to its best hit after the prior and recency weight (ties
/// broken by ID), also when there are fewer candidates than `take`, so a document or
/// pattern is never listed (and numbered as a `[DOC #n]`) twice.
///
/// Scores come from [`crate::qdrant::Qdrant::hybrid_search`]: normalized fused scores in
/// [0, 1] (ranks with the default RRF, [`crate::qdrant::Fusion`]), on the same scale for
/// every question, like the cosine similarities the priors and MMR weights were chosen
/// for.
pub fn rerank_mmr_signals(candidates: &[Hit], take: usize, focus: &TimeFocus) -> Vec<Hit> {
    let adjusted = candidates.iter().cloned().map(|mut h| {
        h.score *= prior(&h.doc.kind) * recency_weight(&h.doc, focus);
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

    /// When the questions in these tests are asked.
    const NOW: &str = "2026-03-12T09:00:00Z";

    fn at(s: &str) -> DateTime<Utc> {
        parse_utc(s).unwrap()
    }

    /// No window, asked at [`NOW`]: undated hits weigh [`BASELINE_WITHOUT_WINDOW`].
    fn no_window() -> TimeFocus {
        TimeFocus::unbounded(at(NOW))
    }

    /// 2 March (a window ten days before [`NOW`]).
    fn march_2() -> TimeFocus {
        TimeFocus {
            now: at(NOW),
            from: Some(at("2026-03-02T00:00:00Z")),
            to: Some(at("2026-03-03T00:00:00Z")),
        }
    }

    fn close(got: f32, want: f32) -> bool {
        (got - want).abs() < 1e-5
    }

    /// An undated hit: its adjusted score is score × prior × baseline.
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

    fn dated(mut h: Hit, ts: &str) -> Hit {
        h.doc.timestamp = Some(ts.into());
        h
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

    fn weight(h: &Hit, focus: &TimeFocus) -> f32 {
        recency_weight(&h.doc, focus)
    }

    #[test]
    fn empty_input_and_take_zero_select_nothing() {
        assert!(rerank_mmr_signals(&[], 5, &no_window()).is_empty());
        let one = [hit("a", "alpha", 0.9, SourceKind::Logs)];
        assert!(rerank_mmr_signals(&one, 0, &no_window()).is_empty());
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
        let out = rerank_mmr_signals(&candidates, 16, &no_window());
        assert_eq!(ids(&out), ["doc1#c1", "doc2#c0"]);
        assert!(close(out[0].score, 0.9 * 0.98 * 0.75));
    }

    /// The days of one log pattern are one source, represented by the day with the best
    /// score after the recency weight (ties broken by ID); another pattern stays separate.
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
        let candidates = vec![
            day("p_2026-01-01", "p", 0.9, "2026-01-01T10:00:00Z"),
            day("p_today", "p", 0.8, "2026-03-12T08:00:00Z"),
            day("p_2026-01-02", "p", 0.9, "2026-01-02T10:00:00Z"),
            day("q_2026-01-01", "q", 0.5, "2026-01-01T10:00:00Z"),
            day("q_2026-01-02", "q", 0.5, "2026-01-02T10:00:00Z"),
        ];
        let out = rerank_mmr_signals(&candidates, 16, &no_window());
        // An hour old, 0.8 weighs 0.989 and beats 0.9 at the 0.75 baseline; equal days
        // pick the lower ID.
        assert_eq!(ids(&out), ["p_today#c0", "q_2026-01-01#c0"]);
        let reversed: Vec<Hit> = candidates.into_iter().rev().collect();
        assert_eq!(
            ids(&rerank_mmr_signals(&reversed, 16, &no_window())),
            ids(&out)
        );
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
        let out = rerank_mmr_signals(&candidates, 7, &no_window());
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
            assert!(close(*got, want * 0.75), "{scores:?}");
        }
    }

    /// Monitors, dashboards, SLOs and metrics weigh the baseline whatever their
    /// timestamp says, with and without a window.
    #[test]
    fn timeless_kinds_are_not_weighted_by_their_timestamp() {
        let stamps = [
            None,
            Some("2026-03-12T09:00:00Z"),
            Some("2026-03-02T12:00:00Z"),
            Some("2020-01-01T00:00:00Z"),
        ];
        for kind in TIMELESS_KINDS {
            for ts in stamps {
                let mut h = hit("t", "t", 1.0, kind.clone());
                h.doc.timestamp = ts.map(String::from);
                assert_eq!(weight(&h, &no_window()), BASELINE_WITHOUT_WINDOW);
                assert_eq!(weight(&h, &march_2()), BASELINE_WITH_WINDOW);
            }
        }
    }

    /// With a window, an event in it weighs 1 however long ago the window was; outside
    /// it, the weight decays with the distance from the nearest bound.
    #[test]
    fn events_in_the_window_weigh_one_whatever_their_age() {
        let w = march_2();
        let event = |ts: &str| dated(hit("e", "e", 1.0, SourceKind::Incident), ts);
        assert_eq!(weight(&event("2026-03-02T00:00:00Z"), &w), 1.0);
        assert_eq!(weight(&event("2026-03-02T23:59:59Z"), &w), 1.0);
        // Half-open: the end bound is outside, a day before `from` is 24 h away.
        assert!(close(weight(&event("2026-03-03T00:00:00Z"), &w), 1.0));
        assert!(close(weight(&event("2026-03-01T00:00:00Z"), &w), 0.75));
        assert!(close(weight(&event("2026-03-04T00:00:00Z"), &w), 0.75));
        assert!(close(weight(&event("2026-02-01T00:00:00Z"), &w), 0.5));
        // Undated or unparsable: nothing shows it is in the window.
        assert_eq!(weight(&hit("u", "u", 1.0, SourceKind::Incident), &w), 0.5);
        assert_eq!(weight(&event("last tuesday"), &w), 0.5);

        // A log pattern day counts from its first log: this one started the evening
        // before the window and logged into it.
        let mut day = dated(hit("d", "d", 1.0, SourceKind::Logs), "2026-03-02T01:00:00Z");
        day.doc
            .metadata
            .insert(FIRST_SEEN.into(), serde_json::json!("2026-03-01T22:00:00Z"));
        assert_eq!(weight(&day, &w), 1.0);
        day.doc.timestamp = Some("2026-03-04T02:00:00Z".into());
        day.doc
            .metadata
            .insert(FIRST_SEEN.into(), serde_json::json!("2026-03-04T00:00:00Z"));
        assert!(close(weight(&day, &w), 0.75));

        // An open bound: everything since the start is in the window.
        let since = TimeFocus { to: None, ..w };
        assert_eq!(weight(&event("2026-03-12T08:00:00Z"), &since), 1.0);
        assert!(close(weight(&event("2026-03-01T00:00:00Z"), &since), 0.75));
        let until = TimeFocus { from: None, ..w };
        assert_eq!(weight(&event("2025-01-01T00:00:00Z"), &until), 1.0);
    }

    /// Without a window: `0.75 + 0.25·2^(−age / 24 h)`, from `now`.
    #[test]
    fn events_without_a_window_decay_towards_the_baseline() {
        let event = |ts: &str| dated(hit("e", "e", 1.0, SourceKind::Logs), ts);
        let w = no_window();
        for (ts, want) in [
            ("2026-03-12T09:00:00Z", 1.0),
            // A future timestamp counts as just now.
            ("2026-03-12T10:00:00Z", 1.0),
            ("2026-03-11T21:00:00Z", 0.75 + 0.25 * 0.5f32.sqrt()),
            ("2026-03-11T09:00:00Z", 0.875),
            ("2026-03-10T09:00:00Z", 0.8125),
            ("2026-03-07T09:00:00Z", 0.757_812_5),
            ("2025-03-12T09:00:00Z", 0.75),
        ] {
            let got = weight(&event(ts), &w);
            assert!(close(got, want), "{ts}: {got} != {want}");
        }
        // A pattern day's age is that of its last log (`Timestamp`).
        let mut day = event("2026-03-12T09:00:00Z");
        day.doc
            .metadata
            .insert(FIRST_SEEN.into(), serde_json::json!("2026-03-11T09:00:00Z"));
        assert_eq!(weight(&day, &w), 1.0);
        // Undated and unparsable events have no recency to reward.
        assert_eq!(weight(&hit("u", "u", 1.0, SourceKind::Logs), &w), 0.75);
        assert_eq!(weight(&event("yesterday"), &w), 0.75);
        // The weight is relative to the given `now`, not the wall clock.
        let later = TimeFocus::unbounded(at("2026-03-13T09:00:00Z"));
        assert!(close(weight(&event("2026-03-12T09:00:00Z"), &later), 0.875));
    }

    /// Regression (q42): an incident a week old, first in every search (fused 1.0), and
    /// a monitor second in every search (0.667). When dated evidence was halved and
    /// undated was not, the monitor ranked first (0.70 against 0.55).
    #[test]
    fn older_event_that_matches_better_beats_a_weaker_timeless_document() {
        let incident = dated(
            hit("incident", "export paused", 1.0, SourceKind::Incident),
            "2026-03-05T02:10:00Z",
        );
        let monitor = hit("monitor", "replica lag", 2.0 / 3.0, SourceKind::Monitor);
        let dashboard = dated(
            hit("dashboard", "replication", 2.0 / 3.0, SourceKind::Dashboard),
            "2026-03-12T08:00:00Z",
        );
        let out = rerank_mmr_signals(&[monitor, dashboard, incident], 3, &no_window());
        assert_eq!(ids(&out), ["incident", "monitor", "dashboard"]);
        let week = 0.75 + 0.25 * 0.5f32.powf((7.0 * 24.0 + 6.0 + 50.0 / 60.0) / 24.0);
        assert!(close(out[0].score, 1.10 * week));
        assert!(close(out[1].score, 2.0 / 3.0 * 1.05 * 0.75));
        // The dashboard's fresh timestamp is its creation date: no bonus.
        assert!(close(out[2].score, 2.0 / 3.0 * 0.75));
    }

    /// Without a window, recency orders comparable matches but does not overturn a clear
    /// difference in rank: at most 1.33×, less than 1st vs 2nd in every search (1.5×).
    #[test]
    fn recency_breaks_near_ties_but_not_clear_rank_differences() {
        let log = |id: &str, score: f32, ts: &str| dated(hit(id, id, score, SourceKind::Logs), ts);
        let equal = [
            log("a_five_days_ago", 0.833, "2026-03-07T09:00:00Z"),
            log("b_today", 0.833, "2026-03-12T08:40:00Z"),
        ];
        let out = rerank_mmr_signals(&equal, 2, &no_window());
        assert_eq!(ids(&out), ["b_today", "a_five_days_ago"]);

        let apart = [
            log("a_stale_first", 1.0, "2026-02-24T08:30:00Z"),
            log("b_fresh_second", 2.0 / 3.0, "2026-03-12T07:10:00Z"),
        ];
        let out = rerank_mmr_signals(&apart, 2, &no_window());
        assert_eq!(ids(&out), ["a_stale_first", "b_fresh_second"]);
    }

    /// Regression (q09): with a window, an event in it outranks a timeless document that
    /// matched the words better but says nothing about that day; the old decay from the
    /// wall clock also left dated in-window evidence at half weight.
    #[test]
    fn event_in_the_window_beats_a_better_matching_timeless_document() {
        let incident = dated(
            hit(
                "incident",
                "errors after deploy",
                0.583,
                SourceKind::Incident,
            ),
            "2026-03-02T08:00:00Z",
        );
        let dashboard = dated(
            hit("dashboard", "checkout overview", 1.0, SourceKind::Dashboard),
            "2025-06-01T10:00:00Z",
        );
        let slo = hit("slo", "checkout availability", 0.583, SourceKind::SLO);
        let out = rerank_mmr_signals(&[dashboard, slo, incident], 3, &march_2());
        assert_eq!(ids(&out), ["incident", "dashboard", "slo"]);
        assert!(close(out[0].score, 0.583 * 1.10));
        assert!(close(out[1].score, 0.5));
        assert!(close(out[2].score, 0.583 * 1.03 * 0.5));
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
        let out = rerank_mmr_signals(&candidates, 2, &no_window());
        // Undated logs: score × 0.98 × 0.75.
        // b: 0.75·0.654 + 0.25·(1 − 1) = 0.491; c: 0.75·0.588 + 0.25·1 = 0.691.
        assert_eq!(ids(&out), ["a", "c"]);
        // With room for all three, the duplicate comes last.
        assert_eq!(
            ids(&rerank_mmr_signals(&candidates, 3, &no_window())),
            ["a", "c", "b"]
        );
    }

    #[test]
    fn relevance_still_wins_over_small_diversity_gains() {
        let candidates = vec![
            hit("1", "alpha beta", 0.9, SourceKind::Monitor),
            hit("2", "gamma delta", 0.8, SourceKind::Monitor),
            hit("3", "epsilon zeta", 0.2, SourceKind::Monitor),
        ];
        let out = rerank_mmr_signals(&candidates, 2, &no_window());
        assert_eq!(ids(&out), ["1", "2"]);
        // Fields other than the score pass through untouched.
        assert_eq!(out[0].doc.title, "Title 1");
        assert_eq!(out[0].doc.source_uri, "http://example.com/1");
    }
}
