//! Human-readable rendering of `/ask` and `/ask/plan` responses. Pure: the
//! caller decides color, width and timezone (see [`RenderOptions`]).

use anstyle::Style;
use chrono::{DateTime, Duration, NaiveTime};
use chrono_tz::Tz;
use serde_json::Value;
use std::fmt::Write;

/// Live-evidence skip reasons not worth showing: the question was not
/// diagnostic, or the user turned live evidence off (`--no-live-evidence`).
const HIDDEN_SKIP_REASONS: [&str; 2] = ["not_diagnostic", "disabled_by_request"];

const NO_EVIDENCE_HINT: &str = "Try widening the time window, removing --service/--env filters, \
or checking that the relevant data has been indexed.";

#[derive(Debug, Clone)]
pub struct RenderOptions {
    /// Bold headings and dim links (ANSI). Off means plain text.
    pub color: bool,
    /// Wrap prose to this many columns; `None` leaves lines unwrapped.
    pub width: Option<usize>,
    /// Zone used to show times (the one the CLI sent to the API).
    pub tz: Tz,
}

impl RenderOptions {
    /// `tz` is an IANA name; unknown or absent names show times in UTC.
    pub fn new(color: bool, width: Option<usize>, tz: Option<&str>) -> Self {
        let tz = tz.and_then(|t| t.parse().ok()).unwrap_or(Tz::UTC);
        Self { color, width, tz }
    }

    fn heading(&self, out: &mut String, title: &str) {
        if !out.is_empty() {
            out.push('\n');
        }
        let _ = writeln!(out, "{}", self.style(Style::new().bold(), title));
    }

    fn style(&self, style: Style, text: &str) -> String {
        if self.color {
            format!("{style}{text}{style:#}")
        } else {
            text.to_string()
        }
    }

    fn dim(&self, text: &str) -> String {
        self.style(Style::new().dimmed(), text)
    }

    /// Wrap `text` (one paragraph) with `first` before the first line and
    /// `rest` before the others.
    fn wrap(&self, out: &mut String, text: &str, first: &str, rest: &str) {
        for line in wrap(text, self.width, first, rest) {
            out.push_str(&line);
            out.push('\n');
        }
    }
}

/// Render an `/ask` response: answer, scope, evidence, live evidence, sources
/// and clarifying questions.
pub fn render_ask(resp: &Value, opts: &RenderOptions) -> String {
    let mut out = String::new();

    opts.heading(&mut out, "Answer");
    let answer = rewrite_citations(resp["answer"].as_str().unwrap_or(""));
    for line in answer.trim_end().lines() {
        if line.trim().is_empty() {
            out.push('\n');
            continue;
        }
        // Keep the model's indentation and bullets; continue under the text.
        let body = line.trim_start();
        let lead = &line[..line.len() - body.len()];
        let marker = list_marker(body);
        let first = format!("  {lead}");
        let rest = format!("  {lead}{}", " ".repeat(marker.chars().count()));
        opts.wrap(&mut out, body, &first, &rest);
    }

    if let Some(scope) = scope_line(&resp["scope"], opts) {
        opts.heading(&mut out, "Scope");
        opts.wrap(&mut out, &scope, "  ", "  ");
    }

    let timeline = &resp["timeline"];
    let collected = timeline["status"] == "collected";
    opts.heading(&mut out, "Evidence");
    if resp["evidence"] == "none" {
        opts.wrap(
            &mut out,
            "None: no indexed documents or live observations matched.",
            "  ",
            "  ",
        );
        for line in wrap(NO_EVIDENCE_HINT, opts.width, "  ", "  ") {
            let _ = writeln!(out, "{}", opts.dim(&line));
        }
    } else {
        let mut parts = vec![];
        if let Some(sources) = resp["sources"].as_array() {
            parts.push(plural(sources.len(), "indexed document"));
        }
        if collected {
            parts.push(plural(
                list(&timeline["observations"]).len(),
                "live observation",
            ));
        }
        if parts.is_empty() {
            parts.push("found".to_string());
        }
        opts.wrap(&mut out, &parts.join(" · "), "  ", "  ");
    }

    let hidden = !collected
        && timeline["skipReason"]
            .as_str()
            .is_some_and(|r| HIDDEN_SKIP_REASONS.contains(&r));
    if timeline.is_object() && !hidden {
        render_timeline(&mut out, timeline, opts);
    }

    let sources = list(&resp["sources"]);
    if !sources.is_empty() {
        opts.heading(&mut out, "Sources");
        for s in sources {
            let mut details = vec![text(&s["kind"])];
            if let Some(ts) = s["timestamp"].as_str() {
                details.push(local_time(ts, opts.tz));
            }
            details.retain(|d| !d.is_empty());
            let n = s["n"].to_string();
            let mut line = format!("[{n}] {}", text(&s["title"]));
            if !details.is_empty() {
                let _ = write!(line, " ({})", details.join(", "));
            }
            if let Some(uri) = s["uri"].as_str().filter(|u| !u.is_empty()) {
                let _ = write!(line, " · {uri}");
            }
            let rest = " ".repeat(n.len() + 5);
            opts.wrap(&mut out, &line, "  ", &rest);
        }
    }

    let questions = list(&resp["plan"]["clarifyingQuestions"]);
    if !questions.is_empty() {
        opts.heading(&mut out, "Need more info");
        for q in questions {
            opts.wrap(&mut out, &text(&q), "  - ", "    ");
        }
    }
    out
}

fn render_timeline(out: &mut String, t: &Value, opts: &RenderOptions) {
    opts.heading(out, "Live evidence");
    if let Some(range) = range(&t["window"]["fromUtc"], &t["window"]["toUtc"], opts.tz) {
        opts.wrap(out, &format!("Window: {range} {}", opts.tz), "  ", "  ");
    }
    if t["status"] == "collected" {
        let _ = writeln!(out, "  Observed:");
        let obs = list(&t["observations"]);
        if obs.is_empty() {
            let _ = writeln!(out, "    (none)");
        }
        for o in obs {
            let when = range(&o["startUtc"], &o["endUtc"], opts.tz).unwrap_or_default();
            let line = format!("[{}] {when} {}", text(&o["id"]), text(&o["summary"]));
            opts.wrap(out, &line, "    ", "      ");
            if let Some(link) = o["link"].as_str().filter(|l| !l.is_empty()) {
                let _ = writeln!(out, "      {}", opts.dim(link));
            }
        }
    } else if let Some(reason) = t["skipReason"].as_str() {
        let _ = writeln!(out, "  Not collected: {}", reason.replace('_', " "));
    }
    let hyp = list(&t["hypotheses"]);
    if !hyp.is_empty() {
        let _ = writeln!(out, "  Hypotheses (unverified):");
        for h in hyp {
            let ids: Vec<String> = list(&h["observationIds"]).iter().map(text).collect();
            let line = format!("{} [{}]", text(&h["statement"]), ids.join(", "));
            opts.wrap(out, &line, "    - ", "      ");
        }
    }
    let missing = list(&t["missingEvidence"]);
    if !missing.is_empty() {
        let _ = writeln!(out, "  Not checked:");
        for m in missing {
            let line = format!(
                "{} ({}): {}",
                text(&m["subject"]),
                text(&m["reason"]),
                text(&m["detail"])
            );
            opts.wrap(out, &line, "    - ", "      ");
        }
    }
}

/// Render an `/ask/plan` response (`{"plan": {...}}` or a bare plan) as
/// aligned key/value lines.
pub fn render_plan(resp: &Value, opts: &RenderOptions) -> String {
    let plan = if resp["plan"].is_object() {
        &resp["plan"]
    } else {
        resp
    };
    let field = |key: &str| {
        plan[key]
            .as_str()
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    };
    let joined = |key: &str| {
        let items: Vec<String> = list(&plan[key]).iter().map(text).collect();
        (!items.is_empty()).then(|| items.join(", "))
    };
    let window = range(
        &plan["window"]["fromUtc"],
        &plan["window"]["toUtc"],
        opts.tz,
    )
    .map(|r| format!("{r} {}", opts.tz));
    let rows: Vec<(&str, String)> = [
        ("Intent", field("intent")),
        ("Service", field("service")),
        ("Environment", field("environment")),
        ("Window", window),
        ("Metric", field("metric")),
        ("Monitor", field("monitorId")),
        ("Incident", field("incidentId")),
        ("SLO", field("sloId")),
        ("Filters", joined("filters")),
        ("Missing", joined("missingFields")),
        ("Rewritten query", field("rewrittenQuery")),
    ]
    .into_iter()
    .filter_map(|(label, value)| value.map(|v| (label, v)))
    .collect();

    let pad = rows.iter().map(|(l, _)| l.len()).max().unwrap_or(0) + 2;
    let mut out = String::new();
    opts.heading(&mut out, "Plan");
    for (label, value) in rows {
        let first = format!(
            "  {}",
            opts.style(Style::new().bold(), &format!("{label}:"))
        );
        // Styling adds invisible bytes; pad by visible width.
        let first = format!("{first}{}", " ".repeat(pad - label.len() - 1));
        let rest = " ".repeat(pad + 2);
        for (i, line) in wrap(
            &value,
            opts.width.map(|w| w.saturating_sub(pad + 2)),
            "",
            "",
        )
        .into_iter()
        .enumerate()
        {
            let _ = writeln!(out, "{}{line}", if i == 0 { &first } else { &rest });
        }
    }
    let questions = list(&plan["clarifyingQuestions"]);
    if !questions.is_empty() {
        opts.heading(&mut out, "Need more info");
        for q in questions {
            opts.wrap(&mut out, &text(&q), "  - ", "    ");
        }
    }
    out
}

/// `service · environment · window · kinds`, omitting absent parts.
fn scope_line(scope: &Value, opts: &RenderOptions) -> Option<String> {
    let mut parts: Vec<String> = ["service", "environment"]
        .iter()
        .filter_map(|k| scope[*k].as_str().filter(|v| !v.is_empty()))
        .map(str::to_string)
        .collect();
    if let Some(r) = range(&scope["fromUtc"], &scope["toUtc"], opts.tz) {
        parts.push(format!("{r} {}", opts.tz));
    }
    let kinds: Vec<String> = list(&scope["kinds"]).iter().map(text).collect();
    if !kinds.is_empty() {
        parts.push(kinds.join(", "));
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// `[DOC #n]` citations become `[n]`, matching the Sources list; a bare
/// `DOC #n` becomes `[n]`. `[obs-N]` citations are left alone.
pub fn rewrite_citations(s: &str) -> String {
    const TAG: &str = "DOC #";
    let mut out = String::with_capacity(s.len());
    let mut depth = 0usize;
    let mut rest = s;
    while let Some(pos) = rest.find(TAG) {
        let (before, after) = rest.split_at(pos);
        for c in before.chars() {
            match c {
                '[' => depth += 1,
                ']' => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
        out.push_str(before);
        let digits: String = after[TAG.len()..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        if digits.is_empty() {
            out.push_str(TAG);
        } else if depth > 0 {
            out.push_str(&digits);
        } else {
            let _ = write!(out, "[{digits}]");
        }
        rest = &after[TAG.len() + digits.len()..];
    }
    out.push_str(rest);
    out
}

/// A list item's marker (`- `, `* `, `• `, `1. `, `2) `), or `""`.
fn list_marker(line: &str) -> &str {
    for m in ["- ", "* ", "• ", "+ "] {
        if line.starts_with(m) {
            return m;
        }
    }
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    if digits > 0 {
        let tail = &line[digits..];
        if tail.starts_with(". ") || tail.starts_with(") ") {
            return &line[..digits + 2];
        }
    }
    ""
}

/// Greedy word wrap by character count. Words longer than the width (such as
/// URLs) get a line of their own and are never split. `None` does not wrap.
fn wrap(text: &str, width: Option<usize>, first: &str, rest: &str) -> Vec<String> {
    let Some(width) = width else {
        return vec![format!("{first}{text}")];
    };
    let mut lines = vec![];
    let mut line = first.to_string();
    let mut len = first.chars().count();
    let mut empty = true;
    for word in text.split_whitespace() {
        let wlen = word.chars().count();
        if !empty && len + 1 + wlen > width {
            lines.push(std::mem::replace(&mut line, rest.to_string()));
            len = rest.chars().count();
            empty = true;
        }
        if !empty {
            line.push(' ');
            len += 1;
        }
        line.push_str(word);
        len += wlen;
        empty = false;
    }
    lines.push(line);
    lines
}

fn parse(ts: &str, tz: Tz) -> Option<DateTime<Tz>> {
    DateTime::parse_from_rfc3339(ts)
        .ok()
        .map(|t| t.with_timezone(&tz))
}

/// `2026-09-23 12:00` in `tz`, or the input unchanged if it doesn't parse.
fn local_time(ts: &str, tz: Tz) -> String {
    parse(ts, tz).map_or_else(
        || ts.to_string(),
        |t| t.format("%Y-%m-%d %H:%M").to_string(),
    )
}

/// A local time range: `2026-09-23 00:00–24:00` for one day,
/// `2026-09-23 10:00–11:00` within a day, otherwise both ends in full.
/// Open ends read `since …` / `until …`. `None` when both are absent.
fn range(from: &Value, to: &Value, tz: Tz) -> Option<String> {
    match (from.as_str(), to.as_str()) {
        (Some(f), Some(t)) => {
            let (Some(fl), Some(tl)) = (parse(f, tz), parse(t, tz)) else {
                return Some(format!("{f} – {t}"));
            };
            let start = fl.format("%Y-%m-%d %H:%M");
            let end = if tl.date_naive() == fl.date_naive() {
                tl.format("%H:%M").to_string()
            } else if tl.time() == NaiveTime::MIN
                && tl.date_naive() == fl.date_naive() + Duration::days(1)
            {
                "24:00".to_string()
            } else {
                return Some(format!("{start} – {}", tl.format("%Y-%m-%d %H:%M")));
            };
            Some(format!("{start}–{end}"))
        }
        (Some(f), None) => Some(format!("since {}", local_time(f, tz))),
        (None, Some(t)) => Some(format!("until {}", local_time(t, tz))),
        (None, None) => None,
    }
}

fn plural(n: usize, noun: &str) -> String {
    format!("{n} {noun}{}", if n == 1 { "" } else { "s" })
}

fn text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn list(v: &Value) -> Vec<Value> {
    v.as_array().cloned().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn plain(width: Option<usize>) -> RenderOptions {
        RenderOptions::new(false, width, Some("Europe/Stockholm"))
    }

    fn diagnostic() -> Value {
        json!({
            "answer": "auth-api returned 5xx between 12:05 and 12:40 local time [DOC #1]. Latency spiked [obs-2] and the monitor fired [DOC #2, DOC #1].\n\nTop signals:\n- Error rate above 5% on auth-api in prod for most of the hour, see DOC #1\n- Connection pool exhausted in error logs\n\nNext steps:\n1. Check the database connection pool size and recent deploys to auth-api",
            "evidence": "found",
            "sources": [
                {"n": 1, "title": "auth-api 5xx spike", "kind": "incident", "timestamp": "2026-09-23T10:00:00Z",
                 "service": "auth-api", "environment": "prod", "uri": "https://app.datadoghq.eu/incidents/1"},
                {"n": 2, "title": "auth-api latency", "kind": "monitor", "timestamp": null,
                 "service": "auth-api", "environment": "prod", "uri": "https://app.datadoghq.eu/monitors/7"}
            ],
            "plan": {
                "intent": "rootCauseWindow", "service": "auth-api", "environment": "prod",
                "missingFields": ["environment"],
                "clarifyingQuestions": ["Which environment do you mean: prod or staging?"]
            },
            "scope": {
                "service": "auth-api", "environment": "prod",
                "fromUtc": "2026-09-22T22:00:00Z", "toUtc": "2026-09-23T22:00:00Z",
                "kinds": ["logs", "incident"]
            },
            "timeline": {
                "status": "collected",
                "window": {"fromUtc": "2026-09-22T22:00:00Z", "toUtc": "2026-09-23T22:00:00Z"},
                "baseline": {"fromUtc": "2026-09-21T22:00:00Z", "toUtc": "2026-09-22T22:00:00Z"},
                "observations": [
                    {"id": "obs-1", "kind": "seriesSummary", "startUtc": "2026-09-22T22:00:00Z",
                     "endUtc": "2026-09-23T22:00:00Z", "summary": "latency mean 0.3 vs baseline 0.2",
                     "link": "https://app.datadoghq.eu/metric/explorer?a"},
                    {"id": "obs-2", "kind": "spike", "startUtc": "2026-09-23T10:00:00Z",
                     "endUtc": "2026-09-23T11:00:00Z", "summary": "latency above baseline",
                     "link": "https://app.datadoghq.eu/metric/explorer?b"},
                    {"id": "obs-3", "kind": "logBurst", "startUtc": "2026-09-23T10:10:00Z",
                     "endUtc": "2026-09-23T10:20:00Z", "summary": "6 error logs",
                     "link": "https://app.datadoghq.eu/logs?c"}
                ],
                "hypotheses": [{"statement": "Pool exhaustion slowed requests", "observationIds": ["obs-2", "obs-3"]}],
                "missingEvidence": [{"subject": "metrics for billing", "reason": "no_metrics_discovered", "detail": "only logs checked"}]
            }
        })
    }

    fn position(out: &str, needle: &str) -> usize {
        out.find(needle)
            .unwrap_or_else(|| panic!("{needle:?} missing from:\n{out}"))
    }

    #[test]
    fn diagnostic_answer_has_all_sections_in_order() {
        let out = render_ask(&diagnostic(), &plain(None));
        let order: Vec<usize> = [
            "Answer\n",
            "\nScope\n",
            "\nEvidence\n",
            "\nLive evidence",
            "\nSources\n",
            "\nNeed more info\n",
        ]
        .iter()
        .map(|h| position(&out, h))
        .collect();
        assert!(order.windows(2).all(|w| w[0] < w[1]), "{out}");

        assert!(
            out.contains("local time [1]. Latency spiked [obs-2] and the monitor fired [2, 1].")
        );
        assert!(out.contains("most of the hour, see [1]\n"));
        assert!(!out.contains("DOC #"));
        assert!(out.contains(
            "  auth-api · prod · 2026-09-23 00:00–24:00 Europe/Stockholm · logs, incident\n"
        ));
        assert!(out.contains("  2 indexed documents · 3 live observations\n"));
        assert!(out.contains(
            "Live evidence\n  Window: 2026-09-23 00:00–24:00 Europe/Stockholm\n  Observed:\n"
        ));
        assert!(out.contains(
            "    [obs-2] 2026-09-23 12:00–13:00 latency above baseline\n      https://app.datadoghq.eu/metric/explorer?b\n"
        ));
        assert!(out.contains(
            "  Hypotheses (unverified):\n    - Pool exhaustion slowed requests [obs-2, obs-3]\n"
        ));
        assert!(out.contains(
            "  Not checked:\n    - metrics for billing (no_metrics_discovered): only logs checked\n"
        ));
        assert!(out.contains(
            "  [1] auth-api 5xx spike (incident, 2026-09-23 12:00) · https://app.datadoghq.eu/incidents/1\n"
        ));
        assert!(
            out.contains(
                "  [2] auth-api latency (monitor) · https://app.datadoghq.eu/monitors/7\n"
            )
        );
        assert!(
            out.contains("Need more info\n  - Which environment do you mean: prod or staging?\n")
        );
        // The model's line breaks and list items are kept.
        assert!(out.contains("\n\n  Top signals:\n  - Error rate"));
        assert!(out.contains("\n  1. Check the database"));
    }

    #[test]
    fn no_ansi_codes_without_color_and_styles_with_color() {
        let out = render_ask(&diagnostic(), &plain(Some(60)));
        assert!(!out.contains('\x1b'));
        let out = render_plan(&diagnostic(), &plain(Some(60)));
        assert!(!out.contains('\x1b'));

        let color = RenderOptions::new(true, Some(60), Some("Europe/Stockholm"));
        let out = render_ask(&diagnostic(), &color);
        assert!(out.contains("\x1b[1mAnswer\x1b[0m"));
    }

    #[test]
    fn wraps_to_width_with_hanging_indent_for_list_items() {
        let out = render_ask(&diagnostic(), &plain(Some(40)));
        for line in out.lines() {
            // Only unbreakable words (URLs) may overflow.
            assert!(
                line.chars().count() <= 40 || !line.trim().contains(' '),
                "line too long: {line:?}"
            );
        }
        assert!(out.contains("  - Error rate above 5% on auth-api in\n    prod for most"));
        assert!(out.contains("  1. Check the database connection pool\n     size and"));
        // Links stay whole on their own line.
        assert!(out.contains("\n      https://app.datadoghq.eu/metric/explorer?b\n"));
    }

    #[test]
    fn non_diagnostic_answer_has_no_live_evidence_section() {
        let resp = json!({
            "answer": "The auth-api dashboard is [DOC #1].",
            "evidence": "found",
            "sources": [{"n": 1, "title": "auth-api overview", "kind": "dashboard", "timestamp": null,
                         "service": null, "environment": null, "uri": "https://app.datadoghq.eu/dashboard/abc"}],
            "plan": {"intent": "dashboardLookup", "missingFields": [], "clarifyingQuestions": []},
            "scope": {"service": null, "environment": null, "fromUtc": null, "toUtc": null, "kinds": []},
            "timeline": {"status": "skipped", "skipReason": "not_diagnostic", "window": null,
                         "baseline": null, "observations": [], "hypotheses": [], "missingEvidence": []}
        });
        let out = render_ask(&resp, &plain(None));
        assert!(out.contains("The auth-api dashboard is [1]."));
        assert!(!out.contains("Live evidence"));
        assert!(!out.contains("Scope"));
        assert!(!out.contains("Need more info"));
        assert!(out.contains("  1 indexed document\n"));
    }

    #[test]
    fn no_evidence_shows_answer_and_hint() {
        let resp = json!({
            "answer": "No matching evidence was found in the indexed data for this question.",
            "evidence": "none",
            "sources": [],
            "plan": {"intent": "unknown", "clarifyingQuestions": []},
            "scope": {"service": "payments", "environment": null,
                      "fromUtc": "2026-09-23T08:00:00Z", "toUtc": null, "kinds": []},
            "timeline": {"status": "skipped", "skipReason": "not_diagnostic",
                         "observations": [], "hypotheses": [], "missingEvidence": []}
        });
        let out = render_ask(&resp, &plain(None));
        assert!(out.starts_with("Answer\n  No matching evidence was found"));
        assert!(
            out.contains("Evidence\n  None: no indexed documents or live observations matched.\n")
        );
        assert!(out.contains("widening the time window, removing --service/--env filters"));
        assert!(out.contains("  payments · since 2026-09-23 10:00 Europe/Stockholm\n"));
        assert!(!out.contains("Sources"));
    }

    #[test]
    fn skipped_timeline_hidden_by_request_but_shown_when_not_configured() {
        let mut resp = diagnostic();
        resp["timeline"] = json!({
            "status": "skipped", "skipReason": "disabled_by_request", "window": null, "baseline": null,
            "observations": [], "hypotheses": [],
            "missingEvidence": [{"subject": "live Datadog data", "reason": "skipped", "detail": "disabled by the request"}]
        });
        let out = render_ask(&resp, &plain(None));
        assert!(!out.contains("Live evidence"), "{out}");
        assert!(!out.contains("live observation"));

        resp["timeline"]["skipReason"] = json!("not_configured");
        resp["timeline"]["missingEvidence"][0]["detail"] =
            json!("DD_API_KEY/DD_APP_KEY are not configured");
        let out = render_ask(&resp, &plain(None));
        assert!(out.contains(
            "Live evidence\n  Not collected: not configured\n  Not checked:\n    - live Datadog data (skipped): DD_API_KEY/DD_APP_KEY are not configured\n"
        ), "{out}");
    }

    #[test]
    fn renders_plan_as_key_values_in_local_time() {
        let resp = json!({"plan": {
            "intent": "rootCauseWindow", "service": "checkout", "environment": null,
            "monitorId": "123", "incidentId": null, "metric": "trace.http.request.duration",
            "sloId": null,
            "window": {"fromUtc": "2026-09-22T22:00:00Z", "toUtc": "2026-09-23T22:00:00Z"},
            "filters": ["kind:logs"], "missingFields": ["environment"],
            "clarifyingQuestions": ["Which environment?"],
            "rewrittenQuery": "checkout failures staging"
        }});
        let out = render_plan(&resp, &plain(None));
        assert_eq!(
            out,
            "Plan\n\
             \x20 Intent:          rootCauseWindow\n\
             \x20 Service:         checkout\n\
             \x20 Window:          2026-09-23 00:00–24:00 Europe/Stockholm\n\
             \x20 Metric:          trace.http.request.duration\n\
             \x20 Monitor:         123\n\
             \x20 Filters:         kind:logs\n\
             \x20 Missing:         environment\n\
             \x20 Rewritten query: checkout failures staging\n\
             \n\
             Need more info\n\
             \x20 - Which environment?\n"
        );
    }

    #[test]
    fn window_uses_the_given_timezone_across_dst() {
        let tz = |name| RenderOptions::new(false, None, Some(name)).tz;
        // 2026-10-25 is the 25-hour day when Stockholm leaves summer time.
        let (from, to) = (json!("2026-10-24T22:00:00Z"), json!("2026-10-25T23:00:00Z"));
        assert_eq!(
            range(&from, &to, tz("Europe/Stockholm")).unwrap(),
            "2026-10-25 00:00–24:00"
        );
        assert_eq!(
            range(&from, &to, tz("UTC")).unwrap(),
            "2026-10-24 22:00 – 2026-10-25 23:00"
        );
        // Spring forward: 23-hour day.
        assert_eq!(
            range(
                &json!("2026-03-28T23:00:00Z"),
                &json!("2026-03-29T22:00:00Z"),
                tz("Europe/Stockholm")
            )
            .unwrap(),
            "2026-03-29 00:00–24:00"
        );
        assert_eq!(
            range(
                &json!("2026-09-23T10:00:00Z"),
                &json!("2026-09-23T11:30:00Z"),
                tz("Asia/Tokyo")
            )
            .unwrap(),
            "2026-09-23 19:00–20:30"
        );
        // Unknown zones fall back to UTC.
        assert_eq!(tz("Mars/Olympus"), Tz::UTC);
    }

    #[test]
    fn rewrites_doc_citations_only() {
        assert_eq!(rewrite_citations("a [DOC #12] b"), "a [12] b");
        assert_eq!(rewrite_citations("[DOC #1][DOC #2]"), "[1][2]");
        assert_eq!(rewrite_citations("[DOC #1, DOC #3]"), "[1, 3]");
        assert_eq!(rewrite_citations("see DOC #4."), "see [4].");
        assert_eq!(rewrite_citations("[obs-1] DOC #x"), "[obs-1] DOC #x");
        assert_eq!(
            rewrite_citations("[link](https://x/y) [DOC #2]"),
            "[link](https://x/y) [2]"
        );
    }
}
