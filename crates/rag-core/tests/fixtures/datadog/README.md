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

The incident search `facets` object is emptied to keep the fixtures small; the
adapter does not read it. Everything else is as recorded.
