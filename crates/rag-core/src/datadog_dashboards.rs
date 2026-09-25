//! Dashboard documents: the list entry (title, description) plus what the dashboard
//! shows, read from its definition: widget titles, widget queries and template
//! variables. The definition also gives the dashboard a service and environment, so a
//! service-scoped question can find it.
//!
//! Endpoints (checked against the Datadog API reference and the v1 OpenAPI spec of
//! `DataDog/datadog-api-client-go`, September 2026, and the recorded
//! `Get-a-dashboard-returns-OK-response` cassette of `datadog-api-client-rust`):
//! - `GET /api/v1/dashboard` (documented): summaries with `id`, `title`, `description`,
//!   `author_handle`, `created_at` and `modified_at`.
//! - `GET /api/v1/dashboard/{dashboard_id}` (documented): the definition, with
//!   `template_variables` (`name`, `prefix`, `defaults` or the deprecated `default`;
//!   null when there are none) and `widgets` (`definition.title`, requests carrying
//!   queries as `q`, `queries[].query` or `search.query`; group widgets nest
//!   `definition.widgets`).
//!
//! # Service and environment
//!
//! Candidates come from the `service`/`env` template variables (prefix `service`/`env`,
//! or that name without a prefix) and from `service:`/`env:` filters in widget queries.
//! Template variable references (`$service`), wildcards and negations are ignored. The
//! stored value is, in order:
//! 1. the template variable's default, when it has exactly one concrete default;
//! 2. otherwise the only value found in widget queries;
//! 3. otherwise the value found in strictly the most widget queries;
//! 4. otherwise none (a tie): the dashboard stays unscoped, so a service-scoped question
//!    excludes it, as before.
//!
//! Every candidate is listed in the text and in `metadata.services` /
//! `metadata.environments`. The payload keeps one `Service` string, so the retrieval
//! filter is unchanged.
//!
//! # API budget
//!
//! A definition costs one call per dashboard. It is stored in the document metadata
//! (`definition`, bounded) together with the list's `modified_at`; when the list reports
//! the same `modified_at` as the stored point, the stored definition is reused and the
//! dashboard is not fetched. At most [`MAX_DEFINITION_FETCHES`] definitions are fetched
//! per run, [`DEFINITION_CONCURRENCY`] at a time; the rest (and dashboards whose fetch
//! failed) keep their stored definition with its old `modified_at`, so a later run fetches
//! them. A dashboard with nothing stored is indexed from its list entry until then.

use crate::datadog::{Datadog, normalize_scope_value};
use crate::domain::{RagDocument, SourceKind};
use crate::error::UpstreamError;
use crate::resilience::send_with_retry;
use crate::text::{TRUNCATION_MARKER, truncate_bytes, truncate_with_marker};
use futures::{StreamExt, stream};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// Dashboard definitions fetched at the same time.
pub const DEFINITION_CONCURRENCY: usize = 4;
/// Dashboard definitions fetched per run at most.
pub const MAX_DEFINITION_FETCHES: usize = 200;
/// Widgets kept per dashboard (depth first, group widgets included).
pub const MAX_WIDGETS: usize = 100;
/// Queries kept per widget.
pub const QUERIES_PER_WIDGET: usize = 5;
/// Longest widget query, in bytes.
pub const WIDGET_QUERY_MAX_BYTES: usize = 300;
/// Longest widget title (or note text), in bytes.
pub const WIDGET_TITLE_MAX_BYTES: usize = 200;
/// Template variables kept per dashboard.
pub const MAX_TEMPLATE_VARIABLES: usize = 20;
/// Longest definition section of the document text, in bytes. Widgets beyond it are
/// not kept at all, so the stored definition (repeated in every chunk's metadata)
/// stays about this size too.
pub const DEFINITION_TEXT_MAX_BYTES: usize = 8000;

/// Metadata key of the stored definition.
pub const DEFINITION_KEY: &str = "definition";
/// Metadata key of the list's modification time.
pub const MODIFIED_AT_KEY: &str = "modified_at";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Widget {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub queries: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TemplateVariable {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub defaults: Vec<String>,
}

/// What the document keeps of a dashboard definition (bounded).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DashboardDefinition {
    #[serde(default)]
    pub widgets: Vec<Widget>,
    #[serde(default)]
    pub template_variables: Vec<TemplateVariable>,
}

fn non_empty(v: &Value) -> Option<String> {
    v.as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn one_line(s: &str, max: usize) -> String {
    let flat = s.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_bytes(&flat, max).to_string()
}

/// Queries under `v` (`q` or `query` strings at any depth, nested widgets excluded).
fn collect_queries(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Object(map) => {
            for (k, v) in map {
                match (k.as_str(), v) {
                    ("widgets", _) => {}
                    ("q" | "query", Value::String(s)) => {
                        let q = one_line(s, WIDGET_QUERY_MAX_BYTES);
                        if !q.is_empty() && !out.contains(&q) && out.len() < QUERIES_PER_WIDGET {
                            out.push(q);
                        }
                    }
                    _ => collect_queries(v, out),
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|i| collect_queries(i, out)),
        _ => {}
    }
}

fn collect_widgets(widgets: &Value, out: &mut Vec<Widget>) {
    for w in widgets.as_array().into_iter().flatten() {
        if out.len() >= MAX_WIDGETS {
            return;
        }
        let def = &w["definition"];
        let title = non_empty(&def["title"])
            .or_else(|| {
                (def["type"] == "note")
                    .then(|| non_empty(&def["content"]))
                    .flatten()
            })
            .map(|t| one_line(&t, WIDGET_TITLE_MAX_BYTES));
        let mut queries = vec![];
        collect_queries(def, &mut queries);
        if title.is_some() || !queries.is_empty() {
            out.push(Widget { title, queries });
        }
        collect_widgets(&def["widgets"], out);
    }
}

/// The bounded definition of a `GET /api/v1/dashboard/{id}` response. Missing or null
/// `widgets` and `template_variables` are empty.
pub fn parse_definition(body: &Value) -> DashboardDefinition {
    let mut widgets = vec![];
    collect_widgets(&body["widgets"], &mut widgets);
    // Keep whole widgets while their text fits the definition budget.
    let mut used = 0usize;
    let fits = widgets
        .iter()
        .take_while(|w| {
            used += w.title.as_ref().map_or(0, String::len)
                + w.queries.iter().map(|q| q.len() + 3).sum::<usize>()
                + 4;
            used <= DEFINITION_TEXT_MAX_BYTES
        })
        .count();
    widgets.truncate(fits);
    let template_variables = body["template_variables"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|tv| {
            let name = non_empty(&tv["name"])?;
            let mut defaults: Vec<String> = match tv["defaults"].as_array() {
                Some(d) => d.iter().filter_map(non_empty).collect(),
                None => non_empty(&tv["default"]).into_iter().collect(),
            };
            defaults.truncate(QUERIES_PER_WIDGET);
            Some(TemplateVariable {
                name: one_line(&name, WIDGET_TITLE_MAX_BYTES),
                prefix: non_empty(&tv["prefix"]).map(|p| one_line(&p, WIDGET_TITLE_MAX_BYTES)),
                defaults: defaults
                    .iter()
                    .map(|d| one_line(d, WIDGET_TITLE_MAX_BYTES))
                    .collect(),
            })
        })
        .take(MAX_TEMPLATE_VARIABLES)
        .collect();
    DashboardDefinition {
        widgets,
        template_variables,
    }
}

/// A concrete tag value: not a template variable reference, wildcard or group.
fn concrete(v: &str) -> bool {
    !v.is_empty()
        && !v.starts_with('$')
        && v.chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | '/' | ':'))
}

/// Values of `key:` filters in a query (`{service:checkout,env:prod}`,
/// `service:checkout env:prod`), normalized; negated, wildcard and `$var` values are
/// skipped.
pub fn query_tag_values(query: &str, key: &str) -> Vec<String> {
    let pat = format!("{key}:");
    let mut out = vec![];
    for (i, _) in query.match_indices(&pat) {
        let prev = query[..i].chars().next_back();
        if !matches!(prev, None | Some('{' | ',' | '(' | '"' | '\'') | Some(' ')) {
            continue;
        }
        let rest = &query[i + pat.len()..];
        let end = rest
            .find(|c: char| matches!(c, ',' | '}' | ')' | '"' | '\'') || c.is_whitespace())
            .unwrap_or(rest.len());
        let v = &rest[..end];
        if concrete(v) {
            let v = normalize_scope_value(v);
            if !out.contains(&v) {
                out.push(v);
            }
        }
    }
    out
}

/// The dashboard's value for `key` (`service` or `env`) by the rule in the module docs,
/// and every candidate (template variable defaults first, then by query count).
pub fn derive_scope(def: &DashboardDefinition, key: &str) -> (String, Vec<String>) {
    let mut from_vars: Vec<String> = vec![];
    for tv in &def.template_variables {
        let tag = tv.prefix.as_deref().unwrap_or(&tv.name);
        if !tag.eq_ignore_ascii_case(key) {
            continue;
        }
        for d in &tv.defaults {
            let v = normalize_scope_value(d);
            if concrete(&v) && !from_vars.contains(&v) {
                from_vars.push(v);
            }
        }
    }
    let mut counts: Vec<(String, usize)> = vec![];
    for q in def.widgets.iter().flat_map(|w| &w.queries) {
        for v in query_tag_values(q, key) {
            match counts.iter_mut().find(|(c, _)| *c == v) {
                Some((_, n)) => *n += 1,
                None => counts.push((v, 1)),
            }
        }
    }
    counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    let chosen = if from_vars.len() == 1 {
        from_vars[0].clone()
    } else if from_vars.is_empty() {
        match counts.as_slice() {
            [(only, _)] => only.clone(),
            [(top, n), (_, m), ..] if n > m => top.clone(),
            _ => String::new(),
        }
    } else {
        String::new()
    };
    let mut all = from_vars;
    for (v, _) in counts {
        if !all.contains(&v) {
            all.push(v);
        }
    }
    (chosen, all)
}

/// The definition as document text: template variables, then one line per widget.
fn definition_text(def: &DashboardDefinition) -> String {
    let mut lines = vec![];
    if !def.template_variables.is_empty() {
        let vars: Vec<String> = def
            .template_variables
            .iter()
            .map(|tv| {
                let mut s = format!("${}", tv.name);
                if let Some(p) = &tv.prefix {
                    s.push_str(&format!(" ({p})"));
                }
                if !tv.defaults.is_empty() {
                    s.push_str(&format!(" = {}", tv.defaults.join(", ")));
                }
                s
            })
            .collect();
        lines.push(format!("Template variables: {}", vars.join("; ")));
    }
    if !def.widgets.is_empty() {
        lines.push("Widgets:".to_string());
        for w in &def.widgets {
            let queries = w.queries.join(" | ");
            lines.push(match (&w.title, queries.is_empty()) {
                (Some(t), true) => format!("- {t}"),
                (Some(t), false) => format!("- {t}: {queries}"),
                (None, _) => format!("- {queries}"),
            });
        }
    }
    truncate_with_marker(
        &lines.join("\n"),
        DEFINITION_TEXT_MAX_BYTES,
        TRUNCATION_MARKER,
    )
    .into_owned()
}

/// The stored definition and modification time in a stored point's metadata.
fn stored_definition(
    md: &serde_json::Map<String, Value>,
) -> Option<(DashboardDefinition, Option<String>)> {
    let def = serde_json::from_value(md.get(DEFINITION_KEY)?.clone()).ok()?;
    Some((def, md.get(MODIFIED_AT_KEY).and_then(non_empty)))
}

/// Document ID of a dashboard list entry.
pub fn dashboard_doc_id(summary: &Value) -> String {
    format!("dashboard_{}", summary["id"].as_str().unwrap_or(""))
}

impl Datadog {
    /// `GET /api/v1/dashboard/{id}`, retried per `self.retry`.
    pub async fn dashboard_definition(
        &self,
        id: &str,
    ) -> Result<DashboardDefinition, UpstreamError> {
        let url = format!(
            "{}/api/v1/dashboard/{}",
            self.api_base,
            urlencoding::encode(id)
        );
        let r = send_with_retry(&self.retry, "datadog dashboard", || {
            self.http
                .get(&url)
                .header("DD-API-KEY", &self.api_key)
                .header("DD-APPLICATION-KEY", &self.app_key)
        })
        .await
        .map_err(|f| f.error)?;
        let body: Value = r.json().await.map_err(UpstreamError::from_reqwest)?;
        Ok(parse_definition(&body))
    }

    /// Documents for dashboard list entries, in order, with their definitions: reused
    /// from `stored` (stored point metadata by document ID) when `modified_at` is
    /// unchanged, otherwise fetched within the per-run budget. See the module docs.
    pub async fn dashboard_documents(
        &self,
        summaries: &[Value],
        stored: &HashMap<String, serde_json::Map<String, Value>>,
    ) -> Vec<RagDocument> {
        let mut budget = MAX_DEFINITION_FETCHES;
        let mut over_budget = 0usize;
        let jobs: Vec<(&Value, Option<DashboardDefinition>, bool)> = summaries
            .iter()
            .map(|summary| {
                let listed = non_empty(&summary[MODIFIED_AT_KEY]);
                let reuse = stored
                    .get(&dashboard_doc_id(summary))
                    .and_then(stored_definition)
                    .filter(|(_, m)| listed.is_some() && *m == listed)
                    .map(|(d, _)| d);
                let fetch = reuse.is_none() && budget > 0;
                if fetch {
                    budget -= 1;
                } else if reuse.is_none() {
                    over_budget += 1;
                }
                (summary, reuse, fetch)
            })
            .collect();
        if over_budget > 0 {
            tracing::warn!(
                over_budget,
                budget = MAX_DEFINITION_FETCHES,
                "dashboard definition budget reached; the rest are fetched in later runs"
            );
        }
        stream::iter(jobs)
            .map(|(summary, reuse, fetch)| async move {
                let listed = non_empty(&summary[MODIFIED_AT_KEY]);
                if let Some(def) = reuse {
                    return self.dashboard_document(summary, Some(&def), listed.as_deref());
                }
                let id = summary["id"].as_str().unwrap_or("");
                if fetch {
                    match self.dashboard_definition(id).await {
                        Ok(def) => {
                            return self.dashboard_document(summary, Some(&def), listed.as_deref());
                        }
                        Err(e) => {
                            tracing::warn!(dashboard = id, error = %e, "could not fetch dashboard definition; keeping what is stored");
                        }
                    }
                }
                // The stored definition with its old modification time, so a later run
                // fetches it again; with nothing stored, the list entry alone.
                match stored.get(&dashboard_doc_id(summary)).and_then(stored_definition) {
                    Some((def, modified)) => {
                        self.dashboard_document(summary, Some(&def), modified.as_deref())
                    }
                    None => self.dashboard_document(summary, None, listed.as_deref()),
                }
            })
            .buffered(DEFINITION_CONCURRENCY)
            .collect()
            .await
    }

    /// One dashboard as a document: list entry plus, when known, its definition.
    /// `modified_at` is stored so the next run can tell whether the definition changed.
    pub fn dashboard_document(
        &self,
        summary: &Value,
        definition: Option<&DashboardDefinition>,
        modified_at: Option<&str>,
    ) -> RagDocument {
        let id = summary["id"].as_str().unwrap_or("").to_string();
        let title = summary["title"].as_str().unwrap_or("").to_string();
        let description = non_empty(&summary["description"]);
        let author_handle = summary["author_handle"].as_str().unwrap_or("").to_string();
        let created = summary["created_at"].as_str().map(|s| s.to_string());

        let mut metadata = serde_json::Map::new();
        metadata.insert("author".to_string(), Value::String(author_handle));
        if let Some(m) = modified_at {
            metadata.insert(MODIFIED_AT_KEY.into(), Value::String(m.to_string()));
        }

        let mut sections = vec![title.clone()];
        if let Some(d) = description {
            sections.push(d);
        }
        let (mut service, mut environment) = (String::new(), String::new());
        if let Some(def) = definition {
            let (svc, services) = derive_scope(def, "service");
            let (env, environments) = derive_scope(def, "env");
            let mut scope = vec![];
            if !services.is_empty() {
                scope.push(format!("Services: {}", services.join(", ")));
                metadata.insert("services".into(), serde_json::json!(services));
            }
            if !environments.is_empty() {
                scope.push(format!("Environments: {}", environments.join(", ")));
                metadata.insert("environments".into(), serde_json::json!(environments));
            }
            if !scope.is_empty() {
                sections.push(scope.join("\n"));
            }
            let text = definition_text(def);
            if !text.is_empty() {
                sections.push(text);
            }
            metadata.insert(
                DEFINITION_KEY.into(),
                serde_json::to_value(def).expect("definitions serialize"),
            );
            (service, environment) = (svc, env);
        }

        RagDocument {
            id: format!("dashboard_{}", id),
            title,
            text: sections.join("\n\n"),
            source_uri: format!("https://app.{}/dashboard/{}", self.site, id),
            kind: SourceKind::Dashboard,
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
    use wiremock::matchers::{method, path, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn fixture(name: &str) -> Value {
        let path = format!(
            "{}/tests/fixtures/datadog/{}",
            env!("CARGO_MANIFEST_DIR"),
            name
        );
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap()
    }

    fn dd() -> Datadog {
        Datadog::new("k".into(), "a".into(), "datadoghq.eu".into())
    }

    fn client(server: &MockServer) -> Datadog {
        let mut dd = dd();
        dd.api_base = server.uri();
        dd.retry = RetryPolicy::none();
        dd
    }

    fn widget(title: &str, q: &str) -> Value {
        json!({"id": 1, "definition": {"type": "timeseries", "title": title, "requests": [{"q": q}]}})
    }

    #[test]
    fn recorded_definitions_are_parsed() {
        // Null template variables and a profile query with its search under `search.query`.
        let def = parse_definition(&fixture("dashboard_get.json"));
        assert_eq!(def.template_variables, vec![]);
        assert_eq!(
            def.widgets,
            vec![Widget {
                title: None,
                queries: vec!["runtime:jvm".into()]
            }]
        );
        // `requests` as an object (hostmap) and template variable defaults.
        let def = parse_definition(&fixture("dashboard_template_variables.json"));
        assert_eq!(
            def.template_variables,
            vec![TemplateVariable {
                name: "host1".into(),
                prefix: Some("host".into()),
                defaults: vec!["my-host".into()]
            }]
        );
        assert_eq!(def.widgets[0].queries, ["avg:system.cpu.user{*}"]);
        assert_eq!(derive_scope(&def, "service"), (String::new(), vec![]));
    }

    #[test]
    fn widgets_queries_and_variables_in_every_documented_shape() {
        let def = parse_definition(&json!({
            "template_variables": [
                {"name": "env", "prefix": "env", "default": "prod"},
                {"name": "svc", "prefix": null, "defaults": []},
                {"prefix": "missing-name"}
            ],
            "widgets": [
                widget("Latency", "avg:trace.http.request.duration{service:checkout}"),
                {"definition": {"type": "query_value", "title": "Errors", "requests": [{"queries": [
                    {"data_source": "metrics", "name": "q1", "query": "sum:errors{service:checkout}"},
                    {"data_source": "logs", "name": "q2", "search": {"query": "service:checkout status:error"}},
                    {"data_source": "metrics", "name": "q3", "query": "sum:errors{service:checkout}"}
                ]}]}},
                {"definition": {"type": "group", "title": "Dependencies", "widgets": [
                    widget("Payments", "avg:latency{service:payments}")
                ]}},
                {"definition": {"type": "note", "content": "Read   me\nfirst"}},
                {"definition": {"type": "image", "url": "https://x"}}
            ]
        }));
        let titles: Vec<_> = def.widgets.iter().map(|w| w.title.as_deref()).collect();
        assert_eq!(
            titles,
            [
                Some("Latency"),
                Some("Errors"),
                Some("Dependencies"),
                Some("Payments"),
                Some("Read me first")
            ]
        );
        // Duplicates dropped; the group's own entry has no queries of its children.
        assert_eq!(def.widgets[1].queries.len(), 2);
        assert!(def.widgets[2].queries.is_empty());
        assert_eq!(def.template_variables.len(), 2);
        assert_eq!(def.template_variables[0].defaults, ["prod"]);

        let doc = dd().dashboard_document(
            &json!({"id": "d-1", "title": "Checkout", "description": "Shop"}),
            Some(&def),
            Some("2026-01-01T00:00:00+00:00"),
        );
        assert_eq!(
            (doc.service.as_str(), doc.environment.as_str()),
            ("checkout", "prod")
        );
        assert_eq!(
            doc.text,
            "Checkout\n\nShop\n\nServices: checkout, payments\nEnvironments: prod\n\n\
             Template variables: $env (env) = prod; $svc\n\
             Widgets:\n\
             - Latency: avg:trace.http.request.duration{service:checkout}\n\
             - Errors: sum:errors{service:checkout} | service:checkout status:error\n\
             - Dependencies\n\
             - Payments: avg:latency{service:payments}\n\
             - Read me first"
        );
        assert_eq!(doc.metadata[MODIFIED_AT_KEY], "2026-01-01T00:00:00+00:00");
        assert_eq!(doc.metadata["services"], json!(["checkout", "payments"]));
    }

    #[test]
    fn query_tag_values_skip_variables_wildcards_and_negations() {
        let q = "avg:x{service:Checkout,env:prod,!service:auth,-service:a2,service:$service,\
                 service:web*,myservice:no} by {service} service:(a OR b) \"service:quoted\"";
        assert_eq!(query_tag_values(q, "service"), ["checkout", "quoted"]);
        assert_eq!(query_tag_values(q, "env"), ["prod"]);
        assert_eq!(query_tag_values("env:prod env:prod", "env"), ["prod"]);
        assert_eq!(
            query_tag_values("service:", "service"),
            Vec::<String>::new()
        );
        // Multibyte values are kept whole.
        assert_eq!(
            query_tag_values("{service:sökmotor}", "service"),
            ["sökmotor"]
        );
    }

    fn def_with(vars: Value, queries: &[&str]) -> DashboardDefinition {
        let widgets: Vec<Value> = queries.iter().map(|q| widget("w", q)).collect();
        parse_definition(&json!({"template_variables": vars, "widgets": widgets}))
    }

    #[test]
    fn scope_rules() {
        // 1. A single concrete template variable default wins over queries.
        let d = def_with(
            json!([{"name": "service", "prefix": "service", "defaults": ["Payments"]}]),
            &["a{service:checkout}", "b{service:checkout}"],
        );
        assert_eq!(
            derive_scope(&d, "service"),
            (
                "payments".into(),
                vec!["payments".into(), "checkout".into()]
            )
        );
        // A variable named like the tag without a prefix counts too; `*` does not.
        let d = def_with(
            json!([{"name": "SERVICE", "defaults": ["*"]}]),
            &["a{service:x}"],
        );
        assert_eq!(derive_scope(&d, "service").0, "x");
        // Several defaults: no single value from the variable, and queries are not used.
        let d = def_with(
            json!([{"name": "service", "prefix": "service", "defaults": ["a", "b"]}]),
            &["q{service:a}"],
        );
        assert_eq!(derive_scope(&d, "service").0, "");
        // 2./3. The only or the strictly dominant query value.
        let d = def_with(
            json!(null),
            &["a{service:x}", "b{service:x}", "c{service:y}"],
        );
        assert_eq!(
            derive_scope(&d, "service"),
            ("x".into(), vec!["x".into(), "y".into()])
        );
        // 4. A tie leaves the dashboard unscoped.
        let d = def_with(json!(null), &["a{service:x}", "b{service:y}"]);
        assert_eq!(derive_scope(&d, "service").0, "");
        assert_eq!(derive_scope(&DashboardDefinition::default(), "env").0, "");
        // Values survive the planner's validation unchanged, so a filter can match.
        let d = def_with(json!(null), &["a{service:Auth-API}"]);
        let stored = derive_scope(&d, "service").0;
        assert_eq!(
            crate::planner::sanitize_tag_value(&json!("AUTH-API"), "service"),
            Some(stored)
        );
    }

    #[test]
    fn definitions_are_bounded_and_utf8_safe() {
        let widgets: Vec<Value> = (0..150)
            .map(|i| {
                json!({"definition": {"type": "timeseries", "title": format!("{i} {}", "Översikt 📊 ".repeat(40)),
                    "requests": (0..8).map(|j| json!({"q": format!("{j}{}", "å{service:sökmotor}".repeat(40))})).collect::<Vec<_>>()}})
            })
            .collect();
        let def = parse_definition(&json!({"widgets": widgets}));
        assert!(!def.widgets.is_empty() && def.widgets.len() < MAX_WIDGETS);
        for w in &def.widgets {
            assert!(w.title.as_ref().unwrap().len() <= WIDGET_TITLE_MAX_BYTES);
            assert_eq!(w.queries.len(), QUERIES_PER_WIDGET);
            assert!(w.queries.iter().all(|q| q.len() <= WIDGET_QUERY_MAX_BYTES));
        }
        let doc = dd().dashboard_document(&json!({"id": "x", "title": "t"}), Some(&def), None);
        assert_eq!(doc.service, "sökmotor");
        let section = &doc.text[doc.text.find("Widgets:").unwrap()..];
        assert!(section.len() <= DEFINITION_TEXT_MAX_BYTES + TRUNCATION_MARKER.len());
        let stored = serde_json::to_string(&doc.metadata[DEFINITION_KEY]).unwrap();
        assert!(
            stored.len() < 2 * DEFINITION_TEXT_MAX_BYTES,
            "{} bytes",
            stored.len()
        );

        // Many small widgets stop at the widget count.
        let small: Vec<Value> = (0..150).map(|i| widget(&format!("w{i}"), "q")).collect();
        assert_eq!(
            parse_definition(&json!({"widgets": small})).widgets.len(),
            MAX_WIDGETS
        );
    }

    fn summary(id: &str, modified: &str) -> Value {
        json!({"id": id, "title": format!("Dash {id}"), "description": null,
               "author_handle": "a@b.c", "created_at": "2025-01-01T00:00:00+00:00",
               "modified_at": modified})
    }

    fn definition_body(service: &str) -> Value {
        json!({"template_variables": null, "widgets": [widget("Latency", &format!("avg:x{{service:{service}}}"))]})
    }

    /// Stored point metadata of `doc`, as the indexer reads it back (chunk 0).
    fn stored_of(doc: &RagDocument) -> (String, serde_json::Map<String, Value>) {
        let mut md = doc.metadata.clone();
        md.insert("chunk_index".into(), json!(0));
        md.insert("chunk_of".into(), json!(doc.id));
        (doc.id.clone(), md)
    }

    #[tokio::test]
    async fn unchanged_dashboards_reuse_the_stored_definition() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/dashboard/d-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(definition_body("checkout")))
            .expect(1)
            .mount(&server)
            .await;
        let dd = client(&server);
        let first = dd
            .dashboard_documents(
                &[summary("d-1", "2026-01-01T00:00:00+00:00")],
                &HashMap::new(),
            )
            .await;
        assert_eq!(first[0].service, "checkout");

        // Same modified_at: no request, and exactly the same document (same hash).
        let stored: HashMap<_, _> = [stored_of(&first[0])].into();
        let again = dd
            .dashboard_documents(&[summary("d-1", "2026-01-01T00:00:00+00:00")], &stored)
            .await;
        assert_eq!(
            serde_json::to_value(&again[0]).unwrap(),
            serde_json::to_value(&first[0]).unwrap()
        );
        assert_eq!(
            crate::chunk::content_hash(&again[0], 1800, 200, "m"),
            crate::chunk::content_hash(&first[0], 1800, 200, "m")
        );
    }

    #[tokio::test]
    async fn changed_or_failing_dashboards() {
        let server = MockServer::start().await;
        // d-1 changed since it was stored: fetched again.
        Mock::given(method("GET"))
            .and(path("/api/v1/dashboard/d-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(definition_body("payments")))
            .expect(1)
            .mount(&server)
            .await;
        // d-2 changed but its fetch fails: the stored definition is kept, with the stored
        // modified_at so the next run fetches again.
        Mock::given(method("GET"))
            .and(path("/api/v1/dashboard/d-2"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        // d-3 is new and its fetch fails: indexed from the list entry.
        Mock::given(method("GET"))
            .and(path("/api/v1/dashboard/d-3"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        let dd = client(&server);
        let old_def = parse_definition(&definition_body("checkout"));
        let old = |id: &str| {
            stored_of(&dd.dashboard_document(
                &summary(id, "2025-12-01T00:00:00+00:00"),
                Some(&old_def),
                Some("2025-12-01T00:00:00+00:00"),
            ))
        };
        let stored: HashMap<_, _> = [old("d-1"), old("d-2")].into();
        let docs = dd
            .dashboard_documents(
                &[
                    summary("d-1", "2026-01-01T00:00:00+00:00"),
                    summary("d-2", "2026-01-01T00:00:00+00:00"),
                    summary("d-3", "2026-01-01T00:00:00+00:00"),
                ],
                &stored,
            )
            .await;
        assert_eq!(docs[0].service, "payments");
        assert_eq!(
            docs[0].metadata[MODIFIED_AT_KEY],
            "2026-01-01T00:00:00+00:00"
        );
        assert_eq!(docs[1].service, "checkout");
        assert_eq!(
            docs[1].metadata[MODIFIED_AT_KEY],
            "2025-12-01T00:00:00+00:00"
        );
        assert_eq!(docs[2].service, "");
        assert_eq!(docs[2].text, "Dash d-3");
        assert!(!docs[2].metadata.contains_key(DEFINITION_KEY));
    }

    #[tokio::test]
    async fn definition_fetches_are_capped_per_run() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex(r"^/api/v1/dashboard/.+$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(definition_body("x")))
            .expect(MAX_DEFINITION_FETCHES as u64)
            .mount(&server)
            .await;
        let summaries: Vec<Value> = (0..MAX_DEFINITION_FETCHES + 3)
            .map(|i| summary(&format!("d-{i}"), "2026-01-01T00:00:00+00:00"))
            .collect();
        let docs = client(&server)
            .dashboard_documents(&summaries, &HashMap::new())
            .await;
        assert_eq!(docs.len(), summaries.len());
        assert_eq!(docs[MAX_DEFINITION_FETCHES - 1].service, "x");
        assert_eq!(docs[MAX_DEFINITION_FETCHES].service, "");
    }
}
