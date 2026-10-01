use pulse_protocol::ids::UserId;
use rand::Rng;
use sqlx::SqlitePool;

const ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";

/// Create a single-use invite code (10 chars, unambiguous alphabet).
pub async fn create(db: &SqlitePool, by: Option<UserId>) -> anyhow::Result<String> {
    // ThreadRng is !Send: keep it out of the async state machine.
    let code: String = {
        let mut rng = rand::rng();
        (0..10)
            .map(|_| ALPHABET[rng.random_range(0..ALPHABET.len())] as char)
            .collect()
    };
    sqlx::query("INSERT INTO invites (code, created_by, created_at) VALUES (?, ?, ?)")
        .bind(&code)
        .bind(by.map(|u| u.to_string()))
        .bind(crate::db::now())
        .execute(db)
        .await?;
    Ok(code)
}
