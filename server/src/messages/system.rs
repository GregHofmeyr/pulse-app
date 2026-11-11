//! System lines ("sam added riley"): author-less messages with a JSON payload for clients.

use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ChannelId, MessageId};
use pulse_protocol::rest::{
    CallLine, Message, MessageKind, PinLine, PollEnded, QotyCard, StageShareLine, WheelSpin,
};
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
        clip: None,
        embeds: vec![],
        call: call_line(&stored),
        qoty_card: qoty_card(&stored),
        pin: None,
        pin_line: pin_line(&stored),
        wheel: wheel_spin(&stored).map(Box::new),
        poll: None,
        poll_ended: poll_ended(&stored).map(Box::new),
        stage_share_line: stage_share_line(&stored),
        gif: None,
        attachments: vec![],
        caption_is_fallback: false,
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

/// The payload `type` of a "shared to the stage" line (#234). New shares post the link as the sharer's own message
/// instead (#288); the lines already in the history still read back.
pub const STAGE_SHARE_LINE: &str = "stage_share";

/// The stage share line in a stored payload, if it is one.
pub fn stage_share_line(payload: &str) -> Option<StageShareLine> {
    let v: serde_json::Value = serde_json::from_str(payload).ok()?;
    if v.get("type")?.as_str()? != STAGE_SHARE_LINE {
        return None;
    }
    serde_json::from_value(v).ok()
}

/// The payload `type` of a DM or group's call line (#50).
pub const CALL_LINE: &str = "call";

/// A call line's stored payload.
pub fn call_line_payload(line: &CallLine) -> AppResult<serde_json::Value> {
    let mut v = serde_json::to_value(line).map_err(anyhow::Error::from)?;
    v["type"] = CALL_LINE.into();
    Ok(v)
}

/// The call line in a stored payload, if it is one.
pub fn call_line(payload: &str) -> Option<CallLine> {
    let v: serde_json::Value = serde_json::from_str(payload).ok()?;
    if v.get("type")?.as_str()? != CALL_LINE {
        return None;
    }
    serde_json::from_value(v).ok()
}

/// The payload `type` of a pin's line (#178).
pub const PIN_LINE: &str = "pin";

/// A pin line's stored payload.
pub fn pin_line_payload(line: &PinLine) -> AppResult<serde_json::Value> {
    let mut v = serde_json::to_value(line).map_err(anyhow::Error::from)?;
    v["type"] = PIN_LINE.into();
    Ok(v)
}

/// The pin line in a stored payload, if it is one.
pub fn pin_line(payload: &str) -> Option<PinLine> {
    let v: serde_json::Value = serde_json::from_str(payload).ok()?;
    if v.get("type")?.as_str()? != PIN_LINE {
        return None;
    }
    serde_json::from_value(v).ok()
}

/// The payload `type` of a picker wheel spin's line (#158).
pub const WHEEL_SPIN: &str = "wheel_spin";

/// A spin line's stored payload.
pub fn wheel_spin_payload(spin: &WheelSpin) -> AppResult<serde_json::Value> {
    let mut v = serde_json::to_value(spin).map_err(anyhow::Error::from)?;
    v["type"] = WHEEL_SPIN.into();
    Ok(v)
}

/// The spin in a stored payload, if it is one.
pub fn wheel_spin(payload: &str) -> Option<WheelSpin> {
    let v: serde_json::Value = serde_json::from_str(payload).ok()?;
    if v.get("type")?.as_str()? != WHEEL_SPIN {
        return None;
    }
    serde_json::from_value(v).ok()
}

/// The payload `type` of a poll's "ended" line (#157).
pub const POLL_ENDED: &str = "poll_ended";

/// An ended line's stored payload.
pub fn poll_ended_payload(line: &PollEnded) -> AppResult<serde_json::Value> {
    let mut v = serde_json::to_value(line).map_err(anyhow::Error::from)?;
    v["type"] = POLL_ENDED.into();
    Ok(v)
}

/// The ended line in a stored payload, if it is one.
pub fn poll_ended(payload: &str) -> Option<PollEnded> {
    let v: serde_json::Value = serde_json::from_str(payload).ok()?;
    if v.get("type")?.as_str()? != POLL_ENDED {
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
    fn a_pin_line_round_trips_through_its_payload() {
        use pulse_protocol::ids::UserId;
        use pulse_protocol::rest::PinDuration;
        let line = PinLine {
            message_id: MessageId::new(),
            by: UserId::new(),
            duration: PinDuration::Day,
        };
        let stored = pin_line_payload(&line).unwrap().to_string();
        assert_eq!(pin_line(&stored), Some(line));
        assert_eq!(call_line(&stored), None);
        assert_eq!(pin_line(r#"{"type":"call"}"#), None);
    }

    #[test]
    fn a_spin_round_trips_through_its_payload() {
        let spin = WheelSpin {
            spinner_id: None,
            wheel_id: Some("w".into()),
            name: Some("Games".into()),
            options: vec!["a".into(), "b".into(), "c".into()],
            winner: 2,
            angle: 12.5,
        };
        let stored = wheel_spin_payload(&spin).unwrap().to_string();
        assert_eq!(wheel_spin(&stored), Some(spin));
        assert_eq!(call_line(&stored), None);
        assert_eq!(qoty_card(&stored), None);
        assert_eq!(wheel_spin(r#"{"type":"call","call_id":"x"}"#), None);
    }

    #[test]
    fn a_poll_ended_line_round_trips_through_its_payload() {
        let line = PollEnded {
            poll_message_id: MessageId::new(),
            author_id: None,
            winners: vec!["Valheim".into()],
            votes: 4,
        };
        let stored = poll_ended_payload(&line).unwrap().to_string();
        assert_eq!(poll_ended(&stored), Some(line));
        assert_eq!(wheel_spin(&stored), None);
        assert_eq!(poll_ended(r#"{"type":"pin"}"#), None);
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
