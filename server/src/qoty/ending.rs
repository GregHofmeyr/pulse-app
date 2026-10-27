//! Ending a vote (early, or by time from the sweeper and at boot), its winner cards and wins, and
//! undoing an ending when an early end is reopened.

use std::collections::HashMap;
use std::time::Duration;

use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ChannelId, MessageId, QotyEventId, ServerId, UserId};
use pulse_protocol::rest::{Message, QotyCard, QotyEndReason, QotyEvent};
use sqlx::SqliteConnection;

use super::{publish, reason_str, votes_of};
use crate::AppState;
use crate::db::now;
use crate::error::AppResult;
use crate::messages::system;
use crate::quotes::{effective_speaker, links_for, parse_id, parse_opt};

/// How often the sweeper ends votes whose time ran out.
pub const SWEEP_EVERY: Duration = Duration::from_secs(10);

/// An ending, committed and waiting to be published.
pub(crate) struct Ended {
    pub event: QotyEventId,
    pub cards: Vec<Message>,
}

/// End the event on `tx` if it is still live: record the winners (most votes, if any) with their
/// speakers as they are now, and post the cards. `None` if it had already ended, so of any
/// endings that race exactly one does this (`UPDATE … WHERE ended_at IS NULL`).
pub(crate) async fn finish(
    tx: &mut SqliteConnection,
    id: QotyEventId,
    reason: QotyEndReason,
    by: Option<UserId>,
    at: &str,
) -> AppResult<Option<Ended>> {
    let ended: Option<(String, i64)> = sqlx::query_as(
        "UPDATE qoty_events SET ended_at = ?, ended_by = ?, end_reason = ?
         WHERE id = ? AND ended_at IS NULL
         RETURNING server_id, year",
    )
    .bind(at)
    .bind(by.map(|u| u.to_string()))
    .bind(reason_str(reason))
    .bind(id.to_string())
    .fetch_optional(&mut *tx)
    .await?;
    let Some((server, year)) = ended else {
        return Ok(None);
    };
    let server: ServerId = parse_id(&server)?;
    let year = year as i32;
    let votes = votes_of(&mut *tx, id).await?;
    let top = votes.values().map(Vec::len).max().unwrap_or(0);
    let card = |message_id: Option<MessageId>, voters: Vec<UserId>| QotyCard {
        event_id: id,
        year,
        message_id,
        votes: voters.len() as u32,
        voters,
        reason,
    };

    let mut cards = Vec::new();
    if top == 0 {
        // Nobody voted: say so in every quotes channel of the server.
        let channels: Vec<String> = sqlx::query_scalar(
            "SELECT id FROM channels WHERE server_id = ? AND kind = 'quotes' ORDER BY position, id",
        )
        .bind(server.to_string())
        .fetch_all(&mut *tx)
        .await?;
        for ch in channels {
            let content =
                format!("Nobody voted in Quote of the Year {year}, so there's no winner.");
            cards.push(post_card(tx, parse_id(&ch)?, content, card(None, vec![])).await?);
        }
        return Ok(Some(Ended { event: id, cards }));
    }

    // Co-winners: every option with the most votes, a card each in its own channel.
    let mut winners: Vec<(String, Vec<UserId>)> =
        votes.into_iter().filter(|(_, v)| v.len() == top).collect();
    winners.sort();
    let links = links_for(&mut *tx, &[server.to_string()]).await?;
    let no_links = HashMap::new();
    let links = links.get(&server.to_string()).unwrap_or(&no_links);
    for (message, voters) in winners {
        sqlx::query("INSERT INTO qoty_winners (event_id, message_id, votes) VALUES (?, ?, ?)")
            .bind(id.to_string())
            .bind(&message)
            .bind(voters.len() as i64)
            .execute(&mut *tx)
            .await?;
        snapshot_speakers(tx, id, &message, links).await?;
        let (channel, content): (String, String) =
            sqlx::query_as("SELECT channel_id, content FROM messages WHERE id = ?")
                .bind(&message)
                .fetch_one(&mut *tx)
                .await?;
        let content = format!("Quote of the Year {year}: {content}");
        let c = card(Some(parse_id(&message)?), voters);
        cards.push(post_card(tx, parse_id(&channel)?, content, c).await?);
    }
    Ok(Some(Ended { event: id, cards }))
}

/// Who said each line of a winning quote at the end: the effective speaker (a member, also by a
/// link) and the plain name if the line has one. Later edits and links don't change it.
async fn snapshot_speakers(
    tx: &mut SqliteConnection,
    event: QotyEventId,
    message: &str,
    links: &HashMap<String, UserId>,
) -> AppResult<()> {
    /// (position, member, plain name's key, plain name)
    type Line = (i64, Option<String>, Option<String>, Option<String>);
    let lines: Vec<Line> = sqlx::query_as(
        "SELECT position, speaker_id, speaker_key, speaker_name FROM quote_lines
         WHERE message_id = ? ORDER BY position",
    )
    .bind(message)
    .fetch_all(&mut *tx)
    .await?;
    for (position, speaker_id, key, name) in lines {
        let who = effective_speaker(parse_opt(speaker_id.as_deref())?, key.as_deref(), links);
        sqlx::query(
            "INSERT INTO qoty_winner_speakers (event_id, message_id, position, user_id, name)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(event.to_string())
        .bind(message)
        .bind(position)
        .bind(who.map(|u| u.to_string()))
        .bind(name)
        .execute(&mut *tx)
        .await?;
    }
    Ok(())
}

async fn post_card(
    tx: &mut SqliteConnection,
    channel: ChannelId,
    content: String,
    card: QotyCard,
) -> AppResult<Message> {
    let payload = system::qoty_card_payload(&card)?;
    system::insert(tx, channel, content, payload).await
}

/// On `tx`, end the server's live vote by time if its deadline has passed but no sweep has ended
/// it yet, so it doesn't stand in the way of a start or reopen. The caller announces the result
/// after commit, under the same `in_order` guard.
pub(crate) async fn end_if_overdue(
    tx: &mut SqliteConnection,
    server: ServerId,
) -> AppResult<Option<Ended>> {
    let live: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT id, ends_at FROM qoty_events WHERE server_id = ? AND ended_at IS NULL",
    )
    .bind(server.to_string())
    .fetch_optional(&mut *tx)
    .await?;
    match live {
        Some((id, ends_at)) if super::past(ends_at.as_deref()) => {
            finish(tx, parse_id(&id)?, QotyEndReason::Time, None, &now()).await
        }
        _ => Ok(None),
    }
}

/// Publish a committed ending: its cards, then `QotyEnded`. Under the caller's `in_order` guard.
pub(crate) async fn announce(s: &AppState, ended: Ended) -> Option<QotyEvent> {
    for message in ended.cards {
        s.hub
            .publish(
                &s.db,
                Event::MessageCreated {
                    message,
                    nonce: None,
                },
            )
            .await;
    }
    publish(s, ended.event, |event| Event::QotyEnded { event }).await
}

/// Take back an ending that is being reopened, on `tx`: its wins go, its cards are deleted
/// outright (returned for `MessageDeleted`), and options whose quote was deleted or won another
/// vote since the end go with their votes, so such a quote can't win (again) after all.
pub(crate) async fn undo(
    tx: &mut SqliteConnection,
    id: QotyEventId,
    server: ServerId,
) -> AppResult<Vec<(ChannelId, MessageId)>> {
    let cards: Vec<(String, String)> = sqlx::query_as(
        "DELETE FROM messages
         WHERE kind = 'system'
           AND channel_id IN (SELECT id FROM channels WHERE server_id = ?)
           AND json_extract(system_payload, '$.type') = ?
           AND json_extract(system_payload, '$.event_id') = ?
         RETURNING channel_id, id",
    )
    .bind(server.to_string())
    .bind(system::QOTY_CARD)
    .bind(id.to_string())
    .fetch_all(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM qoty_winners WHERE event_id = ?")
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    // This event's own wins are gone by now, so any win left is another vote's.
    const GONE: &str = "event_id = ? AND (
        message_id IN (SELECT id FROM messages WHERE deleted_at IS NOT NULL)
        OR message_id IN (SELECT message_id FROM qoty_winners))";
    // Votes go with their option (foreign key cascade), but not by luck: say so.
    for table in ["qoty_votes", "qoty_options"] {
        sqlx::query(&format!("DELETE FROM {table} WHERE {GONE}"))
            .bind(id.to_string())
            .execute(&mut *tx)
            .await?;
    }
    cards
        .into_iter()
        .map(|(ch, m)| Ok((parse_id(&ch)?, parse_id(&m)?)))
        .collect()
}

/// End every live vote whose time ran out by `due` (normally now), as the sweeper does every
/// [`SWEEP_EVERY`] and the server once before it serves. They are stamped as ended once the lock
/// is held, so no vote that committed meanwhile is later than the end. Returns how many it ended.
pub async fn end_overdue(s: &AppState, due: &str) -> AppResult<u32> {
    const DUE: &str = "SELECT id FROM qoty_events
                       WHERE ended_at IS NULL AND ends_at IS NOT NULL AND ends_at <= ?";
    // A cheap look first, so an idle sweep takes no locks.
    let any: Option<String> = sqlx::query_scalar(&format!("{DUE} LIMIT 1"))
        .bind(due)
        .fetch_optional(&s.db)
        .await?;
    if any.is_none() {
        return Ok(0);
    }
    let _order = s.hub.in_order().await;
    let mut tx = crate::db::begin_write(&s.db).await?;
    let ended_at = now();
    let ids: Vec<String> = sqlx::query_scalar(DUE)
        .bind(due)
        .fetch_all(&mut *tx)
        .await?;
    let mut done = Vec::new();
    for id in ids {
        if let Some(ended) = finish(
            &mut tx,
            parse_id(&id)?,
            QotyEndReason::Time,
            None,
            &ended_at,
        )
        .await?
        {
            done.push(ended);
        }
    }
    tx.commit().await?;
    let n = done.len() as u32;
    for ended in done {
        announce(s, ended).await;
    }
    Ok(n)
}

/// End votes on time: every `every` ([`SWEEP_EVERY`] in the server), for as long as the process
/// runs.
pub fn spawn_sweeper(s: AppState, every: Duration) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(every);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            match end_overdue(&s, &now()).await {
                Ok(0) => {}
                Ok(n) => tracing::info!(n, "Quote of the Year votes ended on time"),
                Err(e) => tracing::warn!(error = %e, "Quote of the Year sweep failed"),
            }
        }
    });
}
