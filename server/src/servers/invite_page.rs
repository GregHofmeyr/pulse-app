//! The public side of invites (#80, step 3), for people who don't have Pulse yet.
//!
//! `GET /i/{code}` is a plain server-rendered page: what an invite card shows a stranger (the server's name, icon and
//! counts, who invited them, the join text), "Open in Pulse" (`pulse://invite/<code>`) and "Get Pulse for Windows".
//! A code that doesn't let anyone in right now (unknown, revoked, expired, used up, or its server deleted) gets one
//! and the same page, so a guessed code is never confirmed. Strict CSP (no script at all), `noindex`, no referrer, no
//! caching. Lookups go through the same guard, limits and failure log as the app's (`invites::guard`).
//!
//! `GET /download/windows` serves the latest signed setup `.exe` that `just publish-update` put in the updates dir
//! (the version `latest.json` names), rate limited per address. `GET /robots.txt` keeps crawlers off everything.

use axum::Router;
use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use sha2::{Digest, Sha256};
use std::sync::LazyLock;
use tower::ServiceExt;
use tower_http::services::ServeFile;

use super::invites::{Failure, PublicInvite, Seeker, guard, log_failure, public_invite};
use crate::AppState;
use crate::embeds::page::escape;
use crate::error::AppError;
use crate::limits::{ClientIp, rate_key};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/i/{code}", get(page))
        .route("/download/windows", get(download))
        .route("/robots.txt", get(robots))
}

const STYLE: &str = concat!(
    ":root{color-scheme:dark}*{box-sizing:border-box}",
    "body{margin:0;min-height:100vh;display:flex;align-items:center;justify-content:center;padding:16px;",
    "background:radial-gradient(circle at 50% 0,#2a2340,#121018 60%);color:#ece9f5;",
    "font:16px/1.45 system-ui,-apple-system,'Segoe UI',sans-serif}",
    "main{width:100%;max-width:400px;padding:32px 28px;border-radius:20px;background:#1c1928;",
    "box-shadow:0 20px 60px #0008;text-align:center}",
    ".icon{width:88px;height:88px;margin:0 auto 16px;border-radius:24px;display:flex;align-items:center;",
    "justify-content:center;background:#7c5cff;font-size:40px;font-weight:700;overflow:hidden}",
    ".icon img{width:100%;height:100%;object-fit:cover}",
    ".from{margin:0;color:#a9a3bd;font-size:14px}h1{margin:4px 0 6px;font-size:26px;line-height:1.2;",
    "overflow-wrap:anywhere}.counts{margin:0 0 24px;color:#a9a3bd;font-size:14px}",
    ".btn{display:block;width:100%;margin:10px 0 0;padding:12px 16px;border:0;border-radius:12px;font:inherit;",
    "font-weight:600;text-decoration:none;cursor:pointer}.primary{background:#7c5cff;color:#fff}",
    ".secondary{background:#2c2840;color:#ece9f5}.hint{margin:18px 0 0;color:#8b859e;font-size:13px}",
    ".hp{position:absolute;left:-10000px;width:1px;height:1px;opacity:0}"
);

/// The CSP source that allows [`STYLE`] and nothing else.
static STYLE_HASH: LazyLock<String> = LazyLock::new(|| {
    use base64::Engine;
    format!(
        "'sha256-{}'",
        base64::engine::general_purpose::STANDARD.encode(Sha256::digest(STYLE.as_bytes()))
    )
});

/// A page with the headers every public page here carries.
fn html(status: StatusCode, title: &str, body: &str) -> Response {
    let doc = format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <meta name=\"robots\" content=\"noindex, nofollow\">\n<title>{}</title>\n<style>{STYLE}</style>\n\
         </head>\n<body><main>{body}</main></body>\n</html>\n",
        escape(title)
    );
    let csp = format!(
        "default-src 'none'; img-src 'self'; style-src {}; form-action 'self'; base-uri 'none'; frame-ancestors 'none'",
        *STYLE_HASH
    );
    let mut res = (status, doc).into_response();
    let h = res.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_str(&csp).expect("ascii"),
    );
    h.insert(
        "x-robots-tag",
        HeaderValue::from_static("noindex, nofollow"),
    );
    h.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    res
}

/// The download button, as a form so the page needs no script.
const DOWNLOAD: &str = "<form method=\"get\" action=\"/download/windows\">\
     <input class=\"hp\" type=\"text\" name=\"website\" value=\"\" tabindex=\"-1\" autocomplete=\"off\" aria-hidden=\"true\">\
     <button class=\"btn secondary\" type=\"submit\">Get Pulse for Windows</button></form>";

/// What a code that doesn't work shows: the same for every reason.
fn dead() -> Response {
    html(
        StatusCode::NOT_FOUND,
        "Invite not found · Pulse",
        &format!(
            "<h1>This invite doesn't work</h1>\
             <p class=\"counts\">It may have expired or been used up. Ask whoever sent it for a new one.</p>{DOWNLOAD}"
        ),
    )
}

fn busy() -> Response {
    html(
        StatusCode::TOO_MANY_REQUESTS,
        "Slow down · Pulse",
        "<h1>Too many tries</h1><p class=\"counts\">Try again later.</p>",
    )
}

fn live(code: &str, inv: &PublicInvite) -> Response {
    let name = escape(&inv.name);
    let icon = match &inv.icon_hash {
        Some(h) => format!("<img src=\"/media/{}\" alt=\"\">", escape(h)),
        None => escape(&inv.name.chars().next().unwrap_or('?').to_string()),
    };
    let button = escape(inv.join_text.as_deref().unwrap_or("Open in Pulse"));
    let people = |n: i64, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    html(
        StatusCode::OK,
        &format!("Join {} on Pulse", inv.name),
        &format!(
            "<div class=\"icon\">{icon}</div>\
             <p class=\"from\">{} invited you to</p><h1>{name}</h1>\
             <p class=\"counts\">{} online · {}</p>\
             <a class=\"btn primary\" href=\"pulse://invite/{}\">{button}</a>{DOWNLOAD}\
             <p class=\"hint\">{}</p>",
            escape(&inv.inviter_name),
            inv.online_count,
            people(inv.member_count, "member", "members"),
            escape(code),
            if inv.signup_ok {
                "New to Pulse? Install it, then sign up with this invite link."
            } else {
                "Already on Pulse? Open it to join."
            },
        ),
    )
}

/// Whether `path` is the configured decoy: a hit logs a trap, which blocks that address for a while.
async fn caught(s: &AppState, ip: std::net::IpAddr, headers: &HeaderMap, path: &str) -> bool {
    if path != s.cfg.invite_decoy {
        return false;
    }
    let who = Seeker::anyone(None, ip, headers);
    log_failure(&s.db, &who, path, Failure::Trap).await;
    tracing::warn!("invite decoy hit");
    true
}

/// `GET /i/{code}`: the invite page.
async fn page(
    State(s): State<AppState>,
    ClientIp(ip): ClientIp,
    headers: HeaderMap,
    Path(code): Path<String>,
) -> Response {
    if caught(&s, ip, &headers, &format!("/i/{code}")).await {
        return dead();
    }
    let who = Seeker::anyone(None, ip, &headers);
    match guard(&s, &who).await {
        Ok(()) => {}
        Err(AppError::TooManyRequests(_)) => return busy(),
        Err(e) => return e.into_response(),
    }
    match public_invite(&s, &who, &code).await {
        Ok(Some(inv)) => live(&code, &inv),
        Ok(None) => dead(),
        Err(e) => e.into_response(),
    }
}

/// Every other path: 404, after checking it isn't the decoy.
pub async fn fallback(
    State(s): State<AppState>,
    ClientIp(ip): ClientIp,
    headers: HeaderMap,
    uri: axum::http::Uri,
) -> AppError {
    caught(&s, ip, &headers, uri.path()).await;
    AppError::NotFound
}

#[derive(serde::Deserialize)]
struct DownloadQuery {
    #[serde(default)]
    website: Option<String>,
}

/// `GET /download/windows`: the latest signed setup `.exe`, rate limited per address. 404 until one is published.
async fn download(
    State(s): State<AppState>,
    ClientIp(ip): ClientIp,
    headers: HeaderMap,
    Query(q): Query<DownloadQuery>,
    req: Request,
) -> Response {
    let ip_key = rate_key(ip).to_string();
    match super::invites::lookups_blocked(&s.db, &ip_key, None, chrono::Utc::now()).await {
        Ok(false) => {}
        Ok(true) => return AppError::TooManyRequests(None).into_response(),
        Err(e) => return e.into_response(),
    }
    if q.website.as_deref().is_some_and(|w| !w.trim().is_empty()) {
        let who = Seeker::anyone(None, ip, &headers);
        log_failure(&s.db, &who, "download", Failure::Trap).await;
        return AppError::NotFound.into_response();
    }
    if !s.limits.download_ip.hit(&ip_key) {
        return AppError::TooManyRequests(None).into_response();
    }
    let Some(name) = latest_installer(&s.cfg.updates_dir).await else {
        return AppError::NotFound.into_response();
    };
    // Streamed from disk, with range requests, as the updater's download is.
    let Ok(res) = ServeFile::new(s.cfg.updates_dir.join(&name))
        .oneshot(req)
        .await;
    let mut res = res.into_response();
    let h = res.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    h.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("attachment; filename=\"{name}\"")).expect("checked name"),
    );
    h.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=300"),
    );
    res
}

/// The installer `latest.json` names, if it's there: `Pulse_<version>_x64-setup.exe`.
pub async fn latest_installer(dir: &std::path::Path) -> Option<String> {
    let bytes = tokio::fs::read(dir.join("latest.json")).await.ok()?;
    let manifest: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let version = manifest.get("version")?.as_str()?;
    let name = format!("Pulse_{}_x64-setup.exe", version.trim_start_matches('v'));
    if !crate::updates::installer_name_ok(&name) {
        return None;
    }
    tokio::fs::try_exists(dir.join(&name))
        .await
        .ok()
        .filter(|e| *e)
        .map(|_| name)
}

/// `GET /robots.txt`: nothing here is for crawlers.
async fn robots() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        "User-agent: *\nDisallow: /\n",
    )
}
