//! System lines ("sam added riley"): author-less messages with a JSON payload for clients.

use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ChannelId, MessageId, UserId};
use pulse_protocol::rest::{
    AchievementCard, CallLine, DeadlockCard, GroupLine, GroupLineKind, MemberLine, Message,
    MessageKind, PinLine, PollEnded, QotyCard, SportsCard, StageShareLine, WheelSpin,
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
        embeds_hidden: false,
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
        achievement: achievement(&stored).map(Box::new),
        deadlock: deadlock(&stored).map(Box::new),
        plan: None,
        plan_reminder: crate::plans::reminder(&stored).map(Box::new),
        sports: sports(&stored).map(Box::new),
        f1: crate::f1::card(&stored).map(Box::new),
        forwarded: None,
        group_line: group_line(&stored),
        member_line: member_line(&stored),
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

/// The payload `type` of a Steam achievement card (#379).
pub const ACHIEVEMENT: &str = "achievement";

/// An achievement card's stored payload.
pub fn achievement_payload(card: &AchievementCard) -> AppResult<serde_json::Value> {
    let mut v = serde_json::to_value(card).map_err(anyhow::Error::from)?;
    v["type"] = ACHIEVEMENT.into();
    Ok(v)
}

/// The achievement card in a stored payload, if it is one.
pub fn achievement(payload: &str) -> Option<AchievementCard> {
    let v: serde_json::Value = serde_json::from_str(payload).ok()?;
    if v.get("type")?.as_str()? != ACHIEVEMENT {
        return None;
    }
    serde_json::from_value(v).ok()
}

/// The payload `type` of a Deadlock card (#390).
pub const DEADLOCK: &str = "deadlock";

/// A Deadlock card's stored payload.
pub fn deadlock_payload(card: &DeadlockCard) -> AppResult<serde_json::Value> {
    let mut v = serde_json::to_value(card).map_err(anyhow::Error::from)?;
    v["type"] = DEADLOCK.into();
    Ok(v)
}

/// The Deadlock card in a stored payload, if it is one.
pub fn deadlock(payload: &str) -> Option<DeadlockCard> {
    let v: serde_json::Value = serde_json::from_str(payload).ok()?;
    if v.get("type")?.as_str()? != DEADLOCK {
        return None;
    }
    serde_json::from_value(v).ok()
}

/// The payload `type` of a sports card (#385).
pub const SPORTS: &str = "sports";

/// A sports card's stored payload.
pub fn sports_payload(card: &SportsCard) -> AppResult<serde_json::Value> {
    let mut v = serde_json::to_value(card).map_err(anyhow::Error::from)?;
    v["type"] = SPORTS.into();
    Ok(v)
}

/// The sports card in a stored payload, if it is one.
pub fn sports(payload: &str) -> Option<SportsCard> {
    let v: serde_json::Value = serde_json::from_str(payload).ok()?;
    if v.get("type")?.as_str()? != SPORTS {
        return None;
    }
    serde_json::from_value(v).ok()
}

/// The payload `type` of a server membership line (#418).
pub const MEMBER_LINE: &str = "member_line";

/// A membership line's stored payload.
pub fn member_line_payload(line: &MemberLine) -> AppResult<serde_json::Value> {
    let mut v = serde_json::to_value(line).map_err(anyhow::Error::from)?;
    v["type"] = MEMBER_LINE.into();
    Ok(v)
}

/// The membership line in a stored payload, if it is one.
pub fn member_line(payload: &str) -> Option<MemberLine> {
    let v: serde_json::Value = serde_json::from_str(payload).ok()?;
    if v.get("type")?.as_str()? != MEMBER_LINE {
        return None;
    }
    serde_json::from_value(v).ok()
}

/// "a", "a and b", "a, b and c".
/// A group chat's own line (#114) in a stored payload, if it is one: the payloads `dms::routes` and
/// `servers::routes` write for a group's creation, members and settings, read as one [`GroupLine`]
/// so clients can write it with today's names.
pub fn group_line(payload: &str) -> Option<GroupLine> {
    let v: serde_json::Value = serde_json::from_str(payload).ok()?;
    let id = |key: &str| -> Option<UserId> { serde_json::from_value(v.get(key)?.clone()).ok() };
    let ids =
        |key: &str| -> Option<Vec<UserId>> { serde_json::from_value(v.get(key)?.clone()).ok() };
    let set = |key: &str| v.get(key).is_some_and(|x| !x.is_null());
    let (kind, by, users) = match v.get("type")?.as_str()? {
        "group_created" => (GroupLineKind::Created, id("by")?, ids("with")?),
        "members_added" => (GroupLineKind::MembersAdded, id("by")?, ids("users")?),
        "member_left" => (GroupLineKind::MemberLeft, id("user")?, vec![]),
        "member_removed" => (GroupLineKind::MemberRemoved, id("by")?, vec![id("user")?]),
        "group_renamed" if set("name") => (GroupLineKind::Renamed, id("by")?, vec![]),
        "group_renamed" => (GroupLineKind::NameRemoved, id("by")?, vec![]),
        "group_icon_changed" if set("icon") => (GroupLineKind::IconChanged, id("by")?, vec![]),
        "group_icon_changed" => (GroupLineKind::IconRemoved, id("by")?, vec![]),
        _ => return None,
    };
    let name = match kind {
        GroupLineKind::Renamed => Some(v.get("name")?.as_str()?.to_owned()),
        _ => None,
    };
    Some(GroupLine {
        kind,
        by,
        users,
        name,
    })
}

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
    fn group_lines_read_from_their_payloads() {
        let (alex, sam, kim) = (UserId::new(), UserId::new(), UserId::new());
        let read = |v: serde_json::Value| group_line(&v.to_string());
        let created =
            read(serde_json::json!({"type": "group_created", "by": alex, "with": [sam, kim]}))
                .unwrap();
        assert_eq!(created.kind, GroupLineKind::Created);
        assert_eq!((created.by, created.users), (alex, vec![sam, kim]));
        let left = read(serde_json::json!({"type": "member_left", "user": sam})).unwrap();
        assert_eq!((left.kind, left.by), (GroupLineKind::MemberLeft, sam));
        let removed =
            read(serde_json::json!({"type": "member_removed", "by": alex, "user": kim})).unwrap();
        assert_eq!(removed.users, vec![kim]);
        let renamed =
            read(serde_json::json!({"type": "group_renamed", "by": alex, "name": "Trip"})).unwrap();
        assert_eq!(
            (renamed.kind, renamed.name.as_deref()),
            (GroupLineKind::Renamed, Some("Trip"))
        );
        let unnamed =
            read(serde_json::json!({"type": "group_renamed", "by": alex, "name": null})).unwrap();
        assert_eq!(unnamed.kind, GroupLineKind::NameRemoved);
        let icon = read(serde_json::json!({"type": "group_icon_changed", "by": alex, "icon": "h"}))
            .unwrap();
        assert_eq!(icon.kind, GroupLineKind::IconChanged);
        let no_icon =
            read(serde_json::json!({"type": "group_icon_changed", "by": alex, "icon": null}))
                .unwrap();
        assert_eq!(no_icon.kind, GroupLineKind::IconRemoved);
        assert!(read(serde_json::json!({"type": "call", "by": alex})).is_none());
        assert!(read(serde_json::json!({"type": "members_added", "by": "nope"})).is_none());
    }

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
            weights: Some(vec![1, 2, 3]),
            palette: Some("gems".into()),
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
    fn an_achievement_card_round_trips_through_its_payload() {
        use pulse_protocol::ids::UserId;
        use pulse_protocol::rest::AchievementEntry;
        let card = AchievementCard {
            user_id: UserId::new(),
            app_id: 4_000_001,
            game: "Space Game".into(),
            achievements: vec![AchievementEntry {
                name: "First Steps".into(),
                description: Some("Walk.".into()),
                icon: None,
                global_percent: Some(12.5),
            }],
            more: 0,
            unlocked: 1,
            total: 10,
            complete: false,
            art: None,
        };
        let stored = achievement_payload(&card).unwrap().to_string();
        assert_eq!(achievement(&stored), Some(card));
        assert_eq!(poll_ended(&stored), None);
        assert_eq!(achievement(r#"{"type":"pin"}"#), None);
    }
    #[test]
    fn a_deadlock_card_round_trips_through_its_payload() {
        use pulse_protocol::rest::{DeadlockMatchCard, DeadlockPatchCard};
        let patch = DeadlockCard::Patch(DeadlockPatchCard {
            title: "Gameplay Update".into(),
            link: "https://example.test/notes".into(),
            published_at: "2026-11-05T19:58:14Z".into(),
            summary: Some("Haze: more haze.".into()),
            heroes: vec![],
        });
        let stored = deadlock_payload(&patch).unwrap().to_string();
        assert_eq!(deadlock(&stored), Some(patch));
        let m = DeadlockCard::Match(DeadlockMatchCard {
            match_id: 50_000_001,
            duration_s: 1800,
            players: vec![],
        });
        assert_eq!(
            deadlock(&deadlock_payload(&m).unwrap().to_string()),
            Some(m)
        );
        assert_eq!(deadlock(r#"{"type":"achievement"}"#), None);
        assert_eq!(achievement(&stored), None);
    }

    #[test]
    fn a_sports_card_round_trips_through_its_payload() {
        use pulse_protocol::rest::{SportsLeague, SportsPhase, SportsScorer, SportsSide};
        let card = SportsCard {
            league: SportsLeague::Psl,
            fixture_id: 1_200_001,
            phase: SportsPhase::Live,
            kickoff: "2026-11-21T17:30:00Z".into(),
            status: "2H".into(),
            minute: Some(63),
            home: SportsSide {
                team_id: 1,
                name: "Home FC".into(),
                score: Some(1),
                logo: Some(pulse_protocol::rest::EmbedThumb {
                    hash: "a".repeat(64),
                    width: 96,
                    height: 96,
                }),
            },
            away: SportsSide {
                team_id: 2,
                name: "Away FC".into(),
                score: Some(0),
                logo: None,
            },
            scorers: vec![SportsScorer {
                home: true,
                name: "A. Striker".into(),
                minute: Some(41),
                note: Some("pen".into()),
            }],
            followers: vec![],
            no_updates: true,
        };
        let stored = sports_payload(&card).unwrap().to_string();
        assert_eq!(sports(&stored), Some(card));
        assert_eq!(deadlock(&stored), None);
        assert_eq!(sports(r#"{"type":"deadlock","kind":"patch"}"#), None);
    }

    #[test]
    fn a_member_line_round_trips_through_its_payload() {
        use pulse_protocol::ids::UserId;
        let line = MemberLine {
            user_id: UserId::new(),
            removed_by: Some(UserId::new()),
        };
        let stored = member_line_payload(&line).unwrap().to_string();
        assert_eq!(member_line(&stored), Some(line));
        assert_eq!(pin_line(&stored), None);
        // a group's member lines are another kind
        assert_eq!(member_line(r#"{"type":"member_left","user":"x"}"#), None);
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
