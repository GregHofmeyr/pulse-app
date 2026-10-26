//! Two-factor enrolment, status, turning off and recovery-code regeneration, plus the pieces
//! the login ticket step reuses: the 2FA rate limit and the atomic "spend this code" checks.

use std::future::Future;
use std::net::IpAddr;

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use pulse_protocol::ids::UserId;
use pulse_protocol::rest::{
    LoginMfaRequest, RecoveryCodesResponse, SessionResponse, TwoFactorCodeRequest,
    TwoFactorEnabledResponse, TwoFactorSetupRequest, TwoFactorSetupResponse, TwoFactorStatus, User,
};
use rand::RngCore;
use sha2::{Digest, Sha256};
use sqlx::{SqliteConnection, SqlitePool};

use super::extractor::{AuthUser, BearerToken};
use super::{password, session, totp};
use crate::AppState;
use crate::db::now;
use crate::error::{AppError, AppResult};
use crate::limits::ClientIp;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/auth/login/2fa", post(login_second_step))
        .route("/auth/2fa", get(status))
        .route("/auth/2fa/setup", post(setup))
        .route("/auth/2fa/enable", post(enable))
        .route("/auth/2fa/disable", post(disable))
        .route("/auth/2fa/recovery-codes", post(regenerate))
}

/// Run one 2FA check under the `mfa_ip` and `mfa_user` limits. Both are spent up front so
/// parallel guesses can't all slip past before any pays; if either is out, the other is given
/// back and the answer is 429 (a refused attempt costs nothing). Only a wrong password or code
/// keeps the spend; every other outcome is refunded.
pub async fn limited<T>(
    s: &AppState,
    ip: IpAddr,
    user: UserId,
    check: impl Future<Output = AppResult<T>>,
) -> AppResult<T> {
    limited_by(s, ip, Some(user), is_wrong_code, check).await
}

fn is_wrong_code(e: &AppError) -> bool {
    matches!(e, AppError::ForbiddenMsg(_) | AppError::Unauthorized)
}

/// `limited` with the user bucket optional (no user is known for a ticket that doesn't exist)
/// and the "keeps the spend" outcomes chosen by the caller.
pub async fn limited_by<T>(
    s: &AppState,
    ip: IpAddr,
    user: Option<UserId>,
    keeps_spend: fn(&AppError) -> bool,
    check: impl Future<Output = AppResult<T>>,
) -> AppResult<T> {
    let ip_key = ip.to_string();
    let user_key = user.map(|u| u.to_string());
    let ip_ok = s.limits.mfa_ip.hit(&ip_key);
    let user_ok = user_key.as_deref().is_none_or(|u| s.limits.mfa_user.hit(u));
    let refund = |ip: bool, user: bool| {
        if ip {
            s.limits.mfa_ip.refund(&ip_key);
        }
        if let Some(u) = user_key.as_deref().filter(|_| user) {
            s.limits.mfa_user.refund(u);
        }
    };
    if !ip_ok || !user_ok {
        refund(ip_ok, user_ok);
        return Err(AppError::TooManyRequests);
    }
    let res = check.await;
    if !matches!(&res, Err(e) if keeps_spend(e)) {
        refund(true, true);
    }
    res
}

/// Whether `code` is the user's current TOTP code, and if so spend its step. The step must be
/// newer than the last one used; the `UPDATE ... WHERE last_step < ?` is what makes that atomic,
/// so of two racing requests with the same code only one gets a row back. Works on a pending
/// secret too (enrolment). Call it inside a write transaction when more writes depend on it.
pub async fn accept_totp(
    conn: &mut SqliteConnection,
    key: &totp::TotpKey,
    user: UserId,
    code: &str,
) -> AppResult<bool> {
    let row: Option<(Vec<u8>,)> =
        sqlx::query_as("SELECT secret_enc FROM user_totp WHERE user_id = ?")
            .bind(user.to_string())
            .fetch_optional(&mut *conn)
            .await?;
    let Some((blob,)) = row else {
        return Ok(false);
    };
    let secret = totp::decrypt(key, user, &blob)?;
    let now_unix = u64::try_from(chrono::Utc::now().timestamp()).unwrap_or(0);
    let Some(step) = totp::match_step(&secret, code, now_unix) else {
        return Ok(false);
    };
    let n = sqlx::query("UPDATE user_totp SET last_step = ? WHERE user_id = ? AND last_step < ?")
        .bind(step)
        .bind(user.to_string())
        .bind(step)
        .execute(&mut *conn)
        .await?
        .rows_affected();
    Ok(n == 1)
}

/// Spend a recovery code: the atomic delete must hit exactly one row.
pub async fn consume_recovery(
    conn: &mut SqliteConnection,
    user: UserId,
    code: &str,
) -> AppResult<bool> {
    let n = sqlx::query("DELETE FROM recovery_codes WHERE user_id = ? AND code_hash = ?")
        .bind(user.to_string())
        .bind(totp::hash_recovery(code))
        .execute(&mut *conn)
        .await?
        .rows_affected();
    Ok(n == 1)
}

/// Replace all of the user's recovery codes with ten fresh ones; returns them for display.
async fn replace_recovery_codes(
    conn: &mut SqliteConnection,
    user: UserId,
) -> AppResult<Vec<String>> {
    sqlx::query("DELETE FROM recovery_codes WHERE user_id = ?")
        .bind(user.to_string())
        .execute(&mut *conn)
        .await?;
    let codes = totp::new_recovery_codes();
    for c in &codes {
        sqlx::query("INSERT INTO recovery_codes (user_id, code_hash) VALUES (?, ?)")
            .bind(user.to_string())
            .bind(totp::hash_recovery(c))
            .execute(&mut *conn)
            .await?;
    }
    Ok(codes)
}

fn wrong_code() -> AppError {
    AppError::ForbiddenMsg("wrong code".into())
}

fn key(s: &AppState) -> AppResult<&totp::TotpKey> {
    s.cfg.totp_key.as_ref().ok_or_else(|| {
        AppError::ForbiddenMsg("two-factor authentication isn't available on this server".into())
    })
}

/// `Some(enabled)` when the user has a row (pending or on), `None` when they have none.
async fn state(conn: &mut SqliteConnection, user: UserId) -> AppResult<Option<bool>> {
    let row: Option<(Option<String>,)> =
        sqlx::query_as("SELECT enabled_at FROM user_totp WHERE user_id = ?")
            .bind(user.to_string())
            .fetch_optional(&mut *conn)
            .await?;
    Ok(row.map(|(at,)| at.is_some()))
}

async fn status(
    State(s): State<AppState>,
    AuthUser(u): AuthUser,
) -> AppResult<Json<TwoFactorStatus>> {
    let mut conn = s.db.acquire().await?;
    let enabled = state(&mut conn, u).await? == Some(true);
    let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM recovery_codes WHERE user_id = ?")
        .bind(u.to_string())
        .fetch_one(&mut *conn)
        .await?;
    Ok(Json(TwoFactorStatus {
        enabled,
        available: s.cfg.totp_key.is_some(),
        recovery_codes_left: if enabled {
            u32::try_from(left).unwrap_or(u32::MAX)
        } else {
            0
        },
    }))
}

async fn setup(
    State(s): State<AppState>,
    ClientIp(ip): ClientIp,
    AuthUser(u): AuthUser,
    Json(req): Json<TwoFactorSetupRequest>,
) -> AppResult<Json<TwoFactorSetupResponse>> {
    let key = key(&s)?;
    limited(&s, ip, u, async {
        let (name, hash): (String, String) =
            sqlx::query_as("SELECT username, password_hash FROM users WHERE id = ?")
                .bind(u.to_string())
                .fetch_optional(&s.db)
                .await?
                .ok_or(AppError::NotFound)?;
        if !password::verify_async(req.password, hash).await {
            return Err(AppError::ForbiddenMsg("wrong password".into()));
        }
        let secret = totp::new_secret();
        let mut tx = crate::db::begin_write(&s.db).await?;
        if state(&mut tx, u).await? == Some(true) {
            return Err(AppError::Conflict(
                "two-factor authentication is already on".into(),
            ));
        }
        // A pending setup is replaced, so starting over gives a fresh secret.
        sqlx::query(
            "INSERT INTO user_totp (user_id, secret_enc, enabled_at, last_step, created_at)
             VALUES (?, ?, NULL, -1, ?)
             ON CONFLICT(user_id) DO UPDATE SET secret_enc = excluded.secret_enc,
                 last_step = -1, created_at = excluded.created_at",
        )
        .bind(u.to_string())
        .bind(totp::encrypt(key, u, &secret))
        .bind(now())
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(Json(TwoFactorSetupResponse {
            secret: totp::secret_base32(&secret),
            otpauth_uri: totp::otpauth_uri(&name, &secret),
        }))
    })
    .await
}

async fn enable(
    State(s): State<AppState>,
    ClientIp(ip): ClientIp,
    AuthUser(u): AuthUser,
    BearerToken(t): BearerToken,
    Json(req): Json<TwoFactorCodeRequest>,
) -> AppResult<Json<TwoFactorEnabledResponse>> {
    let key = key(&s)?;
    let recovery_codes = limited(&s, ip, u, async {
        let mut tx = crate::db::begin_write(&s.db).await?;
        match state(&mut tx, u).await? {
            None => return Err(AppError::Conflict("no two-factor setup in progress".into())),
            Some(true) => {
                return Err(AppError::Conflict(
                    "two-factor authentication is already on".into(),
                ));
            }
            Some(false) => {}
        }
        // Recovery codes make no sense before the first code has proved the app works.
        if !totp::is_totp_shaped(&req.code) || !accept_totp(&mut tx, key, u, &req.code).await? {
            return Err(wrong_code());
        }
        sqlx::query("UPDATE user_totp SET enabled_at = ? WHERE user_id = ?")
            .bind(now())
            .bind(u.to_string())
            .execute(&mut *tx)
            .await?;
        let codes = replace_recovery_codes(&mut tx, u).await?;
        tx.commit().await?;
        Ok(codes)
    })
    .await?;
    // Whoever had the password but not the app is out; this device stays.
    let signed_out = super::routes::sign_out_others(&s, u, &t).await?;
    Ok(Json(TwoFactorEnabledResponse {
        recovery_codes,
        signed_out,
    }))
}

async fn disable(
    State(s): State<AppState>,
    ClientIp(ip): ClientIp,
    AuthUser(u): AuthUser,
    Json(req): Json<TwoFactorCodeRequest>,
) -> AppResult<StatusCode> {
    let key = key(&s)?;
    limited(&s, ip, u, async {
        let mut tx = crate::db::begin_write(&s.db).await?;
        if state(&mut tx, u).await? != Some(true) {
            return Err(AppError::Conflict(
                "two-factor authentication is not on".into(),
            ));
        }
        let ok = if totp::is_totp_shaped(&req.code) {
            accept_totp(&mut tx, key, u, &req.code).await?
        } else {
            consume_recovery(&mut tx, u, &req.code).await?
        };
        if !ok {
            return Err(wrong_code());
        }
        for table in ["user_totp", "recovery_codes", "mfa_tickets"] {
            sqlx::query(&format!("DELETE FROM {table} WHERE user_id = ?"))
                .bind(u.to_string())
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(StatusCode::NO_CONTENT)
    })
    .await
}

async fn regenerate(
    State(s): State<AppState>,
    ClientIp(ip): ClientIp,
    AuthUser(u): AuthUser,
    Json(req): Json<TwoFactorCodeRequest>,
) -> AppResult<Json<RecoveryCodesResponse>> {
    let key = key(&s)?;
    limited(&s, ip, u, async {
        let mut tx = crate::db::begin_write(&s.db).await?;
        if state(&mut tx, u).await? != Some(true) {
            return Err(AppError::Conflict(
                "two-factor authentication is not on".into(),
            ));
        }
        if !totp::is_totp_shaped(&req.code) || !accept_totp(&mut tx, key, u, &req.code).await? {
            return Err(wrong_code());
        }
        let recovery_codes = replace_recovery_codes(&mut tx, u).await?;
        tx.commit().await?;
        Ok(Json(RecoveryCodesResponse { recovery_codes }))
    })
    .await
}

/// How long a login ticket lives after the password step.
const TICKET_MINUTES: i64 = 5;
/// Wrong codes a ticket survives; the fifth deletes it.
const TICKET_MAX_FAILURES: i64 = 5;

fn hash_ticket(ticket: &str) -> String {
    hex::encode(Sha256::digest(ticket.as_bytes()))
}

/// Start a login ticket for `user` (the password was right): returns the secret half, which is
/// only ever sent to the client. Expired tickets are pruned on the way.
pub async fn issue_ticket(db: &SqlitePool, user: UserId) -> AppResult<String> {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    let ticket = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
    let expires = chrono::Utc::now() + chrono::Duration::minutes(TICKET_MINUTES);
    sqlx::query("DELETE FROM mfa_tickets WHERE expires_at <= ?")
        .bind(now())
        .execute(db)
        .await?;
    sqlx::query("INSERT INTO mfa_tickets (ticket_hash, user_id, expires_at) VALUES (?, ?, ?)")
        .bind(hash_ticket(&ticket))
        .bind(user.to_string())
        .bind(expires.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        .execute(db)
        .await?;
    Ok(ticket)
}

/// The ticket's user, if it exists, is unexpired and has failures left.
async fn live_ticket(db: impl sqlx::SqliteExecutor<'_>, hash: &str) -> AppResult<Option<UserId>> {
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT user_id FROM mfa_tickets WHERE ticket_hash = ? AND expires_at > ? AND failures < ?",
    )
    .bind(hash)
    .bind(now())
    .bind(TICKET_MAX_FAILURES)
    .fetch_optional(db)
    .await?;
    row.map(|(u,)| u.parse().map_err(anyhow::Error::from).map_err(Into::into))
        .transpose()
}

/// `POST /auth/login/2fa`: trade a ticket and a code for a session. A ticket that is unknown,
/// expired or worn out is 410 (and the address pays, it's a guess); a known ticket is checked
/// under both limits and a wrong code (401) keeps the spend.
async fn login_second_step(
    State(s): State<AppState>,
    ClientIp(ip): ClientIp,
    Json(req): Json<LoginMfaRequest>,
) -> AppResult<Json<SessionResponse>> {
    let key = key(&s)?;
    let hash = hash_ticket(&req.ticket);
    let user = live_ticket(&s.db, &hash).await?;
    let Some(user) = user else {
        return limited_by(&s, ip, None, |e| matches!(e, AppError::Gone), async {
            Err(AppError::Gone)
        })
        .await;
    };
    // A ticket that dies between the read above and the write lock (used or burnt by a racing
    // request) is still a 410 guess, so it keeps the spend like a wrong code does.
    let keeps = |e: &AppError| matches!(e, AppError::Unauthorized | AppError::Gone);
    limited_by(&s, ip, Some(user), keeps, async {
        let mut tx = crate::db::begin_write(&s.db).await?;
        // Re-read under the write lock: a racing request may have used or burnt the ticket.
        if live_ticket(&mut *tx, &hash).await? != Some(user) {
            return Err(AppError::Gone);
        }
        let ok = if totp::is_totp_shaped(&req.code) {
            accept_totp(&mut tx, key, user, &req.code).await?
        } else {
            consume_recovery(&mut tx, user, &req.code).await?
        };
        if !ok {
            // The code checks write nothing on failure, so counting the miss in this same
            // transaction keeps the limit atomic: racing wrong guesses can't all see failures < 5.
            sqlx::query("UPDATE mfa_tickets SET failures = failures + 1 WHERE ticket_hash = ?")
                .bind(&hash)
                .execute(&mut *tx)
                .await?;
            sqlx::query("DELETE FROM mfa_tickets WHERE ticket_hash = ? AND failures >= ?")
                .bind(&hash)
                .bind(TICKET_MAX_FAILURES)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            return Err(AppError::Unauthorized);
        }
        let gone = sqlx::query("DELETE FROM mfa_tickets WHERE ticket_hash = ?")
            .bind(&hash)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        if gone != 1 {
            return Err(AppError::Gone);
        }
        tx.commit().await?;
        let token = session::create(&s.db, user).await?;
        let (username, avatar_hash): (String, Option<String>) =
            sqlx::query_as("SELECT username, avatar_hash FROM users WHERE id = ?")
                .bind(user.to_string())
                .fetch_one(&s.db)
                .await?;
        Ok(Json(SessionResponse {
            token,
            user: User {
                id: user,
                username,
                avatar_hash,
            },
        }))
    })
    .await
}
