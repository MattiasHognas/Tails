//! Runs the built `rag-cli` binary against a mocked API and checks what goes
//! to stdout and stderr, and the exit code.

use std::process::{Command, Output};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TYPED_ERROR: &str = r#"{"error":{"code":"timeout","message":"embedding timed out","stage":"embedding","retryable":true}}"#;

async fn rag(base: &str, args: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rag-cli"));
    cmd.args(args)
        .env("RAG_API_BASE", base)
        .env_remove("RAG_API_TOKEN")
        .env_remove("NO_COLOR");
    tokio::task::spawn_blocking(move || cmd.output().unwrap())
        .await
        .unwrap()
}

async fn api(status: u16, body: &str) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/ask"))
        .respond_with(ResponseTemplate::new(status).set_body_string(body))
        .mount(&server)
        .await;
    server
}

fn text(b: &[u8]) -> String {
    String::from_utf8(b.to_vec()).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn success_prints_plain_text_or_raw_json_on_stdout() {
    let body = r#"{"answer":"All good [DOC #1].","evidence":"found","sources":[],"plan":{},"scope":{},"timeline":null}"#;
    let server = api(200, body).await;

    let out = rag(&server.uri(), &["ask", "q", "--tz", "UTC"]).await;
    assert_eq!(out.status.code(), Some(0));
    let stdout = text(&out.stdout);
    assert!(stdout.starts_with("Answer\n  All good [1].\n"), "{stdout}");
    assert!(!stdout.contains('\x1b'), "piped output has no ANSI codes");
    assert!(out.stderr.is_empty());

    let out = rag(&server.uri(), &["ask", "--json", "q"]).await;
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(text(&out.stdout), format!("{body}\n"));
    assert!(out.stderr.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn typed_api_error_exits_1_with_stderr_only() {
    let server = api(504, TYPED_ERROR).await;

    let out = rag(&server.uri(), &["ask", "--json", "q"]).await;
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    assert_eq!(text(&out.stderr), format!("{TYPED_ERROR}\n"));

    let out = rag(&server.uri(), &["ask", "q"]).await;
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    assert_eq!(
        text(&out.stderr),
        "error [timeout] at stage 'embedding' (HTTP 504): embedding timed out\nThis is likely transient; retry shortly.\n"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn unreachable_api_exits_1_with_a_clean_error() {
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let base = format!("http://127.0.0.1:{port}");

    let out = rag(&base, &["plan", "--json", "q"]).await;
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    let err: serde_json::Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(err["error"]["code"], "api_unreachable");
    assert_eq!(err["error"]["retryable"], true);

    let out = rag(&base, &["plan", "q"]).await;
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    let stderr = text(&out.stderr);
    assert!(
        stderr.starts_with(&format!(
            "error [api_unreachable] ({base}): connection refused\n"
        )),
        "{stderr}"
    );
    assert!(stderr.contains("RAG_API_BASE"));
    assert!(!stderr.contains("Error:"));
}

#[tokio::test(flavor = "multi_thread")]
async fn bad_arguments_exit_2_in_both_modes() {
    for args in [&["ask"][..], &["ask", "--json"], &["ask", "q", "--bogus"]] {
        let out = rag("http://127.0.0.1:9", args).await;
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        assert!(out.stdout.is_empty());
        assert!(text(&out.stderr).starts_with("error:"));
    }
}
