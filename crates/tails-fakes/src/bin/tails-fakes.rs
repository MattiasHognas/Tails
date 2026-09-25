//! `tails-fakes serve`: the fake Datadog API and the fake OpenAI chat model as one
//! standalone server, for the end-to-end run of the built binaries. See
//! [`tails_fakes::server`] for what it serves.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tails_fakes::{datadog, questions, server::Fakes};

#[derive(Parser)]
#[command(
    name = "tails-fakes",
    about = "Fake Datadog and OpenAI chat for end-to-end tests"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Serve the fakes for the incident question set until interrupted.
    Serve {
        /// questions.json (default: the question set in this checkout).
        #[arg(long)]
        questions: Option<PathBuf>,
        /// corpus.json (default: the question set's corpus in this checkout).
        #[arg(long)]
        corpus: Option<PathBuf>,
        /// Recorded Datadog fixtures indexed with the corpus (default:
        /// crates/rag-core/tests/fixtures/datadog in this checkout).
        #[arg(long)]
        fixtures: Option<PathBuf>,
        #[arg(long, default_value = "127.0.0.1:8900")]
        addr: String,
        /// The `DD_SITE` the indexer uses: source URIs, which the answer model cites
        /// by, are built from it.
        #[arg(long, default_value = "datadoghq.eu")]
        site: String,
        /// Also serve `/v1/embeddings` with the deterministic hashed bag-of-words
        /// embeddings. Only for running the end-to-end driver without an embedding
        /// model; the real run takes embeddings from text-embeddings-inference.
        #[arg(long)]
        embeddings: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let Cmd::Serve {
        questions: questions_path,
        corpus,
        fixtures,
        addr,
        site,
        embeddings,
    } = Cli::parse().cmd;
    let dir = questions::data_dir();
    let questions_path = questions_path.unwrap_or_else(|| dir.join("questions.json"));
    let corpus = corpus.unwrap_or_else(|| dir.join("corpus.json"));
    let fixtures = fixtures.unwrap_or_else(datadog::fixture_dir);
    let (dataset, corpus) = questions::load(&questions_path, &corpus, &fixtures);
    let n = dataset.questions.len();
    let fakes = Fakes::new(dataset, corpus, &site, embeddings).await?;
    let listener = std::net::TcpListener::bind(&addr).with_context(|| format!("binding {addr}"))?;
    let server = fakes.serve(listener).await;
    println!(
        "tails-fakes: serving {n} questions on {} (embeddings: {})",
        server.uri(),
        if embeddings { "fake" } else { "off" }
    );
    tokio::signal::ctrl_c().await?;
    Ok(())
}
