use droplink_relay::{room::Rooms, server::serve};
use std::sync::{Arc, Mutex};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .json()
        .init();

    let port = std::env::var("PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(8080);
    let addr = format!("0.0.0.0:{port}");
    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(listener) => listener,
        Err(err) => {
            tracing::error!(error = %err, %addr, "bind failed");
            std::process::exit(1);
        }
    };
    tracing::info!(%addr, "droplink-relay listening");

    let state = Arc::new(Mutex::new(Rooms::new()));
    let shutdown = async {
        let _ = tokio::signal::ctrl_c().await;
        tracing::info!("shutting down");
    };

    if let Err(err) = serve(listener, state, shutdown).await {
        tracing::error!(error = %err, "server error");
        std::process::exit(1);
    }
}
