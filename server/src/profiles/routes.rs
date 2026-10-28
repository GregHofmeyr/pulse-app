use axum::extract::{Path, State};
use axum::routing::{get, put};
use axum::{Json, Router};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::UserId;
use pulse_protocol::rest::{Profile, UpdateProfileRequest};

use super::{load_profile, validate};
use crate::AppState;
use crate::auth::AuthUser;
use crate::error::{AppError, AppResult};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/users/{id}/profile", get(profile))
        .route("/me/profile", put(save))
}

async fn profile(
    State(s): State<AppState>,
    AuthUser(_): AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Profile>> {
    // A malformed id names no user, so it is a 404 like an unknown one.
    let id: UserId = id.parse().map_err(|_| AppError::NotFound)?;
    Ok(Json(
        load_profile(&s.db, id).await?.ok_or(AppError::NotFound)?,
    ))
}

/// A picture hash is acceptable if it is already what the user has, or is a live upload of the
/// right kind.
async fn picture_ok(
    tx: &mut sqlx::SqliteConnection,
    hash: Option<&str>,
    current: Option<&str>,
    kind: &str,
) -> AppResult<()> {
    let Some(hash) = hash else { return Ok(()) };
    if current == Some(hash) {
        return Ok(());
    }
    let found: Option<i64> = sqlx::query_scalar("SELECT 1 FROM media WHERE hash = ? AND kind = ?")
        .bind(hash)
        .bind(kind)
        .fetch_optional(&mut *tx)
        .await?;
    if found.is_none() {
        return Err(AppError::BadRequest(
            "That picture has expired. Upload it again.".into(),
        ));
    }
    Ok(())
}

/// Save the whole editable profile, then tell everyone.
async fn save(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Json(req): Json<UpdateProfileRequest>,
) -> AppResult<Json<Profile>> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    crate::media::writes_ok(&s.cfg)?;
    let v = validate(req)?;

    // Hold the order guard so two quick saves publish in commit order.
    let _order = s.hub.in_order().await;
    let mut tx = crate::db::begin_write(&s.db).await?;
    let (cur_avatar, cur_banner): (Option<String>, Option<String>) =
        sqlx::query_as("SELECT avatar_hash, banner_hash FROM users WHERE id = ?")
            .bind(me.to_string())
            .fetch_one(&mut *tx)
            .await?;
    picture_ok(
        &mut tx,
        v.avatar_hash.as_deref(),
        cur_avatar.as_deref(),
        "avatar",
    )
    .await?;
    picture_ok(
        &mut tx,
        v.banner_hash.as_deref(),
        cur_banner.as_deref(),
        "banner",
    )
    .await?;
    let name_color = (!v.colors.is_empty()).then(|| v.colors.join(","));
    sqlx::query(
        "UPDATE users SET avatar_hash = ?, banner_hash = ?, theme_top = ?, theme_bottom = ?, \
         name_font = ?, name_color = ?, pronouns = ?, bio = ? WHERE id = ?",
    )
    .bind(&v.avatar_hash)
    .bind(&v.banner_hash)
    .bind(&v.theme_top)
    .bind(&v.theme_bottom)
    .bind(&v.font)
    .bind(&name_color)
    .bind(&v.pronouns)
    .bind(&v.bio)
    .bind(me.to_string())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    let profile = load_profile(&s.db, me).await?.ok_or(AppError::NotFound)?;
    s.hub
        .publish(
            &s.db,
            Event::ProfileUpdated {
                user: profile.user.clone(),
                card: profile.card.clone(),
            },
        )
        .await;
    Ok(Json(profile))
}
