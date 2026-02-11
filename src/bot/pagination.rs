use poise::serenity_prelude as serenity;
use serenity::model::application::ButtonStyle;
use serenity::{
    CreateActionRow, CreateButton, CreateInteractionResponse, CreateInteractionResponseFollowup,
    CreateInteractionResponseMessage, EditInteractionResponse,
};
use std::time::Instant;

use super::constants::{BATCH_SIZE, build_safe_content};
use super::setup::{Data, PendingReplays, cleanup_expired_pending_inner};

/// Handle a "Show more" button click.
pub async fn handle_component_interaction(
    ctx: &serenity::Context,
    component: &serenity::ComponentInteraction,
    data: &Data,
) {
    let custom_id = &component.data.custom_id;
    let Some(key) = custom_id.strip_prefix("show_more:") else {
        return;
    };

    // Channel validation + remove pending data under one lock.
    // Short-circuits BEFORE acknowledge/disable-button flow on mismatch.
    enum LookupResult {
        ChannelMismatch,
        Found(PendingReplays),
        NotFound,
    }

    let lookup = {
        let mut map = data.lock_pending_replays();
        cleanup_expired_pending_inner(&mut map);

        // Validate channel BEFORE removing
        match map.get(key) {
            Some(entry) if entry.channel_id != component.channel_id => {
                // Don't consume the entry -- let the rightful channel use it
                LookupResult::ChannelMismatch
            }
            Some(_) => LookupResult::Found(map.remove(key).unwrap()),
            None => LookupResult::NotFound,
        }
        // guard drops here
    };

    if matches!(lookup, LookupResult::ChannelMismatch) {
        let response = CreateInteractionResponse::Message(
            CreateInteractionResponseMessage::new()
                .content("This button is only valid in the original channel.")
                .ephemeral(true),
        );
        let _ = component.create_response(ctx, response).await;
        return;
    }

    let pending = match lookup {
        LookupResult::Found(p) => Some(p),
        _ => None,
    };

    // Acknowledge the interaction without modifying the message (preserves attachments).
    match component
        .create_response(ctx, CreateInteractionResponse::Acknowledge)
        .await
    {
        Ok(()) => tracing::info!("Acknowledged interaction {}", component.id),
        Err(e) => {
            tracing::error!("Failed to acknowledge interaction: {}", e);
            return;
        }
    }

    // Disable the button via edit_response.
    let disabled_button = CreateButton::new("show_more_disabled")
        .label("Processing...")
        .style(ButtonStyle::Secondary)
        .disabled(true);
    match component
        .edit_response(
            ctx,
            EditInteractionResponse::new()
                .components(vec![CreateActionRow::Buttons(vec![disabled_button])]),
        )
        .await
    {
        Ok(msg) => tracing::info!("Disabled button on message {}", msg.id),
        Err(e) => tracing::error!("Failed to disable button: {}", e),
    }

    let Some(pending) = pending else {
        let followup = CreateInteractionResponseFollowup::new()
            .content("This button has expired. Please re-upload the archive.");
        match component.create_followup(ctx, followup).await {
            Ok(msg) => tracing::info!("Sent expiry notice {}", msg.id),
            Err(e) => tracing::error!("Failed to send expiry notice: {}", e),
        }
        return;
    };

    // Process the next batch
    let (attachments, errors) = super::handler::process_replay_batch(data, &pending.replays).await;
    let batch_count = pending.replays.len().min(BATCH_SIZE);
    let new_shown = pending.shown + batch_count;
    let remaining: Vec<(String, Vec<u8>)> = pending.replays.into_iter().skip(batch_count).collect();

    // TOCTOU-safe reinsert: lock -> cleanup -> capacity check -> insert
    // Stable key: reuse the same key (no suffix growth)
    let pending_key = if !remaining.is_empty() {
        let mut map = data.lock_pending_replays();
        cleanup_expired_pending_inner(&mut map);
        if map.len() >= super::constants::MAX_PENDING_ENTRIES {
            None
        } else {
            let new_pending = PendingReplays {
                replays: remaining,
                total: pending.total,
                shown: new_shown,
                created_at: Instant::now(),
                channel_id: pending.channel_id,
            };
            map.insert(key.to_string(), new_pending);
            Some(key.to_string())
        }
        // guard drops here, before any .await
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

    let content = build_safe_content(&parts);
    let mut followup = CreateInteractionResponseFollowup::new().content(content);
    for att in attachments {
        followup = followup.add_file(att);
    }
    if let Some(ref pk) = pending_key {
        let button = CreateButton::new(format!("show_more:{}", pk))
            .label("Show more")
            .style(ButtonStyle::Primary);
        followup = followup.components(vec![CreateActionRow::Buttons(vec![button])]);
    }

    match component.create_followup(ctx, followup).await {
        Ok(msg) => tracing::info!("Sent followup batch {}", msg.id),
        Err(e) => tracing::error!("Failed to send followup: {}", e),
    }

    super::handler::trim_memory();
}
