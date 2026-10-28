//! System lines ("sam added riley"): author-less messages with a JSON payload for clients.

use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ChannelId, MessageId};
use pulse_protocol::rest::{Message, MessageKind, QotyCard};
use sqlx::SqliteConnection;

use crate::AppState;
use crate::db::now;
use crate::error::AppResult;

/// The payload `type` of a Quote of the Year result card.
pub const QOTY_CARD: &str = "qoty_card";

pub async fn post(
    s: &AppState,
    channel: ChannelId,
    content: String,
    payload: serde_json::Value,
) -> AppResult<Message> {
    let mut conn = s.db.acquire().await?;
    let msg = insert(&mut conn, channel, content, payload).await?;
    drop(conn);
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

/// Store a system line on `db` (a caller's transaction, say) without publishing it.
pub async fn insert(
    db: &mut SqliteConnection,
    channel: ChannelId,
    content: String,
    payload: serde_json::Value,
) -> AppResult<Message> {
    let stored = payload.to_string();
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
        reactions: vec![],
        thread_root_id: None,
        quote: None,
        qoty_card: qoty_card(&stored),
    };
    sqlx::query("INSERT INTO messages (id, channel_id, author_id, kind, content, system_payload, created_at) VALUES (?, ?, NULL, 'system', ?, ?, ?)")
        .bind(msg.id.to_string())
        .bind(channel.to_string())
        .bind(&msg.content)
        .bind(stored)
        .bind(&msg.created_at)
        .execute(db)
        .await?;
    Ok(msg)
}

/// The payload of a Quote of the Year card, tagged so it can be told from other system lines.
pub fn qoty_card_payload(card: &QotyCard) -> AppResult<serde_json::Value> {
    let mut v = serde_json::to_value(card).map_err(anyhow::Error::from)?;
    v["type"] = QOTY_CARD.into();
    Ok(v)
}

/// The card in a stored payload, if it is one.
pub fn qoty_card(payload: &str) -> Option<QotyCard> {
    let v: serde_json::Value = serde_json::from_str(payload).ok()?;
    if v.get("type")?.as_str()? != QOTY_CARD {
        return None;
    }
    serde_json::from_value(v).ok()
}

/// "a", "a and b", "a, b and c".
pub fn join_names(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [init @ .., last] => format!("{} and {}", init.join(", "), last),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pulse_protocol::ids::QotyEventId;
    use pulse_protocol::rest::QotyEndReason;

    #[test]
    fn a_card_round_trips_through_its_payload() {
        let card = QotyCard {
            event_id: QotyEventId::new(),
            year: 2026,
            message_id: Some(MessageId::new()),
            votes: 2,
            voters: vec![],
            reason: QotyEndReason::Early,
            speakers: vec![],
        };
        let stored = qoty_card_payload(&card).unwrap().to_string();
        assert_eq!(qoty_card(&stored), Some(card));
        assert_eq!(qoty_card(r#"{"type":"members_added","by":"x"}"#), None);
        assert_eq!(qoty_card("not json"), None);
    }

    #[test]
    fn a_card_from_before_speakers_were_kept_still_parses() {
        let card = QotyCard {
            event_id: QotyEventId::new(),
            year: 2026,
            message_id: Some(MessageId::new()),
            votes: 1,
            voters: vec![],
            reason: QotyEndReason::Time,
            speakers: vec![],
        };
        let mut old = qoty_card_payload(&card).unwrap();
        old.as_object_mut().unwrap().remove("speakers");
        assert_eq!(qoty_card(&old.to_string()), Some(card));
    }
}
