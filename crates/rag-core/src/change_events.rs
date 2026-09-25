//! Deployments and configuration changes from the Datadog Events API, one document per
//! event.
//!
//! Source: `POST /api/v2/events/search` (permission `events_read`) with
//! `{"filter": {"from", "to", "query"}, "page": {"limit", "cursor"}, "sort": "timestamp"}`,
//! at most 1000 events per page, following `meta.page.after` like the log search. Each
//! event is `{"id", "type": "event", "attributes": {"timestamp", "message", "tags",
//! "attributes": {"title", "service", "evt", ...}}}`; change events (category `change`)
//! add `author`, `changed_resource`, `impacted_resources`, `prev_value`/`new_value`
//! inside the inner `attributes`.
//!
//! Which events count as changes is the search query ([`DEFAULT_QUERY`], overridable
//! with `INDEXER_CHANGE_EVENTS_QUERY`). The documents answer "what changed before it
//! broke?": they carry the event time, service and environment (so the retrieval scope
//! and time window apply), version, commit and author.

use crate::datadog::{Datadog, normalize_scope_value};
use crate::domain::{RagDocument, SourceKind};
use crate::planner::format_utc;
use crate::resilience::send_with_retry;
use crate::text::{TRUNCATION_MARKER, truncate_with_marker};
use anyhow::Result;
use chrono::{DateTime, TimeZone, Utc};
use serde_json::{Value, json};

/// Default event search query: Datadog change events (Change Tracking, feature flag
/// and configuration changes) plus events from common deployment tools.
pub const DEFAULT_QUERY: &str = "@evt.category:change OR source:(argocd OR spinnaker OR jenkins OR gitlab OR github OR launchdarkly OR terraform)";
/// Maximum `page.limit` accepted by the event search endpoint.
const PAGE_LIMIT: u64 = 1000;
/// Longest event message kept, in bytes (cut at a char boundary and marked).
pub const MESSAGE_MAX_BYTES: usize = 4000;
/// Longest single value (title, author, version, ...) kept, in bytes.
const VALUE_MAX_BYTES: usize = 300;
/// Most tags kept per event.
const MAX_TAGS: usize = 50;

fn bounded(s: &str, max_bytes: usize) -> String {
    let flat = s.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_with_marker(&flat, max_bytes, TRUNCATION_MARKER).into_owned()
}

/// The raw value of the first `key:value` tag, case preserved (versions, commit SHAs).
fn raw_tag<'a>(tags: &[&'a str], keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|key| {
        tags.iter()
            .find_map(|t| t.strip_prefix(key)?.strip_prefix(':'))
            .map(str::trim)
            .filter(|v| !v.is_empty())
    })
}

/// A string value that is not a placeholder Datadog fills in (`undefined`).
fn present(v: &Value) -> Option<&str> {
    v.as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("undefined"))
}

/// The event time: the outer RFC 3339 `timestamp`, else the inner POSIX milliseconds.
fn event_time(attrs: &Value) -> Option<DateTime<Utc>> {
    attrs["timestamp"]
        .as_str()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&Utc))
        .or_else(|| {
            let ms = attrs["attributes"]["timestamp"].as_i64()?;
            Utc.timestamp_millis_opt(ms).single()
        })
}

/// One change event as a document; `None` without an ID or a parseable time.
pub fn change_document(event: &Value, site: &str) -> Option<RagDocument> {
    let id = present(&event["id"])?.to_string();
    let attrs = &event["attributes"];
    let inner = &attrs["attributes"];
    let at = event_time(attrs)?;
    let tags: Vec<&str> = attrs["tags"]
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();

    let message = attrs["message"].as_str().unwrap_or_default();
    let title = present(&inner["title"])
        .or_else(|| present(&attrs["title"]))
        .map(str::to_string)
        .or_else(|| {
            message
                .lines()
                .map(str::trim)
                .find(|l| !l.is_empty())
                .map(str::to_string)
        })
        .map(|t| bounded(&t, VALUE_MAX_BYTES))
        .unwrap_or_else(|| "Change event".to_string());

    let impacted_service = inner["impacted_resources"].as_array().and_then(|a| {
        a.iter()
            .find(|r| r["type"] == "service")
            .and_then(|r| present(&r["name"]))
    });
    let service = normalize_scope_value(
        present(&inner["service"])
            .or_else(|| raw_tag(&tags, &["service"]))
            .or(impacted_service)
            .unwrap_or_default(),
    );
    let environment =
        normalize_scope_value(raw_tag(&tags, &["env", "environment"]).unwrap_or_default());
    let value = |v: Option<&str>| v.map(|s| bounded(s, VALUE_MAX_BYTES)).unwrap_or_default();
    let version = value(raw_tag(
        &tags,
        &["version", "deployment.version", "app.version"],
    ));
    let commit = value(raw_tag(
        &tags,
        &["git.commit.sha", "git_commit_sha", "commit_sha", "commit"],
    ));
    let author =
        value(present(&inner["author"]["name"]).or_else(|| raw_tag(&tags, &["author", "user"])));
    let source = value(
        present(&inner["source_type_name"])
            .or_else(|| raw_tag(&tags, &["source"]))
            .or_else(|| present(&inner["evt"]["source_type_name"])),
    );
    let changed = &inner["changed_resource"];
    let changed_resource = match (present(&changed["name"]), present(&changed["type"])) {
        (Some(n), Some(t)) => bounded(&format!("{n} ({t})"), VALUE_MAX_BYTES),
        (Some(n), None) => bounded(n, VALUE_MAX_BYTES),
        _ => String::new(),
    };
    let lower = format!("{title} {}", tags.join(" ")).to_lowercase();
    let change_type = present(&changed["type"])
        .map(str::to_string)
        .unwrap_or_else(|| {
            if lower.contains("deploy") || lower.contains("release") || lower.contains("rollout") {
                "deployment".to_string()
            } else {
                "change".to_string()
            }
        });

    let when = format_utc(at);
    let mut lines = vec![title.clone()];
    let mut push = |label: &str, value: &str| {
        if !value.is_empty() {
            lines.push(format!("{label}: {value}"));
        }
    };
    push("Change type", &change_type);
    push("Time", &when);
    push("Service", &service);
    push("Environment", &environment);
    push("Version", &version);
    push("Commit", &commit);
    push("Author", &author);
    push("Changed resource", &changed_resource);
    push("Source", &source);
    let body = message.trim();
    let mut text = lines.join("\n");
    if !body.is_empty() && body != title {
        text.push_str("\n\n");
        text.push_str(&truncate_with_marker(
            body,
            MESSAGE_MAX_BYTES,
            TRUNCATION_MARKER,
        ));
    }

    let mut metadata = serde_json::Map::new();
    for (key, v) in [
        ("change_type", &change_type),
        ("version", &version),
        ("commit", &commit),
        ("author", &author),
        ("source", &source),
        ("changed_resource", &changed_resource),
    ] {
        if !v.is_empty() {
            metadata.insert(key.into(), json!(v));
        }
    }
    let kept: Vec<String> = tags
        .iter()
        .take(MAX_TAGS)
        .map(|t| bounded(t, VALUE_MAX_BYTES))
        .collect();
    metadata.insert("tags".into(), json!(kept));

    Some(RagDocument {
        id: format!("change_{id}"),
        title,
        text,
        source_uri: format!(
            "https://app.{site}/event/explorer?event={}",
            urlencoding::encode(&id)
        ),
        kind: SourceKind::Change,
        timestamp: Some(when),
        service,
        environment,
        metadata,
    })
}

/// Body for `POST /api/v2/events/search`; ascending order makes progress through the
/// window monotonic.
fn search_body(from_iso: &str, to_iso: &str, query: &str, cursor: Option<&str>) -> Value {
    let mut page = json!({ "limit": PAGE_LIMIT });
    if let Some(cursor) = cursor {
        page["cursor"] = json!(cursor);
    }
    json!({
        "filter": {"from": from_iso, "to": to_iso, "query": query},
        "page": page,
        "sort": "timestamp"
    })
}

impl Datadog {
    /// Fetches every event matching `query` in `[from_iso, to_iso]` with
    /// `POST /api/v2/events/search`, oldest first, following `meta.page.after` until
    /// the last page. Events without an ID or a parseable time are skipped. Transient
    /// failures (429, 5xx, timeouts) are retried per `self.retry`.
    pub async fn search_change_events(
        &self,
        from_iso: &str,
        to_iso: &str,
        query: &str,
    ) -> Result<Vec<RagDocument>> {
        let url = format!("{}/api/v2/events/search", self.api_base);
        let mut docs = Vec::new();
        let mut skipped = 0usize;
        let mut cursor: Option<String> = None;
        loop {
            let body = search_body(from_iso, to_iso, query, cursor.as_deref());
            let response = send_with_retry(&self.retry, "datadog events search", || {
                self.http
                    .post(&url)
                    .header("DD-API-KEY", &self.api_key)
                    .header("DD-APPLICATION-KEY", &self.app_key)
                    .json(&body)
            })
            .await
            .map_err(|f| anyhow::anyhow!("Failed to search change events: {}", f.error))?;
            let result: Value = response.json().await?;
            let events = result["data"]
                .as_array()
                .ok_or_else(|| anyhow::anyhow!("Unexpected event search response: missing data"))?;
            for event in events {
                match change_document(event, &self.site) {
                    Some(d) => docs.push(d),
                    None => skipped += 1,
                }
            }
            match result["meta"]["page"]["after"].as_str() {
                Some(after) if !events.is_empty() => {
                    if cursor.as_deref() == Some(after) {
                        anyhow::bail!("Event search returned the same cursor twice");
                    }
                    cursor = Some(after.to_string());
                }
                _ => break,
            }
        }
        if skipped > 0 {
            tracing::warn!(skipped, "skipped events without an ID or a parseable time");
        }
        Ok(docs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn fixture(name: &str) -> Value {
        let path = format!(
            "{}/tests/fixtures/datadog/{}",
            env!("CARGO_MANIFEST_DIR"),
            name
        );
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap()
    }

    fn client(server: &MockServer) -> Datadog {
        let mut dd = Datadog::new("k".into(), "a".into(), "datadoghq.eu".into());
        dd.api_base = server.uri();
        dd.retry = crate::resilience::RetryPolicy {
            max_attempts: 2,
            base_delay: std::time::Duration::from_millis(1),
            max_delay: std::time::Duration::from_millis(5),
        };
        dd
    }

    const FROM: &str = "2022-06-20T17:00:00Z";
    const TO: &str = "2022-06-20T17:15:00Z";

    /// Matches the documented search body for the window and query, carrying `cursor`
    /// (or none for the first page).
    fn search(cursor: Option<&'static str>) -> wiremock::MockBuilder {
        Mock::given(method("POST"))
            .and(path("/api/v2/events/search"))
            .and(header("DD-API-KEY", "k"))
            .and(header("DD-APPLICATION-KEY", "a"))
            .and(body_partial_json(json!({
                "filter": {"from": FROM, "to": TO, "query": DEFAULT_QUERY},
                "page": {"limit": 1000},
                "sort": "timestamp"
            })))
            .and(move |req: &wiremock::Request| {
                let body: Value = serde_json::from_slice(&req.body).unwrap();
                body["page"]["cursor"].as_str() == cursor
            })
    }

    fn cursor_of(name: &str) -> &'static str {
        let v = fixture(name);
        Box::leak(
            v["meta"]["page"]["after"]
                .as_str()
                .unwrap()
                .to_string()
                .into_boxed_str(),
        )
    }

    /// The recorded three-page search: two pages of events, then an empty page.
    #[tokio::test]
    async fn follows_the_recorded_cursor_pagination() {
        let server = MockServer::start().await;
        let (c1, c2) = (
            cursor_of("events_search_page1.json"),
            cursor_of("events_search_page2.json"),
        );
        for (cursor, page) in [
            (None, "events_search_page1.json"),
            (Some(c1), "events_search_page2.json"),
            (Some(c2), "events_search_page3.json"),
        ] {
            search(cursor)
                .respond_with(ResponseTemplate::new(200).set_body_json(fixture(page)))
                .expect(1)
                .mount(&server)
                .await;
        }

        let docs = client(&server)
            .search_change_events(FROM, TO, DEFAULT_QUERY)
            .await
            .unwrap();
        assert_eq!(docs.len(), 3);
        let first = &docs[0];
        assert_eq!(
            first.id,
            "change_AgAAAYGCFTWI2hRvPgAAAAAAAAAYAAAAAEFZR0NGVFdJQUFEUjlDVlFydGhfS19rQgAAACQAAAAAMDE4MTgyMTUtMzU4OC00MDdhLWEyMDgtMjRlYzA5NjU1ZmNi"
        );
        assert_eq!(first.kind, SourceKind::Change);
        assert_eq!(first.title, "[Synthetics] EVMGT pipeline test");
        assert_eq!(first.timestamp.as_deref(), Some("2022-06-20T17:07:17Z"));
        // `service: "undefined"` is a placeholder, not a service; `environment:` counts.
        assert_eq!(first.service, "");
        assert_eq!(first.environment, "staging");
        assert_eq!(first.metadata["source"], "my_apps");
        assert_eq!(first.metadata["change_type"], "change");
        assert!(first.text.contains("Synthetics test check that this event"));
        assert!(
            first
                .source_uri
                .starts_with("https://app.datadoghq.eu/event/explorer?event=AgAAAY")
        );
        let times: Vec<_> = docs.iter().map(|d| d.timestamp.clone().unwrap()).collect();
        assert!(times.windows(2).all(|w| w[0] <= w[1]), "{times:?}");
    }

    #[tokio::test]
    async fn rejects_a_repeated_cursor_and_error_statuses() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v2/events/search"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(fixture("events_search_page1.json")),
            )
            .expect(2)
            .mount(&server)
            .await;
        assert!(
            client(&server)
                .search_change_events(FROM, TO, DEFAULT_QUERY)
                .await
                .is_err()
        );

        let server = MockServer::start().await;
        search(None)
            .respond_with(ResponseTemplate::new(403))
            .expect(1)
            .mount(&server)
            .await;
        let err = client(&server)
            .search_change_events(FROM, TO, DEFAULT_QUERY)
            .await
            .unwrap_err();
        assert!(format!("{err:#}").contains("change events"), "{err:#}");

        // 429 is retried.
        let server = MockServer::start().await;
        search(None)
            .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "0"))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        search(None)
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": []})))
            .expect(1)
            .mount(&server)
            .await;
        let docs = client(&server)
            .search_change_events(FROM, TO, DEFAULT_QUERY)
            .await
            .unwrap();
        assert!(docs.is_empty());
    }

    fn deploy(extra_tags: &[&str], inner: Value) -> Value {
        let mut tags = vec!["service:Checkout", "env:prod", "version:2.14.0-RC1"];
        tags.extend_from_slice(extra_tags);
        json!({"id": "ev-1", "type": "event", "attributes": {
            "timestamp": "2026-03-05T12:55:00.000Z",
            "message": "Rolled out checkout 2.14.0 to prod",
            "tags": tags,
            "attributes": inner
        }})
    }

    #[test]
    fn reads_service_env_version_commit_and_author() {
        let e = deploy(
            &["git.commit.sha:AbC123", "source:argocd"],
            json!({"title": "Deploy checkout 2.14.0", "author": {"name": "åsa.öberg", "type": "user"}}),
        );
        let d = change_document(&e, "datadoghq.com").unwrap();
        assert_eq!(
            (d.service.as_str(), d.environment.as_str()),
            ("checkout", "prod")
        );
        assert_eq!(d.metadata["version"], "2.14.0-RC1", "version case is kept");
        assert_eq!(d.metadata["commit"], "AbC123");
        assert_eq!(d.metadata["author"], "åsa.öberg");
        assert_eq!(d.metadata["source"], "argocd");
        assert_eq!(d.metadata["change_type"], "deployment");
        assert_eq!(d.timestamp.as_deref(), Some("2026-03-05T12:55:00Z"));
        assert_eq!(
            d.text,
            "Deploy checkout 2.14.0\nChange type: deployment\nTime: 2026-03-05T12:55:00Z\n\
             Service: checkout\nEnvironment: prod\nVersion: 2.14.0-RC1\nCommit: AbC123\n\
             Author: åsa.öberg\nSource: argocd\n\nRolled out checkout 2.14.0 to prod"
        );
    }

    #[test]
    fn change_event_attributes_and_fallbacks() {
        // A configuration change: service from the impacted resource, time from the
        // inner POSIX milliseconds, title from the first message line.
        let e = json!({"id": "cfg", "attributes": {
            "message": "\n  max_connections 50 -> 20\nmore",
            "tags": ["environment:Staging"],
            "attributes": {
                "timestamp": 1_772_715_300_000i64,
                "changed_resource": {"name": "db.pool", "type": "configuration"},
                "impacted_resources": [{"name": "ignored", "type": "host"},
                                       {"name": "Payments", "type": "service"}],
                "author": {"name": "terraform", "type": "automation"}
            }
        }});
        let d = change_document(&e, "datadoghq.com").unwrap();
        assert_eq!(d.title, "max_connections 50 -> 20");
        assert_eq!(
            (d.service.as_str(), d.environment.as_str()),
            ("payments", "staging")
        );
        assert_eq!(d.metadata["change_type"], "configuration");
        assert_eq!(d.metadata["changed_resource"], "db.pool (configuration)");
        assert_eq!(d.timestamp.as_deref(), Some("2026-03-05T12:55:00Z"));

        // Missing everything but an ID and a time.
        let bare = json!({"id": "x", "attributes": {"timestamp": "2026-03-05T12:00:00+01:00"}});
        let d = change_document(&bare, "datadoghq.com").unwrap();
        assert_eq!(d.title, "Change event");
        assert_eq!(
            d.text,
            "Change event\nChange type: change\nTime: 2026-03-05T11:00:00Z"
        );
        assert_eq!((d.service.as_str(), d.environment.as_str()), ("", ""));
        assert_eq!(d.metadata["tags"], json!([]));

        // No ID or no time: skipped.
        assert!(
            change_document(
                &json!({"attributes": {"timestamp": "2026-03-05T12:00:00Z"}}),
                "s"
            )
            .is_none()
        );
        assert!(
            change_document(
                &json!({"id": "y", "attributes": {"timestamp": "yesterday"}}),
                "s"
            )
            .is_none()
        );
    }

    #[test]
    fn long_multibyte_messages_are_bounded_at_char_boundaries() {
        let message = format!("{}{}", "å".repeat(3000), "🚀".repeat(1000));
        let mut e = deploy(&[], json!({"title": "t".repeat(1000)}));
        e["attributes"]["message"] = json!(message);
        let d = change_document(&e, "datadoghq.com").unwrap();
        assert!(d.title.len() <= VALUE_MAX_BYTES + TRUNCATION_MARKER.len());
        let body = d.text.split_once("\n\n").unwrap().1;
        assert!(body.len() <= MESSAGE_MAX_BYTES + TRUNCATION_MARKER.len());
        assert!(body.ends_with(TRUNCATION_MARKER));
        assert!(body.starts_with("ååå"));
    }
}
