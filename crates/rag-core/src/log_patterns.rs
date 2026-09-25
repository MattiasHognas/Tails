//! Error/warning logs grouped by message pattern and UTC day.
//!
//! During an incident Datadog returns thousands of near-identical error logs. Indexing
//! each as its own document costs an embedding call per log and floods the top-K with
//! copies of one message, so the indexer groups them instead: one document per
//! (service, environment, status, [`normalize_message`] pattern, UTC day), carrying that
//! day's count, first and last log, a few sample messages and log IDs, and logs per UTC
//! hour. Per-day documents keep a pattern logged on Monday and Friday out of a question
//! about Wednesday, and let the answer count exactly the hours of the asked window
//! ([`occurrences`]). The days of one pattern share [`LogPattern::pattern_id`], so the
//! answer lists them as one source.
//!
//! # Incremental runs
//!
//! Document IDs are stable per pattern and day ([`LogPattern::doc_id`]), so every run
//! updates the same points. Runs fetch overlapping windows (`INDEXER_OVERLAP_MINUTES`),
//! starting at a whole minute ([`fetch_start`]), and a fetch can reach into the previous
//! UTC day. Each day's document is merged with its stored state separately
//! ([`LogPattern::merge_stored`]), with `F` the fetch start:
//!
//! - hours of the day that end at or before `F` keep their stored counts;
//! - hours that start at or after `F` were fetched in full, so the fetched counts replace
//!   the stored ones;
//! - the hour that contains `F` keeps its stored logs before `F`: its stored count minus
//!   the stored per-minute counts at or after `F`, plus the fetched count.
//!
//! The per-minute counts are the only finer state kept: those of the fetch that last
//! wrote the document, within its day, so that the hour containing the next fetch's start
//! can be split. Checkpoints only move forward, so a later fetch never starts before
//! them. Logs in the overlap are therefore counted once, logs that reached Datadog late
//! (inside the overlap) are added, re-running a window changes nothing, and a fetch that
//! starts before a day (or before its first stored log) recounts that day from scratch.
//! The exception is a checkpoint moved back (a deleted checkpoint file or a larger
//! overlap) to a minute before the stored per-minute counts and not on a whole hour: logs
//! of that one hour between the fetch start and the stored minutes may be counted twice.

use crate::chunk::stable_id;
use crate::domain::{RagDocument, SourceKind};
use crate::text::truncate_bytes;
use chrono::{DateTime, Duration, DurationRound, NaiveDate, SecondsFormat, Timelike, Utc};
use chrono_tz::Tz;
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, HashSet};

/// Longest pattern, in chars; longer messages are cut and end in `…`.
pub const PATTERN_MAX_CHARS: usize = 160;
/// Most distinct sample messages kept per pattern and day.
pub const MAX_SAMPLES: usize = 3;
/// Longest sample message, in bytes (cut at a char boundary).
pub const SAMPLE_MAX_BYTES: usize = 4000;
/// Most sample log IDs kept per pattern and day (the most recent ones).
pub const MAX_SAMPLE_IDS: usize = 5;
/// Prefix of pattern IDs and of pattern document IDs.
pub const DOC_ID_PREFIX: &str = "logpattern_";
/// Characters of the pattern shown in a document title.
const TITLE_PATTERN_CHARS: usize = 80;

/// Metadata keys of a pattern document.
pub mod keys {
    pub const STATUS: &str = "status";
    pub const PATTERN: &str = "pattern";
    /// The pattern across days ([`super::LogPattern::pattern_id`]); the reranker lists
    /// the days of one pattern as one source.
    pub const PATTERN_ID: &str = "pattern_id";
    /// The UTC day, `YYYY-MM-DD`.
    pub const DAY: &str = "day";
    pub const COUNT: &str = "count";
    /// RFC 3339, the day's first log; `Timestamp` holds its last. The retrieval filter
    /// reads this key to keep a day whose logs started before a window and went on after
    /// it.
    pub const FIRST_SEEN: &str = "first_seen";
    pub const LAST_SEEN: &str = "last_seen";
    pub const SAMPLES: &str = "samples";
    pub const SAMPLE_LOG_IDS: &str = "sample_log_ids";
    /// 24 counts, one per UTC hour of the day.
    pub const HOUR_COUNTS: &str = "hour_counts";
    /// Merge state only: per-minute counts of the fetch that last wrote the document.
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

/// The state of one pattern document: one pattern on one UTC day.
#[derive(Debug, Clone, PartialEq)]
pub struct LogPattern {
    pub service: String,
    pub environment: String,
    pub status: String,
    pub pattern: String,
    /// The UTC day of every log counted here.
    pub day: NaiveDate,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
    /// Distinct raw messages (cut to [`SAMPLE_MAX_BYTES`]), oldest first.
    pub samples: Vec<String>,
    /// The most recent log IDs, oldest first.
    pub sample_ids: Vec<String>,
    /// Logs per UTC hour of `day`.
    pub hour_counts: [u64; 24],
    /// Logs per minute (keyed by the minute's start) of the fetch that wrote this state,
    /// within `day`; only used to merge the next overlapping fetch.
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

/// Start of UTC hour `hour` of `day`.
fn hour_start(day: NaiveDate, hour: usize) -> DateTime<Utc> {
    day.and_hms_opt(0, 0, 0).expect("midnight exists").and_utc() + Duration::hours(hour as i64)
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

/// Groups `events` by (service, environment, status, pattern, UTC day). Events with an
/// ID seen before are counted once. The result is sorted by document ID.
pub fn group(events: &[LogEvent]) -> Vec<LogPattern> {
    let mut sorted: Vec<&LogEvent> = events.iter().collect();
    sorted.sort_by(|a, b| a.timestamp.cmp(&b.timestamp).then(a.id.cmp(&b.id)));
    let mut seen_ids = HashSet::new();
    let mut groups: BTreeMap<(String, String, String, String, NaiveDate), LogPattern> =
        BTreeMap::new();
    for e in sorted {
        if !e.id.is_empty() && !seen_ids.insert(e.id.as_str()) {
            continue;
        }
        let pattern = normalize_message(&e.message);
        let day = e.timestamp.date_naive();
        let key = (
            e.service.clone(),
            e.environment.clone(),
            e.status.clone(),
            pattern.clone(),
            day,
        );
        let g = groups.entry(key).or_insert_with(|| LogPattern {
            service: e.service.clone(),
            environment: e.environment.clone(),
            status: e.status.clone(),
            pattern,
            day,
            first_seen: e.timestamp,
            last_seen: e.timestamp,
            samples: vec![],
            sample_ids: vec![],
            hour_counts: [0; 24],
            minute_counts: BTreeMap::new(),
        });
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
        g.hour_counts[e.timestamp.hour() as usize] += 1;
        *g.minute_counts.entry(fetch_start(e.timestamp)).or_default() += 1;
    }
    let mut out: Vec<LogPattern> = groups.into_values().collect();
    out.sort_by_key(LogPattern::doc_id);
    out
}

impl LogPattern {
    /// Logs of the day.
    pub fn count(&self) -> u64 {
        self.hour_counts.iter().sum()
    }

    /// The pattern across days: stable per (service, environment, status, pattern).
    pub fn pattern_id(&self) -> String {
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

    /// Stable per pattern and UTC day, so runs update one point per day:
    /// `<pattern_id>_<YYYY-MM-DD>`.
    pub fn doc_id(&self) -> String {
        format!("{}_{}", self.pattern_id(), self.day)
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
    /// service, environment and status from the day's first to its last log.
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
            "Pattern: {}\nOccurrences on {} (UTC): {}{} log(s), first seen {}, last seen {}\n\n\
             Samples:\n{}",
            self.pattern,
            self.day,
            self.count(),
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
        metadata.insert(keys::PATTERN_ID.into(), json!(self.pattern_id()));
        metadata.insert(keys::DAY.into(), json!(self.day.to_string()));
        metadata.insert(keys::COUNT.into(), json!(self.count()));
        metadata.insert(keys::FIRST_SEEN.into(), json!(rfc3339(self.first_seen)));
        metadata.insert(keys::LAST_SEEN.into(), json!(rfc3339(self.last_seen)));
        metadata.insert(keys::SAMPLES.into(), json!(self.samples));
        metadata.insert(keys::SAMPLE_LOG_IDS.into(), json!(self.sample_ids));
        metadata.insert(keys::HOUR_COUNTS.into(), json!(self.hour_counts));
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
    /// else.
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
        let hours: Vec<u64> = md
            .get(keys::HOUR_COUNTS)?
            .as_array()?
            .iter()
            .map(Value::as_u64)
            .collect::<Option<_>>()?;
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
            day: md.get(keys::DAY)?.as_str()?.parse().ok()?,
            first_seen: parse_time(md.get(keys::FIRST_SEEN)?)?,
            last_seen: parse_time(md.get(keys::LAST_SEEN)?)?,
            samples: strings(keys::SAMPLES)?,
            sample_ids: strings(keys::SAMPLE_LOG_IDS)?,
            hour_counts: hours.try_into().ok()?,
            minute_counts,
        })
    }

    /// Combines this state, counted from a fetch that started at `fetch_start`, with the
    /// `stored` state of the same pattern and day written by an earlier run (see the
    /// module docs). Logs the stored state counted at or after `fetch_start` were fetched
    /// again and are only counted here.
    pub fn merge_stored(self, stored: &LogPattern, fetch_start: DateTime<Utc>) -> LogPattern {
        if stored.day != self.day || fetch_start <= stored.first_seen {
            return self;
        }
        let mut kept = [0u64; 24];
        for (h, kept) in kept.iter_mut().enumerate() {
            let (start, end) = (hour_start(self.day, h), hour_start(self.day, h + 1));
            *kept = if end <= fetch_start {
                stored.hour_counts[h]
            } else if start >= fetch_start {
                0
            } else {
                let refetched: u64 = stored
                    .minute_counts
                    .range(fetch_start..end)
                    .map(|(_, c)| c)
                    .sum();
                stored.hour_counts[h].saturating_sub(refetched)
            };
        }
        if kept.iter().all(|c| *c == 0) {
            return self;
        }
        let mut hour_counts = self.hour_counts;
        for (h, c) in hour_counts.iter_mut().enumerate() {
            *c += kept[h];
        }
        LogPattern {
            hour_counts,
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

    /// Logs per UTC hour (the hour's start and its count) that may fall in
    /// `[from, to)`: hours whose logs, known to lie between the day's first and last log,
    /// overlap the window. An hour cut by a window bound counts in full.
    pub fn hours_in(
        &self,
        from: Option<DateTime<Utc>>,
        to: Option<DateTime<Utc>>,
    ) -> impl Iterator<Item = (DateTime<Utc>, u64)> + '_ {
        (0..24).filter_map(move |h| {
            let count = self.hour_counts[h];
            let start = hour_start(self.day, h).max(self.first_seen);
            let last =
                (hour_start(self.day, h + 1) - Duration::milliseconds(1)).min(self.last_seen);
            let inside = from.is_none_or(|f| last >= f) && to.is_none_or(|t| start < t);
            (count > 0 && inside).then(|| (hour_start(self.day, h), count))
        })
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

/// The pattern state of a retrieved log pattern document (or one of its chunks).
pub fn from_document(doc: &RagDocument) -> Option<LogPattern> {
    if doc.kind != SourceKind::Logs {
        return None;
    }
    LogPattern::from_metadata(&doc.service, &doc.environment, &doc.metadata)
}

/// How often one pattern occurred, from the states of its retrieved days, for the
/// answer prompt. With a window (`from`/`to`, either may be open) it counts the hours
/// that overlap the window ([`LogPattern::hours_in`]); without one, every hour of the
/// retrieved days. Counts are summed per calendar day in `tz` (an hour belongs to the
/// day it starts in), e.g. `Occurrences in the question's window: 165 (Fri 2026-02-06:
/// 120 · Mon 2026-02-09: 45; days in Europe/Helsinki, hour precision)`.
pub fn occurrences(
    days: &[LogPattern],
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
    tz: Tz,
) -> String {
    let mut per_day: BTreeMap<NaiveDate, u64> = BTreeMap::new();
    for d in days {
        for (start, count) in d.hours_in(from, to) {
            *per_day
                .entry(start.with_timezone(&tz).date_naive())
                .or_default() += count;
        }
    }
    let total: u64 = per_day.values().sum();
    let what = if from.is_some() || to.is_some() {
        "in the question's window"
    } else {
        "on the retrieved days"
    };
    if total == 0 {
        return format!("Occurrences {what}: 0");
    }
    let breakdown: Vec<String> = per_day
        .iter()
        .map(|(day, count)| format!("{}: {count}", day.format("%a %Y-%m-%d")))
        .collect();
    format!(
        "Occurrences {what}: {total} ({}; days in {tz}, hour precision)",
        breakdown.join(" · ")
    )
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
    fn groups_by_scope_status_pattern_and_utc_day() {
        let mut other_env = event("e1", "2026-03-11T10:00:05Z", "timeout after 5ms");
        other_env.environment = "staging".into();
        let mut warn = event("w1", "2026-03-11T10:00:06Z", "timeout after 5ms");
        warn.status = "warn".into();
        let events = [
            event("b", "2026-03-11T10:01:30.500Z", "timeout after 20ms"),
            event("a", "2026-03-11T10:00:01Z", "timeout after 10ms"),
            event("c", "2026-03-11T11:01:59Z", "timeout after 10ms"),
            // A repeat of an ID (pagination overlap) is counted once.
            event("a", "2026-03-11T10:00:01Z", "timeout after 10ms"),
            event("x", "2026-03-11T10:02:00Z", "disk full"),
            // Just after midnight UTC: the same pattern on the next day.
            event("d", "2026-03-12T00:00:00Z", "timeout after 30ms"),
            other_env,
            warn,
        ];
        let groups = group(&events);
        assert_eq!(groups.len(), 5);
        let prod_errors: Vec<&LogPattern> = groups
            .iter()
            .filter(|g| {
                g.pattern == "timeout after #ms" && g.status == "error" && g.environment == "prod"
            })
            .collect();
        let [g, next] = prod_errors.as_slice() else {
            panic!("{prod_errors:?}")
        };
        assert_eq!(g.day, "2026-03-11".parse::<NaiveDate>().unwrap());
        assert_eq!(g.count(), 3);
        assert_eq!(g.first_seen, ts("2026-03-11T10:00:01Z"));
        assert_eq!(g.last_seen, ts("2026-03-11T11:01:59Z"));
        assert_eq!(g.samples, ["timeout after 10ms", "timeout after 20ms"]);
        assert_eq!(g.sample_ids, ["a", "b", "c"]);
        let mut hours = [0; 24];
        hours[10] = 2;
        hours[11] = 1;
        assert_eq!(g.hour_counts, hours);
        assert_eq!(
            g.minute_counts,
            BTreeMap::from([
                (ts("2026-03-11T10:00:00Z"), 1),
                (ts("2026-03-11T10:01:00Z"), 1),
                (ts("2026-03-11T11:01:00Z"), 1)
            ])
        );
        assert_eq!(next.day, "2026-03-12".parse::<NaiveDate>().unwrap());
        assert_eq!((next.count(), next.hour_counts[0]), (1, 1));
        assert_eq!(next.pattern_id(), g.pattern_id());
        assert_ne!(next.doc_id(), g.doc_id());
        // Sorted by (stable) document ID.
        let ids: Vec<String> = groups.iter().map(LogPattern::doc_id).collect();
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(ids, sorted);
    }

    #[test]
    fn ids_are_stable_per_pattern_and_day() {
        let a = &group(&[event("a", "2026-03-11T00:00:00Z", "timeout after 10ms")])[0];
        let b = &group(&[event(
            "zz",
            "2026-03-11T23:59:59.999Z",
            "timeout after 99999ms",
        )])[0];
        assert_eq!(a.doc_id(), b.doc_id());
        let prefix = format!("{DOC_ID_PREFIX}{}", "0".repeat(32));
        assert_eq!(a.pattern_id().len(), prefix.len());
        assert!(a.pattern_id().starts_with(DOC_ID_PREFIX));
        assert_eq!(a.doc_id(), format!("{}_2026-03-11", a.pattern_id()));
        // Another day: another document of the same pattern.
        let c = &group(&[event("c", "2026-03-12T00:00:00Z", "timeout after 10ms")])[0];
        assert_eq!(c.pattern_id(), a.pattern_id());
        assert_eq!(c.doc_id(), format!("{}_2026-03-12", a.pattern_id()));
        for edit in [
            |e: &mut LogEvent| e.service = "payments".into(),
            |e: &mut LogEvent| e.environment = "staging".into(),
            |e: &mut LogEvent| e.status = "warn".into(),
            |e: &mut LogEvent| e.message = "timeout before 10ms".into(),
        ] {
            let mut e = event("a", "2026-03-11T10:00:00Z", "timeout after 10ms");
            edit(&mut e);
            assert_ne!(group(&[e])[0].pattern_id(), a.pattern_id());
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
        let g = groups.iter().find(|g| g.count() == 20).unwrap();
        assert_eq!(g.samples.len(), 1, "cut samples are equal");
        assert!(g.samples[0].len() <= SAMPLE_MAX_BYTES);
        assert!(long.starts_with(&g.samples[0]));
        assert_eq!(g.sample_ids, ["id-15", "id-16", "id-17", "id-18", "id-19"]);
        let doc = g.to_document(APP);
        assert!(doc.text.contains(&g.samples[0]));
    }

    #[test]
    fn document_carries_the_day_its_hours_samples_and_a_search_link() {
        let events = [
            event("a", "2026-03-11T10:00:01Z", "timeout after 10ms"),
            event("b", "2026-03-11T12:05:00.250Z", "timeout after 20ms"),
        ];
        let g = &group(&events)[0];
        let doc = g.to_document(APP);
        assert_eq!(doc.id, g.doc_id());
        assert_eq!(doc.kind, SourceKind::Logs);
        assert_eq!(doc.title, "Log: checkout - error - timeout after #ms");
        assert_eq!(doc.timestamp.as_deref(), Some("2026-03-11T12:05:00.250Z"));
        assert_eq!(
            doc.text,
            "Pattern: timeout after #ms\nOccurrences on 2026-03-11 (UTC): 2 error log(s), \
             first seen 2026-03-11T10:00:01.000Z, last seen 2026-03-11T12:05:00.250Z\n\n\
             Samples:\ntimeout after 10ms\n\ntimeout after 20ms\n\nSample log IDs: a, b"
        );
        // A log search limited to the day's first..last log.
        assert_eq!(
            doc.source_uri,
            "https://app.datadoghq.eu/logs?query=service%3Acheckout%20env%3Aprod%20status%3Aerror\
             &from_ts=1773223201000&to_ts=1773230700251&live=false"
        );
        assert_eq!(doc.metadata["day"], "2026-03-11");
        assert_eq!(doc.metadata["pattern_id"], g.pattern_id());
        assert_eq!(doc.metadata["first_seen"], "2026-03-11T10:00:01.000Z");
        assert_eq!(doc.metadata["count"], 2);
        assert_eq!(
            doc.metadata["hour_counts"],
            json!([
                0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
            ])
        );
        assert_eq!(
            doc.metadata["minute_counts"],
            json!({"2026-03-11T10:00:00Z": 1, "2026-03-11T12:05:00Z": 1})
        );
        // The state reads back from the metadata unchanged.
        let back = LogPattern::from_metadata("checkout", "prod", &doc.metadata).unwrap();
        assert_eq!(&back, g);
        assert_eq!(from_document(&doc).as_ref(), Some(g));
        assert!(LogPattern::from_metadata("checkout", "prod", &Map::new()).is_none());
        let mut short = doc.metadata.clone();
        short.insert("hour_counts".into(), json!([1, 2]));
        assert!(LogPattern::from_metadata("checkout", "prod", &short).is_none());
    }

    fn cache_miss(id: &str, at: &str) -> LogEvent {
        event(id, at, &format!("cache miss for key {}", id.len() * 4711))
    }

    /// The stored state of `events` merged with a fetch of `fresh` from `start`.
    fn merged(stored: &LogPattern, fresh: &[LogEvent], start: &str) -> LogPattern {
        let fresh = &group(fresh)[0];
        let doc = merge_document(
            fresh.to_document(APP),
            &stored.to_document(APP).metadata,
            ts(start),
        );
        from_document(&doc).unwrap()
    }

    /// Run N fetches [10:00, 10:30]; run N+1 re-fetches from 10:20 (the overlap, inside
    /// the 10 o'clock hour) to 10:45 and also sees a log that reached Datadog late.
    #[test]
    fn overlapping_runs_count_every_log_once() {
        let run_n = [
            cache_miss("1", "2026-03-11T09:05:00Z"),
            cache_miss("2", "2026-03-11T10:15:00Z"),
            cache_miss("3", "2026-03-11T10:21:10Z"),
            cache_miss("4", "2026-03-11T10:29:59Z"),
        ];
        let stored = group(&run_n)[0].clone();

        let start = fetch_start(ts("2026-03-11T10:20:30Z"));
        assert_eq!(start, ts("2026-03-11T10:20:00Z"));
        let run_n1 = [
            cache_miss("3", "2026-03-11T10:21:10Z"),
            cache_miss("late", "2026-03-11T10:25:00Z"),
            cache_miss("4", "2026-03-11T10:29:59Z"),
            cache_miss("6", "2026-03-11T11:44:00Z"),
        ];
        assert_eq!(
            group(&run_n1)[0].doc_id(),
            stored.doc_id(),
            "one point per day"
        );
        let m = merged(&stored, &run_n1, "2026-03-11T10:20:00Z");
        assert_eq!(m.count(), 6, "1, 2 from run N; 3, late, 4, 6 from run N+1");
        assert_eq!(
            (m.hour_counts[9], m.hour_counts[10], m.hour_counts[11]),
            (1, 4, 1)
        );
        assert_eq!(m.first_seen, ts("2026-03-11T09:05:00Z"));
        assert_eq!(m.last_seen, ts("2026-03-11T11:44:00Z"));
        assert_eq!(m.sample_ids, ["2", "3", "4", "late", "6"]);
        let doc = m.to_document(APP);
        assert_eq!(doc.timestamp.as_deref(), Some("2026-03-11T11:44:00.000Z"));
        assert!(
            doc.text
                .contains("Occurrences on 2026-03-11 (UTC): 6 error log(s)")
        );
        assert!(doc.source_uri.contains("&from_ts=1773219900000&"));
        // Only the fetch's own minutes are kept.
        assert_eq!(m.minute_counts.len(), 4);

        // Re-running the same window (e.g. after a failed checkpoint save) is a no-op.
        assert_eq!(merged(&m, &run_n1, "2026-03-11T10:20:00Z"), m);
        // The next run with nothing new in its window keeps the total.
        let next = merged(
            &m,
            &[cache_miss("6", "2026-03-11T11:44:00Z")],
            "2026-03-11T11:35:00Z",
        );
        assert_eq!(next.count(), 6);
        assert_eq!(next.first_seen, ts("2026-03-11T09:05:00Z"));

        // A fetch reaching back before the first stored log recounts from scratch.
        let all = [run_n.as_slice(), run_n1.as_slice()].concat();
        let recount = merged(&m, &all, "2026-03-11T09:00:00Z");
        assert_eq!(recount, group(&all)[0]);
        assert_eq!(recount.count(), 6);
        // A checkpoint moved back to a whole hour is still exact: whole hours are replaced.
        let back = merged(&next, &all[1..], "2026-03-11T10:00:00Z");
        assert_eq!(back.count(), 6);
    }

    /// Run N fetched 23:20..23:50 on 11 March; run N+1 fetches 23:40..00:30 and sees a
    /// late log before midnight and new logs after it: each day's document is merged on
    /// its own, and the new day is counted from scratch.
    #[test]
    fn a_fetch_across_midnight_updates_both_days() {
        let run_n = [
            cache_miss("a", "2026-03-11T22:10:00Z"),
            cache_miss("b", "2026-03-11T23:30:00Z"),
            cache_miss("c", "2026-03-11T23:45:00Z"),
        ];
        let stored = group(&run_n)[0].clone();
        let run_n1 = [
            cache_miss("c", "2026-03-11T23:45:00Z"),
            cache_miss("late", "2026-03-11T23:49:00Z"),
            cache_miss("d", "2026-03-12T00:05:00Z"),
            cache_miss("e", "2026-03-12T00:20:00Z"),
        ];
        let fetched = group(&run_n1);
        assert_eq!(fetched.len(), 2);
        let [before, after] = [&fetched[0], &fetched[1]].map(|g| {
            let doc = g.to_document(APP);
            // The new day has no stored point yet; the old day merges with its own.
            let stored_md = (g.day == stored.day).then(|| stored.to_document(APP).metadata);
            let doc = match stored_md {
                Some(md) => merge_document(doc, &md, ts("2026-03-11T23:40:00Z")),
                None => doc,
            };
            from_document(&doc).unwrap()
        });
        let (old, new) = if before.day == stored.day {
            (before, after)
        } else {
            (after, before)
        };
        assert_eq!(old.count(), 4, "a, b from run N; c, late from run N+1");
        assert_eq!((old.hour_counts[22], old.hour_counts[23]), (1, 3));
        assert_eq!(old.last_seen, ts("2026-03-11T23:49:00Z"));
        assert_eq!(new.count(), 2);
        assert_eq!(new.first_seen, ts("2026-03-12T00:05:00Z"));
        assert_eq!(new.pattern_id(), old.pattern_id());

        // The following run starts at 00:15 and merges only into the new day; a stored
        // day that the fetch starts before is recounted.
        let run_n2 = [
            cache_miss("e", "2026-03-12T00:20:00Z"),
            cache_miss("f", "2026-03-12T01:00:00Z"),
        ];
        let newer = merged(&new, &run_n2, "2026-03-12T00:15:00Z");
        assert_eq!(newer.count(), 3);
        let recount = merged(&new, &run_n1[2..], "2026-03-11T23:40:00Z");
        assert_eq!(recount, new);
    }

    #[test]
    fn documents_that_are_not_pattern_days_are_not_merged() {
        let fresh = group(&[event("a", "2026-03-11T10:00:00Z", "x 1")])[0].to_document(APP);
        let other = Map::from_iter([("status".to_string(), json!("error"))]);
        let merged = merge_document(fresh.clone(), &other, ts("2026-03-11T10:00:00Z"));
        assert_eq!(merged.text, fresh.text);
        // Another day's state never merges into this day.
        let other_day = group(&[event("b", "2026-03-10T10:00:00Z", "x 2")])[0].to_document(APP);
        let merged = merge_document(
            fresh.clone(),
            &other_day.metadata,
            ts("2026-03-11T10:30:00Z"),
        );
        assert_eq!(merged.metadata["count"], 1);
    }

    fn burst(at: &str, n: usize, every_secs: i64) -> Vec<LogEvent> {
        (0..n)
            .map(|i| LogEvent {
                timestamp: ts(at) + Duration::seconds(every_secs * i as i64),
                ..event(
                    &format!("{at}-{i}"),
                    at,
                    "inventory sync failed after 502ms",
                )
            })
            .collect()
    }

    /// Asked in Helsinki (UTC+2) for Friday..Monday: the window starts at 22:00 UTC on
    /// Thursday. Thursday's UTC day holds 40 logs in the morning (outside) and 20 after
    /// 22:00 (Friday in Helsinki); counting whole UTC days would add the 40 and put the
    /// 20 on Thursday.
    #[test]
    fn window_counts_use_hours_and_the_askers_days() {
        let events = [
            burst("2026-02-05T08:00:00Z", 40, 60),
            burst("2026-02-05T22:10:00Z", 20, 120),
            burst("2026-02-06T09:00:00Z", 100, 30),
            burst("2026-02-09T07:00:00Z", 45, 60),
        ]
        .concat();
        let days = group(&events);
        assert_eq!(days.len(), 3);
        let helsinki: Tz = "Europe/Helsinki".parse().unwrap();
        let (from, to) = (ts("2026-02-05T22:00:00Z"), ts("2026-02-09T22:00:00Z"));
        assert_eq!(
            occurrences(&days, Some(from), Some(to), helsinki),
            "Occurrences in the question's window: 165 (Fri 2026-02-06: 120 · Mon 2026-02-09: 45; \
             days in Europe/Helsinki, hour precision)"
        );
        // The same window from UTC: Friday starts at midnight UTC.
        assert_eq!(
            occurrences(
                &days,
                Some(ts("2026-02-06T00:00:00Z")),
                Some(ts("2026-02-10T00:00:00Z")),
                Tz::UTC
            ),
            "Occurrences in the question's window: 145 (Fri 2026-02-06: 100 · Mon 2026-02-09: 45; \
             days in UTC, hour precision)"
        );
        // No window: every retrieved day, per day in the asker's zone.
        assert_eq!(
            occurrences(&days, None, None, helsinki),
            "Occurrences on the retrieved days: 205 (Thu 2026-02-05: 40 · Fri 2026-02-06: 120 · \
             Mon 2026-02-09: 45; days in Europe/Helsinki, hour precision)"
        );
        // An open-ended window and a half-hour zone: an hour belongs to the local day it
        // starts in (22:00 UTC is 03:30 on Friday in Kolkata).
        let kolkata: Tz = "Asia/Kolkata".parse().unwrap();
        assert_eq!(
            occurrences(&days, Some(from), None, kolkata),
            "Occurrences in the question's window: 165 (Fri 2026-02-06: 120 · Mon 2026-02-09: 45; \
             days in Asia/Kolkata, hour precision)"
        );
    }

    /// An hour cut by the window counts in full, but hours before the day's first or
    /// after its last log never count.
    #[test]
    fn hours_in_a_window_are_bounded_by_the_first_and_last_log() {
        let g = &group(&[
            event("a", "2026-03-11T09:10:00Z", "x 1"),
            event("b", "2026-03-11T09:50:00Z", "x 2"),
            event("c", "2026-03-11T13:00:00Z", "x 3"),
        ])[0];
        let hours = |from: &str, to: &str| -> Vec<(DateTime<Utc>, u64)> {
            g.hours_in(Some(ts(from)), Some(ts(to))).collect()
        };
        assert_eq!(
            hours("2026-03-11T09:30:00Z", "2026-03-11T10:00:00Z"),
            [(ts("2026-03-11T09:00:00Z"), 2)]
        );
        // Between the logs: nothing, although the day's span overlaps the window.
        assert!(hours("2026-03-11T10:00:00Z", "2026-03-11T12:00:00Z").is_empty());
        // Half-open: a window ending at the first log misses it.
        assert!(hours("2026-03-11T08:00:00Z", "2026-03-11T09:10:00Z").is_empty());
        assert_eq!(
            hours("2026-03-11T13:00:00Z", "2026-03-11T13:00:01Z"),
            [(ts("2026-03-11T13:00:00Z"), 1)]
        );
        assert_eq!(g.hours_in(None, None).count(), 2);
    }
}
