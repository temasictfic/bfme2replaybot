use crate::renderer::{load_font, load_map_image};
use ab_glyph::FontArc;
use image::RgbImage;
use poise::serenity_prelude as serenity;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use super::constants::{COOLDOWN_SECS, PENDING_EXPIRY_SECS};
use super::handler::handle_message;
use super::pagination::handle_component_interaction;

pub struct PendingReplays {
    pub replays: Vec<(String, Vec<u8>)>,
    pub total: usize,
    pub shown: usize,
    pub created_at: Instant,
    pub channel_id: serenity::ChannelId,
}

/// Remove expired entries from the pending replays map (call with lock already held).
pub fn cleanup_expired_pending_inner(map: &mut HashMap<String, PendingReplays>) {
    let now = Instant::now();
    map.retain(|_, v| now.duration_since(v.created_at).as_secs() < PENDING_EXPIRY_SECS);
}

pub struct Data {
    pub font: Arc<FontArc>,
    pub map_image: Arc<RgbImage>,
    pub bot_id: serenity::UserId,
    pub pending_replays: Mutex<HashMap<String, PendingReplays>>,
    pub cooldowns: Mutex<HashMap<serenity::ChannelId, Instant>>,
}

impl Data {
    /// Lock cooldowns mutex. On poison: recover (stale timestamps are harmless).
    pub fn lock_cooldowns(
        &self,
    ) -> std::sync::MutexGuard<'_, HashMap<serenity::ChannelId, Instant>> {
        self.cooldowns.lock().unwrap_or_else(|e| {
            tracing::warn!("Cooldowns mutex poisoned, recovering");
            e.into_inner()
        })
    }

    /// Lock pending replays mutex. On poison: clear state (fail closed).
    pub fn lock_pending_replays(
        &self,
    ) -> std::sync::MutexGuard<'_, HashMap<String, PendingReplays>> {
        self.pending_replays.lock().unwrap_or_else(|e| {
            tracing::warn!("Pending replays mutex poisoned, clearing state");
            let mut guard = e.into_inner();
            guard.clear();
            guard
        })
    }

    /// Check if a channel is on cooldown (returns true if still cooling down)
    pub fn check_cooldown(&self, channel_id: serenity::ChannelId) -> bool {
        let cooldowns = self.lock_cooldowns();
        cooldowns
            .get(&channel_id)
            .is_some_and(|last| last.elapsed().as_secs() < COOLDOWN_SECS)
    }

    /// Record that a channel was just used
    pub fn set_cooldown(&self, channel_id: serenity::ChannelId) {
        let mut cooldowns = self.lock_cooldowns();
        cooldowns.insert(channel_id, Instant::now());
    }
}

type Error = Box<dyn std::error::Error + Send + Sync>;

/// Set up and run the Discord bot
pub async fn setup_bot(token: String, assets_path: PathBuf) -> Result<(), Error> {
    // Load font at startup
    let font_path = assets_path.join("fonts").join("NotoSans-Bold.ttf");
    let font_data = std::fs::read(&font_path)
        .map_err(|e| format!("Failed to load font {:?}: {}", font_path, e))?;
    tracing::info!("Loaded font: {:?} ({} bytes)", font_path, font_data.len());

    let font = load_font(&font_data).map_err(|e| format!("Failed to parse font: {}", e))?;

    // Load map image at startup (only "map wor rhun" is supported)
    let map_image = load_map_image("map wor rhun", &assets_path)
        .map_err(|e| format!("Failed to load map image: {}", e))?;
    tracing::info!(
        "Loaded map image: {}x{}",
        map_image.width(),
        map_image.height()
    );

    let intents = serenity::GatewayIntents::GUILD_MESSAGES
        | serenity::GatewayIntents::MESSAGE_CONTENT
        | serenity::GatewayIntents::DIRECT_MESSAGES;

    let framework = poise::Framework::builder()
        .options(poise::FrameworkOptions {
            event_handler: |ctx, event, framework, data| {
                Box::pin(event_handler(ctx, event, framework, data))
            },
            prefix_options: poise::PrefixFrameworkOptions {
                mention_as_prefix: false,
                ..Default::default()
            },
            ..Default::default()
        })
        .setup(move |_ctx, ready, _framework| {
            Box::pin(async move {
                let bot_id = ready.user.id;
                tracing::info!("Bot is ready! Bot ID: {}", bot_id);
                Ok(Data {
                    font: Arc::new(font),
                    map_image: Arc::new(map_image),
                    bot_id,
                    pending_replays: Mutex::new(HashMap::new()),
                    cooldowns: Mutex::new(HashMap::new()),
                })
            })
        })
        .build();

    // Disable all caching — this bot never reads from the cache
    let mut cache_settings = serenity::cache::Settings::default();
    cache_settings.cache_guilds = false;
    cache_settings.cache_channels = false;
    cache_settings.cache_users = false;

    let mut client = serenity::ClientBuilder::new(token, intents)
        .cache_settings(cache_settings)
        .framework(framework)
        .await?;

    client.start().await?;

    Ok(())
}

/// Handle Discord events
async fn event_handler(
    ctx: &serenity::Context,
    event: &serenity::FullEvent,
    _framework: poise::FrameworkContext<'_, Data, Error>,
    data: &Data,
) -> Result<(), Error> {
    match event {
        serenity::FullEvent::Message { new_message } => {
            handle_message(ctx, new_message, data).await?;
        }
        serenity::FullEvent::InteractionCreate {
            interaction: serenity::Interaction::Component(component),
        } => {
            handle_component_interaction(ctx, component, data).await;
        }
        _ => {}
    }
    Ok(())
}
