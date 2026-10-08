//! THE privacy choke point: who may receive an event. Every broadcast goes through here.

use std::collections::HashSet;

use pulse_protocol::gateway::Event;
use pulse_protocol::ids::UserId;
use sqlx::SqlitePool;

use crate::access::load_channel;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Audience {
    /// Every connected user (server-scoped data; all servers are public to members of the group).
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
    let Some(channel_id) = event.channel_id() else {
        // ServerCreated / MemberJoined: server-level, public.
        return Ok(Audience::Everyone);
    };
    let Some(channel) = load_channel(db, channel_id).await? else {
        // Unknown channel: fail closed.
        return Ok(Audience::Users(HashSet::new()));
    };
    if !channel.kind.is_private() {
        return Ok(Audience::Everyone);
    }
    let members: Vec<String> =
        sqlx::query_scalar("SELECT user_id FROM channel_members WHERE channel_id = ?")
            .bind(channel_id.to_string())
            .fetch_all(db)
            .await?;
    Ok(Audience::Users(
        members
            .iter()
            .map(|m| m.parse())
            .collect::<Result<_, _>>()?,
    ))
}
