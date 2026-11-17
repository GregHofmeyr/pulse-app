//! Status (#45): Online / Do Not Disturb / Invisible, and appearing offline to chosen people.
//!
//! THE presence privacy rule lives in [`view_for`]: what one viewer may see of one person. Every
//! payload that carries presence (Ready's people, `PresenceChanged`) is built from it, per viewer,
//! so a viewer someone is invisible or hidden to gets exactly what they'd get if that person had
//! left: offline, with the last seen frozen at the moment they stopped looking online.

pub mod custom;
pub mod routes;
pub mod sweep;

use std::collections::{HashMap, HashSet};

use pulse_protocol::ids::UserId;
use pulse_protocol::rest::{CustomStatus, OwnStatus, PresenceStatus, Status};
use sqlx::SqlitePool;

/// A freeze moment for someone who had never been seen (never connected) when they stopped
/// looking online: they stay "never seen" rather than jumping to a time. Sorts before every time.
pub const NEVER_SEEN: &str = "";

/// Everything about one person that decides how others see them.
#[derive(Clone, Debug)]
pub struct Subject {
    pub id: UserId,
    pub status: Status,
    /// When a timed DND or Invisible ends (#153; the column kept DND's name from 0019). `None`:
    /// until turned off.
    pub dnd_until: Option<String>,
    /// Set while Invisible: the last seen everyone (but the person) sees.
    pub invisible_at: Option<String>,
    /// Their real last disconnect.
    pub last_seen_at: Option<String>,
    /// target → (frozen last seen, still on the owner's list). An off-list row only counts while
    /// Invisible (see the migration).
    pub hidden: HashMap<UserId, (String, bool)>,
    /// Their custom status (#152), and when it clears (`None`: never).
    pub custom: Option<CustomStatus>,
    pub custom_until: Option<String>,
    /// The people they hide what they're playing from (#436).
    pub game_hidden: HashSet<UserId>,
}

/// What one viewer sees of one person.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresenceView {
    pub status: PresenceStatus,
    pub last_seen_at: Option<String>,
    /// Online with every device idle (#53): only ever with `status: Online`.
    pub idle: bool,
    /// Their custom status (#152): only while they look online to the viewer (and always to themselves).
    pub custom: Option<CustomStatus>,
}

impl PresenceView {
    pub fn online(&self) -> bool {
        self.status != PresenceStatus::Offline
    }
}

impl Subject {
    /// The status in force at `now`: a DND or Invisible whose timer ran out is Online.
    pub fn status_at(&self, now: &str) -> Status {
        match self.status {
            Status::Online => Status::Online,
            timed if self.dnd_until.as_deref().is_none_or(|u| u > now) => timed,
            _ => Status::Online,
        }
    }

    /// The custom status in force at `now` (#152): a run-out one is none.
    pub fn custom_at(&self, now: &str) -> Option<CustomStatus> {
        self.custom
            .clone()
            .filter(|_| self.custom_until.as_deref().is_none_or(|u| u > now))
    }

    /// DND in force at `now` (an expired timer is Online).
    pub fn dnd(&self, now: &str) -> bool {
        self.status_at(now) == Status::Dnd
    }

    /// Invisible in force at `now` (an expired timer is Online, #153).
    pub fn invisible(&self, now: &str) -> bool {
        self.status_at(now) == Status::Invisible
    }

    /// The status as it stands at `now`, for the person's own sessions.
    pub fn own(&self, now: &str) -> OwnStatus {
        match self.status_at(now) {
            Status::Online => OwnStatus::default(),
            status => OwnStatus {
                status,
                dnd_until: self.dnd_until.clone(),
            },
        }
    }

    /// The moment this person stopped looking online to `viewer`, if they're hiding from them now
    /// (Invisible, hidden, or both: the earlier moment).
    fn frozen_for(&self, viewer: UserId, now: &str) -> Option<String> {
        let invisible = self.invisible(now);
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

    /// Whether `viewer` may see this person's game activity (#436), when they see them online at all: never hidden
    /// from themselves.
    pub fn shows_game_to(&self, viewer: UserId) -> bool {
        viewer == self.id || !self.game_hidden.contains(&viewer)
    }

    /// Whether this person hides from `viewer` at all right now.
    fn hides_from(&self, viewer: UserId, now: &str) -> bool {
        self.invisible(now) || self.hidden.get(&viewer).is_some_and(|(_, active)| *active)
    }
}

/// What `viewer` may see of `s`. `connected`: `s` has at least one live gateway connection; `idle`:
/// every one of them says it's idle (#53). Idle only ever modifies Online: DND and Invisible win.
pub fn view_for(
    s: &Subject,
    viewer: UserId,
    connected: bool,
    idle: bool,
    now: &str,
) -> PresenceView {
    let offline = |last_seen_at| PresenceView {
        status: PresenceStatus::Offline,
        last_seen_at,
        idle: false,
        custom: None,
    };
    // a custom status shows only with the dot (#152): nobody it's hidden from, or who sees them offline, gets it
    let live = |last_seen_at| {
        if s.dnd(now) {
            PresenceView {
                status: PresenceStatus::Dnd,
                last_seen_at,
                idle: false,
                custom: s.custom_at(now),
            }
        } else {
            PresenceView {
                status: PresenceStatus::Online,
                last_seen_at,
                // your own sessions see you Online while you're Invisible: never Idle then either
                idle: idle && s.status_at(now) == Status::Online,
                custom: s.custom_at(now),
            }
        }
    };
    if viewer == s.id {
        // You always see the truth about yourself (the client draws Invisible from your own status), your custom
        // status included.
        return if connected {
            live(s.last_seen_at.clone())
        } else {
            PresenceView {
                custom: s.custom_at(now),
                ..offline(s.last_seen_at.clone())
            }
        };
    }
    if s.hides_from(viewer, now) {
        return offline(s.frozen_for(viewer, now));
    }
    if connected {
        // Never anything that changes while hidden: an online person's last seen isn't shown.
        live(None)
    } else {
        offline(s.last_seen_at.clone())
    }
}

type Row = (
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);
const SUBJECT_COLS: &str =
    "id, presence, dnd_until, invisible_at, last_seen_at, custom_text, custom_emoji, custom_until";

fn subject(r: Row) -> anyhow::Result<Subject> {
    Ok(Subject {
        id: r.0.parse()?,
        status: Status::parse(&r.1).unwrap_or_default(),
        dnd_until: r.2,
        invisible_at: r.3,
        last_seen_at: r.4,
        hidden: HashMap::new(),
        custom: r.5.map(|text| CustomStatus { text, emoji: r.6 }),
        custom_until: r.7,
        game_hidden: HashSet::new(),
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
    let game_hidden: Vec<String> =
        sqlx::query_scalar("SELECT target_id FROM activity_hidden WHERE user_id = ?")
            .bind(id.to_string())
            .fetch_all(db)
            .await?;
    for t in game_hidden {
        s.game_hidden.insert(t.parse()?);
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
    let game_hidden: Vec<(String, String)> =
        sqlx::query_as("SELECT user_id, target_id FROM activity_hidden")
            .fetch_all(db)
            .await?;
    for (o, t) in game_hidden {
        if let Some(s) = all.get_mut(&o.parse()?) {
            s.game_hidden.insert(t.parse()?);
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
            custom: None,
            custom_until: None,
            game_hidden: HashSet::new(),
        }
    }

    #[test]
    fn visible_people_show_as_they_are() {
        let s = alex();
        let sam = UserId::new();
        assert_eq!(
            view_for(&s, sam, true, false, NOW),
            PresenceView {
                status: PresenceStatus::Online,
                last_seen_at: None,
                idle: false,
                custom: None,
            }
        );
        assert_eq!(
            view_for(&s, sam, false, false, NOW),
            PresenceView {
                status: PresenceStatus::Offline,
                last_seen_at: s.last_seen_at.clone(),
                idle: false,
                custom: None,
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
            idle: false,
            custom: None,
        };
        // connected or not, and whatever their real last disconnect says
        assert_eq!(view_for(&s, UserId::new(), true, false, NOW), want);
        s.last_seen_at = Some("2026-11-05T11:30:00.000Z".into());
        assert_eq!(view_for(&s, UserId::new(), false, false, NOW), want);
    }

    #[test]
    fn hidden_is_offline_to_that_viewer_only() {
        let mut s = alex();
        let sam = UserId::new();
        s.hidden
            .insert(sam, ("2026-11-05T10:30:00.000Z".into(), true));
        assert_eq!(
            view_for(&s, sam, true, false, NOW),
            PresenceView {
                status: PresenceStatus::Offline,
                last_seen_at: Some("2026-11-05T10:30:00.000Z".into()),
                idle: false,
                custom: None,
            }
        );
        assert!(view_for(&s, UserId::new(), true, false, NOW).online());
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
            view_for(&s, sam, true, false, NOW).last_seen_at.as_deref(),
            Some("2026-11-05T08:00:00.000Z")
        );
        s.hidden
            .insert(sam, ("2026-11-05T11:00:00.000Z".into(), true));
        assert_eq!(
            view_for(&s, sam, true, false, NOW).last_seen_at.as_deref(),
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
            view_for(&s, sam, true, false, NOW).last_seen_at.as_deref(),
            Some("2026-11-05T08:00:00.000Z")
        );
        s.status = Status::Online;
        s.invisible_at = None;
        assert!(view_for(&s, sam, true, false, NOW).online());
    }

    #[test]
    fn dnd_shows_until_its_timer_runs_out() {
        let mut s = alex();
        s.status = Status::Dnd;
        let sam = UserId::new();
        assert_eq!(
            view_for(&s, sam, true, false, NOW).status,
            PresenceStatus::Dnd
        );
        s.dnd_until = Some("2026-11-05T13:00:00.000Z".into());
        assert_eq!(
            view_for(&s, sam, true, false, NOW).status,
            PresenceStatus::Dnd
        );
        s.dnd_until = Some("2026-11-05T11:59:59.999Z".into());
        assert_eq!(
            view_for(&s, sam, true, false, NOW).status,
            PresenceStatus::Online
        );
        assert_eq!(s.own(NOW), OwnStatus::default());
        // DND never shows while offline
        assert_eq!(
            view_for(&s, sam, false, false, NOW).status,
            PresenceStatus::Offline
        );
    }

    #[test]
    fn invisible_hides_until_its_timer_runs_out() {
        let mut s = alex();
        s.status = Status::Invisible;
        s.invisible_at = Some("2026-11-05T10:00:00.000Z".into());
        s.dnd_until = Some("2026-11-05T13:00:00.000Z".into());
        let sam = UserId::new();
        s.hidden
            .insert(sam, ("2026-11-05T08:00:00.000Z".into(), false));
        assert_eq!(
            view_for(&s, sam, true, false, NOW).status,
            PresenceStatus::Offline
        );
        assert_eq!(
            s.own(NOW),
            OwnStatus {
                status: Status::Invisible,
                dnd_until: s.dnd_until.clone()
            }
        );
        // run out: Online to everyone, an off-list row no longer freezes, and offline shows the real
        // last disconnect
        s.dnd_until = Some("2026-11-05T11:59:59.999Z".into());
        assert_eq!(
            view_for(&s, sam, true, false, NOW),
            PresenceView {
                status: PresenceStatus::Online,
                last_seen_at: None,
                idle: false,
                custom: None,
            }
        );
        assert_eq!(
            view_for(&s, UserId::new(), false, false, NOW).last_seen_at,
            s.last_seen_at
        );
        assert_eq!(s.own(NOW), OwnStatus::default());
    }

    #[test]
    fn never_seen_stays_never_seen() {
        let mut s = alex();
        s.last_seen_at = None;
        s.status = Status::Invisible;
        s.invisible_at = Some(NEVER_SEEN.into());
        assert_eq!(
            view_for(&s, UserId::new(), true, false, NOW).last_seen_at,
            None
        );
        // a later real disconnect doesn't show either
        s.last_seen_at = Some(NOW.into());
        assert_eq!(
            view_for(&s, UserId::new(), false, false, NOW).last_seen_at,
            None
        );
    }

    #[test]
    fn you_always_see_yourself_truly() {
        let mut s = alex();
        s.status = Status::Invisible;
        s.invisible_at = Some("2026-11-05T10:00:00.000Z".into());
        s.hidden.insert(s.id, ("x".into(), true));
        assert!(view_for(&s, s.id, true, false, NOW).online());
        assert_eq!(s.own(NOW).status, Status::Invisible);
    }

    #[test]
    fn idle_only_modifies_online() {
        let mut s = alex();
        let sam = UserId::new();
        assert_eq!(
            view_for(&s, sam, true, true, NOW),
            PresenceView {
                status: PresenceStatus::Online,
                last_seen_at: None,
                idle: true,
                custom: None,
            }
        );
        // you see your own Idle
        assert!(view_for(&s, s.id, true, true, NOW).idle);
        // never while offline
        assert!(!view_for(&s, sam, false, true, NOW).idle);
        // DND wins
        s.status = Status::Dnd;
        let v = view_for(&s, sam, true, true, NOW);
        assert_eq!((v.status, v.idle), (PresenceStatus::Dnd, false));
        assert!(!view_for(&s, s.id, true, true, NOW).idle);
        // Invisible wins, for others and on your own sessions (which see you Online)
        s.status = Status::Invisible;
        s.invisible_at = Some("2026-11-05T10:00:00.000Z".into());
        let v = view_for(&s, sam, true, true, NOW);
        assert_eq!((v.status, v.idle), (PresenceStatus::Offline, false));
        let mine = view_for(&s, s.id, true, true, NOW);
        assert_eq!((mine.status, mine.idle), (PresenceStatus::Online, false));
        // hidden from one viewer: offline to them, Idle to the rest
        s.status = Status::Online;
        s.invisible_at = None;
        s.hidden
            .insert(sam, ("2026-11-05T10:30:00.000Z".into(), true));
        assert!(!view_for(&s, sam, true, true, NOW).idle);
        assert!(view_for(&s, UserId::new(), true, true, NOW).idle);
    }

    #[test]
    fn custom_status_shows_only_with_the_dot_and_until_it_runs_out() {
        let mut s = alex();
        let sam = UserId::new();
        let gaming = CustomStatus {
            text: "Gaming".into(),
            emoji: Some("🎮".into()),
        };
        s.custom = Some(gaming.clone());
        // online and DND show it
        assert_eq!(
            view_for(&s, sam, true, false, NOW).custom,
            Some(gaming.clone())
        );
        s.status = Status::Dnd;
        assert_eq!(
            view_for(&s, sam, true, true, NOW).custom,
            Some(gaming.clone())
        );
        // offline doesn't
        assert_eq!(view_for(&s, sam, false, false, NOW).custom, None);
        // Invisible doesn't, but you still see your own
        s.status = Status::Invisible;
        s.invisible_at = Some("2026-11-05T10:00:00.000Z".into());
        assert_eq!(view_for(&s, sam, true, false, NOW).custom, None);
        assert_eq!(
            view_for(&s, s.id, true, false, NOW).custom,
            Some(gaming.clone())
        );
        // hidden from one viewer: not to them
        s.status = Status::Online;
        s.invisible_at = None;
        s.hidden
            .insert(sam, ("2026-11-05T10:30:00.000Z".into(), true));
        assert_eq!(view_for(&s, sam, true, false, NOW).custom, None);
        assert_eq!(
            view_for(&s, UserId::new(), true, false, NOW).custom,
            Some(gaming.clone())
        );
        // a run-out timer is none, to everyone
        s.custom_until = Some("2026-11-05T11:59:59.999Z".into());
        assert_eq!(view_for(&s, UserId::new(), true, false, NOW).custom, None);
        assert_eq!(view_for(&s, s.id, true, false, NOW).custom, None);
        s.custom_until = Some("2026-11-05T12:30:00.000Z".into());
        assert_eq!(view_for(&s, s.id, true, false, NOW).custom, Some(gaming));
    }
}
