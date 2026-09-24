use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "rag")]
#[command(about = "Datadog RAG CLI (Rust) — server chooses K; planner can infer service/env")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Ask a question (server plans: infers service/env/time; explicit flags win)
    Ask {
        question: String,
        #[arg(long)]
        env: Option<String>,
        #[arg(long)]
        service: Option<String>,
        /// Restrict evidence to a source kind (repeatable): logs, metrics, monitor,
        /// incident, dashboard, slo, git
        #[arg(long = "kind")]
        kinds: Vec<String>,
        /// IANA timezone for relative times like "yesterday" (default: local zone)
        #[arg(long)]
        tz: Option<String>,
        /// Don't query live Datadog metrics/logs for this question
        #[arg(long)]
        no_live_evidence: bool,
    },
    /// Just view the plan (intent + inferred fields)
    Plan {
        question: String,
        /// IANA timezone for relative times like "yesterday" (default: local zone)
        #[arg(long)]
        tz: Option<String>,
    },
}

/// `--tz`, else `TZ` if it is an IANA name, else the system zone. `None` lets the
/// server default to UTC.
fn timezone(flag: Option<String>) -> Option<String> {
    flag.or_else(|| {
        std::env::var("TZ")
            .ok()
            .map(|tz| tz.trim_start_matches(':').to_string())
            .filter(|tz| tz.contains('/'))
    })
    .or_else(|| iana_time_zone::get_timezone().ok())
}

fn ask_payload(
    question: String,
    env: Option<String>,
    service: Option<String>,
    kinds: Vec<String>,
    timezone: Option<String>,
    no_live_evidence: bool,
) -> serde_json::Value {
    let mut payload = serde_json::json!({
        "question": question,
        "env": env,
        "service": service,
        "timezone": timezone,
    });
    if !kinds.is_empty() {
        payload["kinds"] = serde_json::json!(kinds);
    }
    if no_live_evidence {
        payload["live_evidence"] = serde_json::json!(false);
    }
    payload
}

/// Render the `/ask` response's live-evidence `timeline` as three sections:
/// observed facts, hypotheses and missing evidence. `None` when live evidence
/// was not relevant (non-diagnostic question) or absent.
fn format_timeline(t: &serde_json::Value) -> Option<String> {
    use std::fmt::Write;
    if !t.is_object() || t["skipReason"] == "not_diagnostic" {
        return None;
    }
    let s = |v: &serde_json::Value| v.as_str().unwrap_or("").to_string();
    let list = |v: &serde_json::Value| v.as_array().cloned().unwrap_or_default();
    let mut out = String::from("Live evidence");
    if let Some(w) = t["window"].as_object() {
        let _ = write!(out, " ({} to {})", s(&w["fromUtc"]), s(&w["toUtc"]));
    }
    out.push_str(":\n");
    out.push_str("  Observed:\n");
    let obs = list(&t["observations"]);
    if obs.is_empty() {
        out.push_str("    (none)\n");
    }
    for o in obs {
        let _ = writeln!(
            out,
            "    [{}] {} .. {}  {}\n         {}",
            s(&o["id"]),
            s(&o["startUtc"]),
            s(&o["endUtc"]),
            s(&o["summary"]),
            s(&o["link"])
        );
    }
    out.push_str("  Hypotheses (unverified):\n");
    let hyp = list(&t["hypotheses"]);
    if hyp.is_empty() {
        out.push_str("    (none)\n");
    }
    for h in hyp {
        let ids: Vec<String> = list(&h["observationIds"]).iter().map(s).collect();
        let _ = writeln!(out, "    - {} [{}]", s(&h["statement"]), ids.join(", "));
    }
    out.push_str("  Missing evidence:\n");
    let missing = list(&t["missingEvidence"]);
    if missing.is_empty() {
        out.push_str("    (none)\n");
    }
    for m in missing {
        let _ = writeln!(
            out,
            "    - {} ({}): {}",
            s(&m["subject"]),
            s(&m["reason"]),
            s(&m["detail"])
        );
    }
    Some(out)
}

/// Render a non-2xx API response. Typed errors look like
/// `{"error": {"code", "message", "stage", "retryable"}}`.
fn format_api_error(status: u16, body: &str) -> String {
    let parsed: Option<serde_json::Value> = serde_json::from_str(body).ok();
    let err = parsed.as_ref().map(|v| &v["error"]);
    match err.and_then(|e| e["code"].as_str().map(|code| (e, code))) {
        Some((e, code)) => {
            let mut out = format!("error [{code}]");
            if let Some(stage) = e["stage"].as_str() {
                out.push_str(&format!(" at stage '{stage}'"));
            }
            out.push_str(&format!(
                " (HTTP {status}): {}",
                e["message"].as_str().unwrap_or("no message")
            ));
            if e["retryable"].as_bool() == Some(true) {
                out.push_str("\nThis is likely transient; retry shortly.");
            }
            out
        }
        None => {
            let snippet: String = body.chars().take(300).collect();
            format!("error: API returned HTTP {status}: {snippet}")
        }
    }
}

/// Return the body of a successful response. On an API error, print the
/// typed error to stderr and exit non-zero instead of printing an answer.
async fn body_or_exit(r: reqwest::Response) -> Result<String> {
    let status = r.status();
    let body = r.text().await?;
    if status.is_success() {
        return Ok(body);
    }
    eprintln!("{}", format_api_error(status.as_u16(), &body));
    std::process::exit(1);
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let base = std::env::var("RAG_API_BASE").unwrap_or_else(|_| "http://localhost:5191".into());
    let token = std::env::var("RAG_API_TOKEN").ok(); // Optional: bearer (e.g., AAD via managed identity)
    let http = reqwest::Client::new();

    match cli.cmd {
        Cmd::Plan { question, tz } => {
            let mut req = http
                .post(format!("{}/ask/plan", base))
                .json(&serde_json::json!({ "question": question, "timezone": timezone(tz) }));
            if let Some(t) = token.as_ref() {
                req = req.bearer_auth(t);
            }
            let r = req.send().await?;
            println!("{}", body_or_exit(r).await?);
        }
        Cmd::Ask {
            question,
            env,
            service,
            kinds,
            tz,
            no_live_evidence,
        } => {
            // The server plans (service/env/time inference) and applies the plan to
            // retrieval; explicit flags take precedence over inferred values.
            let payload = ask_payload(
                question,
                env,
                service,
                kinds,
                timezone(tz),
                no_live_evidence,
            );
            let mut req = http.post(format!("{}/ask", base)).json(&payload);
            if let Some(t) = token.as_ref() {
                req = req.bearer_auth(t);
            }
            let text = body_or_exit(req.send().await?).await?;

            // If the planner needs more info, print questions (soft guidance)
            if let Ok(resp) = serde_json::from_str::<serde_json::Value>(&text) {
                if let Some(timeline) = format_timeline(&resp["timeline"]) {
                    eprintln!("{timeline}");
                }
                let plan = &resp["plan"];
                if plan["missingFields"]
                    .as_array()
                    .map(|a| !a.is_empty())
                    .unwrap_or(false)
                    && let Some(qs) = plan["clarifyingQuestions"].as_array()
                {
                    eprintln!("Need more info:");
                    for qn in qs {
                        if let Some(qs) = qn.as_str() {
                            eprintln!("- {}", qs);
                        }
                    }
                }
            }
            println!("{}", text);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_typed_errors() {
        let body = r#"{"error":{"code":"retrieval_failed","message":"retrieval failed: upstream returned HTTP 404","stage":"retrieval","retryable":false}}"#;
        assert_eq!(
            format_api_error(502, body),
            "error [retrieval_failed] at stage 'retrieval' (HTTP 502): retrieval failed: upstream returned HTTP 404"
        );
        let body = r#"{"error":{"code":"timeout","message":"embedding timed out","stage":"embedding","retryable":true}}"#;
        let out = format_api_error(504, body);
        assert!(out.starts_with("error [timeout] at stage 'embedding' (HTTP 504)"));
        assert!(out.contains("retry shortly"));
    }

    #[test]
    fn formats_untyped_errors() {
        assert_eq!(
            format_api_error(500, "boom"),
            "error: API returned HTTP 500: boom"
        );
    }

    #[test]
    fn ask_payload_sends_only_explicit_values() {
        let p = ask_payload(
            "why?".into(),
            None,
            Some("auth-api".into()),
            vec![],
            Some("Europe/Stockholm".into()),
            false,
        );
        assert_eq!(
            p,
            serde_json::json!({
                "question": "why?",
                "env": null,
                "service": "auth-api",
                "timezone": "Europe/Stockholm",
            })
        );
        let p = ask_payload("q".into(), None, None, vec!["logs".into()], None, true);
        assert_eq!(p["kinds"], serde_json::json!(["logs"]));
        assert_eq!(p["live_evidence"], false);
    }

    #[test]
    fn formats_timeline_sections() {
        let t = serde_json::json!({
            "status": "collected",
            "window": {"fromUtc": "2026-09-22T22:00:00Z", "toUtc": "2026-09-23T22:00:00Z"},
            "observations": [{
                "id": "obs-1", "kind": "spike", "startUtc": "2026-09-23T10:00:00Z",
                "endUtc": "2026-09-23T11:00:00Z", "summary": "latency above baseline",
                "link": "https://app.datadoghq.eu/metric/explorer?x"
            }],
            "hypotheses": [{"statement": "pool exhaustion", "observationIds": ["obs-1"]}],
            "missingEvidence": [{"subject": "metrics for billing", "reason": "no_metrics_discovered", "detail": "only logs checked"}]
        });
        let out = format_timeline(&t).unwrap();
        assert!(out.starts_with("Live evidence (2026-09-22T22:00:00Z to 2026-09-23T22:00:00Z):"));
        assert!(out.contains("  Observed:\n    [obs-1] 2026-09-23T10:00:00Z .. 2026-09-23T11:00:00Z  latency above baseline"));
        assert!(out.contains("  Hypotheses (unverified):\n    - pool exhaustion [obs-1]"));
        assert!(
            out.contains("    - metrics for billing (no_metrics_discovered): only logs checked")
        );

        assert!(
            format_timeline(
                &serde_json::json!({"status": "skipped", "skipReason": "not_diagnostic"})
            )
            .is_none()
        );
        assert!(format_timeline(&serde_json::Value::Null).is_none());
    }

    #[test]
    fn explicit_tz_flag_wins() {
        assert_eq!(
            timezone(Some("Asia/Tokyo".into())).as_deref(),
            Some("Asia/Tokyo")
        );
    }
}
