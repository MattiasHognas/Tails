//! Incident documents: what an incident says about itself (summary, root cause,
//! detection, resolution, customer impact, people, services and times) plus, fetched
//! per incident, its timeline and its postmortem.
//!
//! Endpoints (checked against the Datadog API reference and the v2 OpenAPI spec of
//! `DataDog/datadog-api-client-go`, September 2026):
//! - `GET /api/v2/incidents/search` (documented): the incident objects. Most text lives in
//!   `attributes.fields.<name>.value` (`summary`, `root_cause`, `detection_method`,
//!   `services`, `teams`, custom textbox fields); `title`, `customer_impact_scope`,
//!   `created`, `detected`, `resolved`, `severity`, `state` and `commander` are direct
//!   attributes. Both places are read for every field, since which one Datadog uses has
//!   moved between API versions.
//! - `GET /api/v2/incidents/{incident_id}/attachments` (documented, Preview): the
//!   postmortem attachment, `attributes.attachment_type == "postmortem"` with
//!   `attributes.attachment.documentUrl` pointing at a notebook. Requested without
//!   `filter[attachment_type]`, because the reference and the spec disagree on its values
//!   (`postmortem` vs `1`); the type is filtered here.
//! - `GET /api/v1/notebooks/{notebook_id}` (documented): the postmortem text, the
//!   `markdown` cells of `data.attributes.cells`.
//! - `GET /api/v2/incidents/{incident_id}/timeline` (**not documented**): the public
//!   reference only has the timeline cell *create* shape (`incident_timeline_cells` with
//!   `attributes.cell_type` and `attributes.content.content`), and integrations POST it
//!   to this path. The read shape assumed here is that shape in a `data` array. When the
//!   endpoint answers with a client error, timelines are not requested again for the rest
//!   of the run.
//!
//! A failed timeline or postmortem fetch never fails the run: the incident is indexed
//! without it and the failure is logged.

use crate::datadog::{Datadog, normalize_scope_value};
use crate::domain::{RagDocument, SourceKind};
use crate::error::UpstreamError;
use crate::resilience::send_with_retry;
use crate::text::{TRUNCATION_MARKER, truncate_bytes, truncate_with_marker};
use futures::{StreamExt, stream};
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};

/// Incidents whose timeline and postmortem are fetched at the same time.
pub const DETAIL_CONCURRENCY: usize = 4;
/// Incidents per fetch whose timeline and postmortem are requested (up to three calls
/// each); later ones in the window are indexed from the search result alone.
pub const MAX_DETAILED_INCIDENTS: usize = 100;
/// Timeline entries kept per incident (the oldest ones).
pub const MAX_TIMELINE_ENTRIES: usize = 50;
/// Longest timeline entry, in bytes.
pub const TIMELINE_ENTRY_MAX_BYTES: usize = 1000;
/// Longest timeline section, in bytes.
pub const TIMELINE_MAX_BYTES: usize = 8000;
/// Longest postmortem text, in bytes.
pub const POSTMORTEM_MAX_BYTES: usize = 12_000;
/// Longest single field value (summary, root cause, ...), in bytes.
pub const FIELD_MAX_BYTES: usize = 4000;

/// Free-text fields shown first, in this order, as (field name, label).
const TEXT_FIELDS: [(&str, &str); 4] = [
    ("summary", "Summary"),
    ("root_cause", "Root cause"),
    ("resolution", "Resolution"),
    ("remediation", "Remediation"),
];
/// Fields rendered elsewhere in the document (or not at all).
const HANDLED_FIELDS: [&str; 11] = [
    "summary",
    "root_cause",
    "resolution",
    "remediation",
    "severity",
    "state",
    "services",
    "teams",
    "detection_method",
    "env",
    "environment",
];

/// One timeline entry: a note, status update or other cell.
#[derive(Debug, Clone, PartialEq)]
pub struct TimelineEntry {
    pub at: Option<String>,
    /// `cell_type`, e.g. `markdown`.
    pub kind: String,
    pub text: String,
    pub important: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Postmortem {
    pub title: String,
    pub url: String,
    /// The notebook's markdown, bounded; empty when it could not be read.
    pub text: String,
}

/// What the per-incident endpoints added to an incident.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct IncidentDetails {
    pub timeline: Vec<TimelineEntry>,
    pub postmortem: Option<Postmortem>,
}

fn non_empty(v: &Value) -> Option<String> {
    v.as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Every non-empty string of an incident field's `value` (a string or an array).
fn field_values(fields: &Value, name: &str) -> Vec<String> {
    let value = &fields[name]["value"];
    match value {
        Value::Array(items) => items.iter().filter_map(non_empty).collect(),
        v => non_empty(v).into_iter().collect(),
    }
}

/// A text attribute, read from `attributes.<name>` or `attributes.fields.<name>.value`.
fn attr_or_field(attrs: &Value, name: &str) -> Option<String> {
    non_empty(&attrs[name]).or_else(|| field_values(&attrs["fields"], name).into_iter().next())
}

/// Values of a multi-value field, from `attributes.fields.<name>` or `attributes.<name>`.
fn attr_or_field_list(attrs: &Value, name: &str) -> Vec<String> {
    let from_fields = field_values(&attrs["fields"], name);
    if !from_fields.is_empty() {
        return from_fields;
    }
    match &attrs[name] {
        Value::Array(items) => items.iter().filter_map(non_empty).collect(),
        v => non_empty(v).into_iter().collect(),
    }
}

/// The commander's name, handle or email: `attributes.commander` is a user relationship
/// with its attributes inlined (like `created_by`), or null.
fn commander(attrs: &Value) -> Option<String> {
    let user = &attrs["commander"]["data"]["attributes"];
    non_empty(&user["name"])
        .or_else(|| non_empty(&user["handle"]))
        .or_else(|| non_empty(&user["email"]))
        .or_else(|| non_empty(&attrs["commander"]))
}

/// `snake_case` field name as a label: `customer_facing_notes` → `Customer facing notes`.
fn label(name: &str) -> String {
    let words = name.replace(['_', '-'], " ");
    let mut chars = words.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn bounded(s: &str, max: usize) -> String {
    truncate_with_marker(s.trim(), max, TRUNCATION_MARKER).into_owned()
}

/// Whether the search result says the incident has attachments. Unknown (no
/// relationship in the response) counts as yes, so the attachments are requested.
fn may_have_attachments(incident: &Value) -> bool {
    match incident["relationships"]["attachments"]["data"].as_array() {
        Some(items) => !items.is_empty(),
        None => true,
    }
}

/// Timeline entries from a timeline response, oldest first, bounded.
pub fn parse_timeline(body: &Value) -> Vec<TimelineEntry> {
    let Some(cells) = body["data"].as_array() else {
        return vec![];
    };
    let mut entries: Vec<TimelineEntry> = cells
        .iter()
        .filter_map(|cell| {
            let a = &cell["attributes"];
            let content = &a["content"];
            let text = non_empty(&content["content"])
                .or_else(|| non_empty(&content["message"]))
                .or_else(|| non_empty(content))
                .or_else(|| non_empty(&a["message"]))?;
            Some(TimelineEntry {
                at: non_empty(&a["display_time"])
                    .or_else(|| non_empty(&a["created"]))
                    .or_else(|| non_empty(&a["modified"])),
                kind: non_empty(&a["cell_type"]).unwrap_or_default(),
                text: truncate_with_marker(&text, TIMELINE_ENTRY_MAX_BYTES, TRUNCATION_MARKER)
                    .into_owned(),
                important: a["important"].as_bool().unwrap_or(false),
            })
        })
        .collect();
    // Stable: entries without a time keep their place relative to each other.
    entries.sort_by(|a, b| a.at.cmp(&b.at));
    entries.truncate(MAX_TIMELINE_ENTRIES);
    entries
}

/// The first postmortem attachment of an attachments response as (title, document URL).
pub fn postmortem_attachment(body: &Value) -> Option<(String, String)> {
    body["data"].as_array()?.iter().find_map(|att| {
        let a = &att["attributes"];
        if a["attachment_type"].as_str()? != "postmortem" {
            return None;
        }
        let url = non_empty(&a["attachment"]["documentUrl"]).unwrap_or_default();
        let title = non_empty(&a["attachment"]["title"]).unwrap_or_else(|| "Postmortem".into());
        Some((title, url))
    })
}

/// The numeric notebook ID in a notebook URL (`.../notebook/123/Postmortem-IR-123`).
pub fn notebook_id(url: &str) -> Option<u64> {
    let rest = url.split("/notebook/").nth(1)?;
    rest.split(['/', '?', '#']).next()?.parse().ok()
}

/// The markdown cells of a notebook response, joined and bounded.
pub fn notebook_markdown(body: &Value) -> String {
    let text = body["data"]["attributes"]["cells"]
        .as_array()
        .map(|cells| {
            cells
                .iter()
                .map(|c| &c["attributes"]["definition"])
                .filter(|d| d["type"] == "markdown")
                .filter_map(|d| non_empty(&d["text"]))
                .collect::<Vec<_>>()
                .join("\n\n")
        })
        .unwrap_or_default();
    bounded(&text, POSTMORTEM_MAX_BYTES)
}

/// A client error other than 429: asking again will not help this run.
fn is_permanent(e: &UpstreamError) -> bool {
    matches!(e, UpstreamError::Status { status, .. } if (400..500).contains(status) && *status != 429)
}

impl Datadog {
    async fn get_json(
        &self,
        what: &str,
        url: &str,
        query: &[(&str, String)],
    ) -> Result<Value, UpstreamError> {
        let r = send_with_retry(&self.retry, what, || {
            self.http
                .get(url)
                .header("DD-API-KEY", &self.api_key)
                .header("DD-APPLICATION-KEY", &self.app_key)
                .query(query)
        })
        .await
        .map_err(|f| f.error)?;
        r.json().await.map_err(UpstreamError::from_reqwest)
    }

    /// `GET /api/v2/incidents/{id}/timeline` (undocumented read; see the module docs).
    /// One request without paging parameters, since none are documented; the response
    /// is bounded here.
    pub async fn incident_timeline(&self, id: &str) -> Result<Vec<TimelineEntry>, UpstreamError> {
        let url = format!(
            "{}/api/v2/incidents/{}/timeline",
            self.api_base,
            urlencoding::encode(id)
        );
        let body = self
            .get_json("datadog incident timeline", &url, &[])
            .await?;
        Ok(parse_timeline(&body))
    }

    /// The incident's postmortem: its attachment, then the notebook it links to. A
    /// notebook that cannot be read leaves the title and link.
    pub async fn incident_postmortem(&self, id: &str) -> Result<Option<Postmortem>, UpstreamError> {
        let url = format!(
            "{}/api/v2/incidents/{}/attachments",
            self.api_base,
            urlencoding::encode(id)
        );
        let body = self
            .get_json("datadog incident attachments", &url, &[])
            .await?;
        let Some((title, url)) = postmortem_attachment(&body) else {
            return Ok(None);
        };
        let text = match notebook_id(&url) {
            Some(nb) => {
                let nb_url = format!("{}/api/v1/notebooks/{}", self.api_base, nb);
                match self.get_json("datadog notebook", &nb_url, &[]).await {
                    Ok(body) => notebook_markdown(&body),
                    Err(e) => {
                        tracing::warn!(incident = id, notebook = nb, error = %e, "could not read postmortem notebook; indexing its title only");
                        String::new()
                    }
                }
            }
            None => String::new(),
        };
        Ok(Some(Postmortem { title, url, text }))
    }

    /// Timeline and postmortem of one incident; failures are logged and leave that part
    /// out. `timelines` is cleared when the timeline endpoint answers with a client error.
    async fn incident_details(&self, incident: &Value, timelines: &AtomicBool) -> IncidentDetails {
        let Some(id) = incident["id"].as_str() else {
            return IncidentDetails::default();
        };
        let timeline = if timelines.load(Ordering::Relaxed) {
            match self.incident_timeline(id).await {
                Ok(t) => t,
                Err(e) => {
                    if is_permanent(&e) && timelines.swap(false, Ordering::Relaxed) {
                        tracing::warn!(incident = id, error = %e, "incident timeline endpoint unavailable; indexing incidents without timelines this run");
                    } else if !is_permanent(&e) {
                        tracing::warn!(incident = id, error = %e, "could not fetch incident timeline; indexing the incident without it");
                    }
                    vec![]
                }
            }
        } else {
            vec![]
        };
        let postmortem = if may_have_attachments(incident) {
            self.incident_postmortem(id).await.unwrap_or_else(|e| {
                tracing::warn!(incident = id, error = %e, "could not fetch incident postmortem; indexing the incident without it");
                None
            })
        } else {
            None
        };
        IncidentDetails {
            timeline,
            postmortem,
        }
    }

    /// Documents for `incidents` (search result `data` objects), in order, each with its
    /// timeline and postmortem for the first [`MAX_DETAILED_INCIDENTS`].
    pub async fn incident_documents(&self, incidents: Vec<Value>) -> Vec<RagDocument> {
        if incidents.len() > MAX_DETAILED_INCIDENTS {
            tracing::warn!(
                incidents = incidents.len(),
                detailed = MAX_DETAILED_INCIDENTS,
                "more incidents than the per-run detail budget; the rest are indexed without timeline and postmortem"
            );
        }
        let timelines = AtomicBool::new(true);
        let timelines = &timelines;
        stream::iter(incidents.into_iter().enumerate())
            .map(|(i, incident)| async move {
                let details = if i < MAX_DETAILED_INCIDENTS {
                    self.incident_details(&incident, timelines).await
                } else {
                    IncidentDetails::default()
                };
                self.incident_document(&incident, &details)
            })
            .buffered(DETAIL_CONCURRENCY)
            .collect()
            .await
    }

    /// One incident as a document. The most answer-relevant text (summary, root cause,
    /// resolution) comes first so the first chunk and the prompt excerpt carry it.
    pub fn incident_document(&self, incident: &Value, details: &IncidentDetails) -> RagDocument {
        let id = incident["id"].as_str().unwrap_or("").to_string();
        let attrs = &incident["attributes"];
        let fields = &attrs["fields"];
        let title = attrs["title"].as_str().unwrap_or("").to_string();
        let customer_impact = non_empty(&attrs["customer_impact_scope"]).unwrap_or_default();
        let severity = attr_or_field(attrs, "severity").unwrap_or_else(|| "UNKNOWN".to_string());
        let state = attr_or_field(attrs, "state").unwrap_or_default();
        let created = attrs["created"].as_str().map(str::to_string);
        let services = attr_or_field_list(attrs, "services");
        let teams = attr_or_field_list(attrs, "teams");
        let service = normalize_scope_value(services.first().map_or("", String::as_str));
        let environment = normalize_scope_value(
            &attr_or_field(attrs, "env")
                .or_else(|| attr_or_field(attrs, "environment"))
                .unwrap_or_default(),
        );
        let commander = commander(attrs);
        let detection = attr_or_field(attrs, "detection_method");

        let mut sections: Vec<String> = vec![title.clone()];
        let mut facts = vec![format!("Severity: {severity}")];
        if !state.is_empty() {
            facts.push(format!("State: {state}"));
        }
        if let Some(c) = &commander {
            facts.push(format!("Commander: {c}"));
        }
        if !services.is_empty() {
            facts.push(format!("Services: {}", services.join(", ")));
        }
        if !teams.is_empty() {
            facts.push(format!("Teams: {}", teams.join(", ")));
        }
        if let Some(d) = &detection {
            facts.push(format!("Detection method: {d}"));
        }
        for (key, name) in [
            ("created", "Created"),
            ("detected", "Detected"),
            ("resolved", "Resolved"),
        ] {
            if let Some(t) = non_empty(&attrs[key]) {
                facts.push(format!("{name}: {t}"));
            }
        }
        sections.push(facts.join("\n"));

        let mut text_field = |name: &str, label: &str| {
            if let Some(v) = attr_or_field(attrs, name) {
                sections.push(format!("{label}: {}", bounded(&v, FIELD_MAX_BYTES)));
            }
        };
        for (name, l) in TEXT_FIELDS {
            text_field(name, l);
        }
        if !customer_impact.is_empty() {
            sections.push(format!(
                "Customer Impact: {}",
                bounded(&customer_impact, FIELD_MAX_BYTES)
            ));
        }
        // Other free-text fields an organization defined (textbox), by name.
        if let Some(map) = fields.as_object() {
            let mut custom: Vec<(&String, String)> = map
                .iter()
                .filter(|(k, v)| {
                    !HANDLED_FIELDS.contains(&k.as_str())
                        && !TEXT_FIELDS.iter().any(|(n, _)| n == k)
                        && v["type"] == "textbox"
                })
                .filter_map(|(k, v)| Some((k, non_empty(&v["value"])?)))
                .collect();
            custom.sort();
            for (k, v) in custom {
                sections.push(format!("{}: {}", label(k), bounded(&v, FIELD_MAX_BYTES)));
            }
        }

        if let Some(pm) = &details.postmortem {
            let mut s = format!("Postmortem: {}", pm.title);
            if !pm.text.is_empty() {
                s.push('\n');
                s.push_str(&pm.text);
            }
            sections.push(s);
        }
        if !details.timeline.is_empty() {
            let mut lines = vec!["Timeline:".to_string()];
            let mut used = 0usize;
            for e in &details.timeline {
                let flat = e.text.split_whitespace().collect::<Vec<_>>().join(" ");
                let mut line = match &e.at {
                    Some(at) => format!("- {at}: {flat}"),
                    None => format!("- {flat}"),
                };
                if e.important {
                    line.push_str(" (important)");
                }
                if used + line.len() > TIMELINE_MAX_BYTES {
                    let room = TIMELINE_MAX_BYTES.saturating_sub(used);
                    let head = truncate_bytes(&line, room);
                    if !head.is_empty() {
                        lines.push(format!("{head}{TRUNCATION_MARKER}"));
                    }
                    break;
                }
                used += line.len() + 1;
                lines.push(line);
            }
            sections.push(lines.join("\n"));
        }
        let text = sections.join("\n\n");

        let mut metadata = serde_json::Map::new();
        metadata.insert("severity".into(), Value::String(severity));
        metadata.insert("state".into(), Value::String(state));
        metadata.insert("customer_impact".into(), Value::String(customer_impact));
        if let Some(c) = commander {
            metadata.insert("commander".into(), Value::String(c));
        }
        if services.len() > 1 {
            metadata.insert("services".into(), serde_json::json!(services));
        }
        if !teams.is_empty() {
            metadata.insert("teams".into(), serde_json::json!(teams));
        }
        for key in ["detected", "resolved"] {
            if let Some(t) = non_empty(&attrs[key]) {
                metadata.insert(key.into(), Value::String(t));
            }
        }
        if !details.timeline.is_empty() {
            metadata.insert(
                "timeline_entries".into(),
                serde_json::json!(details.timeline.len()),
            );
        }
        if let Some(pm) = &details.postmortem {
            metadata.insert("postmortem_url".into(), Value::String(pm.url.clone()));
        }

        // The web UI addresses incidents by their numeric public ID.
        let app_id = attrs["public_id"]
            .as_u64()
            .map(|n| n.to_string())
            .unwrap_or_else(|| id.clone());

        RagDocument {
            id: format!("incident_{}", id),
            title,
            text,
            source_uri: format!("https://app.{}/incidents/{}", self.site, app_id),
            kind: SourceKind::Incident,
            timestamp: created,
            service,
            environment,
            metadata,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resilience::RetryPolicy;
    use serde_json::json;
    use wiremock::matchers::{method, path, query_param_is_missing};
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
        dd.retry = RetryPolicy::none();
        dd
    }

    fn dd() -> Datadog {
        Datadog::new("k".into(), "a".into(), "datadoghq.eu".into())
    }

    /// A search result incident with every documented text field.
    fn rich_incident() -> Value {
        json!({
            "id": "inc-1",
            "type": "incidents",
            "attributes": {
                "public_id": 7,
                "title": "Checkout outage",
                "created": "2026-02-20T13:05:00+00:00",
                "detected": "2026-02-20T13:07:00+00:00",
                "resolved": "2026-02-20T13:55:00+00:00",
                "customer_impact_scope": "Orders failed for 50 minutes.",
                "severity": "SEV-1",
                "state": "resolved",
                "commander": {"data": {"type": "users", "id": "u", "attributes": {
                    "name": null, "handle": "kim@example.com"}}},
                "fields": {
                    "severity": {"type": "dropdown", "value": "SEV-1"},
                    "services": {"type": "autocomplete", "value": ["Checkout", "payments"]},
                    "teams": {"type": "autocomplete", "value": ["shop"]},
                    "detection_method": {"type": "dropdown", "value": "monitor"},
                    "summary": {"type": "textbox", "value": "Requests hung."},
                    "root_cause": {"type": "textbox", "value": "Connection leak in promo lookup."},
                    "customer_facing_notes": {"type": "textbox", "value": "Apology sent."},
                    "empty_notes": {"type": "textbox", "value": null},
                    "region": {"type": "dropdown", "value": "eu1"}
                }
            },
            "relationships": {"attachments": {"data": []}}
        })
    }

    #[test]
    fn document_reads_attributes_and_fields() {
        let doc = dd().incident_document(&rich_incident(), &IncidentDetails::default());
        assert_eq!(doc.id, "incident_inc-1");
        assert_eq!(doc.service, "checkout");
        assert_eq!(doc.source_uri, "https://app.datadoghq.eu/incidents/7");
        let t = &doc.text;
        for want in [
            "Severity: SEV-1",
            "State: resolved",
            "Commander: kim@example.com",
            "Services: Checkout, payments",
            "Teams: shop",
            "Detection method: monitor",
            "Detected: 2026-02-20T13:07:00+00:00",
            "Resolved: 2026-02-20T13:55:00+00:00",
            "Summary: Requests hung.",
            "Root cause: Connection leak in promo lookup.",
            "Customer Impact: Orders failed for 50 minutes.",
            "Customer facing notes: Apology sent.",
        ] {
            assert!(t.contains(want), "{want} missing from {t}");
        }
        // Dropdowns other than the known ones and empty textboxes are left out.
        assert!(!t.contains("eu1") && !t.contains("Empty notes"), "{t}");
        // The answer-relevant text precedes the impact and custom fields.
        assert!(t.find("Root cause").unwrap() < t.find("Customer Impact").unwrap());
        assert_eq!(doc.metadata["commander"], "kim@example.com");
        assert_eq!(doc.metadata["services"], json!(["Checkout", "payments"]));
        assert_eq!(doc.metadata["resolved"], "2026-02-20T13:55:00+00:00");
        assert!(!doc.metadata.contains_key("timeline_entries"));
    }

    /// Fields may come as direct attributes instead of `fields` entries; both work.
    #[test]
    fn document_reads_direct_attributes_and_tolerates_missing_everything() {
        let doc = dd().incident_document(
            &json!({"id": "x", "attributes": {
                "title": "t", "root_cause": "disk full", "summary": "  ",
                "services": ["Search"], "state": null,
                "commander": {"data": null}
            }}),
            &IncidentDetails::default(),
        );
        assert!(doc.text.contains("Root cause: disk full"), "{}", doc.text);
        assert!(!doc.text.contains("Summary"));
        assert_eq!(doc.service, "search");
        assert_eq!(doc.metadata["severity"], "UNKNOWN");
        assert!(!doc.metadata.contains_key("commander"));

        let bare = dd().incident_document(&json!({}), &IncidentDetails::default());
        assert_eq!(bare.id, "incident_");
        assert_eq!(bare.text, "\n\nSeverity: UNKNOWN");
    }

    #[test]
    fn recorded_search_incident_has_no_empty_sections() {
        let incident =
            &fixture("incidents_search_page1.json")["data"]["attributes"]["incidents"][0]["data"];
        let doc = dd().incident_document(incident, &IncidentDetails::default());
        assert!(doc.text.starts_with("Test-Go-"));
        // Null summary/root cause and an empty impact scope add nothing.
        assert!(!doc.text.contains("Summary") && !doc.text.contains("Customer Impact"));
        assert!(doc.text.contains("Detection method: unknown"));
        assert!(!may_have_attachments(incident));
        assert!(may_have_attachments(&json!({"id": "x"})));
    }

    #[test]
    fn timeline_fixture_is_parsed_in_time_order() {
        let entries = parse_timeline(&fixture("incident_timeline.json"));
        let texts: Vec<_> = entries.iter().map(|e| e.text.as_str()).collect();
        assert_eq!(
            texts,
            [
                "Paged the payments team: card authorisations timing out at the bank gateway.",
                "Rolled back checkout-api to v2.14.1; p95 recovering.",
                "Status changed from active to stable",
            ]
        );
        assert!(entries[1].important);
        assert_eq!(entries[2].kind, "incident_status_change");
        assert_eq!(entries[1].at.as_deref(), Some("2026-03-11T10:40:00+00:00"));
        assert!(parse_timeline(&json!({"errors": ["nope"]})).is_empty());
        let plain = parse_timeline(&json!({"data": [{"attributes": {"content": "plain"}}]}));
        assert_eq!(plain[0].text, "plain");
        assert!(plain[0].at.is_none());
    }

    #[test]
    fn timeline_and_postmortem_are_bounded_and_utf8_safe() {
        let cells: Vec<Value> = (0..80)
            .map(|i| {
                json!({"attributes": {"cell_type": "markdown",
                    "created": format!("2026-03-11T10:{:02}:00Z", i % 60),
                    "content": {"content": format!("{i} {}", "åäö👩\u{200D}💻決済".repeat(200))}}})
            })
            .collect();
        let entries = parse_timeline(&json!({ "data": cells }));
        assert_eq!(entries.len(), MAX_TIMELINE_ENTRIES);
        for e in &entries {
            assert!(e.text.len() <= TIMELINE_ENTRY_MAX_BYTES + TRUNCATION_MARKER.len());
            assert!(e.text.ends_with(TRUNCATION_MARKER));
            assert!(!e.text.contains("\u{200D} …"), "a joiner never dangles");
        }
        let md = |t: String| json!({"attributes": {"definition": {"type": "markdown", "text": t}}});
        let nb = json!({"data": {"attributes": {"cells": [
            md("# Postmortem ✅".into()),
            {"attributes": {"definition": {"type": "timeseries", "requests": []}}},
            md("ö".repeat(POSTMORTEM_MAX_BYTES))
        ]}}});
        let text = notebook_markdown(&nb);
        assert!(text.starts_with("# Postmortem ✅\n\nö"));
        assert!(text.len() <= POSTMORTEM_MAX_BYTES + TRUNCATION_MARKER.len());

        let mut incident = rich_incident();
        incident["attributes"]["fields"]["root_cause"]["value"] = json!("🔥".repeat(5000));
        let doc = dd().incident_document(
            &incident,
            &IncidentDetails {
                timeline: entries,
                postmortem: Some(Postmortem {
                    title: "PM".into(),
                    url: "u".into(),
                    text,
                }),
            },
        );
        let timeline = &doc.text[doc.text.find("Timeline:").unwrap()..];
        assert!(
            timeline.len() <= "Timeline:\n".len() + TIMELINE_MAX_BYTES + TRUNCATION_MARKER.len()
        );
        assert!(timeline.ends_with(TRUNCATION_MARKER));
        let root = doc
            .text
            .lines()
            .find(|l| l.starts_with("Root cause"))
            .unwrap();
        assert!(root.len() <= "Root cause: ".len() + FIELD_MAX_BYTES + TRUNCATION_MARKER.len());
        assert!(doc.text.contains("Postmortem: PM\n# Postmortem ✅"));
        assert_eq!(doc.metadata["timeline_entries"], MAX_TIMELINE_ENTRIES);
    }

    #[test]
    fn postmortem_attachment_and_notebook_id() {
        let (title, url) = postmortem_attachment(&fixture("incident_attachments.json")).unwrap();
        assert!(title.starts_with("Test-List_incident_attachments"));
        // The recorded test URL has no numeric notebook ID.
        assert_eq!(notebook_id(&url), None);
        assert_eq!(
            notebook_id("https://app.datadoghq.com/notebook/123/Postmortem-IR-123"),
            Some(123)
        );
        assert_eq!(
            notebook_id("https://app.datadoghq.eu/notebook/42?tpl=x"),
            Some(42)
        );
        assert_eq!(notebook_id("https://example.com/doc/1"), None);
        let links_only = json!({"data": [{"attributes": {"attachment_type": "link",
            "attachment": {"documentUrl": "https://x", "title": "Status"}}}]});
        assert_eq!(postmortem_attachment(&links_only), None);
        assert_eq!(postmortem_attachment(&json!({"data": null})), None);
        assert_eq!(
            notebook_markdown(&fixture("notebook_get.json")),
            "# Test-Get_a_notebook_returns_OK_response-1652349008 notebook text"
        );
    }

    fn search_item(id: &str, attachments: Option<Value>) -> Value {
        let mut v = json!({"id": id, "type": "incidents", "attributes": {
            "title": format!("Incident {id}"), "created": "2026-03-11T10:00:00+00:00"}});
        if let Some(a) = attachments {
            v["relationships"] = json!({"attachments": {"data": a}});
        }
        v
    }

    #[tokio::test]
    async fn details_are_fetched_from_the_documented_endpoints() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v2/incidents/inc-a/timeline"))
            .and(query_param_is_missing("page[size]"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(fixture("incident_timeline.json")),
            )
            .expect(1)
            .mount(&server)
            .await;
        let mut attachments = fixture("incident_attachments.json");
        attachments["data"][0]["attributes"]["attachment"]["documentUrl"] =
            json!("https://app.datadoghq.com/notebook/2466033/Postmortem");
        Mock::given(method("GET"))
            .and(path("/api/v2/incidents/inc-a/attachments"))
            .respond_with(ResponseTemplate::new(200).set_body_json(attachments))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/notebooks/2466033"))
            .respond_with(ResponseTemplate::new(200).set_body_json(fixture("notebook_get.json")))
            .expect(1)
            .mount(&server)
            .await;
        // No attachments according to the search result: not requested.
        Mock::given(method("GET"))
            .and(path("/api/v2/incidents/inc-b/timeline"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": []})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v2/incidents/inc-b/attachments"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;

        let docs = client(&server)
            .incident_documents(vec![
                search_item(
                    "inc-a",
                    Some(json!([{"type": "incident_attachments", "id": "1"}])),
                ),
                search_item("inc-b", Some(json!([]))),
            ])
            .await;
        assert_eq!(docs[0].id, "incident_inc-a");
        let t = &docs[0].text;
        assert!(
            t.contains("Postmortem: Test-List_incident_attachments"),
            "{t}"
        );
        assert!(t.contains("notebook text"), "{t}");
        assert!(
            t.contains(
                "- 2026-03-11T10:40:00+00:00: Rolled back checkout-api to v2.14.1; \
                 p95 recovering. (important)"
            ),
            "{t}"
        );
        assert_eq!(docs[0].metadata["timeline_entries"], 3);
        assert_eq!(
            docs[0].metadata["postmortem_url"],
            "https://app.datadoghq.com/notebook/2466033/Postmortem"
        );
        assert_eq!(
            docs[1].text,
            "Incident inc-b\n\nSeverity: UNKNOWN\nCreated: 2026-03-11T10:00:00+00:00"
        );
    }

    /// A failing timeline, attachments or notebook request leaves that part out; the
    /// incident is still indexed.
    #[tokio::test]
    async fn failed_detail_fetches_degrade_to_the_search_result() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v2/incidents/inc-a/timeline"))
            .respond_with(ResponseTemplate::new(503))
            .expect(2)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v2/incidents/inc-a/attachments"))
            .respond_with(ResponseTemplate::new(500))
            .expect(2)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v2/incidents/inc-b/timeline"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v2/incidents/inc-b/attachments"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [
                {"attributes": {"attachment_type": "postmortem",
                    "attachment": {"documentUrl": "https://app.datadoghq.com/notebook/9/PM", "title": "PM"}}}
            ]})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/notebooks/9"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&server)
            .await;
        let mut dd = client(&server);
        // Transient errors are retried per the policy before giving up.
        dd.retry = RetryPolicy {
            max_attempts: 2,
            base_delay: std::time::Duration::from_millis(1),
            max_delay: std::time::Duration::from_millis(5),
        };

        let docs = dd
            .incident_documents(vec![search_item("inc-a", None), search_item("inc-b", None)])
            .await;
        let ids: Vec<_> = docs.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(ids, ["incident_inc-a", "incident_inc-b"]);
        assert_eq!(
            docs[0].text,
            "Incident inc-a\n\nSeverity: UNKNOWN\nCreated: 2026-03-11T10:00:00+00:00"
        );
        // The attachment is kept by title when its notebook cannot be read.
        assert!(docs[1].text.ends_with("Postmortem: PM"), "{}", docs[1].text);
        assert!(!docs[1].text.contains("Timeline"));
    }

    /// A client error from the (undocumented) timeline endpoint stops timeline requests
    /// for the rest of the run; only the requests already in flight are made.
    #[tokio::test]
    async fn timeline_client_error_stops_timeline_requests() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(wiremock::matchers::path_regex(
                r"^/api/v2/incidents/[^/]+/timeline$",
            ))
            .respond_with(ResponseTemplate::new(404))
            .expect(DETAIL_CONCURRENCY as u64)
            .mount(&server)
            .await;
        let incidents: Vec<Value> = (0..3 * DETAIL_CONCURRENCY)
            .map(|i| search_item(&format!("inc-{i}"), Some(json!([]))))
            .collect();
        let docs = client(&server).incident_documents(incidents).await;
        assert_eq!(docs.len(), 3 * DETAIL_CONCURRENCY);
        assert_eq!(docs[11].id, "incident_inc-11");
    }

    /// Beyond the per-run budget, incidents are indexed from the search result alone.
    #[tokio::test]
    async fn detail_requests_are_capped_per_run() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(wiremock::matchers::path_regex(
                r"^/api/v2/incidents/[^/]+/timeline$",
            ))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(fixture("incident_timeline.json")),
            )
            .expect(MAX_DETAILED_INCIDENTS as u64)
            .mount(&server)
            .await;
        let incidents: Vec<Value> = (0..MAX_DETAILED_INCIDENTS + 5)
            .map(|i| search_item(&format!("inc-{i}"), Some(json!([]))))
            .collect();
        let docs = client(&server).incident_documents(incidents).await;
        assert!(docs[MAX_DETAILED_INCIDENTS - 1].text.contains("Timeline:"));
        assert!(!docs[MAX_DETAILED_INCIDENTS].text.contains("Timeline:"));
    }
}
