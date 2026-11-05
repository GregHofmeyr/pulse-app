//! Status (#45): Online / Do Not Disturb / Invisible, and appearing offline to chosen people.
//!
//! THE presence privacy rule lives in [`view_for`]: what one viewer may see of one person. Every
//! payload that carries presence (Ready's people, `PresenceChanged`) is built from it, per viewer,
//! so a viewer someone is invisible or hidden to gets exactly what they'd get if that person had
//! left: offline, with the last seen frozen at the moment they stopped looking online.

pub mod routes;
pub mod sweep;

use std::collections::HashMap;

use pulse_protocol::ids::UserId;
use pulse_protocol::rest::{OwnStatus, PresenceStatus, Status};
use sqlx::SqlitePool;

/// A freeze moment for someone who had never been seen (never connected) when they stopped
/// looking online: they stay "never seen" rather than jumping to a time. Sorts before every time.
pub const NEVER_SEEN: &str = "";

/// Everything about one person that decides how others see them.
#[derive(Clone, Debug)]
pub struct Subject {
    pub id: UserId,
    pub status: Status,
    pub dnd_until: Option<String>,
    /// Set while Invisible: the last seen everyone (but the person) sees.
    pub invisible_at: Option<String>,
    /// Their real last disconnect.
    pub last_seen_at: Option<String>,
    /// target → (frozen last seen, still on the owner's list). An off-list row only counts while
    /// Invisible (see the migration).
    pub hidden: HashMap<UserId, (String, bool)>,
}

/// What one viewer sees of one person.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresenceView {
    pub status: PresenceStatus,
    pub last_seen_at: Option<String>,
}

impl PresenceView {
    pub fn online(&self) -> bool {
        self.status != PresenceStatus::Offline
    }
}

impl Subject {
    /// DND in force at `now` (an expired timer is Online).
    pub fn dnd(&self, now: &str) -> bool {
        self.status == Status::Dnd && self.dnd_until.as_deref().is_none_or(|u| u > now)
    }

    /// The status as it stands at `now`, for the person's own sessions.
    pub fn own(&self, now: &str) -> OwnStatus {
        match self.status {
            Status::Dnd if !self.dnd(now) => OwnStatus::default(),
            status => OwnStatus {
                status,
                dnd_until: if status == Status::Dnd {
                    self.dnd_until.clone()
                } else {
                    None
                },
            },
        }
    }

    /// The moment this person stopped looking online to `viewer`, if they're hiding from them now
    /// (Invisible, hidden, or both: the earlier moment).
    fn frozen_for(&self, viewer: UserId) -> Option<String> {
        let invisible = self.status == Status::Invisible;
        let hidden = self
            .hidden
            .get(&viewer)
            .filter(|(_, active)| *active || invisible)
            .map(|(at, _)| at.clone());
        let mine = invisible.then(|| self.invisible_at.clone()).flatten();
        match (mine, hidden) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
        .map_or_else(
            || invisible.then(|| self.last_seen_at.clone()).flatten(),
            // NEVER_SEEN: they had never been seen when they stopped looking online
            |at| (at != NEVER_SEEN).then_some(at),
        )
    }

    /// Whether this person hides from `viewer` at all right now.
    fn hides_from(&self, viewer: UserId) -> bool {
        self.status == Status::Invisible
            || self.hidden.get(&viewer).is_some_and(|(_, active)| *active)
    }
}

/// What `viewer` may see of `s`. `connected`: `s` has at least one live gateway connection.
pub fn view_for(s: &Subject, viewer: UserId, connected: bool, now: &str) -> PresenceView {
    let live = |s: &Subject| {
        if s.dnd(now) {
            PresenceStatus::Dnd
        } else {
            PresenceStatus::Online
        }
    };
    if viewer == s.id {
        // You always see the truth about yourself (the client draws Invisible from your own status).
        return if connected {
            PresenceView {
                status: live(s),
                last_seen_at: s.last_seen_at.clone(),
            }
        } else {
            PresenceView {
                status: PresenceStatus::Offline,
                last_seen_at: s.last_seen_at.clone(),
            }
        };
    }
    if s.hides_from(viewer) {
        return PresenceView {
            status: PresenceStatus::Offline,
            last_seen_at: s.frozen_for(viewer),
        };
    }
    if connected {
        PresenceView {
            status: live(s),
            // Never anything that changes while hidden: an online person's last seen isn't shown.
            last_seen_at: None,
        }
    } else {
        PresenceView {
            status: PresenceStatus::Offline,
            last_seen_at: s.last_seen_at.clone(),
        }
    }
}

type Row = (
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
);
const SUBJECT_COLS: &str = "id, presence, dnd_until, invisible_at, last_seen_at";

fn subject(r: Row) -> anyhow::Result<Subject> {
    Ok(Subject {
        id: r.0.parse()?,
        status: Status::parse(&r.1).unwrap_or_default(),
        dnd_until: r.2,
        invisible_at: r.3,
        last_seen_at: r.4,
        hidden: HashMap::new(),
    })
}

/// One person's status and the people they hide from.
pub async fn load_subject(db: &SqlitePool, id: UserId) -> anyhow::Result<Option<Subject>> {
    let row: Option<Row> =
        sqlx::query_as(&format!("SELECT {SUBJECT_COLS} FROM users WHERE id = ?"))
            .bind(id.to_string())
            .fetch_optional(db)
            .await?;
    let Some(row) = row else { return Ok(None) };
    let mut s = subject(row)?;
    let hidden: Vec<(String, String, i64)> = sqlx::query_as(
        "SELECT target_id, hidden_at, active FROM presence_hidden WHERE owner_id = ?",
    )
    .bind(id.to_string())
    .fetch_all(db)
    .await?;
    for (t, at, active) in hidden {
        s.hidden.insert(t.parse()?, (at, active != 0));
    }
    Ok(Some(s))
}

/// Everyone, by id (Ready reads each table once).
pub async fn load_all(db: &SqlitePool) -> anyhow::Result<HashMap<UserId, Subject>> {
    let rows: Vec<Row> = sqlx::query_as(&format!("SELECT {SUBJECT_COLS} FROM users"))
        .fetch_all(db)
        .await?;
    let mut all = HashMap::new();
    for r in rows {
        let s = subject(r)?;
        all.insert(s.id, s);
    }
    let hidden: Vec<(String, String, String, i64)> =
        sqlx::query_as("SELECT owner_id, target_id, hidden_at, active FROM presence_hidden")
            .fetch_all(db)
            .await?;
    for (o, t, at, active) in hidden {
        if let Some(s) = all.get_mut(&o.parse()?) {
            s.hidden.insert(t.parse()?, (at, active != 0));
        }
    }
    Ok(all)
}

/// The people `owner` appears offline to, for their Ready only.
pub async fn hidden_list(db: &SqlitePool, owner: UserId) -> anyhow::Result<Vec<UserId>> {
    let ids: Vec<String> = sqlx::query_scalar(
        "SELECT target_id FROM presence_hidden WHERE owner_id = ? AND active = 1 ORDER BY hidden_at",
    )
    .bind(owner.to_string())
    .fetch_all(db)
    .await?;
    Ok(ids.iter().map(|i| i.parse()).collect::<Result<_, _>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: &str = "2026-11-05T12:00:00.000Z";

    fn alex() -> Subject {
        Subject {
            id: UserId::new(),
            status: Status::Online,
            dnd_until: None,
            invisible_at: None,
            last_seen_at: Some("2026-11-05T09:00:00.000Z".into()),
            hidden: HashMap::new(),
        }
    }

    #[test]
    fn visible_people_show_as_they_are() {
        let s = alex();
        let sam = UserId::new();
        assert_eq!(
            view_for(&s, sam, true, NOW),
            PresenceView {
                status: PresenceStatus::Online,
                last_seen_at: None
            }
        );
        assert_eq!(
            view_for(&s, sam, false, NOW),
            PresenceView {
                status: PresenceStatus::Offline,
                last_seen_at: s.last_seen_at.clone()
            }
        );
    }

    #[test]
    fn invisible_is_offline_with_the_moment_they_went() {
        let mut s = alex();
        s.status = Status::Invisible;
        s.invisible_at = Some("2026-11-05T10:00:00.000Z".into());
        let want = PresenceView {
            status: PresenceStatus::Offline,
            last_seen_at: Some("2026-11-05T10:00:00.000Z".into()),
        };
        // connected or not, and whatever their real last disconnect says
        assert_eq!(view_for(&s, UserId::new(), true, NOW), want);
        s.last_seen_at = Some("2026-11-05T11:30:00.000Z".into());
        assert_eq!(view_for(&s, UserId::new(), false, NOW), want);
    }

    #[test]
    fn hidden_is_offline_to_that_viewer_only() {
        let mut s = alex();
        let sam = UserId::new();
        s.hidden
            .insert(sam, ("2026-11-05T10:30:00.000Z".into(), true));
        assert_eq!(
            view_for(&s, sam, true, NOW),
            PresenceView {
                status: PresenceStatus::Offline,
                last_seen_at: Some("2026-11-05T10:30:00.000Z".into())
            }
        );
        assert!(view_for(&s, UserId::new(), true, NOW).online());
    }

    #[test]
    fn invisible_and_hidden_stack_and_the_earlier_moment_wins() {
        let mut s = alex();
        let sam = UserId::new();
        s.status = Status::Invisible;
        s.invisible_at = Some("2026-11-05T10:00:00.000Z".into());
        s.hidden
            .insert(sam, ("2026-11-05T08:00:00.000Z".into(), true));
        assert_eq!(
            view_for(&s, sam, true, NOW).last_seen_at.as_deref(),
            Some("2026-11-05T08:00:00.000Z")
        );
        s.hidden
            .insert(sam, ("2026-11-05T11:00:00.000Z".into(), true));
        assert_eq!(
            view_for(&s, sam, true, NOW).last_seen_at.as_deref(),
            Some("2026-11-05T10:00:00.000Z")
        );
    }

    #[test]
    fn an_off_list_row_freezes_only_while_invisible() {
        let mut s = alex();
        let sam = UserId::new();
        s.status = Status::Invisible;
        s.invisible_at = Some("2026-11-05T10:00:00.000Z".into());
        s.hidden
            .insert(sam, ("2026-11-05T08:00:00.000Z".into(), false));
        assert_eq!(
            view_for(&s, sam, true, NOW).last_seen_at.as_deref(),
            Some("2026-11-05T08:00:00.000Z")
        );
        s.status = Status::Online;
        s.invisible_at = None;
        assert!(view_for(&s, sam, true, NOW).online());
    }

    #[test]
    fn dnd_shows_until_its_timer_runs_out() {
        let mut s = alex();
        s.status = Status::Dnd;
        let sam = UserId::new();
        assert_eq!(view_for(&s, sam, true, NOW).status, PresenceStatus::Dnd);
        s.dnd_until = Some("2026-11-05T13:00:00.000Z".into());
        assert_eq!(view_for(&s, sam, true, NOW).status, PresenceStatus::Dnd);
        s.dnd_until = Some("2026-11-05T11:59:59.999Z".into());
        assert_eq!(view_for(&s, sam, true, NOW).status, PresenceStatus::Online);
        assert_eq!(s.own(NOW), OwnStatus::default());
        // DND never shows while offline
        assert_eq!(
            view_for(&s, sam, false, NOW).status,
            PresenceStatus::Offline
        );
    }

    #[test]
    fn never_seen_stays_never_seen() {
        let mut s = alex();
        s.last_seen_at = None;
        s.status = Status::Invisible;
        s.invisible_at = Some(NEVER_SEEN.into());
        assert_eq!(view_for(&s, UserId::new(), true, NOW).last_seen_at, None);
        // a later real disconnect doesn't show either
        s.last_seen_at = Some(NOW.into());
        assert_eq!(view_for(&s, UserId::new(), false, NOW).last_seen_at, None);
    }

    #[test]
    fn you_always_see_yourself_truly() {
        let mut s = alex();
        s.status = Status::Invisible;
        s.invisible_at = Some("2026-11-05T10:00:00.000Z".into());
        s.hidden.insert(s.id, ("x".into(), true));
        assert!(view_for(&s, s.id, true, NOW).online());
        assert_eq!(s.own(NOW).status, Status::Invisible);
    }
}
