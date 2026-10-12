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
