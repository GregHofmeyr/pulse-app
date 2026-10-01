pub mod routes;
pub mod state;

pub use state::VoiceState;

/// After a restart we cannot know when still-open sessions really ended (LiveKit does not
/// re-report participants already connected), so close them at zero length rather than let them
/// run on and corrupt voice-hour stats.
pub async fn reconcile_on_startup(db: &sqlx::SqlitePool) -> anyhow::Result<()> {
    let n = sqlx::query("UPDATE voice_sessions SET left_at = joined_at WHERE left_at IS NULL")
        .execute(db)
        .await?
        .rows_affected();
    if n > 0 {
        tracing::warn!(
            closed = n,
            "closed voice sessions left open by a previous run"
        );
    }
    Ok(())
}
