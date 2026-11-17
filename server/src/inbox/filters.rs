//! Saved inbox filters (#307): `PUT /me/inbox-filters`, your whole list at once. Each narrows the inbox's All and
//! Mentions tabs to messages from some people and/or in some chats, optionally only unread ones; `GET /inbox` takes
//! the same narrowing as `from`, `in` and `unread` (the client sends the filter it has picked). Kept per person
//! (`inbox_filters`), sent only to you (Ready's `inbox_filters` and `InboxFiltersChanged`).
//!
//! A filter may name people and chats you can no longer see (someone left, a chat was deleted): they're kept, and
//! simply match nothing you could be shown.

use axum::extract::State;
use axum::routing::put;
use axum::{Json, Router};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::UserId;
use pulse_protocol::rest::{InboxFilter, InboxFiltersRequest, inbox_filter_limits as limits};
use sqlx::SqlitePool;

use crate::AppState;
use crate::auth::AuthUser;
use crate::error::{AppError, AppResult};

pub fn router() -> Router<AppState> {
    Router::new().route("/me/inbox-filters", put(save))
}

/// `owner`'s saved filters, in order.
pub async fn list_for(db: &SqlitePool, owner: UserId) -> AppResult<Vec<InboxFilter>> {
    let rows: Vec<(String, String, String, i64)> = sqlx::query_as(
        "SELECT name, people, channels, unread_only FROM inbox_filters WHERE user_id = ? ORDER BY position",
    )
    .bind(owner.to_string())
    .fetch_all(db)
    .await?;
    rows.into_iter()
        .map(|(name, people, channels, unread_only)| {
            Ok(InboxFilter {
                name,
                people: serde_json::from_str(&people).map_err(anyhow::Error::from)?,
                channels: serde_json::from_str(&channels).map_err(anyhow::Error::from)?,
                unread_only: unread_only != 0,
            })
        })
        .collect()
}

fn bad(msg: impl Into<String>) -> AppError {
    AppError::BadRequest(msg.into())
}

/// The list as it will be kept: names trimmed, 1-32 characters on one line and unique ignoring case; each filter
/// narrows by something; people and chats each at most [`limits::MAX_PER_FILTER`], without repeats.
pub fn clean(filters: Vec<InboxFilter>) -> AppResult<Vec<InboxFilter>> {
    if filters.len() > limits::MAX_FILTERS {
        return Err(bad(format!(
            "you can save up to {} filters",
            limits::MAX_FILTERS
        )));
    }
    let mut seen: Vec<String> = Vec::new();
    filters
        .into_iter()
        .map(|mut f| {
            let name = f.name.trim().to_string();
            let len = name.chars().count();
            if len == 0 || len > limits::NAME_CHARS || name.chars().any(char::is_control) {
                return Err(bad(format!(
                    "a filter's name is 1-{} characters on one line",
                    limits::NAME_CHARS
                )));
            }
            let key = name.to_lowercase();
            if seen.contains(&key) {
                return Err(AppError::Conflict(
                    "two of your filters have the same name".into(),
                ));
            }
            seen.push(key);
            f.name = name;
            f.people.sort();
            f.people.dedup();
            f.channels.sort();
            f.channels.dedup();
            if f.people.len() > limits::MAX_PER_FILTER || f.channels.len() > limits::MAX_PER_FILTER
            {
                return Err(bad(format!(
                    "a filter names up to {} people and {} chats",
                    limits::MAX_PER_FILTER,
                    limits::MAX_PER_FILTER
                )));
            }
            if f.people.is_empty() && f.channels.is_empty() && !f.unread_only {
                return Err(bad("a filter needs people, chats or unread only"));
            }
            Ok(f)
        })
        .collect()
}

/// `PUT /me/inbox-filters`: replace your saved filters. Answers with them as kept.
async fn save(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Json(req): Json<InboxFiltersRequest>,
) -> AppResult<Json<Vec<InboxFilter>>> {
    let filters = clean(req.filters)?;
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    let mut tx = crate::db::begin_write(&s.db).await?;
    sqlx::query("DELETE FROM inbox_filters WHERE user_id = ?")
        .bind(me.to_string())
        .execute(&mut *tx)
        .await?;
    for (i, f) in filters.iter().enumerate() {
        sqlx::query(
            "INSERT INTO inbox_filters (user_id, position, name, people, channels, unread_only)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(me.to_string())
        .bind(i as i64)
        .bind(&f.name)
        .bind(serde_json::to_string(&f.people).map_err(anyhow::Error::from)?)
        .bind(serde_json::to_string(&f.channels).map_err(anyhow::Error::from)?)
        .bind(i64::from(f.unread_only))
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    let filters = list_for(&s.db, me).await?;
    s.hub
        .publish(
            &s.db,
            Event::InboxFiltersChanged {
                user_id: me,
                filters: filters.clone(),
            },
        )
        .await;
    Ok(Json(filters))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(name: &str) -> InboxFilter {
        InboxFilter {
            name: name.into(),
            people: vec![],
            channels: vec![],
            unread_only: true,
        }
    }

    #[test]
    fn names_are_trimmed_unique_and_each_filter_narrows() {
        assert_eq!(clean(vec![f("  Sam ")]).unwrap()[0].name, "Sam");
        assert!(clean(vec![f(" ")]).is_err());
        assert!(clean(vec![f(&"x".repeat(33))]).is_err());
        assert!(clean(vec![f("a\nb")]).is_err());
        assert!(clean(vec![f("Sam"), f("sam")]).is_err());
        assert!(clean((0..11).map(|i| f(&i.to_string())).collect()).is_err());
        let mut none = f("None");
        none.unread_only = false;
        assert!(clean(vec![none]).is_err());
        let mut many = f("Many");
        many.people = (0..26).map(|_| UserId::new()).collect();
        assert!(clean(vec![many]).is_err());
    }
}
