use crate::models::ReplayError;
use crate::parser::parse_replay;
use crate::renderer::{load_map_image, render_map};
use image::RgbaImage;
use poise::serenity_prelude::{
    self as serenity, CreateActionRow, CreateAttachment, CreateButton, CreateInteractionResponse,
    CreateInteractionResponseFollowup, CreateMessage, EditInteractionResponse,
};
use serenity::model::application::ButtonStyle;
use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

const BATCH_SIZE: usize = 10;
const MAX_REPLAYS_PER_ARCHIVE: usize = 100;
const PENDING_EXPIRY_SECS: u64 = 900;
const MAX_ARCHIVE_UNCOMPRESSED_BYTES: u64 = 500 * 1024 * 1024; // 500MB total
const MAX_ARCHIVE_EXTRACTED_FILES: usize = 200;

struct PendingReplays {
    replays: Vec<(String, Vec<u8>)>,
    total: usize,
    shown: usize,
    created_at: Instant,
    channel_id: serenity::ChannelId,
}

pub struct Data {
    pub font_data: Arc<Vec<u8>>,
    pub map_image: Arc<RgbaImage>,
    pub bot_id: serenity::UserId,
    pending_replays: Mutex<HashMap<String, PendingReplays>>,
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
                    font_data: Arc::new(font_data),
                    map_image: Arc::new(map_image),
                    bot_id,
                    pending_replays: Mutex::new(HashMap::new()),
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

/// Handle incoming messages with replay attachments
async fn handle_message(
    ctx: &serenity::Context,
    new_message: &serenity::Message,
    data: &Data,
) -> Result<(), Error> {
    // Ignore bot messages
    if new_message.author.bot {
        return Ok(());
    }

    // Collect attachments: from this message, replied-to message, or forwarded message.
    // For replied-to forwarded messages, attachments are in the snapshot, not directly.
    let mut is_forwarded = false;
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
        is_forwarded = true;
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

    // Forwarded messages can't contain @mentions (Discord doesn't allow adding text),
    // so auto-process them if they contain relevant replay files.
    // All other messages require the bot to be @mentioned.
    if !is_forwarded && !is_bot_mentioned(ctx, new_message, data.bot_id).await {
        return Ok(());
    }

    for (att_idx, attachment) in attachments.iter().enumerate() {
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
                    send_simple_message(ctx, new_message, "Failed to download replay file").await;
                    continue;
                }
            };

            process_single_replay(ctx, new_message, data, &data_bytes, &attachment.filename).await;
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

            let (replays, total) =
                match tokio::task::spawn_blocking(move || extract_replays_from_zip(&archive_bytes))
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

            let key = format!("{}_{}_{}", new_message.channel_id, new_message.id, att_idx);
            process_archive_replays(ctx, new_message, data, replays, total, &key).await;
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

            let (replays, total) =
                match tokio::task::spawn_blocking(move || extract_replays_from_rar(&archive_bytes))
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

            let key = format!("{}_{}_{}", new_message.channel_id, new_message.id, att_idx);
            process_archive_replays(ctx, new_message, data, replays, total, &key).await;
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
    if !msg.mention_roles.is_empty()
        && let Some(guild_id) = msg.guild_id
        && let Ok(roles) = ctx.http.get_guild_roles(guild_id).await
    {
        let bot_role_mentioned = roles
            .iter()
            .any(|role| role.tags.bot_id == Some(bot_id) && msg.mention_roles.contains(&role.id));
        if bot_role_mentioned {
            return true;
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

/// Render a single replay to image bytes without sending any messages.
async fn render_single_replay(
    data: &Data,
    replay_bytes: &[u8],
    filename: &str,
) -> Result<Vec<u8>, ReplayError> {
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
        Ok(inner) => inner,
        Err(e) => Err(ReplayError::RenderError(format!(
            "Blocking task failed: {}",
            e
        ))),
    }
}

/// Process up to BATCH_SIZE replays and return image attachments + error messages.
async fn process_replay_batch(
    data: &Data,
    replays: &[(String, Vec<u8>)],
) -> (Vec<CreateAttachment>, Vec<String>) {
    let batch = &replays[..replays.len().min(BATCH_SIZE)];
    let mut attachments = Vec::new();
    let mut errors = Vec::new();

    for (idx, (name, bytes)) in batch.iter().enumerate() {
        match render_single_replay(data, bytes, name).await {
            Ok(image_bytes) => {
                let filename = format!("replay_{}.jpg", idx + 1);
                attachments.push(CreateAttachment::bytes(image_bytes, filename));
            }
            Err(ReplayError::UnsupportedMap(map_name)) => {
                tracing::info!("Skipping unsupported map: {}", map_name);
                errors.push(format!("{}: Not a Rhun game (map: {})", name, map_name));
            }
            Err(e) => {
                tracing::error!("Failed to process {}: {}", name, e);
                errors.push(format!("{}: {}", name, e));
            }
        }
    }

    (attachments, errors)
}

/// Send a batch of replay images as a single message, with an optional "Show more" button.
#[allow(clippy::too_many_arguments)]
async fn send_batch_message(
    ctx: &serenity::Context,
    channel_id: serenity::ChannelId,
    attachments: Vec<CreateAttachment>,
    errors: &[String],
    shown: usize,
    total: usize,
    pending_key: Option<&str>,
    cap_note: Option<&str>,
) {
    let mut parts = Vec::new();
    if let Some(note) = cap_note {
        parts.push(note.to_string());
    }
    if total > BATCH_SIZE {
        parts.push(format!("Showing {} of {} replays", shown, total));
    }
    for err in errors {
        parts.push(err.clone());
    }

    let mut message = CreateMessage::new();
    if !parts.is_empty() {
        message = message.content(parts.join("\n"));
    }
    for att in attachments {
        message = message.add_file(att);
    }

    if let Some(key) = pending_key {
        let button = CreateButton::new(format!("show_more:{}", key))
            .label("Show more")
            .style(ButtonStyle::Primary);
        message = message.components(vec![CreateActionRow::Buttons(vec![button])]);
    }

    if let Err(e) = channel_id.send_message(ctx, message).await {
        tracing::error!("Failed to send batch message: {}", e);
    }
}

/// Process an archive's replays: send first batch, store remaining for pagination.
async fn process_archive_replays(
    ctx: &serenity::Context,
    msg: &serenity::Message,
    data: &Data,
    replays: Vec<(String, Vec<u8>)>,
    total: usize,
    key: &str,
) {
    // If total found exceeds what we extracted, clamp and note the cap
    let effective_total = replays.len();
    let cap_note = if total > effective_total {
        Some(format!(
            "Found {} replays, processing first {}",
            total, effective_total
        ))
    } else {
        None
    };

    let (attachments, errors) = process_replay_batch(data, &replays).await;
    let batch_count = replays.len().min(BATCH_SIZE);
    let remaining: Vec<(String, Vec<u8>)> = replays.into_iter().skip(batch_count).collect();

    let pending_key = if !remaining.is_empty() {
        // Clean up expired entries, then store remaining replays
        cleanup_expired_pending(data);
        let pending = PendingReplays {
            replays: remaining,
            total: effective_total,
            shown: batch_count,
            created_at: Instant::now(),
            channel_id: msg.channel_id,
        };
        data.pending_replays
            .lock()
            .unwrap()
            .insert(key.to_string(), pending);
        Some(key.to_string())
    } else {
        None
    };

    let shown = if effective_total > batch_count {
        batch_count
    } else {
        effective_total
    };
    send_batch_message(
        ctx,
        msg.channel_id,
        attachments,
        &errors,
        shown,
        effective_total,
        pending_key.as_deref(),
        cap_note.as_deref(),
    )
    .await;
}

/// Handle a "Show more" button click.
async fn handle_component_interaction(
    ctx: &serenity::Context,
    component: &serenity::ComponentInteraction,
    data: &Data,
) {
    let custom_id = &component.data.custom_id;
    let Some(key) = custom_id.strip_prefix("show_more:") else {
        return;
    };

    // Acknowledge the interaction without modifying the message (preserves attachments).
    // CreateInteractionResponse::UpdateMessage always serializes attachments as [],
    // which Discord interprets as "remove all attachments". Acknowledge sends type 6
    // with null data, leaving the message intact.
    if let Err(e) = component
        .create_response(ctx, CreateInteractionResponse::Acknowledge)
        .await
    {
        tracing::error!("Failed to acknowledge interaction: {}", e);
        return;
    }

    // Disable the button via edit_response. EditInteractionResponse wraps EditWebhookMessage
    // where attachments is Option with skip_serializing_if, so only components is serialized.
    let disabled_button = CreateButton::new("show_more_disabled")
        .label("Processing...")
        .style(ButtonStyle::Secondary)
        .disabled(true);
    let _ = component
        .edit_response(
            ctx,
            EditInteractionResponse::new()
                .components(vec![CreateActionRow::Buttons(vec![disabled_button])]),
        )
        .await;

    // Remove pending data from the map
    let pending = {
        let mut map = data.pending_replays.lock().unwrap();
        cleanup_expired_pending_inner(&mut map);
        map.remove(key)
    };

    let Some(pending) = pending else {
        let followup = CreateInteractionResponseFollowup::new()
            .content("This button has expired. Please re-upload the archive.");
        let _ = component.create_followup(ctx, followup).await;
        return;
    };

    // Process the next batch
    let (attachments, errors) = process_replay_batch(data, &pending.replays).await;
    let batch_count = pending.replays.len().min(BATCH_SIZE);
    let new_shown = pending.shown + batch_count;
    let remaining: Vec<(String, Vec<u8>)> = pending.replays.into_iter().skip(batch_count).collect();

    let pending_key = if !remaining.is_empty() {
        // Generate a new key for the next batch
        let new_key = format!("{}_{}", key, new_shown);
        let new_pending = PendingReplays {
            replays: remaining,
            total: pending.total,
            shown: new_shown,
            created_at: Instant::now(),
            channel_id: pending.channel_id,
        };
        data.pending_replays
            .lock()
            .unwrap()
            .insert(new_key.clone(), new_pending);
        Some(new_key)
    } else {
        None
    };

    // Build followup message with images + optional new button
    let mut parts = Vec::new();
    parts.push(format!(
        "Showing {} of {} replays",
        new_shown, pending.total
    ));
    for err in &errors {
        parts.push(err.clone());
    }

    let mut followup = CreateInteractionResponseFollowup::new().content(parts.join("\n"));
    for att in attachments {
        followup = followup.add_file(att);
    }
    if let Some(ref pk) = pending_key {
        let button = CreateButton::new(format!("show_more:{}", pk))
            .label("Show more")
            .style(ButtonStyle::Primary);
        followup = followup.components(vec![CreateActionRow::Buttons(vec![button])]);
    }

    if let Err(e) = component.create_followup(ctx, followup).await {
        tracing::error!("Failed to send followup: {}", e);
    }
}

/// Remove expired entries from the pending replays map.
fn cleanup_expired_pending(data: &Data) {
    let mut map = data.pending_replays.lock().unwrap();
    cleanup_expired_pending_inner(&mut map);
}

/// Inner cleanup logic (call with lock already held).
fn cleanup_expired_pending_inner(map: &mut HashMap<String, PendingReplays>) {
    let now = Instant::now();
    map.retain(|_, v| now.duration_since(v.created_at).as_secs() < PENDING_EXPIRY_SECS);
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
    let mut extracted_bytes: u64 = 0;

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

        // Check total uncompressed bytes before allocating
        extracted_bytes += file.size();
        if extracted_bytes > MAX_ARCHIVE_UNCOMPRESSED_BYTES {
            tracing::warn!(
                "ZIP extraction byte limit exceeded ({} bytes), stopping",
                extracted_bytes
            );
            break;
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
    let mut extracted_bytes: u64 = 0;
    let mut extracted_files: usize = 0;
    loop {
        let header = match archive.read_header() {
            Ok(Some(header)) => header,
            Ok(None) => break,
            Err(e) => {
                tracing::error!("Failed to read RAR header: {}", e);
                break;
            }
        };

        let is_file = header.entry().is_file();
        let unpacked = header.entry().unpacked_size;

        if is_file {
            extracted_files += 1;
            extracted_bytes += unpacked;
            if extracted_bytes > MAX_ARCHIVE_UNCOMPRESSED_BYTES
                || extracted_files > MAX_ARCHIVE_EXTRACTED_FILES
            {
                tracing::warn!(
                    "RAR extraction limits exceeded ({} bytes, {} files), stopping",
                    extracted_bytes,
                    extracted_files
                );
                let _ = header.skip();
                break;
            }
            archive = match header.extract_with_base(&extract_dir) {
                Ok(a) => a,
                Err(e) => {
                    tracing::warn!("Failed to extract RAR entry: {}", e);
                    break;
                }
            };
        } else {
            archive = match header.skip() {
                Ok(a) => a,
                Err(e) => {
                    tracing::warn!("Failed to skip RAR entry: {}", e);
                    break;
                }
            };
        }
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
        } else if let Some(name) = path.file_name().and_then(|n| n.to_str())
            && name.to_lowercase().ends_with(".bfme2replay")
        {
            *total += 1;

            // Count but don't read bytes beyond the cap
            if replays.len() >= MAX_REPLAYS_PER_ARCHIVE {
                continue;
            }

            // Skip files larger than 5MB
            if let Ok(meta) = path.metadata()
                && meta.len() > 5 * 1024 * 1024
            {
                tracing::warn!("Skipping oversized replay: {} ({} bytes)", name, meta.len());
                continue;
            }

            match std::fs::read(&path) {
                Ok(bytes) => replays.push((name.to_string(), bytes)),
                Err(e) => tracing::warn!("Failed to read {}: {}", name, e),
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
