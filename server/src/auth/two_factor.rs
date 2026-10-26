//! Two-factor enrolment, status, turning off and recovery-code regeneration, plus the pieces
//! the login ticket step reuses: the 2FA rate limit and the atomic "spend this code" checks.

use std::future::Future;
use std::net::IpAddr;

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use pulse_protocol::ids::UserId;
use pulse_protocol::rest::{
    RecoveryCodesResponse, TwoFactorCodeRequest, TwoFactorEnabledResponse, TwoFactorSetupRequest,
    TwoFactorSetupResponse, TwoFactorStatus,
};
use sqlx::SqliteConnection;

use super::extractor::{AuthUser, BearerToken};
use super::{password, totp};
use crate::AppState;
use crate::db::now;
use crate::error::{AppError, AppResult};
use crate::limits::ClientIp;

pub fn router() -> Router<AppState> {
    Router::new()
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
    let (ip_key, user_key) = (ip.to_string(), user.to_string());
    let ip_ok = s.limits.mfa_ip.hit(&ip_key);
    let user_ok = s.limits.mfa_user.hit(&user_key);
    if !ip_ok || !user_ok {
        if ip_ok {
            s.limits.mfa_ip.refund(&ip_key);
        }
        if user_ok {
            s.limits.mfa_user.refund(&user_key);
        }
        return Err(AppError::TooManyRequests);
    }
    let res = check.await;
    if !matches!(res, Err(AppError::ForbiddenMsg(_) | AppError::Unauthorized)) {
        s.limits.mfa_ip.refund(&ip_key);
        s.limits.mfa_user.refund(&user_key);
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
