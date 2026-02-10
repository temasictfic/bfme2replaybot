use poise::serenity_prelude as serenity;
use serenity::model::application::ButtonStyle;
use serenity::{CreateActionRow, CreateAttachment, CreateButton, CreateMessage};

use super::constants::{BATCH_SIZE, build_safe_content};

/// Arguments for sending a batch message
pub struct BatchMessageArgs<'a> {
    pub channel_id: serenity::ChannelId,
    pub attachments: Vec<CreateAttachment>,
    pub errors: &'a [String],
    pub shown: usize,
    pub total: usize,
    pub pending_key: Option<&'a str>,
    pub cap_note: Option<&'a str>,
}

/// Send a batch of replay images as a single message, with an optional "Show more" button.
pub async fn send_batch_message(ctx: &serenity::Context, args: BatchMessageArgs<'_>) {
    let mut parts = Vec::new();
    if let Some(note) = args.cap_note {
        parts.push(note.to_string());
    }
    if args.total > BATCH_SIZE {
        parts.push(format!("Showing {} of {} replays", args.shown, args.total));
    }
    for err in args.errors {
        parts.push(err.clone());
    }

    let mut message = CreateMessage::new();
    if !parts.is_empty() {
        message = message.content(build_safe_content(&parts));
    }
    for att in args.attachments {
        message = message.add_file(att);
    }

    if let Some(key) = args.pending_key {
        let button = CreateButton::new(format!("show_more:{}", key))
            .label("Show more")
            .style(ButtonStyle::Primary);
        message = message.components(vec![CreateActionRow::Buttons(vec![button])]);
    }

    match args.channel_id.send_message(ctx, message).await {
        Ok(msg) => tracing::info!("Sent batch message {}", msg.id),
        Err(e) => tracing::error!("Failed to send batch message: {}", e),
    }
}

/// Send replay image as the only response (no embed)
pub async fn send_replay_image(
    ctx: &serenity::Context,
    msg: &serenity::Message,
    image_bytes: Vec<u8>,
) {
    let attachment = CreateAttachment::bytes(image_bytes, "replay.jpg");
    let message = CreateMessage::new().add_file(attachment);

    match msg.channel_id.send_message(ctx, message).await {
        Ok(sent) => tracing::info!("Sent replay image {}", sent.id),
        Err(e) => tracing::error!("Failed to send image: {}", e),
    }
}

/// Send a simple text message (no embed)
pub async fn send_simple_message(ctx: &serenity::Context, msg: &serenity::Message, text: &str) {
    let message = CreateMessage::new().content(text);

    match msg.channel_id.send_message(ctx, message).await {
        Ok(sent) => tracing::info!("Sent message {}", sent.id),
        Err(e) => tracing::error!("Failed to send message: {}", e),
    }
}
