use pulse_protocol::gateway::Event;
use sqlx::SqlitePool;

/// Fan-out hub. Stub until the gateway task: publish is a no-op with the final signature.
#[derive(Clone, Default)]
pub struct Hub {}

impl Hub {
    pub async fn publish(&self, _db: &SqlitePool, _event: Event) {}
}
