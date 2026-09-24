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
    payload
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
        } => {
            // The server plans (service/env/time inference) and applies the plan to
            // retrieval; explicit flags take precedence over inferred values.
            let payload = ask_payload(question, env, service, kinds, timezone(tz));
            let mut req = http.post(format!("{}/ask", base)).json(&payload);
            if let Some(t) = token.as_ref() {
                req = req.bearer_auth(t);
            }
            let text = body_or_exit(req.send().await?).await?;

            // If the planner needs more info, print questions (soft guidance)
            if let Ok(resp) = serde_json::from_str::<serde_json::Value>(&text) {
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
        let p = ask_payload("q".into(), None, None, vec!["logs".into()], None);
        assert_eq!(p["kinds"], serde_json::json!(["logs"]));
    }

    #[test]
    fn explicit_tz_flag_wins() {
        assert_eq!(
            timezone(Some("Asia/Tokyo".into())).as_deref(),
            Some("Asia/Tokyo")
        );
    }
}
