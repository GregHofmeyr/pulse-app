//! Team logos (#406): API-Sports' `media.api-sports.io/{football|rugby}/teams/{id}.png` (no key, no requests spent),
//! fetched by the server only from [`crate::config::SportsConfig::logo_hosts`], through the guarded fetcher,
//! re-encoded like a link thumbnail and remembered by URL (`sports_media`, kept from the media sweep), as Deadlock's
//! art is. Clients get media hashes and never reach API-Sports. A logo that couldn't be had isn't asked for again for
//! [`MISS_RETRY`].

use std::time::Duration;

use pulse_protocol::ids::UserId;
use pulse_protocol::rest::{EmbedThumb, SportsLeague};
use reqwest::Url;

use crate::AppState;

/// How long a logo that couldn't be fetched is left alone.
pub const MISS_RETRY: Duration = Duration::from_secs(3600);

/// The logo URL for `team` in `league`, under `base`.
pub fn url(base: &str, league: SportsLeague, team: u32) -> String {
    let sport = if league.rugby() { "rugby" } else { "football" };
    format!("{}/{sport}/teams/{team}.png", base.trim_end_matches('/'))
}

/// Whether a logo may be fetched from `url`: http(s), on an allowed host.
pub fn allowed(hosts: &[String], url: &str) -> bool {
    let Ok(u) = Url::parse(url) else {
        return false;
    };
    matches!(u.scheme(), "https" | "http")
        && u.host_str()
            .is_some_and(|h| hosts.iter().any(|a| a.eq_ignore_ascii_case(h)))
}

/// `team`'s logo as the server's copy: remembered by URL, else (with `by` to charge it to) fetched.
pub async fn logo(
    s: &AppState,
    by: Option<UserId>,
    league: SportsLeague,
    team: u32,
) -> Option<EmbedThumb> {
    let cfg = &s.cfg.sports;
    let url = url(&cfg.logo_base, league, team);
    if !allowed(&cfg.logo_hosts, &url) {
        tracing::debug!("a team logo from a host that isn't allowed: skipped");
        return None;
    }
    let known: Option<(String, i64, i64)> =
        sqlx::query_as("SELECT hash, width, height FROM sports_media WHERE url = ?")
            .bind(&url)
            .fetch_optional(&s.db)
            .await
            .ok()
            .flatten();
    if let Some((hash, w, h)) = known {
        let t = EmbedThumb {
            hash,
            width: w.max(0) as u32,
            height: h.max(0) as u32,
        };
        // still on disk (kept from the sweep, but a restore may have lost it)
        if let Ok(Some(t)) = crate::embeds::keep_thumb(&s.db, t).await {
            return Some(t);
        }
    }
    let by = by?;
    if s.sports.logo_missed(&url) {
        return None;
    }
    let Some(t) = s
        .embeds
        .remote_thumb(&s.db, &s.cfg.media_dir, by, &url)
        .await
    else {
        s.sports.logo_miss(&url);
        return None;
    };
    if let Err(e) = sqlx::query(
        "INSERT INTO sports_media (url, hash, width, height) VALUES (?, ?, ?, ?)
         ON CONFLICT (url) DO UPDATE SET hash = excluded.hash, width = excluded.width, height = excluded.height",
    )
    .bind(&url)
    .bind(&t.hash)
    .bind(t.width)
    .bind(t.height)
    .execute(&s.db)
    .await
    {
        tracing::warn!(error = %e, "team logo not kept");
    }
    Some(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logos_by_sport_and_team_from_allowed_hosts_only() {
        assert_eq!(
            url("https://media.api-sports.io/", SportsLeague::Psl, 10),
            "https://media.api-sports.io/football/teams/10.png"
        );
        assert_eq!(
            url("https://media.api-sports.io", SportsLeague::Urc, 501),
            "https://media.api-sports.io/rugby/teams/501.png"
        );
        let hosts = vec!["media.api-sports.io".to_string()];
        assert!(allowed(
            &hosts,
            "https://MEDIA.api-sports.io/football/teams/1.png"
        ));
        assert!(!allowed(&hosts, "https://evil.test/football/teams/1.png"));
        assert!(!allowed(&hosts, "file:///etc/passwd"));
        assert!(!allowed(&hosts, "not a url"));
    }
}
