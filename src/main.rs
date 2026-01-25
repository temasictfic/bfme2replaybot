use std::env;
use std::path::PathBuf;
use tracing_subscriber::EnvFilter;

mod bot;
mod models;
mod parser;
mod renderer;

#[tokio::main]
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

    tracing::info!("Starting DCReplayBot...");
    tracing::info!("Assets path: {:?}", assets_path);

    // Run the bot
    bot::setup_bot(token, assets_path).await?;

    Ok(())
}
