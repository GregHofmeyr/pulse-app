//! Quote awards: one per person per line, never on your own line (by effective speaker).

use std::collections::HashMap;

use axum::Router;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::put;
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ChannelId, LineId, MessageId, ServerId, UserId};
use pulse_protocol::rest::SpeakerCount;
use sqlx::{SqliteConnection, SqlitePool};

use super::{effective_speaker, links_for};
use crate::AppState;
use crate::access::channel_for;
use crate::auth::AuthUser;
use crate::db::now;
use crate::error::{AppError, AppResult};
use crate::servers::routes::is_server_member;

pub fn router() -> Router<AppState> {
    Router::new().route("/quote-lines/{line_id}/award", put(give).delete(take_back))
}

async fn give(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(line): Path<LineId>,
) -> AppResult<StatusCode> {
    set(&s, me, line, true).await
}

async fn take_back(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(line): Path<LineId>,
) -> AppResult<StatusCode> {
    set(&s, me, line, false).await
}

/// A live line's quote, channel and server, plus its speaker as stored.
struct Target {
    message_id: MessageId,
    channel_id: ChannelId,
    server_id: ServerId,
    speaker: Option<UserId>,
    speaker_key: Option<String>,
}

/// The line, if it and its quote exist and the quote isn't deleted.
async fn target(db: &mut SqliteConnection, line: LineId) -> AppResult<Option<Target>> {
    type Row = (String, String, String, Option<String>, Option<String>);
    let row: Option<Row> = sqlx::query_as(
        "SELECT l.message_id, msg.channel_id, c.server_id, l.speaker_id, l.speaker_key
         FROM quote_lines l
         JOIN messages msg ON msg.id = l.message_id AND msg.deleted_at IS NULL
         JOIN channels c ON c.id = msg.channel_id
         WHERE l.id = ? AND c.server_id IS NOT NULL",
    )
    .bind(line.to_string())
    .fetch_optional(db)
    .await?;
    let Some((message, channel, server, speaker, key)) = row else {
        return Ok(None);
    };
    let parse = |e| AppError::Internal(anyhow::Error::from(e));
    Ok(Some(Target {
        message_id: message.parse().map_err(parse)?,
        channel_id: channel.parse().map_err(parse)?,
        server_id: server.parse().map_err(parse)?,
        speaker: speaker
            .as_deref()
            .map(str::parse)
            .transpose()
            .map_err(parse)?,
        speaker_key: key,
    }))
}

/// The line's effective speaker right now.
async fn speaker_of(db: &mut SqliteConnection, t: &Target) -> AppResult<Option<UserId>> {
    let links = links_for(db, &[t.server_id.to_string()]).await?;
    Ok(effective_speaker(
        t.speaker,
        t.speaker_key.as_deref(),
        links
            .get(&t.server_id.to_string())
            .unwrap_or(&HashMap::new()),
    ))
}

/// Give (`give`) or take back `me`'s award on `line`. Idempotent: a repeat changes nothing and
/// publishes nothing. The event carries the new absolute count and counted awarders.
async fn set(s: &AppState, me: UserId, line: LineId, give: bool) -> AppResult<StatusCode> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests);
    }
    // Checks through the pool first (missing, deleted and unseen all look the same), then again
    // on the write transaction, where a delete or edit that committed meanwhile wins.
    let t = {
        let mut conn = s.db.acquire().await?;
        target(&mut conn, line).await?.ok_or(AppError::NotFound)?
    };
    channel_for(&s.db, me, t.channel_id).await?;
    if !is_server_member(&s.db, t.server_id, me).await? {
        return Err(AppError::Forbidden);
    }

    // Events carry absolute state, so publish in commit order (the lock order is in_order, then
    // the write lock).
    let _order = s.hub.in_order().await;
    let mut tx = crate::db::begin_write(&s.db).await?;
    let t = target(&mut tx, line).await?.ok_or(AppError::NotFound)?;
    let speaker = speaker_of(&mut tx, &t).await?;
    let own = speaker == Some(me);
    if give && own {
        return Err(AppError::ForbiddenMsg(
            "You can't award your own line.".into(),
        ));
    }
    let changed = if give {
        sqlx::query(
            "INSERT OR IGNORE INTO quote_awards (line_id, giver_id, created_at) VALUES (?, ?, ?)",
        )
        .bind(line.to_string())
        .bind(me.to_string())
        .bind(now())
        .execute(&mut *tx)
        .await?
        .rows_affected()
    } else {
        sqlx::query("DELETE FROM quote_awards WHERE line_id = ? AND giver_id = ?")
            .bind(line.to_string())
            .bind(me.to_string())
            .execute(&mut *tx)
            .await?
            .rows_affected()
    };
    // Taking back an uncounted award (one the speaker left before a link made it theirs)
    // changes nothing anyone sees.
    if changed == 0 || own {
        return Ok(StatusCode::NO_CONTENT);
    }
    let awarders = counted_awarders(&mut tx, line, speaker).await?;
    tx.commit().await?;
    s.hub
        .publish(
            &s.db,
            Event::QuoteAwardChanged {
                channel_id: t.channel_id,
                message_id: t.message_id,
                line_id: line,
                user_id: me,
                given: give,
                count: awarders.len() as u32,
                awarders,
            },
        )
        .await;
    Ok(StatusCode::NO_CONTENT)
}

/// Who awarded `line`, oldest first, without the effective speaker's own award.
async fn counted_awarders(
    db: &mut SqliteConnection,
    line: LineId,
    speaker: Option<UserId>,
) -> AppResult<Vec<UserId>> {
    let rows: Vec<String> = sqlx::query_scalar(
        "SELECT giver_id FROM quote_awards WHERE line_id = ? ORDER BY created_at, rowid",
    )
    .bind(line.to_string())
    .fetch_all(db)
    .await?;
    rows.iter()
        .map(|g| {
            g.parse::<UserId>()
                .map_err(|e| AppError::Internal(e.into()))
        })
        .filter(|g| g.as_ref().map_or(true, |g| Some(*g) != speaker))
        .collect()
}

/// Counted awards per effective speaker in `server`, most first (ties by name, for a stable
/// order). A linked name counts for its member; a plain name with no link counts under its
/// stored key, shown with the spelling of its most recent quote. Deleted quotes and a speaker's
/// own awards don't count.
pub async fn award_totals(db: &SqlitePool, server: ServerId) -> AppResult<Vec<SpeakerCount>> {
    type Row = (Option<String>, Option<String>, Option<String>, String);
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT l.speaker_id, l.speaker_name, l.speaker_key, a.giver_id
         FROM quote_awards a
         JOIN quote_lines l ON l.id = a.line_id
         JOIN messages msg ON msg.id = l.message_id AND msg.deleted_at IS NULL
         JOIN channels c ON c.id = msg.channel_id
         WHERE c.server_id = ?
         ORDER BY msg.id",
    )
    .bind(server.to_string())
    .fetch_all(db)
    .await?;
    let links = links_for(db, &[server.to_string()])
        .await?
        .remove(&server.to_string())
        .unwrap_or_default();

    let parse = |e| AppError::Internal(anyhow::Error::from(e));
    let mut members: HashMap<UserId, u32> = HashMap::new();
    // name_key to (latest spelling, count)
    let mut names: HashMap<String, (String, u32)> = HashMap::new();
    for (speaker, name, key, giver) in rows {
        let speaker: Option<UserId> = speaker
            .as_deref()
            .map(str::parse)
            .transpose()
            .map_err(parse)?;
        let giver: UserId = giver.parse().map_err(parse)?;
        let who = effective_speaker(speaker, key.as_deref(), &links);
        if who == Some(giver) {
            continue;
        }
        match (who, key, name) {
            (Some(user), ..) => *members.entry(user).or_default() += 1,
            (None, Some(key), Some(name)) => {
                let e = names.entry(key).or_default();
                e.0 = name;
                e.1 += 1;
            }
            _ => {}
        }
    }
    let mut out: Vec<SpeakerCount> = members
        .into_iter()
        .map(|(user, count)| SpeakerCount {
            user_id: Some(user),
            name: None,
            count,
        })
        .chain(names.into_values().map(|(name, count)| SpeakerCount {
            user_id: None,
            name: Some(name),
            count,
        }))
        .collect();
    out.sort_by(|a, b| {
        b.count.cmp(&a.count).then_with(|| {
            (a.name.as_deref(), a.user_id.map(|u| u.to_string()))
                .cmp(&(b.name.as_deref(), b.user_id.map(|u| u.to_string())))
        })
    });
    Ok(out)
}
