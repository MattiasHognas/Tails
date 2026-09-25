//! Deterministic analysis of live data. All arithmetic happens here, never in
//! the LLM.
//!
//! Metrics: a series is fetched for `[baseline_from, to)` where the baseline is
//! the equal-length window just before the question's window. Points are split
//! into baseline and window. A window point is *high* when it exceeds
//! `p95(baseline) + max((SPIKE_FACTOR - 1) * |p95|, SIGMAS * stddev, EPSILON)`
//! and *low* when it is below
//! `p5(baseline) - max((1 - 1 / SPIKE_FACTOR) * |p5|, SIGMAS * stddev, EPSILON)`.
//! A run of at least [`MIN_RUN`] consecutive high (low) points is a spike (drop);
//! a single point qualifies when it clears the band computed with
//! [`SINGLE_POINT_FACTOR`] instead. Runs of at least [`GAP_MIN_POINTS`] missing
//! intervals inside the window are gaps.
//!
//! Logs: events are bucketed into about [`LOG_BUCKETS`] intervals (at least one
//! minute). A bucket is part of a burst when its count is at least
//! `max(BURST_MIN_COUNT, BURST_FACTOR * median bucket count)`.

use chrono::{DateTime, Duration, Utc};
use std::collections::HashMap;

pub const SPIKE_FACTOR: f64 = 1.5;
pub const SINGLE_POINT_FACTOR: f64 = 3.0;
pub const SIGMAS: f64 = 3.0;
pub const MIN_RUN: usize = 2;
pub const MIN_BASELINE_POINTS: usize = 5;
pub const GAP_MIN_POINTS: i64 = 3;
const EPSILON: f64 = 1e-9;

pub const LOG_BUCKETS: i64 = 24;
pub const BURST_MIN_COUNT: usize = 5;
pub const BURST_FACTOR: f64 = 3.0;
pub const TOP_MESSAGES: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stats {
    pub count: usize,
    pub min: f64,
    pub max: f64,
    pub mean: f64,
    pub stddev: f64,
    pub p5: f64,
    pub p50: f64,
    pub p95: f64,
}

/// Nearest-rank percentile of sorted values.
fn percentile(sorted: &[f64], p: f64) -> f64 {
    let rank = ((p / 100.0) * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

pub fn stats(values: &[f64]) -> Option<Stats> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let n = values.len() as f64;
    let mean = values.iter().sum::<f64>() / n;
    let var = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n;
    Some(Stats {
        count: values.len(),
        min: sorted[0],
        max: sorted[sorted.len() - 1],
        mean,
        stddev: var.sqrt(),
        p5: percentile(&sorted, 5.0),
        p50: percentile(&sorted, 50.0),
        p95: percentile(&sorted, 95.0),
    })
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Direction {
    Above,
    Below,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Finding {
    /// A spike (`Above`) or drop (`Below`) relative to the baseline band.
    Excursion {
        direction: Direction,
        start_ms: i64,
        end_ms: i64,
        points: usize,
        /// The most extreme value in the run and when it occurred.
        peak: f64,
        peak_ms: i64,
        /// The band edge that was crossed.
        threshold: f64,
    },
    Gap {
        start_ms: i64,
        end_ms: i64,
        missing_points: i64,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct SeriesAnalysis {
    pub window: Option<Stats>,
    pub baseline: Option<Stats>,
    /// Timestamp of the window's max value.
    pub window_max_ms: Option<i64>,
    pub findings: Vec<Finding>,
}

fn band(b: &Stats, factor: f64) -> (f64, f64) {
    let high = b.p95
        + ((factor - 1.0) * b.p95.abs())
            .max(SIGMAS * b.stddev)
            .max(EPSILON);
    let low = b.p5
        - ((1.0 - 1.0 / factor) * b.p5.abs())
            .max(SIGMAS * b.stddev)
            .max(EPSILON);
    (high, low)
}

/// Analyse `points` (`(unix millis, value)`, ascending) for the window
/// `[window_from_ms, window_to_ms)`; earlier points are the baseline.
/// `interval_ms` is the rollup interval, used for gap detection.
pub fn analyse_series(
    points: &[(i64, Option<f64>)],
    window_from_ms: i64,
    window_to_ms: i64,
    interval_ms: Option<i64>,
) -> SeriesAnalysis {
    let mut points: Vec<(i64, Option<f64>)> = points
        .iter()
        .filter(|(_, v)| v.is_none_or(f64::is_finite))
        .copied()
        .collect();
    points.sort_by_key(|p| p.0);
    let baseline_values: Vec<f64> = points
        .iter()
        .filter(|(t, _)| *t < window_from_ms)
        .filter_map(|(_, v)| *v)
        .collect();
    let window_points: Vec<(i64, Option<f64>)> = points
        .iter()
        .filter(|(t, _)| *t >= window_from_ms && *t < window_to_ms)
        .copied()
        .collect();
    let present: Vec<(i64, f64)> = window_points
        .iter()
        .filter_map(|(t, v)| v.map(|v| (*t, v)))
        .collect();
    let values: Vec<f64> = present.iter().map(|p| p.1).collect();
    let window = stats(&values);
    let window_max_ms = present
        .iter()
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|p| p.0);
    let baseline = stats(&baseline_values).filter(|b| b.count >= MIN_BASELINE_POINTS);

    let mut findings = Vec::new();
    if let Some(b) = &baseline {
        let (high, low) = band(b, SPIKE_FACTOR);
        let (high1, low1) = band(b, SINGLE_POINT_FACTOR);
        for (direction, threshold, single) in [
            (Direction::Above, high, high1),
            (Direction::Below, low, low1),
        ] {
            let beyond = |v: f64, t: f64| match direction {
                Direction::Above => v > t,
                Direction::Below => v < t,
            };
            let mut run: Vec<(i64, f64)> = Vec::new();
            let mut flush = |run: &mut Vec<(i64, f64)>| {
                if run.is_empty() {
                    return;
                }
                let (peak_ms, peak) = match direction {
                    Direction::Above => *run.iter().max_by(|a, b| a.1.total_cmp(&b.1)).unwrap(),
                    Direction::Below => *run.iter().min_by(|a, b| a.1.total_cmp(&b.1)).unwrap(),
                };
                if run.len() >= MIN_RUN || beyond(peak, single) {
                    findings.push(Finding::Excursion {
                        direction,
                        start_ms: run[0].0,
                        end_ms: run[run.len() - 1].0,
                        points: run.len(),
                        peak,
                        peak_ms,
                        threshold,
                    });
                }
                run.clear();
            };
            // Missing points neither extend nor break a run.
            for &(t, v) in &present {
                if beyond(v, threshold) {
                    run.push((t, v));
                } else {
                    flush(&mut run);
                }
            }
            flush(&mut run);
        }
    }

    if let Some(interval) = interval_ms.filter(|i| *i > 0) {
        findings.extend(gaps(&window_points, window_from_ms, window_to_ms, interval));
    }
    findings.sort_by_key(|f| match f {
        Finding::Excursion { start_ms, .. } | Finding::Gap { start_ms, .. } => *start_ms,
    });
    SeriesAnalysis {
        window,
        baseline,
        window_max_ms,
        findings,
    }
}

/// Stretches of at least [`GAP_MIN_POINTS`] missing intervals. Points returned
/// as `null` and intervals with no point at all both count as missing. A
/// window with no data at all is reported by the caller as an empty series,
/// not as a gap.
fn gaps(
    window_points: &[(i64, Option<f64>)],
    from_ms: i64,
    to_ms: i64,
    interval: i64,
) -> Vec<Finding> {
    let present: Vec<i64> = window_points
        .iter()
        .filter(|(_, v)| v.is_some())
        .map(|(t, _)| *t)
        .collect();
    if present.is_empty() {
        return vec![];
    }
    let mut out = Vec::new();
    let mut check = |prev: i64, next: i64| {
        // Intervals strictly between two present points (or a window edge).
        let missing = (next - prev) / interval - 1;
        if missing >= GAP_MIN_POINTS {
            out.push(Finding::Gap {
                start_ms: prev + interval,
                end_ms: next - interval,
                missing_points: missing,
            });
        }
    };
    check(from_ms - interval, present[0]);
    for w in present.windows(2) {
        check(w[0], w[1]);
    }
    check(present[present.len() - 1], to_ms);
    out
}

#[derive(Debug, Clone, PartialEq)]
pub struct LogBurst {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub count: usize,
    pub threshold: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LogAnalysis {
    pub total: usize,
    pub by_status: Vec<(String, usize)>,
    pub first: Option<DateTime<Utc>>,
    pub last: Option<DateTime<Utc>>,
    pub bucket: Duration,
    pub peak_bucket: Option<(DateTime<Utc>, usize)>,
    pub bursts: Vec<LogBurst>,
    /// Most frequent message patterns ([`normalize_message`]), with counts.
    pub top_messages: Vec<(String, usize)>,
}

pub use crate::log_patterns::normalize_message;

/// Analyse error/warning log events `(timestamp, status, message)` in `[from, to)`.
pub fn analyse_logs(
    events: &[(DateTime<Utc>, String, String)],
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> LogAnalysis {
    let span = (to - from).num_seconds().max(1);
    let bucket_secs = ((span + LOG_BUCKETS - 1) / LOG_BUCKETS).max(60);
    let bucket_secs = ((bucket_secs + 59) / 60) * 60;
    let bucket = Duration::seconds(bucket_secs);
    let n_buckets = ((span + bucket_secs - 1) / bucket_secs) as usize;
    let mut counts = vec![0usize; n_buckets.max(1)];
    let mut by_status: HashMap<String, usize> = HashMap::new();
    let mut messages: HashMap<String, usize> = HashMap::new();
    let (mut first, mut last) = (None::<DateTime<Utc>>, None::<DateTime<Utc>>);
    let mut total = 0;
    for (ts, status, msg) in events {
        if *ts < from || *ts >= to {
            continue;
        }
        total += 1;
        let idx = ((*ts - from).num_seconds() / bucket_secs) as usize;
        let last_bucket = counts.len() - 1;
        counts[idx.min(last_bucket)] += 1;
        *by_status.entry(status.clone()).or_default() += 1;
        *messages.entry(normalize_message(msg)).or_default() += 1;
        first = Some(first.map_or(*ts, |f| f.min(*ts)));
        last = Some(last.map_or(*ts, |l| l.max(*ts)));
    }
    let bucket_start = |i: usize| from + Duration::seconds(bucket_secs * i as i64);

    let mut sorted = counts.clone();
    sorted.sort_unstable();
    let median = sorted[sorted.len() / 2];
    let threshold = BURST_MIN_COUNT.max((BURST_FACTOR * median as f64).ceil() as usize);
    let mut bursts = Vec::new();
    let mut i = 0;
    while i < counts.len() {
        if counts[i] >= threshold {
            let start = i;
            let mut count = 0;
            while i < counts.len() && counts[i] >= threshold {
                count += counts[i];
                i += 1;
            }
            bursts.push(LogBurst {
                start: bucket_start(start),
                end: bucket_start(i).min(to),
                count,
                threshold,
            });
        } else {
            i += 1;
        }
    }
    let peak_bucket = counts
        .iter()
        .enumerate()
        .filter(|(_, c)| **c > 0)
        .max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(&a.0)))
        .map(|(i, c)| (bucket_start(i), *c));

    let mut by_status: Vec<(String, usize)> = by_status.into_iter().collect();
    by_status.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let mut top_messages: Vec<(String, usize)> = messages.into_iter().collect();
    top_messages.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    top_messages.truncate(TOP_MESSAGES);

    LogAnalysis {
        total,
        by_status,
        first,
        last,
        bucket,
        peak_bucket,
        bursts,
        top_messages,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn series(values: &[Option<f64>], start_ms: i64, interval_ms: i64) -> Vec<(i64, Option<f64>)> {
        values
            .iter()
            .enumerate()
            .map(|(i, v)| (start_ms + i as i64 * interval_ms, *v))
            .collect()
    }

    #[test]
    fn stats_use_nearest_rank_percentiles() {
        let s = stats(&(1..=20).map(f64::from).collect::<Vec<_>>()).unwrap();
        assert_eq!(
            (s.min, s.max, s.p5, s.p50, s.p95),
            (1.0, 20.0, 1.0, 10.0, 19.0)
        );
        assert_eq!(s.mean, 10.5);
        assert!(stats(&[]).is_none());
    }

    #[test]
    fn flat_series_has_no_findings() {
        let pts = series(&[Some(10.0); 20], 0, 60_000);
        let a = analyse_series(&pts, 600_000, 1_200_000, Some(60_000));
        assert!(a.findings.is_empty(), "{:?}", a.findings);
        assert_eq!(a.window.unwrap().count, 10);
        assert_eq!(a.baseline.unwrap().count, 10);
    }

    #[test]
    fn sustained_rise_is_a_spike_and_single_blip_is_not() {
        let mut v = vec![Some(10.0); 20];
        v[12] = Some(20.0);
        v[13] = Some(25.0);
        v[16] = Some(16.0); // single point, above 1.5x but below 3x
        let a = analyse_series(&series(&v, 0, 60_000), 600_000, 1_200_000, Some(60_000));
        assert_eq!(
            a.findings,
            vec![Finding::Excursion {
                direction: Direction::Above,
                start_ms: 720_000,
                end_ms: 780_000,
                points: 2,
                peak: 25.0,
                peak_ms: 780_000,
                threshold: 15.0,
            }]
        );
    }

    #[test]
    fn single_extreme_point_and_drop_to_zero_are_reported() {
        let mut v = vec![Some(10.0); 20];
        v[11] = Some(100.0);
        v[15] = Some(0.0);
        v[16] = Some(0.0);
        let a = analyse_series(&series(&v, 0, 60_000), 600_000, 1_200_000, Some(60_000));
        let dirs: Vec<_> = a
            .findings
            .iter()
            .map(|f| match f {
                Finding::Excursion {
                    direction, points, ..
                } => (*direction, *points),
                Finding::Gap { .. } => panic!("unexpected gap"),
            })
            .collect();
        assert_eq!(dirs, vec![(Direction::Above, 1), (Direction::Below, 2)]);
    }

    #[test]
    fn zero_baseline_counts_flag_any_sustained_errors() {
        let mut v = vec![Some(0.0); 20];
        v[14] = Some(3.0);
        v[15] = Some(4.0);
        let a = analyse_series(&series(&v, 0, 60_000), 600_000, 1_200_000, Some(60_000));
        assert_eq!(a.findings.len(), 1);
    }

    #[test]
    fn nulls_and_missing_intervals_are_gaps() {
        let mut v = vec![Some(10.0); 20];
        for p in v.iter_mut().skip(12).take(4) {
            *p = None;
        }
        let a = analyse_series(&series(&v, 0, 60_000), 600_000, 1_200_000, Some(60_000));
        assert_eq!(
            a.findings,
            vec![Finding::Gap {
                start_ms: 720_000,
                end_ms: 900_000,
                missing_points: 4
            }]
        );
        // Trailing points absent entirely.
        let pts = series(&[Some(10.0); 14], 0, 60_000);
        let a = analyse_series(&pts, 600_000, 1_200_000, Some(60_000));
        assert!(matches!(
            a.findings[..],
            [Finding::Gap {
                missing_points: 6,
                ..
            }]
        ));
    }

    #[test]
    fn short_baseline_disables_excursion_detection() {
        let mut v = vec![Some(10.0); 14];
        v[12] = Some(100.0);
        v[13] = Some(100.0);
        // Only 4 baseline points.
        let a = analyse_series(&series(&v, 360_000, 60_000), 600_000, 1_200_000, None);
        assert!(a.baseline.is_none());
        assert!(a.findings.is_empty());
        assert_eq!(a.window.unwrap().max, 100.0);
    }

    fn ts(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    #[test]
    fn log_bursts_counts_and_top_messages() {
        let from = ts("2026-09-23T00:00:00Z");
        let to = ts("2026-09-24T00:00:00Z");
        let mut events = vec![(
            ts("2026-09-23T02:10:00Z"),
            "warn".to_string(),
            "slow query 12ms".to_string(),
        )];
        for i in 0..8 {
            events.push((
                ts("2026-09-23T14:05:00Z") + Duration::seconds(i * 60),
                "error".into(),
                format!("upstream timeout after {}ms", 1000 + i),
            ));
        }
        let a = analyse_logs(&events, from, to);
        assert_eq!(a.total, 9);
        assert_eq!(a.bucket, Duration::hours(1));
        assert_eq!(a.by_status, vec![("error".into(), 8), ("warn".into(), 1)]);
        assert_eq!(a.first, Some(ts("2026-09-23T02:10:00Z")));
        assert_eq!(a.last, Some(ts("2026-09-23T14:12:00Z")));
        assert_eq!(a.peak_bucket, Some((ts("2026-09-23T14:00:00Z"), 8)));
        assert_eq!(
            a.bursts,
            vec![LogBurst {
                start: ts("2026-09-23T14:00:00Z"),
                end: ts("2026-09-23T15:00:00Z"),
                count: 8,
                threshold: 5
            }]
        );
        assert_eq!(a.top_messages[0], ("upstream timeout after #ms".into(), 8));
    }

    #[test]
    fn no_logs_means_no_burst() {
        let a = analyse_logs(&[], ts("2026-09-23T00:00:00Z"), ts("2026-09-23T01:00:00Z"));
        assert_eq!(a.total, 0);
        assert!(a.bursts.is_empty());
        assert_eq!(a.bucket, Duration::minutes(3));
    }

    /// Log messages are grouped by their pattern (see `log_patterns` for the masking
    /// and multibyte tests); IDs are masked too, so one pattern is one top message.
    #[test]
    fn message_grouping_uses_the_shared_patterns() {
        let t = |m: &str| {
            (
                ts("2026-03-11T10:00:00Z"),
                "error".to_string(),
                m.to_string(),
            )
        };
        let events = [
            t("決済 timeout 1200ms"),
            t("決済 timeout 900ms"),
            t("åäö 1"),
            t("order 550e8400-e29b-41d4-a716-446655440000 rejected"),
            t("order 16fd2706-8baf-433b-82eb-8c7fada847da rejected"),
            t("order 7c9e6679-7425-40de-944b-e07fc1f90ae7 rejected"),
        ];
        let a = analyse_logs(
            &events,
            ts("2026-03-11T00:00:00Z"),
            ts("2026-03-12T00:00:00Z"),
        );
        assert_eq!(a.top_messages[0], ("order <uuid> rejected".to_string(), 3));
        assert_eq!(a.top_messages[1], ("決済 timeout #ms".to_string(), 2));
    }
}
