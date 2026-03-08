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
    /// Ask a question (planner may infer service/env; server chooses K)
    Ask {
        question: String,
        #[arg(long)]
        env: Option<String>,
        #[arg(long)]
        service: Option<String>,
    },
    /// Just view the plan (intent + inferred fields)
    Plan { question: String },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let base = std::env::var("RAG_API_BASE").unwrap_or_else(|_| "http://localhost:5191".into());
    let token = std::env::var("RAG_API_TOKEN").ok(); // Optional: bearer (e.g., AAD via managed identity)
    let http = reqwest::Client::new();

    match cli.cmd {
        Cmd::Plan { question } => {
            let mut req = http
                .post(format!("{}/ask/plan", base))
                .json(&serde_json::json!({ "question": question }));
            if let Some(t) = token.as_ref() {
                req = req.bearer_auth(t);
            }
            let r = req.send().await?;
            println!("{}", r.text().await?);
        }
        Cmd::Ask {
            question,
            env,
            service,
        } => {
            // 1) Plan
            let mut req = http
                .post(format!("{}/ask/plan", base))
                .json(&serde_json::json!({ "question": &question }));
            if let Some(t) = token.as_ref() {
                req = req.bearer_auth(t);
            }
            let plan_resp: serde_json::Value = req.send().await?.json().await?;
            let plan = &plan_resp["plan"];

            // 2) If the planner needs more info, print questions (soft guidance)
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

            // 3) Prefer user-specified flags; fall back to plan inference
            let eff_env = env.or_else(|| plan["environment"].as_str().map(|s| s.to_string()));
            let eff_service = service.or_else(|| plan["service"].as_str().map(|s| s.to_string()));

            // 4) Ask (server decides K)
            let payload = serde_json::json!({
              "question": question,
              "env": eff_env,
              "service": eff_service,
              "from_utc": plan["window"]["fromUtc"],
              "to_utc": plan["window"]["toUtc"],
              "filters": plan["filters"],
              "rewritten_query": plan["rewrittenQuery"],
            });

            let mut req2 = http.post(format!("{}/ask", base)).json(&payload);
            if let Some(t) = token.as_ref() {
                req2 = req2.bearer_auth(t);
            }
            let r = req2.send().await?;
            println!("{}", r.text().await?);
        }
    }
    Ok(())
}
