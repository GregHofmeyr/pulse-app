use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::UserId;
use pulse_protocol::rest::{InviteResponse, LoginRequest, RegisterRequest, SessionResponse, User};
use sqlx::SqlitePool;

use super::extractor::{AuthUser, BearerToken};
use super::{invites, password, session};
use crate::AppState;
use crate::db::now;
use crate::error::{AppError, AppResult};
use crate::limits::ClientIp;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/auth/register", post(register))
        .route("/auth/login", post(login))
        .route("/auth/logout", post(logout))
        .route("/me", get(me))
        .route("/invites", post(create_invite))
}

fn valid_username(u: &str) -> bool {
    (2..=32).contains(&u.len())
        && u.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'.')
}

pub async fn load_user(db: &SqlitePool, id: UserId) -> AppResult<User> {
    let (username, avatar_hash): (String, Option<String>) =
        sqlx::query_as("SELECT username, avatar_hash FROM users WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(db)
            .await?
            .ok_or(AppError::NotFound)?;
    Ok(User {
        id,
        username,
        avatar_hash,
    })
}

async fn register(
    State(s): State<AppState>,
    ClientIp(ip): ClientIp,
    Json(req): Json<RegisterRequest>,
) -> AppResult<Json<SessionResponse>> {
    let ip_key = ip.to_string();
    if !s.limits.register_ip.allowed(&ip_key) {
        return Err(AppError::TooManyRequests);
    }
    // Wrong/used invite codes spend a token (guessing); good registrations don't.
    let bad_invite = |e: AppError| {
        s.limits.register_ip.hit(&ip_key);
        e
    };
    if !valid_username(&req.username) {
        return Err(AppError::BadRequest(
            "username must be 2-32 chars of a-z 0-9 _ .".into(),
        ));
    }
    if !(8..=256).contains(&req.password.len()) {
        return Err(AppError::BadRequest(
            "password must be at least 8 characters".into(),
        ));
    }
    // Check the invite before anything expensive (and before revealing whether a username exists).
    let used: Option<Option<String>> =
        sqlx::query_scalar("SELECT used_by FROM invites WHERE code = ?")
            .bind(&req.invite_code)
            .fetch_optional(&s.db)
            .await?;
    match used {
        None => return Err(bad_invite(AppError::NotFound)),
        Some(Some(_)) => return Err(bad_invite(AppError::Gone)),
        Some(None) => {}
    }
    let hash = password::hash_async(req.password.clone()).await?;
    let id = UserId::new();
    let at = now();

    let mut tx = s.db.begin().await?;
    let inserted = sqlx::query(
        "INSERT INTO users (id, username, password_hash, created_at) VALUES (?, ?, ?, ?)",
    )
    .bind(id.to_string())
    .bind(&req.username)
    .bind(&hash)
    .bind(&at)
    .execute(&mut *tx)
    .await;
    if let Err(sqlx::Error::Database(e)) = &inserted
        && e.is_unique_violation()
    {
        return Err(AppError::Conflict("username taken".into()));
    }
    inserted?;
    // Atomic single-use redemption: only the transaction that flips used_by wins.
    let claimed = sqlx::query(
        "UPDATE invites SET used_by = ?, used_at = ? WHERE code = ? AND used_by IS NULL",
    )
    .bind(id.to_string())
    .bind(&at)
    .bind(&req.invite_code)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if claimed == 0 {
        tx.rollback().await?;
        let exists: Option<i64> = sqlx::query_scalar("SELECT 1 FROM invites WHERE code = ?")
            .bind(&req.invite_code)
            .fetch_optional(&s.db)
            .await?;
        return Err(bad_invite(if exists.is_some() {
            AppError::Gone
        } else {
            AppError::NotFound
        }));
    }
    tx.commit().await?;

    let token = session::create(&s.db, id).await?;
    let user = User {
        id,
        username: req.username,
        avatar_hash: None,
    };
    s.hub
        .publish(&s.db, Event::UserCreated { user: user.clone() })
        .await;
    Ok(Json(SessionResponse { token, user }))
}

/// Real hash of a throwaway password so unknown usernames cost the same as wrong passwords.
static DUMMY_HASH: std::sync::LazyLock<String> =
    std::sync::LazyLock::new(|| password::hash("timing-equaliser").expect("hash"));

async fn login(
    State(s): State<AppState>,
    ClientIp(ip): ClientIp,
    Json(req): Json<LoginRequest>,
) -> AppResult<Json<SessionResponse>> {
    let (ip_key, user_key) = (ip.to_string(), req.username.to_lowercase());
    if !s.limits.login_ip.allowed(&ip_key) || !s.limits.login_user.allowed(&user_key) {
        return Err(AppError::TooManyRequests);
    }
    // Only failures spend tokens: friends logging in together from one network are fine.
    let failed = || {
        s.limits.login_ip.hit(&ip_key);
        s.limits.login_user.hit(&user_key);
        AppError::Unauthorized
    };
    let row: Option<(String, String, Option<String>)> =
        sqlx::query_as("SELECT id, password_hash, avatar_hash FROM users WHERE username = ?")
            .bind(&req.username)
            .fetch_optional(&s.db)
            .await?;
    let Some((id, hash, avatar_hash)) = row else {
        let _ = password::verify_async(req.password, DUMMY_HASH.clone()).await;
        return Err(failed());
    };
    if !password::verify_async(req.password.clone(), hash).await {
        return Err(failed());
    }
    let id: UserId = id.parse().map_err(anyhow::Error::from)?;
    let token = session::create(&s.db, id).await?;
    Ok(Json(SessionResponse {
        token,
        user: User {
            id,
            username: req.username,
            avatar_hash,
        },
    }))
}

async fn logout(
    State(s): State<AppState>,
    _: AuthUser,
    BearerToken(t): BearerToken,
) -> AppResult<StatusCode> {
    session::revoke(&s.db, &t).await?;
    s.hub.drop_session(&session::hash_token(&t));
    Ok(StatusCode::NO_CONTENT)
}

async fn me(State(s): State<AppState>, AuthUser(u): AuthUser) -> AppResult<Json<User>> {
    Ok(Json(load_user(&s.db, u).await?))
}

async fn create_invite(
    State(s): State<AppState>,
    AuthUser(u): AuthUser,
) -> AppResult<Json<InviteResponse>> {
    Ok(Json(InviteResponse {
        code: invites::create(&s.db, Some(u)).await?,
    }))
}
