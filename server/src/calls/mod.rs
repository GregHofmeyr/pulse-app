//! Calls in DMs and groups (#50).
//!
//! A call starts when a conversation's voice room goes from empty to occupied, however someone got
//! in, and ends once the room has been empty for [`CallTimings::grace`] (so everyone dropping and
//! reconnecting carries on the same call). Only `POST /calls/{channel}` rings: every other member,
//! never the caller, for [`CallTimings::ring`]. A ring ends for one person when they answer (join,
//! on any device) or decline, and for everyone when it times out or the call ends; each end goes
//! to that person's own sessions only, so all their devices agree and the caller is never told.
//!
//! Calls and who joined them are stored (migration 0022); rings live in memory, like streams, so a
//! restart drops them. [`load_open`] picks open calls back up after a restart.
//!
//! Server voice channels never get here: they stay silent (no ring, no line, no record).
//!
//! [`CallTimings::ring`]: crate::config::CallTimings
//! [`CallTimings::grace`]: crate::config::CallTimings

pub mod routes;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use chrono::{Duration as ChronoDuration, Utc};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{CallId, ChannelId, UserId};
use pulse_protocol::rest::{
    CallInfo, CallLine, CallRing, Channel, RingEndReason, StartCallResponse,
};

use crate::AppState;
use crate::db::{now, stamp};
use crate::error::AppResult;
use crate::messages::system;

/// Open calls by conversation. One lock serialises every decision about a call (start, join,
/// leave, decline, the timers), so two people starting at once make one call and one line.
#[derive(Clone, Default)]
pub struct CallState {
    open: Arc<tokio::sync::Mutex<HashMap<ChannelId, Open>>>,
}

struct Open {
    info: CallInfo,
    /// When the room last went empty; `None` while someone is in it.
    empty_since: Option<String>,
    /// Bumped whenever the empty timer is armed or called off: a timer only acts on its own.
    epoch: u64,
    /// Who is still being rung, by whom and until when.
    ringing: HashSet<UserId>,
    ring: Option<(UserId, String)>,
}

impl Open {
    fn new(info: CallInfo) -> Self {
        Self {
            info,
            empty_since: None,
            epoch: 0,
            ringing: HashSet::new(),
            ring: None,
        }
    }

    fn ring_for(&self) -> Option<CallRing> {
        let (caller_id, expires_at) = self.ring.clone()?;
        Some(CallRing {
            call_id: self.info.id,
            channel_id: self.info.channel_id,
            caller_id,
            expires_at,
        })
    }
}

impl CallState {
    /// Open calls in `visible` conversations, and the rings `me` is getting, for Ready.
    pub async fn for_ready(
        &self,
        me: UserId,
        visible: &HashSet<ChannelId>,
    ) -> (Vec<CallInfo>, Vec<CallRing>) {
        let open = self.open.lock().await;
        let mut calls = vec![];
        let mut rings = vec![];
        for (channel, o) in open.iter() {
            if !visible.contains(channel) {
                continue;
            }
            calls.push(o.info.clone());
            if o.ringing.contains(&me)
                && let Some(r) = o.ring_for()
            {
                rings.push(r);
            }
        }
        (calls, rings)
    }

    /// The open call in `channel`, if any.
    pub async fn open_in(&self, channel: ChannelId) -> Option<CallInfo> {
        self.open.lock().await.get(&channel).map(|o| o.info.clone())
    }
}

/// A DM or group: the only conversations with calls.
pub fn is_conversation(ch: &Channel) -> bool {
    ch.server_id.is_none() && ch.kind.is_private()
}

/// `POST /calls/{channel}`: join the call going on in `ch`, or start one and ring everyone else.
/// The caller then connects to the room as for any voice channel.
pub async fn start(s: &AppState, me: UserId, ch: &Channel) -> AppResult<StartCallResponse> {
    let mut open = s.voice.calls.open.lock().await;
    if let Some(o) = open.get(&ch.id) {
        return Ok(StartCallResponse {
            call: o.info.clone(),
            rang: false,
        });
    }
    let (mut call, created) = create(s, ch.id, me).await?;
    let others: HashSet<UserId> = members(s, ch.id)
        .await?
        .into_iter()
        .filter(|u| *u != me)
        .collect();
    let expires_at =
        stamp(Utc::now() + ChronoDuration::from_std(s.cfg.calls.ring).unwrap_or_default());
    call.ringing = others.clone();
    call.ring = Some((me, expires_at));
    // The caller connects after this answer: until they do, the call is empty and waits like any.
    if s.voice.members(ch.id).is_empty() {
        call.empty_since = Some(now());
        call.epoch += 1;
        arm_empty(s, ch.id, call.info.id, call.epoch);
    }
    let info = call.info.clone();
    let ring = call.ring_for();
    open.insert(ch.id, call);
    announce_start(s, &info, created).await;
    if let Some(ring) = ring {
        s.hub
            .publish_to(&s.db, Event::CallRinging { ring }, &others)
            .await;
    }
    let (state, channel, id, after) = (s.clone(), ch.id, info.id, s.cfg.calls.ring);
    tokio::spawn(async move {
        tokio::time::sleep(after).await;
        ring_timed_out(&state, channel, id).await;
    });
    tracing::info!(call = %info.id, channel = %ch.id, rung = others.len(), "call started");
    Ok(StartCallResponse {
        call: info,
        rang: true,
    })
}

/// `user` is in `ch`'s room now (a join webhook, or a restore after a restart): a call starts if
/// none is going (ringing nobody), they count as having joined it, and their ring is answered.
pub async fn joined(s: &AppState, ch: &Channel, user: UserId) {
    if !is_conversation(ch) {
        return;
    }
    if let Err(e) = try_joined(s, ch, user).await {
        tracing::error!(error = ?e, channel = %ch.id, "call bookkeeping failed on a join");
    }
}

async fn try_joined(s: &AppState, ch: &Channel, user: UserId) -> AppResult<()> {
    let mut open = s.voice.calls.open.lock().await;
    let mut created = None;
    if !open.contains_key(&ch.id) {
        let (call, line) = create(s, ch.id, user).await?;
        tracing::info!(call = %call.info.id, channel = %ch.id, "call started by a join");
        created = Some((call.info.clone(), line));
        open.insert(ch.id, call);
    }
    let call = open.get_mut(&ch.id).expect("inserted above");
    if call.empty_since.take().is_some() {
        call.epoch += 1; // the empty timer is off
    }
    sqlx::query(
        "INSERT OR IGNORE INTO call_participants (call_id, user_id, joined_at) VALUES (?, ?, ?)",
    )
    .bind(call.info.id.to_string())
    .bind(user.to_string())
    .bind(now())
    .execute(&s.db)
    .await?;
    let answered = call.ringing.remove(&user);
    let id = call.info.id;
    if let Some((info, line)) = created {
        announce_start(s, &info, line).await;
    }
    if answered {
        ring_ended(s, ch.id, id, user, RingEndReason::Answered).await;
    }
    Ok(())
}

/// Someone left `ch`'s room. Once it's empty, the call waits [`CallTimings::grace`] for someone to
/// come back before it ends.
///
/// [`CallTimings::grace`]: crate::config::CallTimings
pub async fn left(s: &AppState, ch: &Channel) {
    if !is_conversation(ch) || !s.voice.members(ch.id).is_empty() {
        return;
    }
    let mut open = s.voice.calls.open.lock().await;
    let Some(call) = open.get_mut(&ch.id) else {
        return;
    };
    if call.empty_since.is_some() {
        return;
    }
    call.empty_since = Some(now());
    call.epoch += 1;
    arm_empty(s, ch.id, call.info.id, call.epoch);
}

/// Stop ringing `me` for `channel`'s call, on all their devices. Nobody else is told.
pub async fn decline(s: &AppState, me: UserId, channel: ChannelId) {
    let mut open = s.voice.calls.open.lock().await;
    let Some(call) = open.get_mut(&channel) else {
        return;
    };
    if call.ringing.remove(&me) {
        let id = call.info.id;
        ring_ended(s, channel, id, me, RingEndReason::Declined).await;
    }
}

/// `user` left (or was removed from) the group `channel`: their ring stops, and its call starts
/// its wait if they were the last one in it.
pub async fn member_removed(s: &AppState, channel: ChannelId, user: UserId) {
    let mut open = s.voice.calls.open.lock().await;
    let Some(call) = open.get_mut(&channel) else {
        return;
    };
    let id = call.info.id;
    if call.empty_since.is_none() && s.voice.members(channel).is_empty() {
        call.empty_since = Some(now());
        call.epoch += 1;
        arm_empty(s, channel, id, call.epoch);
    }
    if call.ringing.remove(&user) {
        ring_ended(s, channel, id, user, RingEndReason::CallEnded).await;
    }
}

/// After a restart: the calls the database still has open, back in memory (no rings: a restart
/// drops them). Run before the voice restore, whose participants then join them.
pub async fn load_open(s: &AppState) -> anyhow::Result<usize> {
    let rows: Vec<(String, String, Option<String>, String, Option<String>)> = sqlx::query_as(
        "SELECT id, channel_id, started_by, started_at, line_id FROM calls WHERE ended_at IS NULL",
    )
    .fetch_all(&s.db)
    .await?;
    let mut open = s.voice.calls.open.lock().await;
    let n = rows.len();
    for (id, channel, by, at, line) in rows {
        let info = CallInfo {
            id: id.parse()?,
            channel_id: channel.parse()?,
            started_by: by.map(|b| b.parse()).transpose()?,
            started_at: at,
            line_id: line.map(|l| l.parse()).transpose()?,
        };
        open.insert(info.channel_id, Open::new(info));
    }
    Ok(n)
}

/// After the boot restore (whether LiveKit answered or not): every open call whose room is empty
/// starts its wait, as if everyone had just left.
pub async fn arm_empty_calls(s: &AppState) {
    let mut open = s.voice.calls.open.lock().await;
    for (channel, call) in open.iter_mut() {
        if call.empty_since.is_none() && s.voice.members(*channel).is_empty() {
            call.empty_since = Some(now());
            call.epoch += 1;
            arm_empty(s, *channel, call.info.id, call.epoch);
        }
    }
}

/// Store a new call and its line ("alex started a call"); not yet announced.
async fn create(
    s: &AppState,
    channel: ChannelId,
    by: UserId,
) -> AppResult<(Open, pulse_protocol::rest::Message)> {
    let id = CallId::new();
    let username: String = sqlx::query_scalar("SELECT username FROM users WHERE id = ?")
        .bind(by.to_string())
        .fetch_one(&s.db)
        .await?;
    let payload = system::call_line_payload(&CallLine {
        call_id: id,
        started_by: Some(by),
        ended_at: None,
        joined: vec![],
    })?;
    let mut tx = crate::db::begin_write(&s.db).await?;
    let line = system::insert(
        &mut tx,
        channel,
        format!("{username} started a call"),
        payload,
    )
    .await?;
    sqlx::query(
        "INSERT INTO calls (id, channel_id, started_by, started_at, line_id) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(id.to_string())
    .bind(channel.to_string())
    .bind(by.to_string())
    .bind(&line.created_at)
    .bind(line.id.to_string())
    .execute(&mut *tx)
    .await?;
    // Like a new message, the line reopens the conversation for anyone who closed it.
    sqlx::query("DELETE FROM dm_hidden WHERE channel_id = ?")
        .bind(channel.to_string())
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    let info = CallInfo {
        id,
        channel_id: channel,
        started_by: Some(by),
        started_at: line.created_at.clone(),
        line_id: Some(line.id),
    };
    Ok((Open::new(info), line))
}

/// The line, then the call, to the conversation's members.
async fn announce_start(s: &AppState, info: &CallInfo, line: pulse_protocol::rest::Message) {
    s.hub
        .publish(
            &s.db,
            Event::MessageCreated {
                message: line,
                nonce: None,
            },
        )
        .await;
    s.hub
        .publish(&s.db, Event::CallStarted { call: info.clone() })
        .await;
}

fn arm_empty(s: &AppState, channel: ChannelId, id: CallId, epoch: u64) {
    let (state, after) = (s.clone(), s.cfg.calls.grace);
    tokio::spawn(async move {
        tokio::time::sleep(after).await;
        if let Err(e) = end_if_still_empty(&state, channel, id, epoch).await {
            tracing::error!(error = ?e, call = %id, "couldn't end a call");
        }
    });
}

async fn end_if_still_empty(
    s: &AppState,
    channel: ChannelId,
    id: CallId,
    epoch: u64,
) -> AppResult<()> {
    let mut open = s.voice.calls.open.lock().await;
    let Some(call) = open.get(&channel) else {
        return Ok(());
    };
    if call.info.id != id || call.epoch != epoch || !s.voice.members(channel).is_empty() {
        return Ok(());
    }
    let call = open.remove(&channel).expect("checked above");
    end(s, call).await
}

/// Close `call`: it lasted until its room went empty. Its line gets the end and who joined; anyone
/// still being rung stops.
async fn end(s: &AppState, call: Open) -> AppResult<()> {
    let ended_at = call.empty_since.clone().unwrap_or_else(now);
    let id = call.info.id;
    let channel = call.info.channel_id;
    let joined: Vec<String> = sqlx::query_scalar(
        "SELECT user_id FROM call_participants WHERE call_id = ? ORDER BY joined_at, user_id",
    )
    .bind(id.to_string())
    .fetch_all(&s.db)
    .await?;
    let joined: Vec<UserId> = joined
        .iter()
        .map(|u| u.parse())
        .collect::<Result<_, _>>()
        .map_err(anyhow::Error::from)?;
    let payload = system::call_line_payload(&CallLine {
        call_id: id,
        started_by: call.info.started_by,
        ended_at: Some(ended_at.clone()),
        joined,
    })?;
    let mut tx = crate::db::begin_write(&s.db).await?;
    sqlx::query("UPDATE calls SET ended_at = ? WHERE id = ?")
        .bind(&ended_at)
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    if let Some(line) = call.info.line_id {
        sqlx::query("UPDATE messages SET system_payload = ? WHERE id = ? AND deleted_at IS NULL")
            .bind(payload.to_string())
            .bind(line.to_string())
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    tracing::info!(call = %id, channel = %channel, "call ended");
    if let Some(line) = call.info.line_id
        && let Some(message) = crate::messages::routes::load(&s.db, None, line).await?
    {
        s.hub
            .publish(&s.db, Event::MessageUpdated { message })
            .await;
    }
    s.hub
        .publish(
            &s.db,
            Event::CallEnded {
                channel_id: channel,
                call_id: id,
            },
        )
        .await;
    for user in call.ringing {
        ring_ended(s, channel, id, user, RingEndReason::CallEnded).await;
    }
    Ok(())
}

async fn ring_timed_out(s: &AppState, channel: ChannelId, id: CallId) {
    let mut open = s.voice.calls.open.lock().await;
    let Some(call) = open.get_mut(&channel).filter(|c| c.info.id == id) else {
        return;
    };
    for user in std::mem::take(&mut call.ringing) {
        ring_ended(s, channel, id, user, RingEndReason::TimedOut).await;
    }
}

async fn ring_ended(
    s: &AppState,
    channel: ChannelId,
    id: CallId,
    user: UserId,
    reason: RingEndReason,
) {
    s.hub
        .publish(
            &s.db,
            Event::CallRingEnded {
                channel_id: channel,
                call_id: id,
                user_id: user,
                reason,
            },
        )
        .await;
}

async fn members(s: &AppState, channel: ChannelId) -> AppResult<Vec<UserId>> {
    let rows: Vec<String> =
        sqlx::query_scalar("SELECT user_id FROM channel_members WHERE channel_id = ?")
            .bind(channel.to_string())
            .fetch_all(&s.db)
            .await?;
    Ok(rows
        .iter()
        .map(|u| u.parse())
        .collect::<Result<_, _>>()
        .map_err(anyhow::Error::from)?)
}
