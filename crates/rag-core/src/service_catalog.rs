//! Service definitions from the Datadog Software Catalog, one document per service.
//!
//! Source: `GET /api/v2/services/definitions` (permission `apm_service_catalog_read`),
//! paginated with `page[size]` (at most 100) and `page[number]` (from 0); the response
//! carries no total or cursor, so pages are requested until a short one. Each entry is
//! `{"id", "type": "service-definition", "attributes": {"schema": {...}, "meta": {...}}}`
//! where `schema` is in the version the definition was written in (`schema-version`
//! v1, v2, v2.1 or v2.2). A definition in the v3 entity shape (`apiVersion: v3`,
//! `metadata`/`spec`) is also read, as that is the only version that states
//! dependencies (`spec.dependsOn`).
//!
//! The documents answer "who owns X?", "where is the runbook for X?" and "what does X
//! depend on?". They are fully synced (deleted definitions disappear with the next run)
//! and timeless: `timestamp` is `None`, the definition's last modification is kept in
//! metadata only, so neither a time window nor recency decay applies.

use crate::datadog::{Datadog, normalize_scope_value};
use crate::domain::{RagDocument, SourceKind};
use crate::resilience::send_with_retry;
use crate::text::{TRUNCATION_MARKER, truncate_with_marker};
use anyhow::Result;
use serde_json::{Value, json};
use std::collections::HashSet;

/// Maximum `page[size]` of the service definition list.
const PAGE_SIZE: u64 = 100;
/// Upper bound on pages fetched in one run (100 000 services), in case the server
/// ignores `page[number]` and keeps returning full pages.
const MAX_PAGES: u64 = 1000;
/// Longest description kept, in bytes (cut at a char boundary).
const DESCRIPTION_MAX_BYTES: usize = 2000;
/// Longest single value (a contact, link, tag, ...) kept, in bytes.
const VALUE_MAX_BYTES: usize = 300;
/// Most entries kept per list (contacts, links, tags, dependencies, ...).
const MAX_LIST_ITEMS: usize = 30;

/// A service definition read from any supported schema version.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct ServiceDefinition {
    /// `dd-service` (v2+), `info.dd-service` (v1) or `metadata.name` (v3), normalized.
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub team: String,
    pub application: String,
    pub tier: String,
    pub lifecycle: String,
    pub service_type: String,
    pub languages: Vec<String>,
    /// `name (type): contact` lines.
    pub contacts: Vec<String>,
    /// PagerDuty / Opsgenie service URLs.
    pub oncall: Vec<String>,
    /// `(name, type, url)` of runbooks, repos, docs, dashboards, ...
    pub links: Vec<(String, String, String)>,
    pub depends_on: Vec<String>,
    pub tags: Vec<String>,
    pub schema_version: String,
    pub last_modified: String,
}

fn bounded(s: &str, max_bytes: usize) -> String {
    let flat = s.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_with_marker(&flat, max_bytes, TRUNCATION_MARKER).into_owned()
}

fn str_at(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .map(|s| bounded(s, VALUE_MAX_BYTES))
        .unwrap_or_default()
}

/// A list of strings, each bounded; non-strings are skipped.
fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(|s| bounded(s, VALUE_MAX_BYTES))
                .filter(|s| !s.is_empty())
                .take(MAX_LIST_ITEMS)
                .collect()
        })
        .unwrap_or_default()
}

/// `contacts: [{"type", "contact", "name"}]` (v2, v2.1, v2.2, v3).
fn contact_list(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|c| {
                    let contact = str_at(c, "contact");
                    if contact.is_empty() {
                        return None;
                    }
                    let (name, kind) = (str_at(c, "name"), str_at(c, "type"));
                    Some(match (name.is_empty(), kind.is_empty()) {
                        (false, false) => format!("{name} ({kind}): {contact}"),
                        (false, true) => format!("{name}: {contact}"),
                        (true, false) => format!("{kind}: {contact}"),
                        (true, true) => contact,
                    })
                })
                .take(MAX_LIST_ITEMS)
                .collect()
        })
        .unwrap_or_default()
}

/// `[{"name", "type", "url"}]`; `default_type` for lists without a type (v2 `repos`, `docs`).
fn link_list(v: Option<&Value>, default_type: &str) -> Vec<(String, String, String)> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|l| {
                    let url = str_at(l, "url");
                    if url.is_empty() {
                        return None;
                    }
                    let kind = Some(str_at(l, "type"))
                        .filter(|t| !t.is_empty())
                        .unwrap_or_else(|| default_type.to_string());
                    Some((str_at(l, "name"), kind, url))
                })
                .take(MAX_LIST_ITEMS)
                .collect()
        })
        .unwrap_or_default()
}

/// On-call integrations. PagerDuty is a bare URL string in v1/v2 and
/// `{"service-url"}` from v2.1; Opsgenie is `{"service-url", "region"}`; v3 uses
/// `serviceURL`.
fn oncall(integrations: &Value) -> Vec<String> {
    let url = |v: &Value| -> String {
        v.as_str()
            .map(str::to_string)
            .or_else(|| v.get("service-url")?.as_str().map(str::to_string))
            .or_else(|| v.get("serviceURL")?.as_str().map(str::to_string))
            .map(|s| bounded(&s, VALUE_MAX_BYTES))
            .unwrap_or_default()
    };
    [("PagerDuty", "pagerduty"), ("Opsgenie", "opsgenie")]
        .iter()
        .filter_map(|(label, key)| {
            let u = url(&integrations[*key]);
            (!u.is_empty()).then(|| format!("{label}: {u}"))
        })
        .collect()
}

/// Reads a `schema` object in any supported version. `None` without a service name.
pub fn parse_definition(schema: &Value, meta: &Value) -> Option<ServiceDefinition> {
    let version = schema["schema-version"]
        .as_str()
        .or_else(|| schema["apiVersion"].as_str())
        .unwrap_or_default()
        .to_string();
    let mut d = ServiceDefinition {
        schema_version: version.clone(),
        last_modified: str_at(meta, "last-modified-time"),
        ..Default::default()
    };
    if version == "v1" {
        let info = &schema["info"];
        d.name = str_at(info, "dd-service");
        d.display_name = str_at(info, "display-name");
        d.description = schema["info"]["description"]
            .as_str()
            .map(|s| bounded(s, DESCRIPTION_MAX_BYTES))
            .unwrap_or_default();
        d.tier = str_at(info, "service-tier");
        d.team = str_at(&schema["org"], "team");
        d.application = str_at(&schema["org"], "application");
        for (label, key) in [("email", "email"), ("slack", "slack")] {
            let c = str_at(&schema["contact"], key);
            if !c.is_empty() {
                d.contacts.push(format!("{label}: {c}"));
            }
        }
        d.links = link_list(schema.get("external-resources"), "link");
    } else if schema.get("metadata").is_some() && schema.get("dd-service").is_none() {
        // v3 entity shape.
        let md = &schema["metadata"];
        let spec = &schema["spec"];
        d.name = str_at(md, "name");
        d.display_name = str_at(md, "displayName");
        d.description = md["description"]
            .as_str()
            .map(|s| bounded(s, DESCRIPTION_MAX_BYTES))
            .unwrap_or_default();
        d.team = str_at(md, "owner");
        for o in md["additionalOwners"].as_array().into_iter().flatten() {
            let name = str_at(o, "name");
            if !name.is_empty() {
                let kind = str_at(o, "type");
                d.contacts.push(if kind.is_empty() {
                    format!("owner: {name}")
                } else {
                    format!("owner ({kind}): {name}")
                });
            }
        }
        d.contacts.extend(contact_list(md.get("contacts")));
        d.links = link_list(md.get("links"), "link");
        d.tags = strings(md.get("tags"));
        d.tier = str_at(spec, "tier");
        d.lifecycle = str_at(spec, "lifecycle");
        d.service_type = str_at(spec, "type");
        d.languages = strings(spec.get("languages"));
        d.depends_on = strings(spec.get("dependsOn"));
        d.application = strings(spec.get("componentOf")).join(", ");
    } else {
        // v2, v2.1 and v2.2 share their field names; later versions only add fields.
        d.name = str_at(schema, "dd-service");
        d.team = Some(str_at(schema, "team"))
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| str_at(schema, "dd-team"));
        d.description = schema["description"]
            .as_str()
            .map(|s| bounded(s, DESCRIPTION_MAX_BYTES))
            .unwrap_or_default();
        d.application = str_at(schema, "application");
        d.tier = str_at(schema, "tier");
        d.lifecycle = str_at(schema, "lifecycle");
        d.service_type = str_at(schema, "type");
        d.languages = strings(schema.get("languages"));
        d.contacts = contact_list(schema.get("contacts"));
        d.links = link_list(schema.get("links"), "link");
        d.links.extend(link_list(schema.get("repos"), "repo"));
        d.links.extend(link_list(schema.get("docs"), "doc"));
        d.links.truncate(MAX_LIST_ITEMS);
    }
    if d.tags.is_empty() {
        d.tags = strings(schema.get("tags"));
    }
    d.oncall = oncall(&schema["integrations"]);
    d.name = normalize_scope_value(&d.name);
    (!d.name.is_empty()).then_some(d)
}

impl ServiceDefinition {
    /// The indexed document. The text leads with ownership, on-call and links, as the
    /// answer model sees only its first [`crate::rag_service::EXCERPT_MAX_BYTES`].
    pub fn to_document(&self, site: &str) -> RagDocument {
        let mut lines = vec![match self.display_name.as_str() {
            "" => format!("Service catalog entry for {}", self.name),
            dn => format!("Service catalog entry for {} ({dn})", self.name),
        }];
        if !self.description.is_empty() {
            lines.push(self.description.clone());
        }
        let mut push = |label: &str, value: &str| {
            if !value.is_empty() {
                lines.push(format!("{label}: {value}"));
            }
        };
        push("Owner team", &self.team);
        push("Contacts", &self.contacts.join("; "));
        push("On-call", &self.oncall.join("; "));
        let links: Vec<String> = self
            .links
            .iter()
            .map(|(name, kind, url)| match name.as_str() {
                "" => format!("{kind}: {url}"),
                n => format!("{n} ({kind}): {url}"),
            })
            .collect();
        push("Links", &links.join("; "));
        push("Depends on", &self.depends_on.join(", "));
        push("Tier", &self.tier);
        push("Lifecycle", &self.lifecycle);
        push("Type", &self.service_type);
        push("Application", &self.application);
        push("Languages", &self.languages.join(", "));
        push("Tags", &self.tags.join(", "));

        let mut metadata = serde_json::Map::new();
        for (key, value) in [
            ("team", &self.team),
            ("tier", &self.tier),
            ("lifecycle", &self.lifecycle),
            ("schema_version", &self.schema_version),
            ("last_modified", &self.last_modified),
        ] {
            if !value.is_empty() {
                metadata.insert(key.into(), json!(value));
            }
        }
        for (key, value) in [
            ("contacts", &self.contacts),
            ("oncall", &self.oncall),
            ("depends_on", &self.depends_on),
            ("languages", &self.languages),
            ("tags", &self.tags),
        ] {
            if !value.is_empty() {
                metadata.insert(key.into(), json!(value));
            }
        }
        if !self.links.is_empty() {
            let links: Vec<Value> = self
                .links
                .iter()
                .map(|(name, kind, url)| json!({"name": name, "type": kind, "url": url}))
                .collect();
            metadata.insert("links".into(), Value::Array(links));
        }

        RagDocument {
            id: format!("catalog_{}", self.name),
            title: format!("Service: {}", self.name),
            text: lines.join("\n"),
            source_uri: format!(
                "https://app.{site}/services?selectedService={}",
                urlencoding::encode(&self.name)
            ),
            kind: SourceKind::ServiceCatalog,
            timestamp: None,
            service: self.name.clone(),
            environment: String::new(),
            metadata,
        }
    }
}

impl Datadog {
    /// Fetches every service definition with `GET /api/v2/services/definitions`, one
    /// document per service. Transient failures (429, 5xx, timeouts) are retried per
    /// `self.retry`.
    pub async fn list_service_definitions(&self) -> Result<Vec<RagDocument>> {
        let url = format!("{}/api/v2/services/definitions", self.api_base);
        let mut docs = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let mut skipped = 0usize;
        for page in 0..MAX_PAGES {
            let params = [
                ("page[size]", PAGE_SIZE.to_string()),
                ("page[number]", page.to_string()),
            ];
            let response = send_with_retry(&self.retry, "datadog service definitions", || {
                self.http
                    .get(&url)
                    .header("DD-API-KEY", &self.api_key)
                    .header("DD-APPLICATION-KEY", &self.app_key)
                    .query(&params)
            })
            .await
            .map_err(|f| {
                anyhow::Error::new(f.error).context("Failed to fetch service definitions")
            })?;
            let result: Value = response.json().await?;
            let entries = result["data"].as_array().ok_or_else(|| {
                anyhow::anyhow!("Unexpected service definition response: missing data")
            })?;
            let (mut new, mut invalid) = (0usize, 0usize);
            for entry in entries {
                let attrs = &entry["attributes"];
                match parse_definition(&attrs["schema"], &attrs["meta"]) {
                    Some(def) if seen.insert(def.name.clone()) => {
                        new += 1;
                        docs.push(def.to_document(&self.site));
                    }
                    Some(_) => {}
                    None => invalid += 1,
                }
            }
            skipped += invalid;
            if (entries.len() as u64) < PAGE_SIZE {
                break;
            }
            if new == 0 && invalid == 0 {
                anyhow::bail!("Service definition list repeated a page; pagination is ignored");
            }
        }
        if skipped > 0 {
            tracing::warn!(
                skipped,
                "skipped service definitions without a service name"
            );
        }
        Ok(docs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path, query_param};
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

    fn page(number: &str) -> wiremock::MockBuilder {
        Mock::given(method("GET"))
            .and(path("/api/v2/services/definitions"))
            .and(header("DD-API-KEY", "k"))
            .and(header("DD-APPLICATION-KEY", "a"))
            .and(query_param("page[size]", "100"))
            .and(query_param("page[number]", number))
    }

    fn def(name: &str) -> Value {
        json!({"id": name, "type": "service_definitions", "attributes": {
            "schema": {"schema-version": "v2.2", "dd-service": name}, "meta": {}}})
    }

    /// Recorded pages (`page[size]=2`): v2.1 with PagerDuty/Opsgenie objects, v2 with
    /// `repos`/`docs` and PagerDuty as a bare URL, then a v2 definition with an empty
    /// team and schema validation warnings.
    #[tokio::test]
    async fn reads_recorded_v2_and_v2_1_definitions() {
        let server = MockServer::start().await;
        page("0")
            .respond_with(
                ResponseTemplate::new(200).set_body_json(fixture("service_definitions_page1.json")),
            )
            .expect(1)
            .mount(&server)
            .await;
        let docs = client(&server).list_service_definitions().await.unwrap();
        assert_eq!(docs.len(), 2);

        let v21 = &docs[0];
        assert_eq!(
            v21.id,
            "catalog_service-examplecreateorupdateservicedefinitionusingschemav21returnscreatedresponse1680553380"
        );
        assert_eq!(v21.kind, SourceKind::ServiceCatalog);
        assert_eq!(v21.timestamp, None, "catalog entries are undated");
        assert_eq!(
            (v21.service.as_str(), v21.environment.as_str()),
            (&v21.id[8..], "")
        );
        assert_eq!(v21.metadata["team"], "my-team");
        assert_eq!(v21.metadata["schema_version"], "v2.1");
        assert_eq!(v21.metadata["last_modified"], "2023-04-03T20:23:00Z");
        for needle in [
            "Owner team: my-team",
            "Contacts: Team Email (email): contact@datadoghq.com",
            "On-call: PagerDuty: https://my-org.pagerduty.com/service-directory/PMyService; \
             Opsgenie: https://my-org.opsgenie.com/service/123e4567-e89b-12d3-a456-426614174000",
            "Runbook (runbook): https://my-runbook",
            "Source Code (repo): https://github.com/DataDog/schema",
            "Tags: my:tag, service:tag",
        ] {
            assert!(v21.text.contains(needle), "{needle}\n{}", v21.text);
        }
        assert_eq!(
            v21.metadata["links"][1],
            json!({"name": "Runbook", "type": "runbook", "url": "https://my-runbook"})
        );

        let v2 = &docs[1];
        assert_eq!(v2.metadata["schema_version"], "v2");
        for needle in [
            "PagerDuty: https://my-org.pagerduty.com/service-directory/PMyService",
            "Runbook (runbook): https://my-runbook",
            "Source Code (repo): https://github.com/DataDog/schema",
            "Architecture (doc): https://gdrive/mydoc",
        ] {
            assert!(v2.text.contains(needle), "{needle}\n{}", v2.text);
        }

        let server = MockServer::start().await;
        page("0")
            .respond_with(
                ResponseTemplate::new(200).set_body_json(fixture("service_definitions_page2.json")),
            )
            .mount(&server)
            .await;
        let docs = client(&server).list_service_definitions().await.unwrap();
        assert_eq!(docs.len(), 1);
        assert!(
            !docs[0].metadata.contains_key("team"),
            "empty team is left out"
        );
        assert!(!docs[0].text.contains("Owner team"));
        assert!(docs[0].text.contains("AA (slack): AAA"));
        assert_eq!(docs[0].metadata["contacts"].as_array().unwrap().len(), 5);
    }

    /// v2.2 (the documented example), v1, the v3 entity shape with dependencies and a
    /// multibyte description, and a definition without a service name.
    #[tokio::test]
    async fn reads_every_schema_version() {
        let server = MockServer::start().await;
        page("0")
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(fixture("service_definitions_schema_versions.json")),
            )
            .mount(&server)
            .await;
        let docs = client(&server).list_service_definitions().await.unwrap();
        let ids: Vec<&str> = docs.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "catalog_my-service",
                "catalog_myservice",
                "catalog_shopping-cart"
            ],
            "the definition without a name is skipped"
        );

        let v22 = &docs[0];
        assert_eq!(v22.metadata["tier"], "High");
        assert_eq!(v22.metadata["lifecycle"], "sandbox");
        for needle in [
            "Service catalog entry for my-service\nMy service description\nOwner team: my-team",
            "My team channel (slack): https://teams.microsoft.com/myteam",
            "Languages: dotnet, go, java, js, php, python, ruby, c++",
            "Type: web",
            "Application: my-app",
        ] {
            assert!(v22.text.contains(needle), "{needle}\n{}", v22.text);
        }
        assert_eq!(
            v22.source_uri,
            "https://app.datadoghq.eu/services?selectedService=my-service"
        );

        // v1: `info`, `org`, `contact`, `external-resources`; the name is lowercased like
        // every stored service.
        let v1 = &docs[1];
        assert_eq!(v1.service, "myservice");
        assert_eq!(v1.metadata["schema_version"], "v1");
        assert_eq!(v1.metadata["tier"], "Tier 1");
        for needle in [
            "Service catalog entry for myservice (My Service)\nA shopping cart service",
            "email: contact@datadoghq.com; slack: https://yourcompany.slack.com/archives/channel123",
            "PagerDuty: https://my-org.pagerduty.com/service-directory/PMyService",
            "Runbook (runbook): https://my-runbook",
            "Application: E-Commerce",
        ] {
            assert!(v1.text.contains(needle), "{needle}\n{}", v1.text);
        }

        let v3 = &docs[2];
        assert_eq!(v3.metadata["team"], "e-commerce");
        assert_eq!(
            v3.metadata["depends_on"],
            json!(["service:payments", "service:inventory"])
        );
        for needle in [
            "Kundvagn för webbshoppen – håller varukorgen 🛒",
            "owner (team): finance-oncall",
            "Depends on: service:payments, service:inventory",
            "Opsgenie: https://www.opsgenie.com/service/shopping-cart",
            "Lifecycle: production",
        ] {
            assert!(v3.text.contains(needle), "{needle}\n{}", v3.text);
        }
    }

    #[tokio::test]
    async fn follows_page_number_pagination_until_a_short_page() {
        let server = MockServer::start().await;
        let full: Vec<Value> = (0..100).map(|i| def(&format!("svc-{i:03}"))).collect();
        page("0")
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": full})))
            .expect(1)
            .mount(&server)
            .await;
        page("1")
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [def("last")]})))
            .expect(1)
            .mount(&server)
            .await;

        let docs = client(&server).list_service_definitions().await.unwrap();
        assert_eq!(docs.len(), 101);
        assert_eq!(docs[100].id, "catalog_last");
    }

    #[tokio::test]
    async fn a_repeated_full_page_is_an_error_not_an_endless_loop() {
        let server = MockServer::start().await;
        let full: Vec<Value> = (0..100).map(|i| def(&format!("svc-{i:03}"))).collect();
        Mock::given(method("GET"))
            .and(path("/api/v2/services/definitions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": full})))
            .expect(2)
            .mount(&server)
            .await;
        assert!(client(&server).list_service_definitions().await.is_err());
    }

    #[tokio::test]
    async fn retries_rate_limits_and_fails_on_forbidden() {
        let server = MockServer::start().await;
        page("0")
            .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "0"))
            .up_to_n_times(1)
            .expect(1)
            .mount(&server)
            .await;
        page("0")
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [def("a")]})))
            .expect(1)
            .mount(&server)
            .await;
        let docs = client(&server).list_service_definitions().await.unwrap();
        assert_eq!(docs.len(), 1);

        let server = MockServer::start().await;
        page("0")
            .respond_with(ResponseTemplate::new(403))
            .expect(1)
            .mount(&server)
            .await;
        let err = client(&server)
            .list_service_definitions()
            .await
            .unwrap_err();
        assert!(
            format!("{err:#}").contains("service definitions"),
            "{err:#}"
        );
        // The status stays typed, so the indexer can tell a missing permission apart.
        assert!(err.chain().any(|c| matches!(
            c.downcast_ref::<crate::error::UpstreamError>(),
            Some(crate::error::UpstreamError::Status { status: 403, .. })
        )));
    }

    #[test]
    fn missing_fields_and_bounds() {
        // No name: skipped. Wrong types: ignored.
        assert_eq!(
            parse_definition(&json!({"schema-version": "v2.2"}), &Value::Null),
            None
        );
        assert_eq!(parse_definition(&Value::Null, &Value::Null), None);
        let d = parse_definition(
            &json!({"schema-version": "v2.2", "dd-service": " Checkout ", "team": 7,
                    "links": [{"name": "no url", "type": "runbook"}, "junk"],
                    "contacts": [{"type": "email"}], "tags": ["env:prod", 3]}),
            &Value::Null,
        )
        .unwrap();
        assert_eq!(d.name, "checkout");
        assert_eq!(d.team, "");
        assert!(d.links.is_empty() && d.contacts.is_empty());
        assert_eq!(d.tags, ["env:prod"]);
        let doc = d.to_document("datadoghq.com");
        assert_eq!(
            doc.text,
            "Service catalog entry for checkout\nTags: env:prod"
        );
        assert!(!doc.metadata.contains_key("team"));

        // Long multibyte values are cut at a char boundary with a marker.
        let long = "å".repeat(5000);
        let many: Vec<Value> = (0..100)
            .map(|i| json!({"name": format!("l{i}"), "type": "doc", "url": format!("https://x/{i}")}))
            .collect();
        let d = parse_definition(
            &json!({"schema-version": "v2.1", "dd-service": "x", "description": long,
                    "team": long, "links": many}),
            &Value::Null,
        )
        .unwrap();
        assert!(d.description.len() <= DESCRIPTION_MAX_BYTES + TRUNCATION_MARKER.len());
        assert!(d.description.ends_with(TRUNCATION_MARKER));
        assert!(d.team.len() <= VALUE_MAX_BYTES + TRUNCATION_MARKER.len());
        assert_eq!(d.links.len(), MAX_LIST_ITEMS);
    }
}
