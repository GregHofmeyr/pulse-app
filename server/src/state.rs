use std::sync::Arc;

use sqlx::SqlitePool;

use crate::config::Config;
use crate::gateway::Hub;

#[derive(Clone)]
pub struct AppState {
    pub db: SqlitePool,
    pub cfg: Arc<Config>,
    pub hub: Hub,
}
