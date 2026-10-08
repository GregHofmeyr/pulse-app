//! System lines ("sam added riley"): author-less messages with a JSON payload for clients.

use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ChannelId, MessageId};
use pulse_protocol::rest::{Message, MessageKind};

use crate::AppState;
use crate::db::now;
use crate::error::AppResult;

pub async fn post(
    s: &AppState,
    channel: ChannelId,
    content: String,
    payload: serde_json::Value,
) -> AppResult<Message> {
    let msg = Message {
        id: MessageId::new(),
        channel_id: channel,
        author_id: None,
        kind: MessageKind::System,
        content,
        reply_to_id: None,
        created_at: now(),
        edited_at: None,
        deleted: false,
        mentions: vec![],
    };
    sqlx::query("INSERT INTO messages (id, channel_id, author_id, kind, content, system_payload, created_at) VALUES (?, ?, NULL, 'system', ?, ?, ?)")
        .bind(msg.id.to_string())
        .bind(channel.to_string())
        .bind(&msg.content)
        .bind(payload.to_string())
        .bind(&msg.created_at)
        .execute(&s.db)
        .await?;
    s.hub
        .publish(
            &s.db,
            Event::MessageCreated {
                message: msg.clone(),
                nonce: None,
            },
        )
        .await;
    Ok(msg)
}

/// "a", "a and b", "a, b and c".
pub fn join_names(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [init @ .., last] => format!("{} and {}", init.join(", "), last),
    }
}
