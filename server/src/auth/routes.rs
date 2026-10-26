use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::UserId;
use pulse_protocol::rest::{
    InviteResponse, LoginRequest, LogoutOthersResponse, RegisterRequest, SessionResponse, User,
};
use sqlx::SqlitePool;

use super::extractor::{AuthUser, BearerToken};
use super::{invites, password, session};
use crate::AppState;
use crate::db::now;
use crate::error::{AppError, AppResult, Ticket};
use crate::limits::ClientIp;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/auth/register", post(register))
        .route("/auth/login", post(login))
        .route("/auth/logout", post(logout))
        .route("/auth/logout-others", post(logout_others))
        .route("/me", get(me))
        .route("/invites", post(create_invite))
}

/// Names are stored lowercase; any case (and stray spaces) typed at register or login maps to it.
pub(crate) fn normalize_username(u: &str) -> String {
    u.trim().to_ascii_lowercase()
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
    // Spend up front so parallel guesses can't all slip past before any pays. Only a wrong,
    // used or expired invite code (guessing) keeps the spend; every other outcome is refunded.
    let ip_key = crate::limits::rate_key(ip).to_string();
    if !s.limits.register_ip.hit(&ip_key) {
        return Err(AppError::TooManyRequests);
    }
    let res = register_with(&s, req).await;
    if !matches!(res, Err(AppError::NotFound | AppError::Gone)) {
        s.limits.register_ip.refund(&ip_key);
    }
    res
}

async fn register_with(s: &AppState, mut req: RegisterRequest) -> AppResult<Json<SessionResponse>> {
    req.username = normalize_username(&req.username);
    if !valid_username(&req.username) {
        return Err(AppError::BadRequest(
            "username must be 2-32 letters, numbers, _ or .".into(),
        ));
    }
    if !(8..=256).contains(&req.password.len()) {
        return Err(AppError::BadRequest(
            "password must be at least 8 characters".into(),
        ));
    }
    // Check the invite before anything expensive (and before revealing whether a username exists).
    let cutoff = invites::cutoff(chrono::Utc::now());
    let found: Option<(Option<String>, String)> =
        sqlx::query_as("SELECT used_by, created_at FROM invites WHERE code = ?")
            .bind(&req.invite_code)
            .fetch_optional(&s.db)
            .await?;
    match found {
        None => return Err(AppError::NotFound),
        // Expired looks exactly like unknown.
        Some((_, created)) if created <= cutoff => return Err(AppError::NotFound),
        Some((Some(_), _)) => return Err(AppError::Gone),
        Some((None, _)) => {}
    }
    let hash = password::hash_async(req.password.clone()).await?;
    let id = UserId::new();
    let at = now();

    let mut tx = crate::db::begin_write(&s.db).await?;
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
        "UPDATE invites SET used_by = ?, used_at = ? WHERE code = ? AND used_by IS NULL AND created_at > ?",
    )
    .bind(id.to_string())
    .bind(&at)
    .bind(&req.invite_code)
    .bind(&cutoff)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if claimed == 0 {
        tx.rollback().await?;
        let exists: Option<i64> = sqlx::query_scalar("SELECT 1 FROM invites WHERE code = ?")
            .bind(&req.invite_code)
            .fetch_optional(&s.db)
            .await?;
        return Err(if exists.is_some() {
            AppError::Gone
        } else {
            AppError::NotFound
        });
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
    Json(mut req): Json<LoginRequest>,
) -> AppResult<Json<SessionResponse>> {
    let ip_key = crate::limits::rate_key(ip).to_string();
    req.username = normalize_username(&req.username);
    // A name that can't exist gets no bucket of its own (keys stay small); the address still pays.
    let user_key = Some(req.username.clone()).filter(|u| valid_username(u));
    // Spend up front so parallel guesses can't all slip past before any pays. Only a failed
    // login keeps the spend: friends logging in together from one network are fine.
    let ip_ok = s.limits.login_ip.hit(&ip_key);
    let user_ok = user_key
        .as_deref()
        .is_none_or(|u| s.limits.login_user.hit(u));
    let refund = |ip: bool, user: bool| {
        if ip {
            s.limits.login_ip.refund(&ip_key);
        }
        if let Some(u) = user_key.as_deref().filter(|_| user) {
            s.limits.login_user.refund(u);
        }
    };
    if !ip_ok || !user_ok {
        refund(ip_ok, user_ok); // a refused attempt costs nothing
        return Err(AppError::TooManyRequests);
    }
    let res = check_login(&s, req).await;
    if !matches!(res, Err(AppError::Unauthorized)) {
        refund(true, true);
    }
    res
}

async fn check_login(s: &AppState, req: LoginRequest) -> AppResult<Json<SessionResponse>> {
    let row: Option<(String, String, Option<String>)> =
        sqlx::query_as("SELECT id, password_hash, avatar_hash FROM users WHERE username = ?")
            .bind(&req.username)
            .fetch_optional(&s.db)
            .await?;
    let Some((id, hash, avatar_hash)) = row else {
        let _ = password::verify_async(req.password, DUMMY_HASH.clone()).await;
        return Err(AppError::Unauthorized);
    };
    if !password::verify_async(req.password.clone(), hash).await {
        return Err(AppError::Unauthorized);
    }
    let id: UserId = id.parse().map_err(anyhow::Error::from)?;
    let two_factor_on: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM user_totp WHERE user_id = ? AND enabled_at IS NOT NULL")
            .bind(id.to_string())
            .fetch_optional(&s.db)
            .await?;
    if two_factor_on.is_some() {
        return Err(AppError::MfaRequired(Ticket(
            super::two_factor::issue_ticket(&s.db, id).await?,
        )));
    }
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
    let grants = session::revoke(&s.db, &t).await?;
    s.hub.drop_session(&session::hash_token(&t));
    crate::voice::revoke::remove_revoked(&s, &grants).await;
    Ok(StatusCode::NO_CONTENT)
}

/// Sign out every other device: revoke the caller's other sessions and close their sockets
/// (4001, so those clients drop to the login screen), then remove the LiveKit participants they
/// admitted. This device stays signed in, calls included. Returns how many live sessions went.
pub(super) async fn sign_out_others(s: &AppState, user: UserId, token: &str) -> AppResult<u32> {
    let revoked = session::revoke_others(&s.db, user, token).await?;
    Ok(after_revoke_others(s, user, token, revoked).await)
}

/// The rest of signing out other devices once their sessions are deleted (committed): close
/// their sockets and remove their LiveKit participants, both best effort. Returns how many live
/// sessions went.
pub(super) async fn after_revoke_others(
    s: &AppState,
    user: UserId,
    token: &str,
    revoked: session::Revoked,
) -> u32 {
    s.hub.drop_other_sessions(user, &session::hash_token(token));
    crate::voice::revoke::remove_revoked(s, &revoked.grants).await;
    u32::try_from(revoked.live).unwrap_or(u32::MAX)
}

async fn logout_others(
    State(s): State<AppState>,
    AuthUser(u): AuthUser,
    BearerToken(t): BearerToken,
) -> AppResult<Json<LogoutOthersResponse>> {
    Ok(Json(LogoutOthersResponse {
        revoked: sign_out_others(&s, u, &t).await?,
    }))
}

async fn me(State(s): State<AppState>, AuthUser(u): AuthUser) -> AppResult<Json<User>> {
    Ok(Json(load_user(&s.db, u).await?))
}

async fn create_invite(
    State(s): State<AppState>,
    AuthUser(u): AuthUser,
) -> AppResult<Json<InviteResponse>> {
    if !s.limits.invite_user.hit(&u.to_string()) {
        return Err(AppError::TooManyRequests);
    }
    Ok(Json(InviteResponse {
        code: invites::create(&s.db, Some(u)).await?,
    }))
}
