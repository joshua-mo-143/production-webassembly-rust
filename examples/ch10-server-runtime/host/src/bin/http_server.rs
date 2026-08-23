use std::env;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use ch10_host::http::router;
use ch10_host::{RequestLimits, ServerRuntime};
use tokio::net::TcpListener;

const DEFAULT_BIND_ADDRESS: &str = "127.0.0.1:3000";

#[tokio::main]
async fn main() -> Result<()> {
    let component_path = env::args_os().nth(1).map_or_else(
        || PathBuf::from("target/wasm32-wasip2/debug/ch10_guest.wasm"),
        PathBuf::from,
    );
    let bind_address =
        env::var("CH10_BIND_ADDR").unwrap_or_else(|_| DEFAULT_BIND_ADDRESS.to_owned());
    let runtime = Arc::new(ServerRuntime::load(
        component_path,
        RequestLimits::default(),
    )?);
    let listener = TcpListener::bind(&bind_address)
        .await
        .with_context(|| format!("failed to bind HTTP server to {bind_address}"))?;
    let local_address = listener.local_addr()?;

    println!("component_cache: compiled=1");
    println!("http_server: listening=http://{local_address}");

    axum::serve(listener, router(runtime))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("HTTP server failed")
}

async fn shutdown_signal() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        eprintln!("failed to install Ctrl-C handler: {error}");
    }
}
