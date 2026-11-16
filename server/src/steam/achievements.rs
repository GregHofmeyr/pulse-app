//! The pure rules of Steam achievements (#379): what's new since last time, the account completion rate, and which
//! game a person is playing for the poller (with its 10-minute tail).

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use pulse_protocol::ids::UserId;

use super::api::PlayerAchievement;

/// The achievements in `now` that are new: unlocked, and not in `stored`. With nothing stored for the game (never
/// seen) only those unlocked at or after `session_start` (unix seconds) count: the rest are its baseline, so old
/// unlocks are never announced while a brand-new game's first one still is. In Steam's order.
pub fn new_unlocks<'a>(
    stored: Option<&HashSet<String>>,
    now: &'a [PlayerAchievement],
    session_start: i64,
) -> Vec<&'a PlayerAchievement> {
    now.iter()
        .filter(|a| a.achieved)
        .filter(|a| match stored {
            Some(s) => !s.contains(&a.api_name),
            None => a.unlock_time >= session_start,
        })
        .collect()
}

/// The unlocked API names in `now`.
pub fn unlocked(now: &[PlayerAchievement]) -> HashSet<String> {
    now.iter()
        .filter(|a| a.achieved)
        .map(|a| a.api_name.clone())
        .collect()
}

/// One game's completion: `(unlocked, total)`.
pub type Progress = (u32, u32);

/// Steam's account completion rate (AGCR): the average completion, as a percentage, over games with at least one
/// achievement unlocked; `None` when there's no such game. Games with no achievements count for nothing.
pub fn completion_rate(games: &[Progress]) -> Option<f32> {
    let counted: Vec<f64> = games
        .iter()
        .filter(|(u, t)| *u > 0 && *t > 0)
        .map(|(u, t)| (*u).min(*t) as f64 / *t as f64)
        .collect();
    if counted.is_empty() {
        return None;
    }
    let avg = counted.iter().sum::<f64>() / counted.len() as f64 * 100.0;
    Some(((avg * 10.0).round() / 10.0) as f32)
}

/// The fewest started games (at least one achievement) before Achievement Hunter counts the completion rate.
pub const HUNTER_MIN_GAMES: usize = 20;

/// Achievement Hunter's stat: the completion rate as a whole percent, rounded down (89.99 % is 89, never 90), over
/// the same games as [`completion_rate`]; 0 with fewer than [`HUNTER_MIN_GAMES`] of them, so one easy 100 % can't
/// earn it.
pub fn hunter_percent(games: &[Progress]) -> u32 {
    let counted: Vec<f64> = games
        .iter()
        .filter(|(u, t)| *u > 0 && *t > 0)
        .map(|(u, t)| (*u).min(*t) as f64 / *t as f64)
        .collect();
    if counted.len() < HUNTER_MIN_GAMES {
        return 0;
    }
    let pct = counted.iter().sum::<f64>() / counted.len() as f64 * 100.0;
    // the epsilon keeps an exact 90 % (which floats may hold as 89.999…) at 90
    (pct + 1e-9).floor().clamp(0.0, 100.0) as u32
}

/// Games at 100 %.
pub fn perfect_games(games: &[Progress]) -> u32 {
    games.iter().filter(|(u, t)| *t > 0 && u >= t).count() as u32
}

/// Whether a game went to 100 % now: it is complete, wasn't before, and either was known before or got something
/// new now (a game seen for the first time already complete was completed long ago).
pub fn completed_now(before: Option<Progress>, after: Progress, any_new: bool) -> bool {
    let complete = |(u, t): Progress| t > 0 && u >= t;
    complete(after)
        && match before {
            Some(b) => !complete(b),
            None => any_new,
        }
}

/// What the poller remembers of one person's game.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Track {
    app_id: u32,
    /// When this play session began (unix seconds), for [`new_unlocks`]'s baseline.
    session_start: i64,
    last_seen: Instant,
}

/// Which game each linked person is playing, as far as the poller is concerned: their current one, or the last one
/// for `tail` after it ended.
#[derive(Debug, Default)]
pub struct Tracks {
    by_user: HashMap<UserId, Track>,
}

impl Tracks {
    /// `user` is seen playing `playing` (an app id, with the session's start in unix seconds) at `now`; `None`:
    /// nothing with an app id. The game to poll and its session start, if any: the current one, or the last one
    /// while it's within `tail` of ending.
    pub fn observe(
        &mut self,
        user: UserId,
        playing: Option<(u32, i64)>,
        now: Instant,
        tail: Duration,
    ) -> Option<(u32, i64)> {
        match playing {
            Some((app_id, start)) => {
                let t = self.by_user.entry(user).or_insert(Track {
                    app_id,
                    session_start: start,
                    last_seen: now,
                });
                if t.app_id != app_id {
                    t.app_id = app_id;
                    t.session_start = start;
                }
                t.last_seen = now;
                Some((t.app_id, t.session_start))
            }
            None => {
                let t = self.by_user.get(&user)?;
                if now.saturating_duration_since(t.last_seen) <= tail {
                    Some((t.app_id, t.session_start))
                } else {
                    self.by_user.remove(&user);
                    None
                }
            }
        }
    }

    /// The game `user` is (or was, within the tail) playing, without observing anything.
    pub fn current(&self, user: UserId) -> Option<u32> {
        self.by_user.get(&user).map(|t| t.app_id)
    }

    pub fn forget(&mut self, user: UserId) {
        self.by_user.remove(&user);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(name: &str, achieved: bool, at: i64) -> PlayerAchievement {
        PlayerAchievement {
            api_name: name.into(),
            achieved,
            unlock_time: at,
        }
    }

    #[test]
    fn new_is_unlocked_and_not_stored() {
        let now = [a("A", true, 100), a("B", true, 200), a("C", false, 0)];
        let stored: HashSet<String> = ["A".to_string()].into();
        let got: Vec<_> = new_unlocks(Some(&stored), &now, 0)
            .iter()
            .map(|x| x.api_name.as_str())
            .collect();
        assert_eq!(got, ["B"]);
        let all: HashSet<String> = ["A".to_string(), "B".to_string()].into();
        assert!(new_unlocks(Some(&all), &now, 0).is_empty());
    }

    #[test]
    fn a_game_never_seen_only_counts_this_sessions_unlocks() {
        let now = [
            a("OLD", true, 1_000),
            a("NEW", true, 5_000),
            a("NOT", false, 0),
        ];
        let got: Vec<_> = new_unlocks(None, &now, 4_000)
            .iter()
            .map(|x| x.api_name.as_str())
            .collect();
        assert_eq!(got, ["NEW"]);
        assert!(new_unlocks(None, &now, 9_000).is_empty());
    }

    #[test]
    fn the_completion_rate_is_the_average_over_started_games() {
        assert_eq!(completion_rate(&[]), None);
        assert_eq!(completion_rate(&[(0, 10), (0, 0)]), None);
        // 100 %, 50 %, 0 % (not counted), no achievements (not counted)
        assert_eq!(
            completion_rate(&[(10, 10), (5, 10), (0, 30), (0, 0)]),
            Some(75.0)
        );
        assert_eq!(completion_rate(&[(1, 3)]), Some(33.3));
        assert_eq!(perfect_games(&[(10, 10), (5, 10), (0, 0), (3, 3)]), 2);
    }

    #[test]
    fn hunter_percent_needs_20_started_games_and_rounds_down() {
        let full = |n: usize| vec![(10u32, 10u32); n];
        // 19 started games: nothing, however complete; unstarted and achievement-less games never count
        let mut g = full(19);
        g.extend([(0, 10), (0, 0)]);
        assert_eq!(hunter_percent(&g), 0);
        assert_eq!(hunter_percent(&full(20)), 100);
        // 20 games: 18 at 100 % and 2 at 50 % = exactly 95 %
        let mut g = full(18);
        g.extend([(5, 10), (5, 10)]);
        assert_eq!(hunter_percent(&g), 95);
        // just under 90 %: 89.99 % is 89
        let mut g = full(19);
        g.push((7998, 10000)); // (19 + 0.7998) / 20 = 98.99 %
        assert_eq!(hunter_percent(&g), 98);
        let mut g = vec![(9, 10); 19];
        g.push((8998, 10000)); // (19 * 0.9 + 0.8998) / 20 = 89.999 %
        assert_eq!(hunter_percent(&g), 89);
        assert_eq!(hunter_percent(&[(9, 10); 20]), 90, "an exact 90 % is 90");
        // more unlocked than the total (a changed schema) is 100 %
        assert_eq!(hunter_percent(&[(12, 10); 20]), 100);
    }

    #[test]
    fn completed_now_only_on_the_way_to_100() {
        assert!(completed_now(Some((9, 10)), (10, 10), true));
        assert!(!completed_now(Some((10, 10)), (10, 10), false), "already");
        assert!(!completed_now(Some((8, 10)), (9, 10), true));
        assert!(
            !completed_now(None, (10, 10), false),
            "first seen already complete"
        );
        assert!(
            completed_now(None, (1, 1), true),
            "first seen, completed this session"
        );
        assert!(!completed_now(None, (0, 0), true));
    }

    #[test]
    fn tracks_follow_the_game_and_its_tail() {
        let mut t = Tracks::default();
        let u = UserId::new();
        let t0 = Instant::now();
        let tail = Duration::from_secs(600);
        assert_eq!(t.observe(u, None, t0, tail), None);
        assert_eq!(t.observe(u, Some((10, 500)), t0, tail), Some((10, 500)));
        // the same game keeps its session start
        assert_eq!(
            t.observe(u, Some((10, 900)), t0 + Duration::from_secs(60), tail),
            Some((10, 500))
        );
        // ended: still polled within the tail
        let ended = t0 + Duration::from_secs(60);
        assert_eq!(
            t.observe(u, None, ended + Duration::from_secs(599), tail),
            Some((10, 500))
        );
        assert_eq!(t.current(u), Some(10));
        assert_eq!(
            t.observe(u, None, ended + Duration::from_secs(601), tail),
            None
        );
        assert_eq!(t.current(u), None);
        // a new game is a new session
        t.observe(u, Some((10, 500)), t0, tail);
        assert_eq!(t.observe(u, Some((20, 700)), t0, tail), Some((20, 700)));
    }
}
