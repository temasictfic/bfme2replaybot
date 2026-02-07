use crate::models::ReplayError;
use crate::parser::parse_replay;
use crate::renderer::{load_map_image, render_map};
use image::RgbaImage;
use poise::serenity_prelude::{self as serenity, CreateAttachment, CreateMessage};
use std::io::Read;
use std::path::PathBuf;

const MAX_REPLAYS_PER_ARCHIVE: usize = 10;

pub struct Data {
    pub font_data: Vec<u8>,
    pub map_image: RgbaImage,
    pub bot_id: serenity::UserId,
}

type Error = Box<dyn std::error::Error + Send + Sync>;
#[allow(dead_code)]
type Context<'a> = poise::Context<'a, Data, Error>;

/// Set up and run the Discord bot
pub async fn setup_bot(token: String, assets_path: PathBuf) -> Result<(), Error> {
    // Load font at startup
    let font_path = assets_path.join("fonts").join("NotoSans-Bold.ttf");
    let font_data = std::fs::read(&font_path)
        .map_err(|e| format!("Failed to load font {:?}: {}", font_path, e))?;
    tracing::info!("Loaded font: {:?} ({} bytes)", font_path, font_data.len());

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
            ..Default::default()
        })
        .setup(move |_ctx, ready, _framework| {
            Box::pin(async move {
                let bot_id = ready.user.id;
                tracing::info!("Bot is ready! Bot ID: {}", bot_id);
                Ok(Data {
                    font_data,
                    map_image,
                    bot_id,
                })
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

        // Collect attachments: from this message, replied-to message, or forwarded message.
        // For replied-to forwarded messages, attachments are in the snapshot, not directly.
        let attachments = if !new_message.attachments.is_empty() {
            new_message.attachments.clone()
        } else if let Some(ref replied) = new_message.referenced_message {
            if !replied.attachments.is_empty() {
                replied.attachments.clone()
            } else if let Some(snapshot) = replied.message_snapshots.first() {
                snapshot.attachments.clone()
            } else {
                return Ok(());
            }
        } else if let Some(snapshot) = new_message.message_snapshots.first() {
            snapshot.attachments.clone()
        } else {
            return Ok(());
        };

        // Check if any attachment is relevant before doing mention check
        let has_relevant = attachments.iter().any(|a| {
            let f = a.filename.to_lowercase();
            f.ends_with(".bfme2replay") || f.ends_with(".zip") || f.ends_with(".rar")
        });
        if !has_relevant {
            return Ok(());
        }

        // Only respond when the bot is @mentioned (user mention or role mention)
        if !is_bot_mentioned(ctx, new_message, data.bot_id).await {
            return Ok(());
        }

        for attachment in &attachments {
            let filename_lower = attachment.filename.to_lowercase();

            if filename_lower.ends_with(".bfme2replay") {
                // Direct replay file
                if attachment.size > 5 * 1024 * 1024 {
                    tracing::warn!("Replay file too large: {} bytes", attachment.size);
                    send_simple_message(ctx, new_message, "Replay file too large (max 5MB)").await;
                    continue;
                }

                tracing::info!("Processing replay file: {}", attachment.filename);

                let data_bytes = match attachment.download().await {
                    Ok(bytes) => bytes,
                    Err(e) => {
                        tracing::error!("Failed to download attachment: {}", e);
                        send_simple_message(ctx, new_message, "Failed to download replay file")
                            .await;
                        continue;
                    }
                };

                process_single_replay(ctx, new_message, data, &data_bytes, &attachment.filename)
                    .await;
            } else if filename_lower.ends_with(".zip") {
                // ZIP archive
                if attachment.size > 25 * 1024 * 1024 {
                    tracing::warn!("ZIP file too large: {} bytes", attachment.size);
                    send_simple_message(ctx, new_message, "Archive too large (max 25MB)").await;
                    continue;
                }

                tracing::info!("Processing ZIP archive: {}", attachment.filename);

                let archive_bytes = match attachment.download().await {
                    Ok(bytes) => bytes,
                    Err(e) => {
                        tracing::error!("Failed to download ZIP: {}", e);
                        send_simple_message(ctx, new_message, "Failed to download archive").await;
                        continue;
                    }
                };

                let (replays, total) = match tokio::task::spawn_blocking(move || {
                    extract_replays_from_zip(&archive_bytes)
                })
                .await
                {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::error!("ZIP extraction task failed: {}", e);
                        send_simple_message(ctx, new_message, "Failed to extract archive").await;
                        continue;
                    }
                };
                if replays.is_empty() {
                    send_simple_message(ctx, new_message, "No .BfME2Replay files found in archive")
                        .await;
                    continue;
                }

                for (name, bytes) in &replays {
                    process_single_replay(ctx, new_message, data, bytes, name).await;
                }
                if total > MAX_REPLAYS_PER_ARCHIVE {
                    send_simple_message(
                        ctx,
                        new_message,
                        &format!("Showing {} of {} replays", MAX_REPLAYS_PER_ARCHIVE, total),
                    )
                    .await;
                }
            } else if filename_lower.ends_with(".rar") {
                // RAR archive
                if attachment.size > 25 * 1024 * 1024 {
                    tracing::warn!("RAR file too large: {} bytes", attachment.size);
                    send_simple_message(ctx, new_message, "Archive too large (max 25MB)").await;
                    continue;
                }

                tracing::info!("Processing RAR archive: {}", attachment.filename);

                let archive_bytes = match attachment.download().await {
                    Ok(bytes) => bytes,
                    Err(e) => {
                        tracing::error!("Failed to download RAR: {}", e);
                        send_simple_message(ctx, new_message, "Failed to download archive").await;
                        continue;
                    }
                };

                let (replays, total) = match tokio::task::spawn_blocking(move || {
                    extract_replays_from_rar(&archive_bytes)
                })
                .await
                {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::error!("RAR extraction task failed: {}", e);
                        send_simple_message(ctx, new_message, "Failed to extract archive").await;
                        continue;
                    }
                };
                if replays.is_empty() {
                    send_simple_message(ctx, new_message, "No .BfME2Replay files found in archive")
                        .await;
                    continue;
                }

                for (name, bytes) in &replays {
                    process_single_replay(ctx, new_message, data, bytes, name).await;
                }
                if total > MAX_REPLAYS_PER_ARCHIVE {
                    send_simple_message(
                        ctx,
                        new_message,
                        &format!("Showing {} of {} replays", MAX_REPLAYS_PER_ARCHIVE, total),
                    )
                    .await;
                }
            }
        }
    }

    Ok(())
}

/// Check if the bot was mentioned (direct user mention or bot's managed role mention)
async fn is_bot_mentioned(
    ctx: &serenity::Context,
    msg: &serenity::Message,
    bot_id: serenity::UserId,
) -> bool {
    // Check direct user mention in content: <@BOT_ID>
    let bot_mention = format!("<@{}>", bot_id);
    if msg.content.contains(&bot_mention) {
        return true;
    }

    // Check mentions vec populated by Discord
    if msg.mentions.iter().any(|u| u.id == bot_id) {
        return true;
    }

    // Check role mentions: look up guild roles to find the bot's managed role
    if !msg.mention_roles.is_empty() {
        if let Some(guild_id) = msg.guild_id {
            if let Ok(roles) = ctx.http.get_guild_roles(guild_id).await {
                let bot_role_mentioned = roles.iter().any(|role| {
                    role.tags.bot_id == Some(bot_id) && msg.mention_roles.contains(&role.id)
                });
                if bot_role_mentioned {
                    return true;
                }
            }
        }
    }

    false
}

/// Process a single replay file: parse, render, and send the image
async fn process_single_replay(
    ctx: &serenity::Context,
    msg: &serenity::Message,
    data: &Data,
    replay_bytes: &[u8],
    filename: &str,
) {
    // Clone data into owned values for the blocking task
    let bytes_owned = replay_bytes.to_vec();
    let font_data = data.font_data.clone();
    let map_image = data.map_image.clone();
    let filename_owned = filename.to_string();

    let result = tokio::task::spawn_blocking(move || {
        let replay = parse_replay(&bytes_owned)?;
        let image_bytes = render_map(&replay, &font_data, &map_image, &filename_owned)
            .map_err(ReplayError::RenderError)?;
        Ok::<Vec<u8>, ReplayError>(image_bytes)
    })
    .await;

    match result {
        Ok(Ok(image_bytes)) => {
            send_replay_image(ctx, msg, image_bytes).await;
        }
        Ok(Err(ReplayError::UnsupportedMap(map_name))) => {
            tracing::info!("Skipping unsupported map: {}", map_name);
            send_simple_message(ctx, msg, &format!("Not a Rhun game (map: {})", map_name)).await;
        }
        Ok(Err(ReplayError::InvalidHeader)) => {
            tracing::error!("Invalid replay header");
            send_simple_message(ctx, msg, "Invalid replay file").await;
        }
        Ok(Err(ReplayError::NoPlayers)) => {
            tracing::error!("No players found in replay");
            send_simple_message(ctx, msg, "No players found in replay").await;
        }
        Ok(Err(e)) => {
            tracing::error!("Failed to process replay: {}", e);
            send_simple_message(ctx, msg, &format!("Error: {}", e)).await;
        }
        Err(e) => {
            tracing::error!("Replay processing task failed: {}", e);
            send_simple_message(ctx, msg, "Internal error processing replay").await;
        }
    }
}

/// Extract .BfME2Replay files from a ZIP archive (in-memory).
/// Returns (replays, total_count) — only up to MAX_REPLAYS_PER_ARCHIVE are extracted,
/// but total_count reflects how many were found.
fn extract_replays_from_zip(data: &[u8]) -> (Vec<(String, Vec<u8>)>, usize) {
    let cursor = std::io::Cursor::new(data);
    let mut archive = match zip::ZipArchive::new(cursor) {
        Ok(a) => a,
        Err(e) => {
            tracing::error!("Failed to open ZIP archive: {}", e);
            return (Vec::new(), 0);
        }
    };

    let mut replays = Vec::new();
    let mut total = 0usize;

    for i in 0..archive.len() {
        let mut file = match archive.by_index(i) {
            Ok(f) => f,
            Err(e) => {
                tracing::warn!("Failed to read ZIP entry {}: {}", i, e);
                continue;
            }
        };

        let name = file.name().to_string();
        if !name.to_lowercase().ends_with(".bfme2replay") || file.is_dir() {
            continue;
        }

        total += 1;

        // Count but don't extract beyond the cap
        if replays.len() >= MAX_REPLAYS_PER_ARCHIVE {
            continue;
        }

        // Skip files larger than 5MB
        if file.size() > 5 * 1024 * 1024 {
            tracing::warn!(
                "Skipping oversized replay in ZIP: {} ({} bytes)",
                name,
                file.size()
            );
            continue;
        }

        let mut buf = Vec::with_capacity(file.size() as usize);
        if let Err(e) = file.read_to_end(&mut buf) {
            tracing::warn!("Failed to extract {}: {}", name, e);
            continue;
        }

        // Use just the filename, not the full path inside the archive
        let short_name = name.rsplit(['/', '\\']).next().unwrap_or(&name).to_string();

        replays.push((short_name, buf));
    }

    (replays, total)
}

/// Extract .BfME2Replay files from a RAR archive (via temp directory).
/// Returns (replays, total_count) — only up to MAX_REPLAYS_PER_ARCHIVE bytes are read,
/// but total_count reflects how many replay files were found on disk.
fn extract_replays_from_rar(data: &[u8]) -> (Vec<(String, Vec<u8>)>, usize) {
    let tmp_dir = match tempfile::tempdir() {
        Ok(d) => d,
        Err(e) => {
            tracing::error!("Failed to create temp dir: {}", e);
            return (Vec::new(), 0);
        }
    };

    // Write RAR data to a temp file (unrar needs a filesystem path)
    let rar_path = tmp_dir.path().join("archive.rar");
    if let Err(e) = std::fs::write(&rar_path, data) {
        tracing::error!("Failed to write temp RAR file: {}", e);
        return (Vec::new(), 0);
    }

    let extract_dir = tmp_dir.path().join("extracted");
    if let Err(e) = std::fs::create_dir_all(&extract_dir) {
        tracing::error!("Failed to create extract dir: {}", e);
        return (Vec::new(), 0);
    }

    // Extract using unrar
    let mut archive =
        match unrar::Archive::new::<str>(&rar_path.to_string_lossy()).open_for_processing() {
            Ok(a) => a,
            Err(e) => {
                tracing::error!("Failed to open RAR archive: {}", e);
                return (Vec::new(), 0);
            }
        };

    // Extract all files (unrar API requires sequential processing)
    loop {
        let header = match archive.read_header() {
            Ok(Some(header)) => header,
            Ok(None) => break,
            Err(e) => {
                tracing::error!("Failed to read RAR header: {}", e);
                break;
            }
        };

        archive = if header.entry().is_file() {
            match header.extract_with_base(&extract_dir) {
                Ok(a) => a,
                Err(e) => {
                    tracing::warn!("Failed to extract RAR entry: {}", e);
                    break;
                }
            }
        } else {
            match header.skip() {
                Ok(a) => a,
                Err(e) => {
                    tracing::warn!("Failed to skip RAR entry: {}", e);
                    break;
                }
            }
        };
    }

    // Collect extracted .BfME2Replay files (reads bytes only up to cap)
    let mut replays = Vec::new();
    let mut total = 0usize;
    collect_replay_files(&extract_dir, &mut replays, &mut total);

    (replays, total)
    // tmp_dir is dropped here, cleaning up all temp files
}

/// Recursively collect .BfME2Replay files from a directory.
/// Only reads file bytes for the first MAX_REPLAYS_PER_ARCHIVE files; counts the rest.
fn collect_replay_files(
    dir: &std::path::Path,
    replays: &mut Vec<(String, Vec<u8>)>,
    total: &mut usize,
) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_replay_files(&path, replays, total);
        } else if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name.to_lowercase().ends_with(".bfme2replay") {
                *total += 1;

                // Count but don't read bytes beyond the cap
                if replays.len() >= MAX_REPLAYS_PER_ARCHIVE {
                    continue;
                }

                // Skip files larger than 5MB
                if let Ok(meta) = path.metadata() {
                    if meta.len() > 5 * 1024 * 1024 {
                        tracing::warn!(
                            "Skipping oversized replay: {} ({} bytes)",
                            name,
                            meta.len()
                        );
                        continue;
                    }
                }

                match std::fs::read(&path) {
                    Ok(bytes) => replays.push((name.to_string(), bytes)),
                    Err(e) => tracing::warn!("Failed to read {}: {}", name, e),
                }
            }
        }
    }
}

/// Send replay image as the only response (no embed)
async fn send_replay_image(ctx: &serenity::Context, msg: &serenity::Message, image_bytes: Vec<u8>) {
    let attachment = CreateAttachment::bytes(image_bytes, "replay.jpg");
    let message = CreateMessage::new().add_file(attachment);

    if let Err(e) = msg.channel_id.send_message(ctx, message).await {
        tracing::error!("Failed to send image: {}", e);
    }
}

/// Send a simple text message (no embed)
async fn send_simple_message(ctx: &serenity::Context, msg: &serenity::Message, text: &str) {
    let message = CreateMessage::new().content(text);

    if let Err(e) = msg.channel_id.send_message(ctx, message).await {
        tracing::error!("Failed to send message: {}", e);
    }
}
