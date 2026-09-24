# Datadog response fixtures

Response bodies used by the adapter tests in `src/datadog.rs`. Each body is a
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

The incident search `facets` object is emptied to keep the fixtures small; the
adapter does not read it. Everything else is as recorded.

`metrics_query.json` is used as-is by the live-evidence tests in
`src/live_evidence/mod.rs`, which treat the second half of the recorded day as the
question's window and the first half as its baseline; tests that need a spike, drop or
gap modify individual points in memory.
