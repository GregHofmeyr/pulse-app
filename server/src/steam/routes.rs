//! Steam's routes (#379): your link (`/me/steam…`), Steam's OpenID return page, and each server's Achievements
//! channel.

use axum::extract::{Path, RawQuery, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use pulse_protocol::ids::{ServerId, UserId};
use pulse_protocol::rest::{
    AchievementsChannel, ChannelKind, SetAchievementsChannelRequest, SteamHours, SteamLinkStart,
    SteamSettingsRequest, SteamStatus,
};
use reqwest::Url;

use super::openid::{self, Refused};
use super::{backfill, store};
use crate::AppState;
use crate::access::load_channel;
use crate::auth::AuthUser;
use crate::error::{AppError, AppResult};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/me/steam", get(mine).delete(unlink))
        .route("/me/steam/link", post(start))
        .route("/me/steam/settings", put(settings))
        .route("/steam/openid/return", get(openid_return))
        .route(
            "/servers/{id}/achievements-channel",
            get(get_channel).put(set_channel),
        )
        .route("/servers/{id}/steam-hours", get(steam_hours))
}

/// `GET /servers/{id}/steam-hours` (#383): the members who played the most Steam hours in the last two weeks.
async fn steam_hours(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ServerId>,
) -> AppResult<Json<SteamHours>> {
    crate::servers::routes::require_member(&s.db, id, me).await?;
    Ok(Json(SteamHours {
        entries: super::hours::board(&s.db, id).await?,
    }))
}

async fn mine(State(s): State<AppState>, AuthUser(me): AuthUser) -> AppResult<Json<SteamStatus>> {
    Ok(Json(store::status(&s, me).await?))
}

/// `POST /me/steam/link`: a fresh one-time sign-in for the browser.
async fn start(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
) -> AppResult<Json<SteamLinkStart>> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    let Some(public) = s.cfg.embeds.public_url.as_ref() else {
        return Err(AppError::Unavailable(
            "Linking Steam isn't set up on this server (it needs PULSE_PUBLIC_URL).".into(),
        ));
    };
    let state = s.steam.new_state(me);
    let ret = openid::return_to(public, &state);
    let url = openid::login_url(&s.cfg.steam.openid_endpoint, public, &ret)
        .ok_or_else(|| anyhow::anyhow!("the Steam OpenID endpoint isn't a URL"))?;
    Ok(Json(SteamLinkStart { url }))
}

async fn unlink(State(s): State<AppState>, AuthUser(me): AuthUser) -> AppResult<Json<SteamStatus>> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    if store::unlink(&s.db, me).await? {
        s.steam.forget(me);
        store::tell(&s, me).await;
    }
    Ok(Json(store::status(&s, me).await?))
}

async fn settings(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Json(req): Json<SteamSettingsRequest>,
) -> AppResult<Json<SteamStatus>> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    if !store::set_switches(&s.db, me, &req).await? {
        return Err(AppError::NotFound);
    }
    match req.share_achievements {
        Some(true) => backfill::kick(&s),
        Some(false) => s.steam.forget(me),
        None => {}
    }
    if req.share_hours == Some(true) {
        super::hours::kick_one(&s, me);
    }
    if req.share_join == Some(false) && s.hub.set_steam_join(me, None) {
        s.hub.sync_presence(&s.db, me).await;
    }

    store::tell(&s, me).await;
    Ok(Json(store::status(&s, me).await?))
}

/// The little page the browser lands on after Steam.
fn page(title: &str, body: &str) -> Response {
    let html = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width\">\
         <title>{title}</title><style>body{{font:16px system-ui,sans-serif;background:#111318;color:#e8e9ee;\
         display:grid;place-items:center;min-height:90vh;margin:0}}main{{max-width:28rem;padding:1rem;text-align:center}}\
         p{{color:#a4a8b4}}</style></head><body><main><h1>{title}</h1><p>{body}</p></main></body></html>"
    );
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (
                header::CONTENT_SECURITY_POLICY,
                "default-src 'none'; style-src 'unsafe-inline'; frame-ancestors 'none'",
            ),
            (header::CACHE_CONTROL, "no-store"),
            (header::REFERRER_POLICY, "no-referrer"),
        ],
        html,
    )
        .into_response()
}

fn failed(body: &str) -> Response {
    page("Steam not linked", body)
}

/// `GET /steam/openid/return`: where Steam sends the browser. No session: the one-time state says whose sign-in this
/// is. Steam is asked to vouch for the assertion before anything is stored.
async fn openid_return(State(s): State<AppState>, RawQuery(q): RawQuery) -> Response {
    let Some(public) = s.cfg.embeds.public_url.clone() else {
        return failed("Linking Steam isn't set up on this server.");
    };
    let params: Vec<(String, String)> = Url::parse(&format!("http://x/?{}", q.unwrap_or_default()))
        .map(|u| u.query_pairs().into_owned().collect())
        .unwrap_or_default();
    let Some(state) = params
        .iter()
        .find(|(k, _)| k == "state")
        .map(|(_, v)| v.clone())
    else {
        return failed("This link is missing its sign-in. Start again from Pulse's Settings.");
    };
    let Some(user) = s.steam.take_state(&state) else {
        return failed(
            "This sign-in has expired or was already used. Start again from Pulse's Settings.",
        );
    };
    let expected = openid::return_to(&public, &state);
    let steam_id = match openid::verify(&params, &s.cfg.steam.openid_endpoint, &expected) {
        Ok(id) => id,
        Err(Refused::NotSignedIn) => {
            return failed("You didn't sign in to Steam. Start again from Pulse's Settings.");
        }
        Err(Refused::Mismatch | Refused::BadId) => {
            return failed("Steam's answer didn't check out. Start again from Pulse's Settings.");
        }
    };
    if !s.steam.api().check_authentication(&params).await {
        return failed("Steam didn't confirm the sign-in. Start again from Pulse's Settings.");
    }
    match finish(&s, user, &steam_id).await {
        Ok(store::Linked::Taken) => {
            failed("That Steam account is already linked to someone else on this server.")
        }
        Ok(_) => page(
            "Steam linked",
            "You can close this tab and go back to Pulse.",
        ),
        Err(e) => {
            tracing::warn!(error = ?e, "Steam link not stored");
            failed("Something went wrong on the Pulse server. Try again in a minute.")
        }
    }
}

async fn finish(s: &AppState, user: UserId, steam_id: &str) -> AppResult<store::Linked> {
    let persona = s.steam.api().persona(steam_id).await;
    let linked = store::link(&s.db, user, steam_id, persona.as_deref()).await?;
    if linked == store::Linked::New {
        s.steam.forget(user);
        backfill::kick(s);
        super::hours::kick_one(s, user);
    }
    if linked != store::Linked::Taken {
        store::tell(s, user).await;
    }
    Ok(linked)
}

async fn get_channel(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ServerId>,
) -> AppResult<Json<AchievementsChannel>> {
    crate::servers::routes::require_member(&s.db, id, me).await?;
    Ok(Json(AchievementsChannel {
        channel_id: channel_of(&s, id).await?,
    }))
}

async fn channel_of(
    s: &AppState,
    server: ServerId,
) -> AppResult<Option<pulse_protocol::ids::ChannelId>> {
    let id: Option<String> = sqlx::query_scalar(
        "SELECT a.channel_id FROM server_achievement_channels a
         JOIN channels c ON c.id = a.channel_id AND c.deleted_at IS NULL
         WHERE a.server_id = ?",
    )
    .bind(server.to_string())
    .fetch_optional(&s.db)
    .await?;
    Ok(id.and_then(|i| i.parse().ok()))
}

/// `PUT /servers/{id}/achievements-channel`: the server's owner picks one of its text channels, or none.
async fn set_channel(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ServerId>,
    Json(req): Json<SetAchievementsChannelRequest>,
) -> AppResult<Json<AchievementsChannel>> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    crate::servers::routes::require_member(&s.db, id, me).await?;
    let owner: String = sqlx::query_scalar("SELECT created_by FROM servers WHERE id = ?")
        .bind(id.to_string())
        .fetch_one(&s.db)
        .await?;
    if owner != me.to_string() {
        return Err(AppError::ForbiddenMsg(
            "Only the server's owner can pick its Achievements channel.".into(),
        ));
    }
    match req.channel_id {
        None => {
            sqlx::query("DELETE FROM server_achievement_channels WHERE server_id = ?")
                .bind(id.to_string())
                .execute(&s.db)
                .await?;
        }
        Some(ch) => {
            let channel = load_channel(&s.db, ch).await?.ok_or(AppError::NotFound)?;
            if channel.server_id != Some(id) || channel.kind != ChannelKind::Text {
                return Err(AppError::BadRequest(
                    "pick one of this server's text channels".into(),
                ));
            }
            sqlx::query(
                "INSERT INTO server_achievement_channels (server_id, channel_id) VALUES (?, ?)
                 ON CONFLICT (server_id) DO UPDATE SET channel_id = excluded.channel_id",
            )
            .bind(id.to_string())
            .bind(ch.to_string())
            .execute(&s.db)
            .await?;
        }
    }
    Ok(Json(AchievementsChannel {
        channel_id: channel_of(&s, id).await?,
    }))
}
