mod render;

use clap::{Parser, Subcommand};
use render::{RenderOptions, render_ask, render_plan};
use std::io::{IsTerminal, Write};

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
        /// incident, dashboard, slo, git, catalog (service catalog), change (deploys and
        /// config changes)
        #[arg(long = "kind")]
        kinds: Vec<String>,
        /// IANA timezone for relative times like "yesterday" (default: local zone)
        #[arg(long)]
        tz: Option<String>,
        /// Don't query live Datadog metrics/logs for this question
        #[arg(long)]
        no_live_evidence: bool,
        /// Print the raw API response (and errors) as JSON, for scripts
        #[arg(long)]
        json: bool,
    },
    /// Just view the plan (intent + inferred fields)
    Plan {
        question: String,
        /// IANA timezone for relative times like "yesterday" (default: local zone)
        #[arg(long)]
        tz: Option<String>,
        /// Print the raw API response (and errors) as JSON, for scripts
        #[arg(long)]
        json: bool,
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

/// Why a command failed. Printed to stderr as one readable line (plus a hint)
/// or, with `--json`, as `{"error": {"code", "message", "stage", "retryable"}}`.
#[derive(Debug)]
enum Failure {
    /// The API answered with a non-2xx status.
    Api { status: u16, body: String },
    /// No usable response: `api_unreachable`, `timeout`, `request_failed` or
    /// `invalid_response`.
    Client {
        code: &'static str,
        message: String,
        retryable: bool,
        hint: Option<&'static str>,
    },
}

impl Failure {
    fn from_reqwest(e: &reqwest::Error, base: &str) -> Self {
        let cause = root_cause(e);
        if e.is_connect() {
            Failure::Client {
                code: "api_unreachable",
                message: format!("could not connect to {base}: {cause}"),
                retryable: true,
                hint: Some("Check that rag-api is running and that RAG_API_BASE points at it."),
            }
        } else if e.is_timeout() {
            Failure::Client {
                code: "timeout",
                message: format!("request to {base} timed out: {cause}"),
                retryable: true,
                hint: Some("This is likely transient; retry shortly."),
            }
        } else {
            Failure::Client {
                code: "request_failed",
                message: format!("request to {base} failed: {cause}"),
                retryable: false,
                hint: Some("Check RAG_API_BASE and RAG_API_TOKEN."),
            }
        }
    }

    fn invalid_response(status: u16, body: &str) -> Self {
        let snippet: String = body.chars().take(200).collect();
        Failure::Client {
            code: "invalid_response",
            message: format!("API returned HTTP {status} with a body that is not JSON: {snippet}"),
            retryable: false,
            hint: Some("Check that RAG_API_BASE points at rag-api."),
        }
    }

    /// One readable line, e.g. `error [api_unreachable] (http://…): connection refused`,
    /// plus a hint line.
    fn human(&self, base: &str) -> String {
        match self {
            Failure::Api { status, body } => format_api_error(*status, body),
            Failure::Client {
                code,
                message,
                hint,
                ..
            } => {
                // The base URL is shown once, in parentheses.
                let detail = message
                    .split_once(&format!("{base}: "))
                    .map_or(message.as_str(), |(_, d)| d);
                let mut out = format!("error [{code}] ({base}): {detail}");
                if let Some(h) = hint {
                    out.push('\n');
                    out.push_str(h);
                }
                out
            }
        }
    }

    /// A single-line JSON error. A typed API error body is passed through;
    /// anything else is wrapped in the same shape.
    fn json(&self) -> String {
        let synthesized = |code: &str, message: String, retryable: bool| {
            serde_json::json!({"error": {
                "code": code, "message": message, "stage": null, "retryable": retryable,
            }})
            .to_string()
        };
        match self {
            Failure::Api { status, body } => {
                match serde_json::from_str::<serde_json::Value>(body) {
                    Ok(v) if v["error"]["code"].is_string() => {
                        if body.contains('\n') {
                            v.to_string()
                        } else {
                            body.trim().to_string()
                        }
                    }
                    _ => {
                        let snippet: String = body.trim().chars().take(200).collect();
                        synthesized(
                            "upstream_http_error",
                            format!("API returned HTTP {status}: {snippet}"),
                            *status >= 500 || *status == 429,
                        )
                    }
                }
            }
            Failure::Client {
                code,
                message,
                retryable,
                ..
            } => synthesized(code, message.clone(), *retryable),
        }
    }
}

/// The innermost error message, e.g. `connection refused` instead of reqwest's
/// `error sending request for url (…)`.
fn root_cause(e: &(dyn std::error::Error + 'static)) -> String {
    let mut cur = e;
    while let Some(next) = cur.source() {
        cur = next;
    }
    if let Some(io) = cur.downcast_ref::<std::io::Error>()
        && io.kind() == std::io::ErrorKind::ConnectionRefused
    {
        return "connection refused".into();
    }
    cur.to_string()
}

/// Where the CLI talks to and how it prints.
struct Settings {
    base: String,
    token: Option<String>,
    color: bool,
    /// Wrap width for human output; `None` does not wrap.
    width: Option<usize>,
}

impl Settings {
    fn from_env() -> Self {
        let stdout = std::io::stdout();
        let terminal = stdout.is_terminal();
        let no_color = std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
        // Piped output is not wrapped, so pagers and files get whole lines.
        let width = terminal.then(|| {
            terminal_size::terminal_size_of(&stdout)
                .map_or(80, |(w, _)| usize::from(w.0))
                .max(40)
        });
        Self {
            base: std::env::var("RAG_API_BASE").unwrap_or_else(|_| "http://localhost:5191".into()),
            token: std::env::var("RAG_API_TOKEN").ok(), // Optional: bearer (e.g., AAD via managed identity)
            color: terminal && !no_color,
            width,
        }
    }
}

/// What to print: the result on stdout, errors on stderr.
#[derive(Debug, PartialEq)]
struct Output {
    stdout: String,
    stderr: String,
    code: i32,
}

/// POST `payload` to `route` and return the parsed JSON body with its raw text.
async fn call(
    s: &Settings,
    route: &str,
    payload: &serde_json::Value,
) -> Result<(String, serde_json::Value), Failure> {
    let http = reqwest::Client::new();
    let mut req = http.post(format!("{}{route}", s.base)).json(payload);
    if let Some(t) = s.token.as_ref() {
        req = req.bearer_auth(t);
    }
    let r = req
        .send()
        .await
        .map_err(|e| Failure::from_reqwest(&e, &s.base))?;
    let status = r.status();
    let body = r
        .text()
        .await
        .map_err(|e| Failure::from_reqwest(&e, &s.base))?;
    if !status.is_success() {
        return Err(Failure::Api {
            status: status.as_u16(),
            body,
        });
    }
    match serde_json::from_str(&body) {
        Ok(v) => Ok((body, v)),
        Err(_) => Err(Failure::invalid_response(status.as_u16(), &body)),
    }
}

async fn run(cmd: Cmd, s: &Settings) -> Output {
    let (route, payload, json, tz) = match cmd {
        Cmd::Plan { question, tz, json } => {
            let tz = timezone(tz);
            let payload = serde_json::json!({ "question": question, "timezone": tz });
            ("/ask/plan", payload, json, tz)
        }
        Cmd::Ask {
            question,
            env,
            service,
            kinds,
            tz,
            no_live_evidence,
            json,
        } => {
            // The server plans (service/env/time inference) and applies the plan to
            // retrieval; explicit flags take precedence over inferred values.
            let tz = timezone(tz);
            let payload = ask_payload(question, env, service, kinds, tz.clone(), no_live_evidence);
            ("/ask", payload, json, tz)
        }
    };
    match call(s, route, &payload).await {
        Ok((body, value)) => {
            let stdout = if json {
                format!("{body}\n")
            } else {
                let opts = RenderOptions::new(s.color, s.width, tz.as_deref());
                if route == "/ask" {
                    render_ask(&value, &opts)
                } else {
                    render_plan(&value, &opts)
                }
            };
            Output {
                stdout,
                stderr: String::new(),
                code: 0,
            }
        }
        Err(f) => Output {
            stdout: String::new(),
            stderr: format!("{}\n", if json { f.json() } else { f.human(&s.base) }),
            code: 1,
        },
    }
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let out = run(cli.cmd, &Settings::from_env()).await;
    // Ignore write errors such as a closed pipe (`rag-cli ask … | head`).
    let _ = std::io::stdout().write_all(out.stdout.as_bytes());
    let _ = std::io::stderr().write_all(out.stderr.as_bytes());
    std::process::exit(out.code);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

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
    fn explicit_tz_flag_wins() {
        assert_eq!(
            timezone(Some("Asia/Tokyo".into())).as_deref(),
            Some("Asia/Tokyo")
        );
    }

    fn settings(base: String) -> Settings {
        Settings {
            base,
            token: None,
            color: false,
            width: None,
        }
    }

    fn ask(json: bool) -> Cmd {
        Cmd::Ask {
            question: "why did auth-api fail yesterday?".into(),
            env: None,
            service: None,
            kinds: vec![],
            tz: Some("Europe/Stockholm".into()),
            no_live_evidence: false,
            json,
        }
    }

    async fn api(route: &str, resp: ResponseTemplate) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(route))
            .respond_with(resp)
            .mount(&server)
            .await;
        server
    }

    /// A base URL nothing listens on.
    fn unused_base() -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        format!("http://127.0.0.1:{port}")
    }

    const ASK_BODY: &str = r#"{"answer":"It failed [DOC #1].","evidence":"found","sources":[{"n":1,"title":"auth-api 5xx","kind":"incident","timestamp":null,"service":"auth-api","environment":"prod","uri":"https://dd/incidents/1"}],"plan":{"clarifyingQuestions":[]},"scope":{},"timeline":null}"#;
    const TYPED_ERROR: &str = r#"{"error":{"code":"upstream_unavailable","message":"retrieval unavailable","stage":"retrieval","retryable":true}}"#;

    #[tokio::test]
    async fn json_flag_passes_the_body_through_unchanged() {
        let server = api("/ask", ResponseTemplate::new(200).set_body_string(ASK_BODY)).await;
        let out = run(ask(true), &settings(server.uri())).await;
        assert_eq!(
            out,
            Output {
                stdout: format!("{ASK_BODY}\n"),
                stderr: String::new(),
                code: 0
            }
        );

        let body = r#"{"plan":{"intent":"unknown"}}"#;
        let server = api(
            "/ask/plan",
            ResponseTemplate::new(200).set_body_string(body),
        )
        .await;
        let plan = Cmd::Plan {
            question: "q".into(),
            tz: None,
            json: true,
        };
        let out = run(plan, &settings(server.uri())).await;
        assert_eq!(out.stdout, format!("{body}\n"));
    }

    #[tokio::test]
    async fn default_output_is_rendered_on_stdout() {
        let server = api("/ask", ResponseTemplate::new(200).set_body_string(ASK_BODY)).await;
        let out = run(ask(false), &settings(server.uri())).await;
        assert_eq!(out.code, 0);
        assert!(out.stderr.is_empty());
        assert!(
            out.stdout.starts_with("Answer\n  It failed [1].\n"),
            "{}",
            out.stdout
        );
        assert!(
            out.stdout
                .contains("Sources\n  [1] auth-api 5xx (incident) · https://dd/incidents/1\n")
        );
    }

    #[tokio::test]
    async fn typed_api_errors_go_to_stderr_in_both_modes() {
        let server = api(
            "/ask",
            ResponseTemplate::new(503).set_body_string(TYPED_ERROR),
        )
        .await;
        let out = run(ask(true), &settings(server.uri())).await;
        assert_eq!(
            out,
            Output {
                stdout: String::new(),
                stderr: format!("{TYPED_ERROR}\n"),
                code: 1
            }
        );
        let out = run(ask(false), &settings(server.uri())).await;
        assert_eq!(
            out,
            Output {
                stdout: String::new(),
                stderr: "error [upstream_unavailable] at stage 'retrieval' (HTTP 503): retrieval unavailable\nThis is likely transient; retry shortly.\n".into(),
                code: 1
            }
        );
    }

    #[tokio::test]
    async fn untyped_error_bodies_are_synthesized_in_json_mode() {
        let html = "<html>\n<body>Bad Gateway</body>\n</html>";
        let server = api("/ask", ResponseTemplate::new(502).set_body_string(html)).await;
        let out = run(ask(true), &settings(server.uri())).await;
        assert_eq!(out.code, 1);
        assert!(out.stdout.is_empty());
        assert_eq!(out.stderr.lines().count(), 1);
        let err: serde_json::Value = serde_json::from_str(&out.stderr).unwrap();
        assert_eq!(
            err,
            json!({"error": {"code": "upstream_http_error",
                "message": format!("API returned HTTP 502: {html}"),
                "stage": null, "retryable": true}})
        );
        let out = run(ask(false), &settings(server.uri())).await;
        assert_eq!(
            out.stderr,
            format!("error: API returned HTTP 502: {html}\n")
        );

        let server = api("/ask", ResponseTemplate::new(404).set_body_string("nope")).await;
        let out = run(ask(true), &settings(server.uri())).await;
        let err: serde_json::Value = serde_json::from_str(&out.stderr).unwrap();
        assert_eq!(err["error"]["retryable"], false);
    }

    #[tokio::test]
    async fn non_json_success_is_invalid_response() {
        let server = api("/ask", ResponseTemplate::new(200).set_body_string("hello")).await;
        let base = server.uri();
        let out = run(ask(true), &settings(base.clone())).await;
        assert_eq!(out.code, 1);
        assert!(out.stdout.is_empty());
        let err: serde_json::Value = serde_json::from_str(&out.stderr).unwrap();
        assert_eq!(err["error"]["code"], "invalid_response");
        assert_eq!(err["error"]["retryable"], false);
        let out = run(ask(false), &settings(base.clone())).await;
        assert_eq!(
            out.stderr,
            format!(
                "error [invalid_response] ({base}): API returned HTTP 200 with a body that is not JSON: hello\nCheck that RAG_API_BASE points at rag-api.\n"
            )
        );
    }

    #[tokio::test]
    async fn unreachable_api_is_a_clean_error_in_both_modes() {
        let base = unused_base();
        let out = run(ask(false), &settings(base.clone())).await;
        assert_eq!(
            out,
            Output {
                stdout: String::new(),
                stderr: format!(
                    "error [api_unreachable] ({base}): connection refused\nCheck that rag-api is running and that RAG_API_BASE points at it.\n"
                ),
                code: 1
            }
        );

        let out = run(ask(true), &settings(base.clone())).await;
        assert_eq!(out.code, 1);
        assert!(out.stdout.is_empty());
        assert_eq!(out.stderr.lines().count(), 1);
        let err: serde_json::Value = serde_json::from_str(&out.stderr).unwrap();
        assert_eq!(
            err,
            json!({"error": {"code": "api_unreachable",
                "message": format!("could not connect to {base}: connection refused"),
                "stage": null, "retryable": true}})
        );
    }
}
