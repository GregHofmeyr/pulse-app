//! THE privacy choke point: who may receive an event. Every broadcast goes through here.

use std::collections::HashSet;

use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ServerId, UserId};
use pulse_protocol::rest::ChannelKind;
use sqlx::SqlitePool;

use crate::access::load_channel;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Audience {
    /// Every connected user. Nothing goes to everyone since accounts and profiles narrowed (#486); it stays as what an
    /// event about nobody's server, conversation or person would get.
    Everyone,
    /// Exactly these users. Empty means nobody.
    Users(HashSet<UserId>),
}

impl Audience {
    pub fn includes(&self, user: UserId) -> bool {
        match self {
            Self::Everyone => true,
            Self::Users(u) => u.contains(&user),
        }
    }
}

pub async fn audience_for(db: &SqlitePool, event: &Event) -> anyhow::Result<Audience> {
    // Per-user events (read points, mutes, closing, removal) reach that user's own sessions only.
    if let Some(owner) = event.only_for() {
        return Ok(Audience::Users([owner].into_iter().collect()));
    }
    // What someone listens to (#275) or plays (#380) is told per viewer by `Hub::sync_presence`, never to an
    // audience: published by mistake, it reaches nobody.
    if matches!(
        event,
        Event::ListeningChanged { .. } | Event::PlayingChanged { .. }
    ) {
        return Ok(Audience::Users(HashSet::new()));
    }
    // A server-level event reaches that server's members only (#80): servers are private. Someone who just left (or
    // was removed) hears it too, so their devices drop the server.
    if let Some(server) = event.server_scope() {
        let mut members = server_members(db, server).await?;
        if let Event::MemberLeft { user_id, .. } = event {
            members.insert(*user_id);
        }
        return Ok(Audience::Users(members));
    }
    // A new account or a profile change (#486): only people who share a server, a DM or a group with them, as their
    // presence, and their own sessions. Someone who comes to share one later is introduced by the presence sync.
    if let Event::UserCreated { user } | Event::ProfileUpdated { user, .. } = event {
        let mut known = crate::presence::known_to(db, user.id).await?;
        known.insert(user.id);
        return Ok(Audience::Users(known));
    }
    let Some(channel_id) = event.channel_id() else {
        // About no server, conversation or person.
        return Ok(Audience::Everyone);
    };
    let Some(channel) = load_channel(db, channel_id).await? else {
        // Unknown channel: fail closed.
        return Ok(Audience::Users(HashSet::new()));
    };
    if let Some(server) = channel.server_id {
        // a server channel: its server's members (#80)
        return Ok(Audience::Users(server_members(db, server).await?));
    }
    if !channel.kind.is_private() {
        // a channel in no server that isn't a conversation: none exist, so fail closed
        return Ok(Audience::Users(HashSet::new()));
    }
    let members: Vec<String> =
        sqlx::query_scalar("SELECT user_id FROM channel_members WHERE channel_id = ?")
            .bind(channel_id.to_string())
            .fetch_all(db)
            .await?;
    let mut members: HashSet<UserId> = members
        .iter()
        .map(|m| m.parse())
        .collect::<Result<_, _>>()?;
    // A 1:1 DM with someone you've blocked (#474): what they send and do there never reaches you.
    if channel.kind == ChannelKind::Dm {
        for u in crate::blocks::dm_drop(db, event, &members).await? {
            members.remove(&u);
        }
    }
    Ok(Audience::Users(members))
}

/// Everyone in `server`, deleted or not: a deleted server's events (its delete, its restore) reach the members it
/// keeps.
async fn server_members(db: &SqlitePool, server: ServerId) -> anyhow::Result<HashSet<UserId>> {
    let ids: Vec<String> =
        sqlx::query_scalar("SELECT user_id FROM server_members WHERE server_id = ?")
            .bind(server.to_string())
            .fetch_all(db)
            .await?;
    Ok(ids.iter().map(|m| m.parse()).collect::<Result<_, _>>()?)
}
