use std::sync::Arc;

use sqlx::SqlitePool;

use crate::config::Config;
use crate::gateway::Hub;
use crate::streams::state::StreamState;
use crate::voice::VoiceState;

#[derive(Clone)]
pub struct AppState {
    pub db: SqlitePool,
    pub cfg: Arc<Config>,
    pub hub: Hub,
    pub voice: VoiceState,
    pub streams: StreamState,
    pub limits: Arc<crate::limits::Limits>,
    /// Clip video storage; `None` when the server has none configured.
    pub r2: Option<crate::media::r2::R2>,
    /// Link previews: the guarded fetcher and its cache.
    pub embeds: Arc<crate::embeds::Unfurler>,
    /// Steam linking and achievements (#379): sign-ins in progress, the poller's memory, the Web API client.
    pub steam: crate::steam::Steam,
    /// Deadlock stats (#390): the deadlock-api client, its health, and the poller's memory.
    pub deadlock: crate::deadlock::Deadlock,
    /// Sports score pings (#385): the API-Sports client, its allowance and health.
    pub sports: crate::sports::Sports,
}
