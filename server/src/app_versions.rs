//! Who's on which app version (#534). Each person's last update check (`GET /updates/latest.json`) and last gateway
//! connect are kept in `app_versions`, one row each, so an install that never updates can be spotted after the
//! server's logs are gone. Read by `pulse-app-server versions` (`just versions`); never shown in the app.
use axum::http::HeaderMap;
use pulse_protocol::ids::UserId;
use sqlx::SqlitePool;

use crate::db::now;
use crate::updates::{PLATFORM_HEADER, VERSION_HEADER};

/// The app's own version from its `X-Pulse-Version` header: up to 32 of `0-9 A-Z a-z . + -`, `None` when absent
/// or anything else (it's stored and printed, so nothing odd gets in).
pub fn header_version(headers: &HeaderMap) -> Option<String> {
    let v = headers.get(VERSION_HEADER)?.to_str().ok()?.trim();
    let ok = !v.is_empty()
        && v.len() <= 32
        && v.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'+' | b'-'));
    ok.then(|| v.to_string())
}

/// The app's platform from its `X-Pulse-Platform` header (0.2.18 on), lower case: up to 16 of `a-z 0-9 _ -`,
/// `None` when absent or anything else.
pub fn header_platform(headers: &HeaderMap) -> Option<String> {
    let v = headers
        .get(PLATFORM_HEADER)?
        .to_str()
        .ok()?
        .trim()
        .to_ascii_lowercase();
    let ok = !v.is_empty()
        && v.len() <= 16
        && v.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'));
    ok.then_some(v)
}

/// The straggler rescue: whether the gateway closes this connect with 4005 (update required) so the app reaches its
/// forced update screen. Only apps that say no version (every app before 0.2.17), and never one that says it isn't
/// Windows (no updater there). Apps before 0.2.18 say no platform, so a Linux build without the version header is
/// turned away too until it's rebuilt.
pub fn needs_rescue(enabled: bool, version: Option<&str>, platform: Option<&str>) -> bool {
    enabled && version.is_none() && platform.is_none_or(|p| p == "windows")
}

/// `X.Y.Z` as numbers; anything after a `-` or `+` is ignored. `None` when it isn't three numbers.
fn parse(v: &str) -> Option<(u64, u64, u64)> {
    let core = v.split(['-', '+']).next()?;
    let mut it = core.split('.').map(|p| p.parse::<u64>().ok());
    let t = (it.next()??, it.next()??, it.next()??);
    it.next().is_none().then_some(t)
}

/// What an update check told the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckResult {
    /// The manifest's version is newer than the app's: the app was offered the update.
    Offered,
    UpToDate,
    /// Nothing is published (404).
    NoManifest,
    /// The app sent no version (or the manifest has none) to compare.
    Unknown,
}

impl CheckResult {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Offered => "offered",
            Self::UpToDate => "up_to_date",
            Self::NoManifest => "no_manifest",
            Self::Unknown => "unknown",
        }
    }

    /// From the app's version and the manifest's (`None` for the manifest = nothing published).
    pub fn of(app: Option<&str>, manifest: Option<&str>) -> Self {
        let Some(manifest) = manifest else {
            return Self::NoManifest;
        };
        match (app.and_then(parse), parse(manifest)) {
            (Some(a), Some(m)) if m > a => Self::Offered,
            (Some(_), Some(_)) => Self::UpToDate,
            _ => Self::Unknown,
        }
    }
}

/// The `version` field of a `latest.json`, if it has one.
pub fn manifest_version(bytes: &[u8]) -> Option<String> {
    let v: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    Some(v.get("version")?.as_str()?.to_string())
}

/// Note `me`'s update check. Best effort: a failed write is logged, never fails the check.
pub async fn record_check(db: &SqlitePool, me: UserId, version: Option<&str>, result: CheckResult) {
    let r = sqlx::query(
        "INSERT INTO app_versions (user_id, check_version, check_result, checked_at) VALUES (?, ?, ?, ?)
         ON CONFLICT (user_id) DO UPDATE SET
           check_version = excluded.check_version,
           check_result = excluded.check_result,
           checked_at = excluded.checked_at",
    )
    .bind(me.to_string())
    .bind(version)
    .bind(result.as_str())
    .bind(now())
    .execute(db)
    .await;
    if let Err(e) = r {
        tracing::warn!(error = %e, "couldn't record the update check");
    }
}

/// Note `me`'s gateway connect and the version it said (`None` from apps before #534). Best effort.
pub async fn record_connect(db: &SqlitePool, me: UserId, version: Option<&str>) {
    let r = sqlx::query(
        "INSERT INTO app_versions (user_id, connect_version, connected_at) VALUES (?, ?, ?)
         ON CONFLICT (user_id) DO UPDATE SET
           connect_version = excluded.connect_version,
           connected_at = excluded.connected_at",
    )
    .bind(me.to_string())
    .bind(version)
    .bind(now())
    .execute(db)
    .await;
    if let Err(e) = r {
        tracing::warn!(error = %e, "couldn't record the connect version");
    }
}

/// One person's line in `versions`.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Row {
    pub username: String,
    pub check_version: Option<String>,
    pub check_result: Option<String>,
    pub checked_at: Option<String>,
    pub connect_version: Option<String>,
    pub connected_at: Option<String>,
    /// When their last connection closed (`users.last_seen_at`).
    pub last_seen_at: Option<String>,
}

/// Everyone, most recently seen first.
pub async fn list(db: &SqlitePool) -> sqlx::Result<Vec<Row>> {
    let mut rows: Vec<Row> = sqlx::query_as(
        "SELECT u.username, v.check_version, v.check_result, v.checked_at, v.connect_version, v.connected_at,
                u.last_seen_at
         FROM users u LEFT JOIN app_versions v ON v.user_id = u.id",
    )
    .fetch_all(db)
    .await?;
    rows.sort_by(|a, b| {
        last_seen(b)
            .cmp(&last_seen(a))
            .then_with(|| a.username.cmp(&b.username))
    });
    Ok(rows)
}

/// The later of their last connect and their last disconnect (stored times compare as strings).
fn last_seen(r: &Row) -> Option<&str> {
    r.connected_at.as_deref().max(r.last_seen_at.as_deref())
}

/// The version they're on as far as we know: whichever of the check and the connect is newer and said one.
fn current_version(r: &Row) -> &str {
    let check = r.check_version.as_deref().zip(r.checked_at.as_deref());
    let connect = r.connect_version.as_deref().zip(r.connected_at.as_deref());
    match (check, connect) {
        (Some((v, at)), Some((w, bt))) => {
            if at >= bt {
                v
            } else {
                w
            }
        }
        (Some((v, _)), None) | (None, Some((v, _))) => v,
        (None, None) => "?",
    }
}

/// `2026-11-20T08:15:30.123Z` → `2026-11-20 08:15`; `-` for none.
fn short(t: Option<&str>) -> String {
    match t {
        Some(t) if t.len() >= 16 => format!("{} {}", &t[..10], &t[11..16]),
        Some(t) => t.to_string(),
        None => "-".into(),
    }
}

/// The `versions` table: username, version, last check (and what it was told), last seen. `latest`: the published
/// version, when known.
pub fn format_report(rows: &[Row], latest: Option<&str>) -> String {
    let mut out = vec![format!("published: {}", latest.unwrap_or("?"))];
    let w = rows
        .iter()
        .map(|r| r.username.chars().count())
        .max()
        .unwrap_or(0)
        .max("user".len());
    out.push(format!(
        "{:<w$}  {:<10}  {:<29}  {}",
        "user", "version", "last update check", "last seen"
    ));
    for r in rows {
        let check = match &r.checked_at {
            Some(at) => format!(
                "{} {}",
                short(Some(at)),
                r.check_result.as_deref().unwrap_or("?")
            ),
            None => "never".into(),
        };
        let mut version = current_version(r).to_string();
        if let (Some(l), Some(v)) = (latest.and_then(parse), parse(&version))
            && v < l
        {
            version.push('*'); // behind the published version
        }
        out.push(format!(
            "{:<w$}  {:<10}  {:<29}  {}",
            r.username,
            version,
            check,
            short(last_seen(r))
        ));
    }
    out.push("* = behind the published version; ? = never said (an app from before 0.2.17 that never checked)".into());
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_version_takes_only_a_plain_version() {
        let mut h = HeaderMap::new();
        assert_eq!(header_version(&h), None);
        h.insert(VERSION_HEADER, "0.2.17".parse().unwrap());
        assert_eq!(header_version(&h).as_deref(), Some("0.2.17"));
        h.insert(VERSION_HEADER, "0.2.17-rc.1+abc".parse().unwrap());
        assert_eq!(header_version(&h).as_deref(), Some("0.2.17-rc.1+abc"));
        for bad in ["", "0.2 17", "0.2.17;drop", &"9".repeat(33)] {
            h.insert(VERSION_HEADER, bad.parse().unwrap());
            assert_eq!(header_version(&h), None, "{bad}");
        }
    }

    #[test]
    fn header_platform_takes_a_plain_name() {
        let mut h = HeaderMap::new();
        assert_eq!(header_platform(&h), None);
        h.insert(PLATFORM_HEADER, " Windows ".parse().unwrap());
        assert_eq!(header_platform(&h).as_deref(), Some("windows"));
        for bad in ["", "lin ux", "linux;x", &"a".repeat(17)] {
            h.insert(PLATFORM_HEADER, bad.parse().unwrap());
            assert_eq!(header_platform(&h), None, "{bad}");
        }
    }

    #[test]
    fn rescue_only_for_unversioned_apps_that_can_update() {
        assert!(needs_rescue(true, None, None));
        assert!(needs_rescue(true, None, Some("windows")));
        assert!(!needs_rescue(true, None, Some("linux")));
        assert!(!needs_rescue(true, None, Some("macos")));
        assert!(!needs_rescue(true, Some("0.2.17"), None));
        assert!(!needs_rescue(true, Some("0.2.18"), Some("windows")));
        assert!(!needs_rescue(false, None, None));
    }

    #[test]
    fn check_result_compares_numerically() {
        use CheckResult::*;
        assert_eq!(CheckResult::of(Some("0.2.14"), Some("0.2.16")), Offered);
        assert_eq!(CheckResult::of(Some("0.2.9"), Some("0.2.10")), Offered);
        assert_eq!(CheckResult::of(Some("0.2.16"), Some("0.2.16")), UpToDate);
        assert_eq!(CheckResult::of(Some("0.3.0"), Some("0.2.16")), UpToDate);
        assert_eq!(CheckResult::of(Some("0.2.16"), None), NoManifest);
        assert_eq!(CheckResult::of(None, Some("0.2.16")), Unknown);
        assert_eq!(CheckResult::of(Some("0.2"), Some("0.2.16")), Unknown);
        assert_eq!(CheckResult::of(Some("0.2.16"), Some("x")), Unknown);
    }

    #[test]
    fn manifest_version_reads_the_field() {
        assert_eq!(
            manifest_version(br#"{"version":"0.2.16","platforms":{}}"#).as_deref(),
            Some("0.2.16")
        );
        assert_eq!(manifest_version(b"{}"), None);
        assert_eq!(manifest_version(b"not json"), None);
    }

    fn row(name: &str) -> Row {
        Row {
            username: name.into(),
            check_version: None,
            check_result: None,
            checked_at: None,
            connect_version: None,
            connected_at: None,
            last_seen_at: None,
        }
    }

    #[test]
    fn report_shows_the_newest_version_and_who_is_behind() {
        let alex = Row {
            check_version: Some("0.2.14".into()),
            check_result: Some("offered".into()),
            checked_at: Some("2026-11-19T10:00:00.000Z".into()),
            connect_version: Some("0.2.16".into()),
            connected_at: Some("2026-11-20T07:30:00.000Z".into()),
            ..row("alex")
        };
        let sam = Row {
            check_version: Some("0.2.14".into()),
            check_result: Some("offered".into()),
            checked_at: Some("2026-11-20T06:00:00.000Z".into()),
            last_seen_at: Some("2026-11-20T06:40:00.000Z".into()),
            ..row("sam")
        };
        let kim = Row {
            connected_at: Some("2026-11-18T21:00:00.000Z".into()),
            ..row("kim")
        };
        let out = format_report(&[alex, sam, kim, row("jo")], Some("0.2.16"));
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "published: 0.2.16");
        assert!(lines[2].starts_with("alex  0.2.16 "), "{out}");
        assert!(lines[2].contains("2026-11-19 10:00 offered"), "{out}");
        assert!(lines[2].ends_with("2026-11-20 07:30"), "{out}");
        assert!(lines[3].starts_with("sam   0.2.14*"), "{out}");
        assert!(lines[3].ends_with("2026-11-20 06:40"), "{out}");
        assert!(lines[4].starts_with("kim   ?  "), "{out}");
        assert!(lines[4].contains("never"), "{out}");
        assert!(
            lines[5].starts_with("jo    ?") && lines[5].ends_with('-'),
            "{out}"
        );
    }
}
