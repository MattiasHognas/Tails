# Datadog response fixtures

Response bodies used by the adapter tests in `src/datadog.rs`, `src/service_catalog.rs`
and `src/change_events.rs`. Each body is a
recorded Datadog API response from the official client's test cassettes
(`DataDog/datadog-api-client-rust`, `tests/scenarios/cassettes`):

| Fixture | Endpoint | Cassette |
| --- | --- | --- |
| `incidents_search_page1.json` | `GET /api/v2/incidents/search` | `v2/incidents/Search-for-incidents-returns-OK-response-with-pagination` (1st page) |
| `incidents_search_page2.json` | `GET /api/v2/incidents/search` | same cassette (2nd page) |
| `monitors.json` | `GET /api/v1/monitor` | `v1/monitors/Get-all-monitors-returns-OK-response-with-pagination` |
| `dashboards.json` | `GET /api/v1/dashboard` | `v1/dashboards/Get-all-dashboards-returns-OK-response` |
| `slos.json` | `GET /api/v1/slo` | `v1/service_level_objectives/Get-all-SLOs-returns-OK-response` |
| `logs_search_page1.json` | `POST /api/v2/logs/events/search` | `v2/logs/Search-logs-returns-OK-response-with-pagination` (1st page) |
| `logs_search_page2.json` | `POST /api/v2/logs/events/search` | same cassette (2nd page, requested with the 1st page's `meta.page.after` cursor) |
| `logs_search_page3.json` | `POST /api/v2/logs/events/search` | same cassette (3rd page: empty, no `meta.page.after`) |
| `metrics.json` | `GET /api/v1/metrics` | no cassette; documented response example from the v1 OpenAPI spec |
| `metrics_query.json` | `GET /api/v1/query` | `v1/metrics/Query-timeseries-points-returns-OK-response` (`from=1641343852&to=1641430252&query=system.cpu.idle{*}`) |
| `service_definitions_page1.json` | `GET /api/v2/services/definitions` | `v2/service_definition/Get-all-service-definitions-returns-OK-response-with-pagination` (1st page, `page[size]=2`: a v2.1 and a v2 definition) |
| `service_definitions_page2.json` | `GET /api/v2/services/definitions` | same cassette (2nd page: a v2 definition with an empty team and schema warnings) |
| `service_definitions_schema_versions.json` | `GET /api/v2/services/definitions` | no cassette; crafted from the OpenAPI spec: the documented v2.2 request example, a v1 definition built from the v1 schema's field examples, a v3 entity (`apiVersion: v3`, from the Software Catalog upsert example plus `spec.dependsOn` and a multibyte description), and a definition without `dd-service` |
| `events_search_page1.json` | `POST /api/v2/events/search` | `v2/events/Search-events-returns-OK-response-with-pagination` (1st page, `page.limit=2`) |
| `events_search_page2.json` | `POST /api/v2/events/search` | same cassette (2nd page, requested with the 1st page's `meta.page.after` cursor) |
| `events_search_page3.json` | `POST /api/v2/events/search` | same cassette (3rd page: `{"data": []}`) |

The incident search `facets` object is emptied to keep the fixtures small; the
adapter does not read it. Everything else is as recorded.

`metrics_query.json` is used as-is by the live-evidence tests in
`src/live_evidence/mod.rs`, which treat the second half of the recorded day as the
question's window and the first half as its baseline; tests that need a spike, drop or
gap modify individual points in memory.
