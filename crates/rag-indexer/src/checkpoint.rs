//! Per-source indexing checkpoints, persisted as JSON:
//!
//! ```json
//! { "sources": { "logs": { "indexed_until": "2025-01-01T00:15:00Z" } } }
//! ```
//!
//! A legacy watermark file holding a single RFC3339 timestamp is still accepted and
//! applies to every source.

use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Checkpoints {
    pub sources: BTreeMap<String, SourceCheckpoint>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceCheckpoint {
    /// End of the last window whose documents were all embedded and upserted.
    pub indexed_until: DateTime<Utc>,
}

impl Checkpoints {
    pub fn get(&self, source: &str) -> Option<DateTime<Utc>> {
        self.sources.get(source).map(|c| c.indexed_until)
    }

    pub fn set(&mut self, source: &str, indexed_until: DateTime<Utc>) {
        self.sources
            .insert(source.to_string(), SourceCheckpoint { indexed_until });
    }

    /// Parses the checkpoint file, accepting the legacy single-timestamp watermark
    /// as the checkpoint of every source in `sources`.
    pub fn parse(content: &str, sources: &[&str]) -> Result<Self> {
        if let Ok(checkpoints) = serde_json::from_str::<Checkpoints>(content) {
            return Ok(checkpoints);
        }
        let legacy = DateTime::parse_from_rfc3339(content.trim()).map_err(|_| {
            anyhow::anyhow!("Checkpoint file is neither checkpoint JSON nor an RFC3339 timestamp")
        })?;
        let mut checkpoints = Checkpoints::default();
        for source in sources {
            checkpoints.set(source, legacy.with_timezone(&Utc));
        }
        Ok(checkpoints)
    }

    /// Loads checkpoints from `path`; a missing file means no source has been indexed yet.
    pub async fn load(path: &Path, sources: &[&str]) -> Result<Self> {
        match tokio::fs::read_to_string(path).await {
            Ok(content) => Self::parse(&content, sources),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }

    /// Writes the checkpoints atomically: a synced temporary file renamed over `path`.
    pub async fn save(&self, path: &Path) -> Result<()> {
        let tmp = temp_path(path);
        let mut file = tokio::fs::File::create(&tmp).await?;
        file.write_all(&serde_json::to_vec_pretty(self)?).await?;
        file.sync_all().await?;
        drop(file);
        tokio::fs::rename(&tmp, path).await?;
        Ok(())
    }
}

fn temp_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".tmp");
    path.with_file_name(name)
}

/// Time range fetched for one source.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Window {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
}

/// Window for a source: from its checkpoint minus `overlap` (so late-arriving records
/// are picked up again), or `now - lookback` on the first run, until `now`. The end is
/// not held back: the overlap already re-reads records that arrive after a run, so a
/// delayed end would only make fresh data slower to appear.
pub fn window(
    checkpoint: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
    lookback: Duration,
    overlap: Duration,
) -> Window {
    let from = match checkpoint {
        Some(indexed_until) => (indexed_until - overlap).min(now),
        None => now - lookback,
    };
    Window { from, to: now }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCES: [&str; 3] = ["monitors", "incidents", "logs"];

    fn ts(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn temp_file(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rag-indexer-{}-{}", std::process::id(), name));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("checkpoints.json")
    }

    #[test]
    fn test_parse_checkpoint_json() {
        let content = r#"{"sources": {
            "logs": {"indexed_until": "2025-01-01T00:15:00Z"},
            "incidents": {"indexed_until": "2025-01-01T00:00:00Z"}
        }}"#;
        let checkpoints = Checkpoints::parse(content, &SOURCES).unwrap();

        assert_eq!(checkpoints.get("logs"), Some(ts("2025-01-01T00:15:00Z")));
        assert_eq!(
            checkpoints.get("incidents"),
            Some(ts("2025-01-01T00:00:00Z"))
        );
        assert_eq!(checkpoints.get("monitors"), None);
    }

    #[test]
    fn test_parse_legacy_watermark_applies_to_all_sources() {
        let checkpoints = Checkpoints::parse("2025-01-01T12:00:00+02:00\n", &SOURCES).unwrap();

        for source in SOURCES {
            assert_eq!(checkpoints.get(source), Some(ts("2025-01-01T10:00:00Z")));
        }
        assert_eq!(checkpoints.sources.len(), SOURCES.len());
    }

    #[test]
    fn test_parse_rejects_garbage() {
        assert!(Checkpoints::parse("invalid timestamp", &SOURCES).is_err());
    }

    #[tokio::test]
    async fn test_load_missing_file_is_empty() {
        let path = temp_file("missing");
        let _ = tokio::fs::remove_file(&path).await;

        let checkpoints = Checkpoints::load(&path, &SOURCES).await.unwrap();
        assert_eq!(checkpoints, Checkpoints::default());
    }

    #[tokio::test]
    async fn test_load_legacy_file_then_save_migrates_format() {
        let path = temp_file("legacy");
        tokio::fs::write(&path, "2025-01-01T00:00:00Z")
            .await
            .unwrap();

        let mut checkpoints = Checkpoints::load(&path, &SOURCES).await.unwrap();
        assert_eq!(checkpoints.get("logs"), Some(ts("2025-01-01T00:00:00Z")));

        checkpoints.set("logs", ts("2025-01-01T00:15:00Z"));
        checkpoints.save(&path).await.unwrap();

        let reloaded = Checkpoints::load(&path, &SOURCES).await.unwrap();
        assert_eq!(reloaded, checkpoints);
        assert_eq!(reloaded.get("logs"), Some(ts("2025-01-01T00:15:00Z")));
        assert_eq!(reloaded.get("incidents"), Some(ts("2025-01-01T00:00:00Z")));
        let raw: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(&path).await.unwrap()).unwrap();
        assert!(raw["sources"]["logs"]["indexed_until"].is_string());
    }

    #[tokio::test]
    async fn test_save_replaces_file_and_leaves_no_temp_file() {
        let path = temp_file("atomic");
        tokio::fs::write(&path, "old contents").await.unwrap();

        let mut checkpoints = Checkpoints::default();
        checkpoints.set("logs", ts("2025-01-02T00:00:00Z"));
        checkpoints.save(&path).await.unwrap();

        assert_eq!(
            Checkpoints::load(&path, &SOURCES).await.unwrap(),
            checkpoints
        );
        assert!(!temp_path(&path).exists());
    }

    #[test]
    fn test_temp_path_is_next_to_target() {
        assert_eq!(
            temp_path(Path::new("/data/watermark.json")),
            PathBuf::from("/data/watermark.json.tmp")
        );
    }

    #[test]
    fn test_window_first_run_uses_lookback() {
        let now = ts("2025-01-01T12:00:00Z");
        let w = window(None, now, Duration::minutes(90), Duration::minutes(10));
        assert_eq!(w.from, ts("2025-01-01T10:30:00Z"));
        assert_eq!(w.to, now);
    }

    #[test]
    fn test_window_overlaps_checkpoint() {
        let now = ts("2025-01-01T12:00:00Z");
        let w = window(
            Some(ts("2025-01-01T11:45:00Z")),
            now,
            Duration::minutes(90),
            Duration::minutes(10),
        );
        assert_eq!(w.from, ts("2025-01-01T11:35:00Z"));
        assert_eq!(w.to, now);
    }

    #[test]
    fn test_window_resumes_old_checkpoint_beyond_lookback() {
        // After an outage the whole gap is fetched, not just the lookback.
        let now = ts("2025-01-03T00:00:00Z");
        let w = window(
            Some(ts("2025-01-01T00:00:00Z")),
            now,
            Duration::minutes(90),
            Duration::minutes(10),
        );
        assert_eq!(w.from, ts("2024-12-31T23:50:00Z"));
    }

    #[test]
    fn test_window_never_starts_after_now() {
        // A checkpoint in the future (clock skew) yields an empty, not inverted, window.
        let now = ts("2025-01-01T12:00:00Z");
        let w = window(
            Some(ts("2025-01-01T13:00:00Z")),
            now,
            Duration::minutes(90),
            Duration::minutes(10),
        );
        assert_eq!(w.from, now);
        assert_eq!(w.to, now);
    }

    #[test]
    fn test_window_zero_overlap() {
        let now = ts("2025-01-01T12:00:00Z");
        let checkpoint = ts("2025-01-01T11:45:00Z");
        let w = window(
            Some(checkpoint),
            now,
            Duration::minutes(90),
            Duration::zero(),
        );
        assert_eq!(w.from, checkpoint);
    }
}
