use crate::models::ReplayInfo;
use crate::parser::parse_replay;
use crate::renderer::render_map;
use poise::serenity_prelude::{self as serenity, CreateAttachment, CreateEmbed, CreateMessage};
use std::path::PathBuf;

pub struct Data {
    pub assets_path: PathBuf,
}

type Error = Box<dyn std::error::Error + Send + Sync>;
#[allow(dead_code)]
type Context<'a> = poise::Context<'a, Data, Error>;

/// Set up and run the Discord bot
pub async fn setup_bot(token: String, assets_path: PathBuf) -> Result<(), Error> {
    let intents = serenity::GatewayIntents::GUILD_MESSAGES
        | serenity::GatewayIntents::MESSAGE_CONTENT
        | serenity::GatewayIntents::DIRECT_MESSAGES;

    let framework = poise::Framework::builder()
        .options(poise::FrameworkOptions {
            event_handler: |ctx, event, framework, data| {
                Box::pin(event_handler(ctx, event, framework, data))
            },
            ..Default::default()
        })
        .setup(move |_ctx, _ready, _framework| {
            Box::pin(async move {
                tracing::info!("Bot is ready!");
                Ok(Data { assets_path })
            })
        })
        .build();

    let mut client = serenity::ClientBuilder::new(token, intents)
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
    if let serenity::FullEvent::Message { new_message } = event {
        // Ignore bot messages
        if new_message.author.bot {
            return Ok(());
        }

        // Check for replay file attachments
        for attachment in &new_message.attachments {
            if attachment.filename.ends_with(".BfME2Replay") {
                tracing::info!("Processing replay file: {}", attachment.filename);

                // Download the attachment
                let data_bytes = match attachment.download().await {
                    Ok(bytes) => bytes,
                    Err(e) => {
                        tracing::error!("Failed to download attachment: {}", e);
                        send_error(ctx, new_message, "Failed to download replay file").await;
                        continue;
                    }
                };

                // Parse the replay
                let replay = match parse_replay(&data_bytes) {
                    Ok(replay) => replay,
                    Err(e) => {
                        tracing::error!("Failed to parse replay: {}", e);
                        send_error(ctx, new_message, &format!("Failed to parse replay: {}", e))
                            .await;
                        continue;
                    }
                };

                // Generate map image
                let image_result = render_map(&replay, &data.assets_path);

                // Send response
                send_replay_info(ctx, new_message, &replay, image_result).await;
            }
        }
    }

    Ok(())
}

/// Send replay information as an embed with the map image
async fn send_replay_info(
    ctx: &serenity::Context,
    msg: &serenity::Message,
    replay: &ReplayInfo,
    image_result: Result<Vec<u8>, String>,
) {
    let mut embed = CreateEmbed::new()
        .title(format!("🗺️ {}", replay.map_name))
        .color(0x8B4513);

    // Build player list
    let mut player_text = String::new();
    for player in &replay.players {
        player_text.push_str(&format!(
            "**{}** - {} (Team {})\n",
            player.name, player.faction, player.team
        ));
    }

    if !player_text.is_empty() {
        embed = embed.field("Players", &player_text, false);
    }

    let mut message = CreateMessage::new().embed(embed);

    // Attach map image if available
    if let Ok(image_bytes) = image_result {
        let attachment = CreateAttachment::bytes(image_bytes, "map.png");
        message = message.add_file(attachment);
    }

    if let Err(e) = msg.channel_id.send_message(ctx, message).await {
        tracing::error!("Failed to send message: {}", e);
    }
}

/// Send an error message
async fn send_error(ctx: &serenity::Context, msg: &serenity::Message, error: &str) {
    let embed = CreateEmbed::new()
        .title("❌ Error")
        .description(error)
        .color(0xFF0000);

    if let Err(e) = msg
        .channel_id
        .send_message(ctx, CreateMessage::new().embed(embed))
        .await
    {
        tracing::error!("Failed to send error message: {}", e);
    }
}
