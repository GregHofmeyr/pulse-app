//! The `<channel>~s` room as LiveKit's webhook reports it. Only `#v` identities matter for
//! state; `#a` (the audio connection) is logged and ignored.

use livekit_protocol::WebhookEvent;
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ChannelId, UserId};
use pulse_protocol::rest::{StreamEndReason, StreamRole, StreamStatus};

use crate::AppState;
use crate::error::AppResult;

/// `"<user ulid>#v"` or `"<user ulid>#a"`; anything else is not a stream-room identity.
pub fn parse_identity(identity: &str) -> Option<(UserId, StreamRole)> {
    let (id, suffix) = identity.split_once('#')?;
    let role = match suffix {
        "v" => StreamRole::V,
        "a" => StreamRole::A,
        _ => return None,
    };
    Some((id.parse().ok()?, role))
}

pub async fn handle(s: &AppState, channel: &str, ev: &WebhookEvent) -> AppResult<()> {
    let Ok(channel_id) = channel.parse::<ChannelId>() else {
        return Ok(());
    };
    let Some(p) = ev.participant.as_ref() else {
        return Ok(());
    };
    let Some((user, role)) = parse_identity(&p.identity) else {
        return Ok(());
    };
    tracing::info!(
        target: "share",
        "share: {}",
        serde_json::json!({
            "what": "stream_room_webhook",
            "event": ev.event,
            "channel": channel_id.to_string(),
            "user": user.to_string(),
            "role": if role == StreamRole::V { "v" } else { "a" },
        })
    );
    // A join needs a live session behind it, `#a` included (see `voice::revoke::admit`).
    if ev.event == "participant_joined"
        && !crate::voice::revoke::admit(s, &format!("{channel}~s"), p).await?
    {
        return Ok(());
    }
    if ev.event == "participant_left" {
        crate::voice::revoke::left(s, p).await?;
    }
    if role != StreamRole::V {
        return Ok(());
    }
    match ev.event.as_str() {
        "participant_left" => {
            if !s.streams.video_left(channel_id, user, &p.sid) {
                // LiveKit kicked this connection for a newer one, which is still in
                return Ok(());
            }
            streamer_dropped(s, channel_id, user).await;
            for (streamer, viewers) in s.streams.remove_viewer_everywhere(channel_id, user) {
                crate::streams::routes::publish_viewers(s, channel_id, streamer, viewers).await;
            }
        }
        "participant_joined" => {
            s.streams.video_joined(channel_id, user, &p.sid);
            if let Some(stream) = s.streams.set_status(channel_id, user, StreamStatus::Live) {
                s.streams.bump_epoch(channel_id, user);
                s.hub.publish(&s.db, Event::StreamUpdated { stream }).await;
            }
        }
        _ => {}
    }
    Ok(())
}

/// A live streamer's video connection is gone: show `Reconnecting` and give them
/// `stream_media_timeout` to come back before the stream ends.
async fn streamer_dropped(s: &AppState, channel: ChannelId, user: UserId) {
    let Some(stream) = s
        .streams
        .set_status(channel, user, StreamStatus::Reconnecting)
    else {
        return;
    };
    let epoch = s.streams.bump_epoch(channel, user);
    s.hub.publish(&s.db, Event::StreamUpdated { stream }).await;

    let s = s.clone();
    tokio::spawn(async move {
        tokio::time::sleep(s.cfg.stream_media_timeout).await;
        // A re-announce moves Reconnecting -> Live without a new epoch, so check both.
        let still_down = s.streams.epoch(channel, user) == Some(epoch)
            && s.streams
                .get(channel, user)
                .is_some_and(|i| i.status == StreamStatus::Reconnecting);
        if still_down && s.streams.end(channel, user).is_some() {
            tracing::info!(user = %user, channel = %channel, reason = "timed_out", "stream ended");
            s.hub
                .publish(
                    &s.db,
                    Event::StreamEnded {
                        channel_id: channel,
                        user_id: user,
                        reason: StreamEndReason::TimedOut,
                    },
                )
                .await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_identity_accepts_v_and_a_only() {
        let x = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
        let id: UserId = x.parse().unwrap();
        assert_eq!(parse_identity(&format!("{x}#v")), Some((id, StreamRole::V)));
        assert_eq!(parse_identity(&format!("{x}#a")), Some((id, StreamRole::A)));
        assert_eq!(parse_identity(x), None);
        assert_eq!(parse_identity(&format!("{x}#v2")), None);
        assert_eq!(parse_identity("g_x#v"), None);
    }
}
