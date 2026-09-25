//! Error/warning logs grouped by message pattern.
//!
//! During an incident Datadog returns thousands of near-identical error logs. Indexing
//! each as its own document costs an embedding call per log and floods the top-K with
//! copies of one message, so the indexer groups them instead: one document per
//! (service, environment, status, [`normalize_message`] pattern), carrying the count,
//! first and last time seen, a few sample messages and log IDs, and per-minute counts.
//!
//! # Incremental runs
//!
//! Document IDs are stable per pattern ([`LogPattern::doc_id`]), so every run updates
//! the same point. Runs fetch overlapping windows (`INDEXER_OVERLAP_MINUTES`), starting
//! at a whole minute ([`fetch_start`]). A pattern's stored document keeps the per-minute
//! counts of the window that last wrote it; the next run subtracts the stored minutes it
//! fetched again and adds what it counted itself ([`LogPattern::merge_stored`]):
//!
//! `count = stored.count - stored minutes at or after the fetch start + fetched count`
//!
//! Logs in the overlap are therefore counted once, logs that reached Datadog late (inside
//! the overlap) are added, and re-running the same window (for example after a failed
//! checkpoint save) changes nothing. Checkpoints only move forward, so later runs never
//! start before a stored document's minutes. The exception is a checkpoint that moves
//! back (a deleted checkpoint file or a larger overlap): the stored count before the
//! stored minutes cannot be split, so logs between the new fetch start and the stored
//! minutes may be counted twice, unless the fetch reaches back to the pattern's first
//! log, in which case it is recounted from scratch.

use crate::chunk::stable_id;
use crate::domain::{RagDocument, SourceKind};
use crate::text::truncate_bytes;
use chrono::{DateTime, Duration, DurationRound, SecondsFormat, Utc};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, HashSet};

/// Longest pattern, in chars; longer messages are cut and end in `…`.
pub const PATTERN_MAX_CHARS: usize = 160;
/// Most distinct sample messages kept per pattern.
pub const MAX_SAMPLES: usize = 3;
/// Longest sample message, in bytes (cut at a char boundary).
pub const SAMPLE_MAX_BYTES: usize = 4000;
/// Most sample log IDs kept per pattern (the most recent ones).
pub const MAX_SAMPLE_IDS: usize = 5;
/// Most per-minute counts kept per pattern (the most recent ones): a day.
pub const MAX_MINUTE_BUCKETS: usize = 1440;
/// Prefix of pattern document IDs.
pub const DOC_ID_PREFIX: &str = "logpattern_";
/// Characters of the pattern shown in a document title.
const TITLE_PATTERN_CHARS: usize = 80;

/// Metadata keys of a pattern document.
pub mod keys {
    pub const STATUS: &str = "status";
    pub const PATTERN: &str = "pattern";
    pub const COUNT: &str = "count";
    /// RFC 3339; `Timestamp` holds the last time seen. The retrieval filter reads this
    /// key to keep patterns that started before a window and were still seen after it.
    pub const FIRST_SEEN: &str = "first_seen";
    pub const LAST_SEEN: &str = "last_seen";
    pub const SAMPLES: &str = "samples";
    pub const SAMPLE_LOG_IDS: &str = "sample_log_ids";
    pub const MINUTE_COUNTS: &str = "minute_counts";
}

/// Groups similar messages: whitespace is collapsed, UUIDs become `<uuid>`, hex IDs of
/// 8+ chars and alphanumeric IDs of 16+ chars that contain a digit become `<id>`, other
/// ASCII digit runs become `#`, and the result is cut at [`PATTERN_MAX_CHARS`] chars.
/// Non-ASCII text (including non-ASCII digits) is kept as is.
pub fn normalize_message(msg: &str) -> String {
    let chars: Vec<char> = msg
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .collect();
    let mut out = String::new();
    let mut len = 0usize;
    let mut i = 0;
    while i < chars.len() {
        let piece = if chars[i].is_ascii_alphanumeric() {
            if is_uuid_at(&chars, i) {
                i += 36;
                "<uuid>".to_string()
            } else {
                let start = i;
                while i < chars.len() && chars[i].is_ascii_alphanumeric() {
                    i += 1;
                }
                mask_word(&chars[start..i])
            }
        } else {
            i += 1;
            chars[i - 1].to_string()
        };
        let n = piece.chars().count();
        if len + n > PATTERN_MAX_CHARS {
            out.extend(piece.chars().take(PATTERN_MAX_CHARS - len));
            out.push('…');
            return out;
        }
        out.push_str(&piece);
        len += n;
        if len == PATTERN_MAX_CHARS && i < chars.len() {
            out.push('…');
            return out;
        }
    }
    out
}

/// A UUID (`8-4-4-4-12` hex digits) starting at `i` and not followed by more alphanumerics.
fn is_uuid_at(chars: &[char], i: usize) -> bool {
    const GROUPS: [usize; 5] = [8, 4, 4, 4, 12];
    let mut j = i;
    for (g, n) in GROUPS.iter().enumerate() {
        if g > 0 {
            if chars.get(j) != Some(&'-') {
                return false;
            }
            j += 1;
        }
        for _ in 0..*n {
            if !chars.get(j).is_some_and(char::is_ascii_hexdigit) {
                return false;
            }
            j += 1;
        }
    }
    !chars.get(j).is_some_and(char::is_ascii_alphanumeric)
}

/// Masks one run of ASCII alphanumerics.
fn mask_word(word: &[char]) -> String {
    let digits = word.iter().any(char::is_ascii_digit);
    if !digits {
        return word.iter().collect();
    }
    if word.iter().all(char::is_ascii_digit) {
        return "#".into();
    }
    if (word.len() >= 8 && word.iter().all(char::is_ascii_hexdigit)) || word.len() >= 16 {
        return "<id>".into();
    }
    let mut out = String::new();
    let mut prev_digit = false;
    for c in word {
        if c.is_ascii_digit() {
            if !prev_digit {
                out.push('#');
            }
            prev_digit = true;
        } else {
            out.push(*c);
            prev_digit = false;
        }
    }
    out
}

/// Start of the log fetch for a window starting at `from`: the whole minute at or before
/// it, so the per-minute counts of one run line up with the next run's fetch.
pub fn fetch_start(from: DateTime<Utc>) -> DateTime<Utc> {
    from.duration_trunc(Duration::minutes(1)).unwrap_or(from)
}

/// One error/warning log as the grouping needs it.
#[derive(Debug, Clone, PartialEq)]
pub struct LogEvent {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    /// Normalized like the `Service` payload.
    pub service: String,
    /// Normalized like the `Environment` payload.
    pub environment: String,
    pub status: String,
    pub message: String,
}

/// The state of one pattern document.
#[derive(Debug, Clone, PartialEq)]
pub struct LogPattern {
    pub service: String,
    pub environment: String,
    pub status: String,
    pub pattern: String,
    pub count: u64,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
    /// Distinct raw messages (cut to [`SAMPLE_MAX_BYTES`]), oldest first.
    pub samples: Vec<String>,
    /// The most recent log IDs, oldest first.
    pub sample_ids: Vec<String>,
    /// Logs per minute (keyed by the minute's start) of the window that wrote this state.
    pub minute_counts: BTreeMap<DateTime<Utc>, u64>,
}

fn rfc3339(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn parse_time(v: &Value) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(v.as_str()?)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

/// `items` without repeats, keeping first occurrences in order.
fn dedup(items: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = HashSet::new();
    items
        .into_iter()
        .filter(|s| seen.insert(s.clone()))
        .collect()
}

/// The last `n` of `items`.
fn last_n(mut items: Vec<String>, n: usize) -> Vec<String> {
    let drop = items.len().saturating_sub(n);
    items.drain(..drop);
    items
}

/// Groups `events` by (service, environment, status, pattern). Events with an ID seen
/// before are counted once. The result is sorted by document ID.
pub fn group(events: &[LogEvent]) -> Vec<LogPattern> {
    let mut sorted: Vec<&LogEvent> = events.iter().collect();
    sorted.sort_by(|a, b| a.timestamp.cmp(&b.timestamp).then(a.id.cmp(&b.id)));
    let mut seen_ids = HashSet::new();
    let mut groups: BTreeMap<(String, String, String, String), LogPattern> = BTreeMap::new();
    for e in sorted {
        if !e.id.is_empty() && !seen_ids.insert(e.id.as_str()) {
            continue;
        }
        let pattern = normalize_message(&e.message);
        let key = (
            e.service.clone(),
            e.environment.clone(),
            e.status.clone(),
            pattern.clone(),
        );
        let g = groups.entry(key).or_insert_with(|| LogPattern {
            service: e.service.clone(),
            environment: e.environment.clone(),
            status: e.status.clone(),
            pattern,
            count: 0,
            first_seen: e.timestamp,
            last_seen: e.timestamp,
            samples: vec![],
            sample_ids: vec![],
            minute_counts: BTreeMap::new(),
        });
        g.count += 1;
        g.first_seen = g.first_seen.min(e.timestamp);
        g.last_seen = g.last_seen.max(e.timestamp);
        let sample = truncate_bytes(&e.message, SAMPLE_MAX_BYTES);
        if g.samples.len() < MAX_SAMPLES && !g.samples.iter().any(|s| s == sample) {
            g.samples.push(sample.to_string());
        }
        if !e.id.is_empty() {
            g.sample_ids.push(e.id.clone());
            if g.sample_ids.len() > MAX_SAMPLE_IDS {
                g.sample_ids.remove(0);
            }
        }
        *g.minute_counts.entry(fetch_start(e.timestamp)).or_default() += 1;
    }
    let mut out: Vec<LogPattern> = groups
        .into_values()
        .map(|mut g| {
            while g.minute_counts.len() > MAX_MINUTE_BUCKETS {
                g.minute_counts.pop_first();
            }
            g
        })
        .collect();
    out.sort_by_key(LogPattern::doc_id);
    out
}

impl LogPattern {
    /// Stable per (service, environment, status, pattern), so runs update one point.
    pub fn doc_id(&self) -> String {
        format!(
            "{DOC_ID_PREFIX}{}",
            stable_id(&[
                &self.service,
                &self.environment,
                &self.status,
                &self.pattern
            ])
        )
    }

    /// Datadog log search for this pattern's service, environment and status.
    pub fn search_query(&self) -> String {
        [
            ("service", &self.service),
            ("env", &self.environment),
            ("status", &self.status),
        ]
        .iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, v)| format!("{k}:{v}"))
        .collect::<Vec<_>>()
        .join(" ")
    }

    /// The document for this state. `app_base` is the Datadog web app, e.g.
    /// `https://app.datadoghq.eu`; the source links to a log search for the pattern's
    /// service, environment and status from its first to its last log.
    pub fn to_document(&self, app_base: &str) -> RagDocument {
        let short: String = if self.pattern.chars().count() > TITLE_PATTERN_CHARS {
            let mut s: String = self.pattern.chars().take(TITLE_PATTERN_CHARS).collect();
            s.push('…');
            s
        } else {
            self.pattern.clone()
        };
        let title = [self.service.as_str(), self.status.as_str(), short.as_str()]
            .iter()
            .filter(|p| !p.is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join(" - ");
        let status = if self.status.is_empty() {
            String::new()
        } else {
            format!(" {}", self.status)
        };
        let mut text = format!(
            "Pattern: {}\nOccurrences: {}{} log(s), first seen {}, last seen {}\n\nSamples:\n{}",
            self.pattern,
            self.count,
            status,
            rfc3339(self.first_seen),
            rfc3339(self.last_seen),
            self.samples.join("\n\n")
        );
        if !self.sample_ids.is_empty() {
            text.push_str(&format!(
                "\n\nSample log IDs: {}",
                self.sample_ids.join(", ")
            ));
        }
        let source_uri = format!(
            "{app_base}/logs?query={}&from_ts={}&to_ts={}&live=false",
            urlencoding::encode(&self.search_query()),
            self.first_seen.timestamp_millis(),
            self.last_seen.timestamp_millis() + 1,
        );
        let minute_counts: Map<String, Value> = self
            .minute_counts
            .iter()
            .map(|(t, c)| (t.to_rfc3339_opts(SecondsFormat::Secs, true), json!(c)))
            .collect();
        let mut metadata = Map::new();
        metadata.insert(keys::STATUS.into(), json!(self.status));
        metadata.insert(keys::PATTERN.into(), json!(self.pattern));
        metadata.insert(keys::COUNT.into(), json!(self.count));
        metadata.insert(keys::FIRST_SEEN.into(), json!(rfc3339(self.first_seen)));
        metadata.insert(keys::LAST_SEEN.into(), json!(rfc3339(self.last_seen)));
        metadata.insert(keys::SAMPLES.into(), json!(self.samples));
        metadata.insert(keys::SAMPLE_LOG_IDS.into(), json!(self.sample_ids));
        metadata.insert(keys::MINUTE_COUNTS.into(), Value::Object(minute_counts));
        RagDocument {
            id: self.doc_id(),
            title: format!("Log: {title}"),
            text,
            source_uri,
            kind: SourceKind::Logs,
            timestamp: Some(rfc3339(self.last_seen)),
            service: self.service.clone(),
            environment: self.environment.clone(),
            metadata,
        }
    }

    /// Reads the state back from a pattern document's metadata; `None` for anything
    /// else (for example a log point written before grouping).
    pub fn from_metadata(
        service: &str,
        environment: &str,
        md: &Map<String, Value>,
    ) -> Option<Self> {
        let strings = |key: &str| -> Option<Vec<String>> {
            md.get(key)?
                .as_array()?
                .iter()
                .map(|v| v.as_str().map(str::to_string))
                .collect()
        };
        let minute_counts = md
            .get(keys::MINUTE_COUNTS)?
            .as_object()?
            .iter()
            .map(|(t, c)| Some((parse_time(&json!(t))?, c.as_u64()?)))
            .collect::<Option<BTreeMap<_, _>>>()?;
        Some(Self {
            service: service.to_string(),
            environment: environment.to_string(),
            status: md.get(keys::STATUS)?.as_str()?.to_string(),
            pattern: md.get(keys::PATTERN)?.as_str()?.to_string(),
            count: md.get(keys::COUNT)?.as_u64()?,
            first_seen: parse_time(md.get(keys::FIRST_SEEN)?)?,
            last_seen: parse_time(md.get(keys::LAST_SEEN)?)?,
            samples: strings(keys::SAMPLES)?,
            sample_ids: strings(keys::SAMPLE_LOG_IDS)?,
            minute_counts,
        })
    }

    /// Combines this state, counted from a fetch that started at `fetch_start`, with the
    /// `stored` state of the same pattern written by an earlier run (see the module
    /// docs). Logs the stored state counted at or after `fetch_start` were fetched again
    /// and are only counted here.
    pub fn merge_stored(self, stored: &LogPattern, fetch_start: DateTime<Utc>) -> LogPattern {
        let recounted: u64 = stored
            .minute_counts
            .range(fetch_start..)
            .map(|(_, c)| c)
            .sum();
        let before = if fetch_start <= stored.first_seen {
            0
        } else {
            stored.count.saturating_sub(recounted)
        };
        if before == 0 {
            return self;
        }
        LogPattern {
            count: before + self.count,
            first_seen: stored.first_seen.min(self.first_seen),
            last_seen: stored.last_seen.max(self.last_seen),
            samples: dedup(stored.samples.iter().chain(&self.samples).cloned())
                .into_iter()
                .take(MAX_SAMPLES)
                .collect(),
            sample_ids: last_n(
                dedup(stored.sample_ids.iter().chain(&self.sample_ids).cloned()),
                MAX_SAMPLE_IDS,
            ),
            ..self
        }
    }
}

/// The Datadog web app base of a pattern document's source link, e.g.
/// `https://app.datadoghq.eu`.
fn app_base(doc: &RagDocument) -> &str {
    doc.source_uri
        .split_once("/logs?")
        .map_or(doc.source_uri.as_str(), |(base, _)| base)
}

/// `doc` (a freshly fetched pattern document, counted from a fetch that started at
/// `fetch_start`) combined with the metadata of its stored point. Returns `doc`
/// unchanged when either is not a pattern document.
pub fn merge_document(
    doc: RagDocument,
    stored: &Map<String, Value>,
    fetch_start: DateTime<Utc>,
) -> RagDocument {
    let (Some(new), Some(old)) = (
        LogPattern::from_metadata(&doc.service, &doc.environment, &doc.metadata),
        LogPattern::from_metadata(&doc.service, &doc.environment, stored),
    ) else {
        return doc;
    };
    new.merge_stored(&old, fetch_start)
        .to_document(app_base(&doc))
}

#[cfg(test)]
mod tests {
    use super::*;

    const APP: &str = "https://app.datadoghq.eu";

    fn ts(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    fn event(id: &str, at: &str, message: &str) -> LogEvent {
        LogEvent {
            id: id.into(),
            timestamp: ts(at),
            service: "checkout".into(),
            environment: "prod".into(),
            status: "error".into(),
            message: message.into(),
        }
    }

    #[test]
    fn masks_numbers_uuids_and_ids() {
        let cases = [
            (
                "upstream timeout after 1200ms",
                "upstream timeout after #ms",
            ),
            ("pool exhausted: 50/50 in use", "pool exhausted: #/# in use"),
            (
                "order 550e8400-e29b-41d4-a716-446655440000 failed",
                "order <uuid> failed",
            ),
            (
                "user=550E8400-E29B-41D4-A716-446655440000, retry 3",
                "user=<uuid>, retry #",
            ),
            ("trace a1b2c3d4e5f60718 dropped", "trace <id> dropped"),
            ("commit deadbeef is fine", "commit deadbeef is fine"),
            ("req AQAAAYAZdbh47dGwNw failed", "req <id> failed"),
            ("v2 api e2e", "v# api e#e"),
            ("  spaced\n\tout  ", "spaced out"),
            // Not a UUID: a group is too long, so only the digit runs are masked.
            ("550e8400-e29b-41d4-a716-4466554400001", "<id>-e#b-#d#-a#-#"),
        ];
        for (raw, want) in cases {
            assert_eq!(normalize_message(raw), want, "{raw}");
        }
        // Messages differing only in IDs and numbers share a pattern.
        assert_eq!(
            normalize_message(
                "GET /orders/8f14e45fceea167a took 2503ms (key=7c9e6679-7425-40de-944b-e07fc1f90ae7)"
            ),
            normalize_message(
                "GET /orders/c9f0f895fb98ab91 took 17ms (key=16fd2706-8baf-433b-82eb-8c7fada847da)"
            ),
        );
    }

    /// Multibyte text (and non-ASCII digits, which are not masked) survives the cap.
    #[test]
    fn patterns_are_char_safe_for_multibyte_text() {
        assert_eq!(
            normalize_message("Återförsök 3 av 5 misslyckades  för   åsa 🚀 決済 ２"),
            "Återförsök # av # misslyckades för åsa 🚀 決済 ２"
        );
        for pad in 150..=165 {
            let msg = format!("{}決済エラー 42 e\u{0301} 👩\u{200D}💻", "å".repeat(pad));
            let out = normalize_message(&msg);
            assert!(out.chars().count() <= PATTERN_MAX_CHARS + 1, "{pad}: {out}");
            assert!(!out.chars().any(|c| c.is_ascii_digit()), "{pad}: {out}");
            if pad >= PATTERN_MAX_CHARS {
                assert_eq!(out, format!("{}…", "å".repeat(PATTERN_MAX_CHARS)));
            }
        }
        // A placeholder straddling the cap is cut like any other text.
        let msg = format!("{} 550e8400-e29b-41d4-a716-446655440000", "x".repeat(157));
        assert_eq!(normalize_message(&msg), format!("{} <u…", "x".repeat(157)));
        // Exactly at the cap: nothing was cut, so no ellipsis.
        let exact = "ö".repeat(PATTERN_MAX_CHARS);
        assert_eq!(normalize_message(&exact), exact);
    }

    #[test]
    fn groups_by_scope_status_and_pattern() {
        let mut other_env = event("e1", "2026-03-11T10:00:05Z", "timeout after 5ms");
        other_env.environment = "staging".into();
        let mut warn = event("w1", "2026-03-11T10:00:06Z", "timeout after 5ms");
        warn.status = "warn".into();
        let events = [
            event("b", "2026-03-11T10:01:30.500Z", "timeout after 20ms"),
            event("a", "2026-03-11T10:00:01Z", "timeout after 10ms"),
            event("c", "2026-03-11T10:01:59Z", "timeout after 10ms"),
            // A repeat of an ID (pagination overlap) is counted once.
            event("a", "2026-03-11T10:00:01Z", "timeout after 10ms"),
            event("x", "2026-03-11T10:02:00Z", "disk full"),
            other_env,
            warn,
        ];
        let groups = group(&events);
        assert_eq!(groups.len(), 4);
        let g = groups
            .iter()
            .find(|g| {
                g.pattern == "timeout after #ms" && g.status == "error" && g.environment == "prod"
            })
            .unwrap();
        assert_eq!(g.count, 3);
        assert_eq!(g.first_seen, ts("2026-03-11T10:00:01Z"));
        assert_eq!(g.last_seen, ts("2026-03-11T10:01:59Z"));
        assert_eq!(g.samples, ["timeout after 10ms", "timeout after 20ms"]);
        assert_eq!(g.sample_ids, ["a", "b", "c"]);
        assert_eq!(
            g.minute_counts,
            BTreeMap::from([
                (ts("2026-03-11T10:00:00Z"), 1),
                (ts("2026-03-11T10:01:00Z"), 2)
            ])
        );
        // Sorted by (stable) document ID.
        let ids: Vec<String> = groups.iter().map(LogPattern::doc_id).collect();
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(ids, sorted);
    }

    #[test]
    fn document_ids_are_stable_per_pattern() {
        let a = &group(&[event("a", "2026-03-11T10:00:00Z", "timeout after 10ms")])[0];
        let b = &group(&[event("zz", "2026-03-12T18:30:00Z", "timeout after 99999ms")])[0];
        assert_eq!(a.doc_id(), b.doc_id());
        assert!(a.doc_id().starts_with(DOC_ID_PREFIX));
        assert_eq!(a.doc_id().len(), DOC_ID_PREFIX.len() + 32);
        for edit in [
            |e: &mut LogEvent| e.service = "payments".into(),
            |e: &mut LogEvent| e.environment = "staging".into(),
            |e: &mut LogEvent| e.status = "warn".into(),
            |e: &mut LogEvent| e.message = "timeout before 10ms".into(),
        ] {
            let mut e = event("a", "2026-03-11T10:00:00Z", "timeout after 10ms");
            edit(&mut e);
            assert_ne!(group(&[e])[0].doc_id(), a.doc_id());
        }
    }

    #[test]
    fn samples_and_ids_are_bounded_and_utf8_safe() {
        let long = format!("{}決済", "å".repeat(SAMPLE_MAX_BYTES));
        let mut events: Vec<LogEvent> = (0..20)
            .map(|i| {
                event(
                    &format!("id-{i:02}"),
                    &format!("2026-03-11T10:{i:02}:00Z"),
                    &format!("{long} {i}"),
                )
            })
            .collect();
        events.push(event("tail", "2026-03-11T11:00:00Z", "å 7"));
        let groups = group(&events);
        let g = groups.iter().find(|g| g.count == 20).unwrap();
        assert_eq!(g.samples.len(), 1, "cut samples are equal");
        assert!(g.samples[0].len() <= SAMPLE_MAX_BYTES);
        assert!(long.starts_with(&g.samples[0]));
        assert_eq!(g.sample_ids, ["id-15", "id-16", "id-17", "id-18", "id-19"]);
        let doc = g.to_document(APP);
        assert!(doc.text.contains(&g.samples[0]));
    }

    #[test]
    fn document_carries_counts_times_samples_and_a_search_link() {
        let events = [
            event("a", "2026-03-11T10:00:01Z", "timeout after 10ms"),
            event("b", "2026-03-11T10:05:00.250Z", "timeout after 20ms"),
        ];
        let doc = group(&events)[0].to_document(APP);
        assert_eq!(doc.kind, SourceKind::Logs);
        assert_eq!(doc.title, "Log: checkout - error - timeout after #ms");
        assert_eq!(doc.timestamp.as_deref(), Some("2026-03-11T10:05:00.250Z"));
        assert_eq!(
            doc.text,
            "Pattern: timeout after #ms\nOccurrences: 2 error log(s), first seen \
             2026-03-11T10:00:01.000Z, last seen 2026-03-11T10:05:00.250Z\n\nSamples:\n\
             timeout after 10ms\n\ntimeout after 20ms\n\nSample log IDs: a, b"
        );
        assert_eq!(
            doc.source_uri,
            "https://app.datadoghq.eu/logs?query=service%3Acheckout%20env%3Aprod%20status%3Aerror\
             &from_ts=1773223201000&to_ts=1773223500251&live=false"
        );
        assert_eq!(doc.metadata["first_seen"], "2026-03-11T10:00:01.000Z");
        assert_eq!(doc.metadata["count"], 2);
        assert_eq!(
            doc.metadata["minute_counts"],
            json!({"2026-03-11T10:00:00Z": 1, "2026-03-11T10:05:00Z": 1})
        );
        // The state reads back from the metadata unchanged.
        let back = LogPattern::from_metadata("checkout", "prod", &doc.metadata).unwrap();
        assert_eq!(back, group(&events)[0]);
        assert!(LogPattern::from_metadata("checkout", "prod", &Map::new()).is_none());
    }

    /// Run N fetches [10:00, 10:30]; run N+1 re-fetches from 10:20 (the overlap) to 10:45
    /// and also sees a log that reached Datadog late at 10:25.
    #[test]
    fn overlapping_runs_count_every_log_once() {
        let msg = |i: u32| format!("cache miss for key {i}");
        let run_n = [
            event("1", "2026-03-11T10:05:00Z", &msg(1)),
            event("2", "2026-03-11T10:15:00Z", &msg(2)),
            event("3", "2026-03-11T10:21:10Z", &msg(3)),
            event("4", "2026-03-11T10:29:59Z", &msg(4)),
        ];
        let stored = group(&run_n)[0].to_document(APP);

        let start = fetch_start(ts("2026-03-11T10:20:30Z"));
        assert_eq!(start, ts("2026-03-11T10:20:00Z"));
        let run_n1 = [
            event("3", "2026-03-11T10:21:10Z", &msg(3)),
            event("late", "2026-03-11T10:25:00Z", &msg(5)),
            event("4", "2026-03-11T10:29:59Z", &msg(4)),
            event("6", "2026-03-11T10:44:00Z", &msg(6)),
        ];
        let fresh = group(&run_n1)[0].to_document(APP);
        assert_eq!(fresh.id, stored.id, "one point per pattern");
        let merged = merge_document(fresh.clone(), &stored.metadata, start);
        let m = LogPattern::from_metadata("checkout", "prod", &merged.metadata).unwrap();
        assert_eq!(m.count, 6, "1, 2 from run N; 3, late, 4, 6 from run N+1");
        assert_eq!(m.first_seen, ts("2026-03-11T10:05:00Z"));
        assert_eq!(m.last_seen, ts("2026-03-11T10:44:00Z"));
        assert_eq!(
            merged.timestamp.as_deref(),
            Some("2026-03-11T10:44:00.000Z")
        );
        assert_eq!(m.sample_ids, ["2", "3", "4", "late", "6"]);
        assert_eq!(m.samples, [msg(1), msg(2), msg(3)]);
        assert!(merged.text.contains("Occurrences: 6 error log(s)"));
        assert!(merged.source_uri.contains("&from_ts=1773223500000&"));

        // Re-running the same window (e.g. after a failed checkpoint save) is a no-op.
        let again = merge_document(fresh.clone(), &merged.metadata, start);
        assert_eq!(
            serde_json::to_value(&again).unwrap(),
            serde_json::to_value(&merged).unwrap()
        );
        // The next run with nothing new in its window keeps the total.
        let next_start = ts("2026-03-11T10:35:00Z");
        let only_6 = group(&[event("6", "2026-03-11T10:44:00Z", &msg(6))])[0].to_document(APP);
        let next = merge_document(only_6, &merged.metadata, next_start);
        assert_eq!(next.metadata["count"], 6);
        assert_eq!(next.metadata["first_seen"], "2026-03-11T10:05:00.000Z");

        // A fetch reaching back before the first stored log recounts from scratch.
        let full = group(&[run_n.as_slice(), run_n1.as_slice()].concat())[0].to_document(APP);
        let recount = merge_document(full.clone(), &merged.metadata, ts("2026-03-11T10:00:00Z"));
        assert_eq!(recount.metadata["count"], 6);
        assert_eq!(
            serde_json::to_value(&recount).unwrap(),
            serde_json::to_value(&full).unwrap()
        );
    }

    #[test]
    fn non_pattern_documents_are_not_merged() {
        let fresh = group(&[event("a", "2026-03-11T10:00:00Z", "x 1")])[0].to_document(APP);
        let legacy = Map::from_iter([("status".to_string(), json!("error"))]);
        let merged = merge_document(fresh.clone(), &legacy, ts("2026-03-11T10:00:00Z"));
        assert_eq!(merged.text, fresh.text);
    }

    #[test]
    fn minute_counts_are_capped_to_the_most_recent() {
        let start = ts("2026-03-10T00:00:00Z");
        let events: Vec<LogEvent> = (0..MAX_MINUTE_BUCKETS as i64 + 10)
            .map(|i| LogEvent {
                timestamp: start + Duration::minutes(i),
                ..event(&format!("{i}"), "2026-03-10T00:00:00Z", "tick")
            })
            .collect();
        let g = &group(&events)[0];
        assert_eq!(g.count, MAX_MINUTE_BUCKETS as u64 + 10);
        assert_eq!(g.minute_counts.len(), MAX_MINUTE_BUCKETS);
        assert_eq!(
            g.minute_counts.keys().next(),
            Some(&(start + Duration::minutes(10)))
        );
    }
}
