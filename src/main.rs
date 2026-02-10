use std::env;
use std::path::PathBuf;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

use dcreplaybot::bot::setup_bot;

/// Minimal HTTP health check server
async fn health_check_server(port: u16) {
    let addr = format!("0.0.0.0:{}", port);
    let listener = match TcpListener::bind(&addr).await {
        Ok(l) => {
            tracing::info!("Health check listening on {}", addr);
            l
        }
        Err(e) => {
            tracing::error!("Failed to bind health check on {}: {}", addr, e);
            return;
        }
    };

    loop {
        match listener.accept().await {
            Ok((mut stream, _)) => {
                // Read and discard the request bytes before responding
                let mut discard = [0u8; 1024];
                let _ = stream.read(&mut discard).await;
                let response = "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nOK";
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.shutdown().await;
            }
            Err(e) => {
                tracing::warn!("Health check accept error: {}", e);
            }
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    // Load .env file if present
    let _ = dotenvy::dotenv();

    // Get Discord token
    let token = env::var("DISCORD_TOKEN").expect("DISCORD_TOKEN environment variable not set");

    // Determine assets path
    let assets_path = env::var("ASSETS_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("assets"));

    // Health check port (default 8000 for Koyeb)
    let port: u16 = env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8000);

    tracing::info!("Starting DCReplayBot...");
    tracing::info!("Assets path: {:?}", assets_path);

    // Start health check server in background
    tokio::spawn(health_check_server(port));

    // Run the bot
    setup_bot(token, assets_path).await?;

    Ok(())
}
