use crate::models::ReplayError;
use crate::parser::parse_replay;
use crate::renderer::render_map;
use poise::serenity_prelude as serenity;
use serenity::CreateAttachment;
use std::time::Instant;

use super::archive::{extract_replays_from_rar, extract_replays_from_zip};
use super::constants::BATCH_SIZE;
use super::messages::{
    BatchMessageArgs, send_batch_message, send_replay_image, send_simple_message,
};
use super::setup::{Data, PendingReplays, cleanup_expired_pending_inner};

const MAX_SINGLE_REPLAY_BYTES: u64 = 5 * 1024 * 1024; // 5MB
const MAX_ARCHIVE_BYTES: u64 = 25 * 1024 * 1024; // 25MB

type Error = Box<dyn std::error::Error + Send + Sync>;

/// Handle incoming messages with replay attachments
pub async fn handle_message(
    ctx: &serenity::Context,
    new_message: &serenity::Message,
    data: &Data,
) -> Result<(), Error> {
    // Ignore bot messages
    if new_message.author.bot {
        return Ok(());
    }

    // Collect attachments: from this message, replied-to message, or forwarded message.
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

    // Forwarded messages can't contain @mentions, so auto-process them.
    // All other messages require the bot to be @mentioned.
    if !is_forwarded && !is_bot_mentioned(ctx, new_message, data.bot_id).await {
        return Ok(());
    }

    // Per-channel cooldown
    if data.check_cooldown(new_message.channel_id) {
        return Ok(());
    }
    data.set_cooldown(new_message.channel_id);

    for (att_idx, attachment) in attachments.iter().enumerate() {
        let filename_lower = attachment.filename.to_lowercase();

        if filename_lower.ends_with(".bfme2replay") {
            process_single_attachment(ctx, new_message, data, attachment).await;
        } else if filename_lower.ends_with(".zip") || filename_lower.ends_with(".rar") {
            process_archive_attachment(ctx, new_message, data, attachment, att_idx).await;
        }
    }

    Ok(())
}

/// Process a single replay file attachment
async fn process_single_attachment(
    ctx: &serenity::Context,
    msg: &serenity::Message,
    data: &Data,
    attachment: &serenity::Attachment,
) {
    if u64::from(attachment.size) > MAX_SINGLE_REPLAY_BYTES {
        tracing::warn!("Replay file too large: {} bytes", attachment.size);
        send_simple_message(ctx, msg, "Replay file too large (max 5MB)").await;
        return;
    }

    tracing::info!("Processing replay file: {}", attachment.filename);

    let data_bytes = match attachment.download().await {
        Ok(bytes) => bytes,
        Err(e) => {
            tracing::error!("Failed to download attachment: {}", e);
            send_simple_message(ctx, msg, "Failed to download replay file").await;
            return;
        }
    };

    process_single_replay(ctx, msg, data, &data_bytes, &attachment.filename).await;
}

/// Process an archive attachment (ZIP or RAR)
async fn process_archive_attachment(
    ctx: &serenity::Context,
    msg: &serenity::Message,
    data: &Data,
    attachment: &serenity::Attachment,
    att_idx: usize,
) {
    if u64::from(attachment.size) > MAX_ARCHIVE_BYTES {
        tracing::warn!("Archive too large: {} bytes", attachment.size);
        send_simple_message(ctx, msg, "Archive too large (max 25MB)").await;
        return;
    }

    let is_rar = attachment.filename.to_lowercase().ends_with(".rar");
    let label = if is_rar { "RAR" } else { "ZIP" };
    tracing::info!("Processing {} archive: {}", label, attachment.filename);

    let archive_bytes = match attachment.download().await {
        Ok(bytes) => bytes,
        Err(e) => {
            tracing::error!("Failed to download {}: {}", label, e);
            send_simple_message(ctx, msg, "Failed to download archive").await;
            return;
        }
    };

    let (replays, total) = if is_rar {
        match tokio::task::spawn_blocking(move || extract_replays_from_rar(&archive_bytes)).await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("{} extraction task failed: {}", label, e);
                send_simple_message(ctx, msg, "Failed to extract archive").await;
                return;
            }
        }
    } else {
        match tokio::task::spawn_blocking(move || extract_replays_from_zip(&archive_bytes)).await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("{} extraction task failed: {}", label, e);
                send_simple_message(ctx, msg, "Failed to extract archive").await;
                return;
            }
        }
    };

    if replays.is_empty() {
        send_simple_message(ctx, msg, "No .BfME2Replay files found in archive").await;
        return;
    }

    let key = format!("{}_{}_{}", msg.channel_id, msg.id, att_idx);
    process_archive_replays(ctx, msg, data, replays, total, &key).await;
}

/// Process a single replay file: parse, render, and send the image
async fn process_single_replay(
    ctx: &serenity::Context,
    msg: &serenity::Message,
    data: &Data,
    replay_bytes: &[u8],
    filename: &str,
) {
    let bytes_owned = replay_bytes.to_vec();
    let font = data.font.clone();
    let map_image = data.map_image.clone();
    let filename_owned = filename.to_string();

    let result = tokio::task::spawn_blocking(move || {
        let replay = parse_replay(&bytes_owned)?;
        let image_bytes = render_map(&replay, &font, &map_image, &filename_owned)
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

/// Process up to BATCH_SIZE replays and return image attachments + error messages.
/// Uses JoinSet for parallel rendering.
pub async fn process_replay_batch(
    data: &Data,
    replays: &[(String, Vec<u8>)],
) -> (Vec<CreateAttachment>, Vec<String>) {
    let batch = &replays[..replays.len().min(BATCH_SIZE)];
    let mut set = tokio::task::JoinSet::new();

    for (idx, (name, bytes)) in batch.iter().enumerate() {
        let font = data.font.clone();
        let map_image = data.map_image.clone();
        let name_owned = name.clone();
        let name_for_render = name.clone();
        let bytes_owned = bytes.clone();

        set.spawn_blocking(move || {
            let replay = parse_replay(&bytes_owned);
            (
                idx,
                name_owned,
                replay.and_then(|r| {
                    render_map(&r, &font, &map_image, &name_for_render)
                        .map_err(ReplayError::RenderError)
                }),
            )
        });
    }

    // Collect results in order
    let mut results: Vec<(usize, String, Result<Vec<u8>, ReplayError>)> = Vec::new();
    while let Some(join_result) = set.join_next().await {
        match join_result {
            Ok(tuple) => results.push(tuple),
            Err(e) => tracing::error!("Batch render task panicked: {}", e),
        }
    }
    results.sort_by_key(|(idx, _, _)| *idx);

    let mut attachments = Vec::new();
    let mut errors = Vec::new();

    for (idx, name, result) in results {
        match result {
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

/// Process an archive's replays: send first batch, store remaining for pagination.
async fn process_archive_replays(
    ctx: &serenity::Context,
    msg: &serenity::Message,
    data: &Data,
    replays: Vec<(String, Vec<u8>)>,
    total: usize,
    key: &str,
) {
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
    let remaining: Vec<(String, Vec<u8>)> = if replays.len() > batch_count {
        replays.into_iter().skip(batch_count).collect()
    } else {
        Vec::new()
    };

    // TOCTOU-safe: lock -> cleanup -> capacity check -> insert, all under one guard
    let pending_key = if !remaining.is_empty() {
        let mut map = data.lock_pending_replays();
        cleanup_expired_pending_inner(&mut map);
        if map.len() >= super::constants::MAX_PENDING_ENTRIES {
            tracing::warn!("Pending replays map is full, discarding remaining replays");
            None
        } else {
            let pending = PendingReplays {
                replays: remaining,
                total: effective_total,
                shown: batch_count,
                created_at: Instant::now(),
                channel_id: msg.channel_id,
            };
            map.insert(key.to_string(), pending);
            Some(key.to_string())
        }
        // guard drops here, before any .await
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
        BatchMessageArgs {
            channel_id: msg.channel_id,
            attachments,
            errors: &errors,
            shown,
            total: effective_total,
            pending_key: pending_key.as_deref(),
            cap_note: cap_note.as_deref(),
        },
    )
    .await;
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
