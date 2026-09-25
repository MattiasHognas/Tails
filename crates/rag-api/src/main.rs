use anyhow::Result;
use rag_api::AppState;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .with(tracing_subscriber::fmt::layer())
        .init();

    let state = AppState::from_env()?;

    let addr = rag_api::listen_addr(std::env::var(rag_api::LISTEN_ADDR_VAR).ok().as_deref())?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("listening on {}", addr);
    rag_api::serve(listener, state).await?;
    Ok(())
}
