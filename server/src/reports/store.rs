use pulse_protocol::ids::{UserId, next_ulid};
use pulse_protocol::rest::{CreateReportRequest, ReportKind};
use sqlx::SqlitePool;

use crate::db::now;
use crate::error::{AppError, AppResult};

pub const MAX_TEXT_CHARS: usize = 4000;
pub const MAX_META_BYTES: usize = 16 * 1024;
pub const MAX_LOG_BYTES: usize = 512 * 1024;

pub fn validate(req: &CreateReportRequest) -> AppResult<()> {
    let bad = |m: &str| Err(AppError::BadRequest(m.into()));
    if req.kind == ReportKind::Bug && req.text.trim().is_empty() {
        return bad("say what happened");
    }
    if req.text.chars().count() > MAX_TEXT_CHARS {
        return bad("report text is too long (4000 characters max)");
    }
    if req.meta.to_string().len() > MAX_META_BYTES {
        return bad("report details are too large");
    }
    if req.log.len() > MAX_LOG_BYTES {
        return bad("report log is too large");
    }
    Ok(())
}

pub async fn insert(db: &SqlitePool, user: UserId, req: &CreateReportRequest) -> AppResult<String> {
    validate(req)?;
    let id = next_ulid().to_string();
    sqlx::query(
        "INSERT INTO bug_reports (id, user_id, created_at, kind, text, meta, log) VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(user.to_string())
    .bind(now())
    .bind(req.kind.as_str())
    .bind(req.text.trim())
    .bind(req.meta.to_string())
    .bind(&req.log)
    .execute(db)
    .await?;
    Ok(id)
}

#[derive(Debug, Clone)]
pub struct Listed {
    pub id: String,
    pub short: String,
    pub created_at: String,
    pub username: String,
    pub kind: String,
    pub seen: bool,
    pub headline: String,
}

#[derive(Debug, Clone)]
pub struct Full {
    pub listed: Listed,
    pub text: String,
    pub meta: serde_json::Value,
    pub log: String,
}

/// What lists show as a report's id: the random tail of the ULID (its head is a timestamp, so
/// reports sent close together would share a prefix).
pub fn short(id: &str) -> String {
    id[id.len().saturating_sub(6)..].to_string()
}

type Row = (
    String,
    String,
    String,
    String,
    Option<String>,
    String,
    String,
    String,
);

fn listed(r: &Row) -> Listed {
    let (id, created_at, username, kind, seen_at, text, meta, _) = r;
    let meta: serde_json::Value = serde_json::from_str(meta).unwrap_or_default();
    let headline = meta["crash"]["message"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| text.clone());
    Listed {
        id: id.clone(),
        short: short(id),
        created_at: created_at.clone(),
        username: username.clone(),
        kind: kind.clone(),
        seen: seen_at.is_some(),
        headline: headline
            .lines()
            .next()
            .unwrap_or_default()
            .chars()
            .take(60)
            .collect(),
    }
}

const SELECT: &str =
    "SELECT r.id, r.created_at, u.username, r.kind, r.seen_at, r.text, r.meta, r.log
     FROM bug_reports r JOIN users u ON u.id = r.user_id";

pub async fn list(db: &SqlitePool, limit: i64) -> AppResult<Vec<Listed>> {
    let rows: Vec<Row> = sqlx::query_as(&format!("{SELECT} ORDER BY r.id DESC LIMIT ?"))
        .bind(limit)
        .fetch_all(db)
        .await?;
    Ok(rows.iter().map(listed).collect())
}

pub async fn find(db: &SqlitePool, short_id: &str) -> AppResult<Full> {
    let rows: Vec<Row> = sqlx::query_as(&format!(
        "{SELECT} WHERE r.id LIKE '%' || ? ORDER BY r.id DESC"
    ))
    .bind(short_id.to_uppercase())
    .fetch_all(db)
    .await?;
    match rows.len() {
        0 => Err(AppError::NotFound),
        1 => {
            let r = &rows[0];
            Ok(Full {
                listed: listed(r),
                text: r.5.clone(),
                meta: serde_json::from_str(&r.6).unwrap_or_default(),
                log: r.7.clone(),
            })
        }
        n => Err(AppError::Conflict(format!(
            "'{short_id}' matches {n} reports; use more characters"
        ))),
    }
}

pub async fn mark_seen(db: &SqlitePool, id: &str) -> AppResult<()> {
    sqlx::query("UPDATE bug_reports SET seen_at = COALESCE(seen_at, ?) WHERE id = ?")
        .bind(now())
        .bind(id)
        .execute(db)
        .await?;
    Ok(())
}
