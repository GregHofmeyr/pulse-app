//! YouTube watch parties in voice (#384): someone in a server voice channel's call starts one from a YouTube card, and
//! everyone in the call who opens it watches the same video at the same moment.
//!
//! The server holds only the shared state ([`WatchParty`]): the video, where it was (`position_ms`) at `set_at_ms` by
//! the server's clock, playing or paused, the host and the queue. Each viewer's client drives its own YouTube player
//! from it (`position + (server now - set_at)` while playing) and corrects its own drift; the server never ticks.
//!
//! Held in memory, like the stage (#234) and the voice state it hangs off: a party is as ephemeral as its call. It ends
//! when its last viewer leaves it or the call. A host who leaves passes it to the next viewer, in the order they joined.
//! Every change is also snapshotted to `watch_party_snapshots` (#409), so a restart picks a party back up
//! ([`restore`]): only a snapshot younger than [`RESTORE_WITHIN_MS`], and only with the viewers still in the call.

pub mod routes;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use pulse_protocol::ids::{ChannelId, UserId};
use pulse_protocol::rest::{EmbedThumb, WatchParty, WatchPartyAction, WatchPartyItem};

/// The longest queue a party keeps.
pub const MAX_QUEUE: usize = 50;

/// The longest title kept, in characters (it's the starter's card's; longer is cut).
pub const MAX_TITLE: usize = 200;

/// The furthest position taken (48 hours): YouTube's longest videos and streams fit, and nothing overflows.
pub const MAX_POSITION_MS: u64 = 48 * 3600 * 1000;

/// How old a snapshot may be and still be restored on boot (5 minutes): a restart for a deploy is well inside it, and
/// a party left over from a long outage is gone.
pub const RESTORE_WITHIN_MS: i64 = 5 * 60 * 1000;

/// A YouTube video id: exactly 11 of `[A-Za-z0-9_-]`.
pub fn valid_video_id(id: &str) -> bool {
    id.len() == 11
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// A title as a party keeps it: trimmed, no control characters, at most [`MAX_TITLE`] characters; `None` when that
/// leaves nothing.
pub fn clean_title(title: Option<String>) -> Option<String> {
    let t: String = title?
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_TITLE)
        .collect();
    let t = t.trim();
    (!t.is_empty()).then(|| t.to_owned())
}

/// Where `party`'s video is at `now_ms` (server clock): it moves on while playing.
pub fn position_at(party: &WatchParty, now_ms: i64) -> u64 {
    if !party.playing {
        return party.position_ms;
    }
    let ran = u64::try_from(now_ms.saturating_sub(party.set_at_ms)).unwrap_or(0);
    party.position_ms.saturating_add(ran).min(MAX_POSITION_MS)
}

/// Why a change was refused.
#[derive(Debug, PartialEq, Eq)]
pub enum Refused {
    /// No party is on in that channel.
    NoParty,
    /// Not yours to do (only the host, or everyone while "let everyone control" is on).
    NotAllowed,
    /// A party is on already, and you may not change its video.
    AlreadyOn,
    /// The request itself is wrong (a bad video id, a full queue, someone not in the call).
    Bad(&'static str),
}

/// What a change did.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    /// The party as it is now.
    Changed(WatchParty),
    /// Nothing changed (a join by a viewer, a leave by someone not watching).
    Unchanged,
    /// The party ended.
    Ended,
}

/// Every voice channel's watch party.
#[derive(Clone, Default)]
pub struct WatchState {
    parties: Arc<Mutex<HashMap<ChannelId, WatchParty>>>,
}

impl WatchState {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<ChannelId, WatchParty>> {
        self.parties.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The party on in `channel`, if any.
    pub fn of(&self, channel: ChannelId) -> Option<WatchParty> {
        self.lock().get(&channel).cloned()
    }

    /// Every party on in a channel in `visible`.
    pub fn visible(&self, visible: &HashSet<ChannelId>) -> Vec<WatchParty> {
        self.lock()
            .iter()
            .filter(|(c, _)| visible.contains(c))
            .map(|(_, p)| p.clone())
            .collect()
    }

    /// `me` starts a party of `video_id` in `channel` at `now_ms`, as its host and first viewer. With one on already,
    /// this changes its video, when `me` may control it.
    pub fn start(
        &self,
        channel: ChannelId,
        me: UserId,
        video_id: &str,
        title: Option<String>,
        now_ms: i64,
    ) -> Result<WatchParty, Refused> {
        if !valid_video_id(video_id) {
            return Err(Refused::Bad("that isn't a YouTube video"));
        }
        let mut map = self.lock();
        if let Some(p) = map.get_mut(&channel) {
            if !may_control(p, me) {
                return Err(Refused::AlreadyOn);
            }
            load(p, video_id, clean_title(title), me, now_ms);
            if !p.viewers.contains(&me) {
                p.viewers.push(me);
            }
            return Ok(p.clone());
        }
        let p = WatchParty {
            channel_id: channel,
            host_id: me,
            video_id: video_id.to_owned(),
            title: clean_title(title),
            position_ms: 0,
            playing: true,
            set_at_ms: now_ms,
            everyone_controls: false,
            viewers: vec![me],
            queue: Vec::new(),
            added_by: Some(me),
        };
        map.insert(channel, p.clone());
        Ok(p)
    }

    /// `me` makes one change to `channel`'s party at `now_ms`. `in_call` tells whether someone is in the channel's
    /// call (a hand-off's target must be).
    pub fn apply(
        &self,
        channel: ChannelId,
        me: UserId,
        action: WatchPartyAction,
        now_ms: i64,
        in_call: impl Fn(UserId) -> bool,
    ) -> Result<Outcome, Refused> {
        let mut map = self.lock();
        let p = map.get_mut(&channel).ok_or(Refused::NoParty)?;
        let control = may_control(p, me);
        let host = p.host_id == me;
        match action {
            WatchPartyAction::Join => {
                if p.viewers.contains(&me) {
                    return Ok(Outcome::Unchanged);
                }
                p.viewers.push(me);
            }
            WatchPartyAction::Leave => {
                return Ok(leave(&mut map, channel, me));
            }
            WatchPartyAction::Play { position_ms } => {
                if !control {
                    return Err(Refused::NotAllowed);
                }
                set(p, position_ms, true, now_ms);
            }
            WatchPartyAction::Pause { position_ms } => {
                if !control {
                    return Err(Refused::NotAllowed);
                }
                set(p, position_ms, false, now_ms);
            }
            WatchPartyAction::Seek { position_ms } => {
                if !control {
                    return Err(Refused::NotAllowed);
                }
                let playing = p.playing;
                set(p, position_ms, playing, now_ms);
            }
            WatchPartyAction::Load { video_id, title } => {
                if !control {
                    return Err(Refused::NotAllowed);
                }
                if !valid_video_id(&video_id) {
                    return Err(Refused::Bad("that isn't a YouTube video"));
                }
                load(p, &video_id, clean_title(title), me, now_ms);
            }
            // anyone in the call (the route checks that) may add one (#464)
            WatchPartyAction::Enqueue { video_id, title } => {
                if !valid_video_id(&video_id) {
                    return Err(Refused::Bad("that isn't a YouTube video"));
                }
                if p.queue.len() >= MAX_QUEUE {
                    return Err(Refused::Bad("the queue is full"));
                }
                p.queue.push(WatchPartyItem {
                    video_id,
                    title: clean_title(title),
                    added_by: me,
                    thumb: None,
                });
            }
            WatchPartyAction::Unqueue { index } => {
                let i = usize::try_from(index).unwrap_or(usize::MAX);
                let Some(item) = p.queue.get(i) else {
                    return Err(Refused::Bad("that isn't in the queue"));
                };
                if !control && item.added_by != me {
                    return Err(Refused::NotAllowed);
                }
                p.queue.remove(i);
            }
            WatchPartyAction::Move { index, to } => {
                let i = usize::try_from(index).unwrap_or(usize::MAX);
                let Some(item) = p.queue.get(i) else {
                    return Err(Refused::Bad("that isn't in the queue"));
                };
                if !control && item.added_by != me {
                    return Err(Refused::NotAllowed);
                }
                let to = usize::try_from(to)
                    .unwrap_or(usize::MAX)
                    .min(p.queue.len() - 1);
                if to == i {
                    return Ok(Outcome::Unchanged);
                }
                let item = p.queue.remove(i);
                p.queue.insert(to, item);
            }
            WatchPartyAction::PlayNow { index } => {
                if !control {
                    return Err(Refused::NotAllowed);
                }
                let i = usize::try_from(index).unwrap_or(usize::MAX);
                if i >= p.queue.len() {
                    return Err(Refused::Bad("that isn't in the queue"));
                }
                let item = p.queue.remove(i);
                play_item(p, item, now_ms);
            }
            WatchPartyAction::Next { from } => {
                if !control && p.added_by != Some(me) {
                    return Err(Refused::NotAllowed);
                }
                // someone else's player (or skip) moved it on already
                if from.is_some_and(|f| f != p.video_id) {
                    return Ok(Outcome::Unchanged);
                }
                if p.queue.is_empty() {
                    // nothing next: it stops where it is (the end, when the video ran out)
                    let at = position_at(p, now_ms);
                    set(p, at, false, now_ms);
                } else {
                    let item = p.queue.remove(0);
                    play_item(p, item, now_ms);
                }
            }
            WatchPartyAction::EveryoneControls { on } => {
                if !host {
                    return Err(Refused::NotAllowed);
                }
                if p.everyone_controls == on {
                    return Ok(Outcome::Unchanged);
                }
                p.everyone_controls = on;
            }
            WatchPartyAction::HandOff { user_id } => {
                if !host {
                    return Err(Refused::NotAllowed);
                }
                if user_id == me {
                    return Ok(Outcome::Unchanged);
                }
                if !in_call(user_id) {
                    return Err(Refused::Bad("they aren't in the call"));
                }
                p.host_id = user_id;
                // the host watches: their client opens it (or, with watch parties off, leaves it, passing it on)
                if !p.viewers.contains(&user_id) {
                    p.viewers.push(user_id);
                }
            }
        }
        Ok(Outcome::Changed(p.clone()))
    }

    /// `user` left `channel`'s call: they're out of its party, as with a leave.
    pub fn left_call(&self, channel: ChannelId, user: UserId) -> Outcome {
        let mut map = self.lock();
        if !map.contains_key(&channel) {
            return Outcome::Unchanged;
        }
        leave(&mut map, channel, user)
    }

    /// Give `video_id`'s card (#464) to the party in `channel`, wherever it lacks one: the current video's title, and
    /// each queued copy's title and thumbnail. `Unchanged` when there was nothing to fill.
    pub fn fill_card(
        &self,
        channel: ChannelId,
        video_id: &str,
        title: Option<&str>,
        thumb: Option<&EmbedThumb>,
    ) -> Outcome {
        let mut map = self.lock();
        let Some(p) = map.get_mut(&channel) else {
            return Outcome::Unchanged;
        };
        let title = clean_title(title.map(str::to_owned));
        let mut changed = false;
        if p.video_id == video_id && p.title.is_none() && title.is_some() {
            p.title.clone_from(&title);
            changed = true;
        }
        for item in p.queue.iter_mut().filter(|i| i.video_id == video_id) {
            if item.title.is_none() && title.is_some() {
                item.title.clone_from(&title);
                changed = true;
            }
            if item.thumb.is_none()
                && let Some(t) = thumb
            {
                item.thumb = Some(t.clone());
                changed = true;
            }
        }
        if changed {
            Outcome::Changed(p.clone())
        } else {
            Outcome::Unchanged
        }
    }

    /// Put back a party restored from its snapshot (#409).
    fn put(&self, party: WatchParty) {
        self.lock().insert(party.channel_id, party);
    }

    /// End everything in `channel`; whether a party was on.
    pub fn remove_channel(&self, channel: ChannelId) -> bool {
        self.lock().remove(&channel).is_some()
    }
}

/// `user` left `channel`'s call: they're out of its party (a host passes it on, the last viewer ends it).
pub async fn left_call(s: &crate::AppState, channel: ChannelId, user: UserId) {
    let _order = s.hub.in_order().await;
    let outcome = s.voice.watch.left_call(channel, user);
    routes::announce(s, channel, outcome).await;
}

/// Keep `channel`'s snapshot in step with what a change did: the party as it is now, or none once it ended.
pub(crate) async fn snapshot(
    db: &sqlx::SqlitePool,
    channel: ChannelId,
    outcome: &Outcome,
    now_ms: i64,
) {
    let r = match outcome {
        Outcome::Unchanged => return,
        Outcome::Changed(party) => {
            let Ok(json) = serde_json::to_string(party) else {
                return;
            };
            sqlx::query(
                "INSERT INTO watch_party_snapshots (channel_id, party, saved_at_ms) VALUES (?, ?, ?) \
                 ON CONFLICT (channel_id) DO UPDATE SET party = excluded.party, saved_at_ms = excluded.saved_at_ms",
            )
            .bind(channel.to_string())
            .bind(json)
            .bind(now_ms)
            .execute(db)
            .await
        }
        Outcome::Ended => {
            sqlx::query("DELETE FROM watch_party_snapshots WHERE channel_id = ?")
                .bind(channel.to_string())
                .execute(db)
                .await
        }
    };
    if let Err(e) = r {
        tracing::warn!(error = %e, %channel, "couldn't snapshot a watch party");
    }
}

/// `party` as restored at boot: only the viewers in `members` (in their order), the host passed on if they're gone;
/// `None` when nobody is left.
pub fn restored(mut party: WatchParty, members: &HashSet<UserId>) -> Option<WatchParty> {
    party.viewers.retain(|v| members.contains(v));
    let first = *party.viewers.first()?;
    if !party.viewers.contains(&party.host_id) {
        party.host_id = first;
    }
    Some(party)
}

/// Boot, once voice presence is back: pick up the parties a previous run snapshotted (#409). A snapshot older than
/// [`RESTORE_WITHIN_MS`], or with nobody from it still in the call, is dropped. Each restored party is announced (a
/// client that connected before this catches up; a later one gets it in Ready). Returns how many came back.
pub async fn restore(
    db: &sqlx::SqlitePool,
    hub: &crate::gateway::hub::Hub,
    voice: &crate::voice::VoiceState,
    now_ms: i64,
) -> anyhow::Result<usize> {
    let rows: Vec<(String, String, i64)> =
        sqlx::query_as("SELECT channel_id, party, saved_at_ms FROM watch_party_snapshots")
            .fetch_all(db)
            .await?;
    let mut back = 0;
    for (id, json, saved_at) in rows {
        let Ok(channel) = id.parse::<ChannelId>() else {
            continue;
        };
        let party = (now_ms.saturating_sub(saved_at) <= RESTORE_WITHIN_MS)
            .then(|| serde_json::from_str::<WatchParty>(&json).ok())
            .flatten()
            .filter(|p| p.channel_id == channel && voice.watch.of(channel).is_none())
            .and_then(|p| restored(p, &voice.members(channel)));
        let Some(party) = party else {
            sqlx::query("DELETE FROM watch_party_snapshots WHERE channel_id = ?")
                .bind(channel.to_string())
                .execute(db)
                .await?;
            continue;
        };
        voice.watch.put(party.clone());
        snapshot(db, channel, &Outcome::Changed(party.clone()), now_ms).await;
        hub.publish(
            db,
            pulse_protocol::gateway::Event::WatchPartyChanged {
                party,
                server_now_ms: now_ms,
            },
        )
        .await;
        back += 1;
    }
    Ok(back)
}

/// `channel` was deleted: its party goes. No event: clients drop the channel's state on `ChannelDeleted`.
pub fn channel_gone(s: &crate::AppState, channel: ChannelId) {
    s.voice.watch.remove_channel(channel);
}

/// Whether `me` may play, pause, seek and change `p`'s video.
fn may_control(p: &WatchParty, me: UserId) -> bool {
    p.host_id == me || (p.everyone_controls && p.viewers.contains(&me))
}

fn set(p: &mut WatchParty, position_ms: u64, playing: bool, now_ms: i64) {
    p.position_ms = position_ms.min(MAX_POSITION_MS);
    p.playing = playing;
    p.set_at_ms = now_ms;
}

fn load(p: &mut WatchParty, video_id: &str, title: Option<String>, by: UserId, now_ms: i64) {
    video_id.clone_into(&mut p.video_id);
    p.title = title;
    p.added_by = Some(by);
    set(p, 0, true, now_ms);
}

/// A queued video on now, from the start, as its adder's.
fn play_item(p: &mut WatchParty, item: WatchPartyItem, now_ms: i64) {
    load(p, &item.video_id, item.title, item.added_by, now_ms);
}

/// `user` out of `channel`'s party: a host passes it to the next viewer, and the last viewer out ends it.
fn leave(map: &mut HashMap<ChannelId, WatchParty>, channel: ChannelId, user: UserId) -> Outcome {
    let Some(p) = map.get_mut(&channel) else {
        return Outcome::Unchanged;
    };
    let was_viewer = p.viewers.contains(&user);
    p.viewers.retain(|v| *v != user);
    if p.viewers.is_empty() {
        map.remove(&channel);
        return Outcome::Ended;
    }
    if p.host_id == user {
        p.host_id = p.viewers[0];
    } else if !was_viewer {
        return Outcome::Unchanged;
    }
    Outcome::Changed(p.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    const V: &str = "dQw4w9WgXcQ";
    const W: &str = "9bZkp7q19f0";

    fn changed(o: Outcome) -> WatchParty {
        match o {
            Outcome::Changed(p) => p,
            o => panic!("{o:?}"),
        }
    }

    #[test]
    fn video_ids_are_strict() {
        assert!(valid_video_id(V));
        assert!(valid_video_id("a-b_c-d_e-f"));
        for bad in [
            "",
            "dQw4w9WgXc",
            "dQw4w9WgXcQQ",
            "dQw4w9WgXc!",
            "dQw4w9WgX c",
            "../../../ab",
            "dQw4w9WgXc\u{e9}",
        ] {
            assert!(!valid_video_id(bad), "{bad}");
        }
    }

    #[test]
    fn titles_are_cleaned() {
        assert_eq!(clean_title(None), None);
        assert_eq!(clean_title(Some("  \n ".into())), None);
        assert_eq!(clean_title(Some(" a\u{7}b ".into())).as_deref(), Some("ab"));
        assert_eq!(
            clean_title(Some("x".repeat(500))).map(|t| t.chars().count()),
            Some(MAX_TITLE)
        );
    }

    #[test]
    fn the_position_runs_on_while_playing() {
        let st = WatchState::default();
        let (ch, alex) = (ChannelId::new(), UserId::new());
        let p = st.start(ch, alex, V, None, 1_000).unwrap();
        assert_eq!(position_at(&p, 1_000), 0);
        assert_eq!(position_at(&p, 4_500), 3_500);
        // a clock that went back never makes it negative
        assert_eq!(position_at(&p, 0), 0);
        let p = changed(
            st.apply(
                ch,
                alex,
                WatchPartyAction::Pause { position_ms: 3_000 },
                5_000,
                |_| true,
            )
            .unwrap(),
        );
        assert!(!p.playing);
        assert_eq!(position_at(&p, 99_000), 3_000);
        let p = changed(
            st.apply(
                ch,
                alex,
                WatchPartyAction::Seek {
                    position_ms: u64::MAX,
                },
                6_000,
                |_| true,
            )
            .unwrap(),
        );
        assert_eq!(p.position_ms, MAX_POSITION_MS);
        assert!(!p.playing, "a seek keeps it paused");
    }

    #[test]
    fn only_the_host_controls_until_everyone_may() {
        let st = WatchState::default();
        let (ch, alex, sam) = (ChannelId::new(), UserId::new(), UserId::new());
        st.start(ch, alex, V, Some("Song".into()), 0).unwrap();
        assert_eq!(st.start(ch, sam, W, None, 0), Err(Refused::AlreadyOn));
        let play = || WatchPartyAction::Play { position_ms: 10 };
        assert_eq!(
            st.apply(ch, sam, play(), 1, |_| true),
            Err(Refused::NotAllowed)
        );
        changed(
            st.apply(ch, sam, WatchPartyAction::Join, 1, |_| true)
                .unwrap(),
        );
        assert_eq!(
            st.apply(ch, sam, WatchPartyAction::Join, 1, |_| true),
            Ok(Outcome::Unchanged)
        );
        assert_eq!(
            st.apply(
                ch,
                sam,
                WatchPartyAction::EveryoneControls { on: true },
                1,
                |_| true
            ),
            Err(Refused::NotAllowed)
        );
        changed(
            st.apply(
                ch,
                alex,
                WatchPartyAction::EveryoneControls { on: true },
                1,
                |_| true,
            )
            .unwrap(),
        );
        changed(st.apply(ch, sam, play(), 2, |_| true).unwrap());
        // and now sam may change the video by starting one
        let p = st.start(ch, sam, W, None, 3).unwrap();
        assert_eq!((p.video_id.as_str(), p.host_id, p.title), (W, alex, None));
        // someone who isn't watching still can't, everyone or not
        let kim = UserId::new();
        assert_eq!(
            st.apply(ch, kim, play(), 4, |_| true),
            Err(Refused::NotAllowed)
        );
    }

    #[test]
    fn bad_video_ids_are_refused() {
        let st = WatchState::default();
        let (ch, alex) = (ChannelId::new(), UserId::new());
        assert!(matches!(
            st.start(ch, alex, "nope", None, 0),
            Err(Refused::Bad(_))
        ));
        assert!(st.of(ch).is_none());
        st.start(ch, alex, V, None, 0).unwrap();
        for a in [
            WatchPartyAction::Load {
                video_id: "x\"><script>".into(),
                title: None,
            },
            WatchPartyAction::Enqueue {
                video_id: "short".into(),
                title: None,
            },
        ] {
            assert!(matches!(
                st.apply(ch, alex, a, 0, |_| true),
                Err(Refused::Bad(_))
            ));
        }
    }

    #[test]
    fn the_queue_plays_in_order() {
        let st = WatchState::default();
        let (ch, alex, sam) = (ChannelId::new(), UserId::new(), UserId::new());
        st.start(ch, alex, V, None, 0).unwrap();
        st.apply(ch, sam, WatchPartyAction::Join, 0, |_| true)
            .unwrap();
        let enqueue = |id: &str| WatchPartyAction::Enqueue {
            video_id: id.into(),
            title: Some("Next".into()),
        };
        st.apply(ch, sam, enqueue(W), 0, |_| true).unwrap();
        let p = changed(st.apply(ch, alex, enqueue(V), 0, |_| true).unwrap());
        assert_eq!(p.queue.len(), 2);
        // sam may take out their own, not alex's
        assert_eq!(
            st.apply(ch, sam, WatchPartyAction::Unqueue { index: 1 }, 0, |_| true),
            Err(Refused::NotAllowed)
        );
        assert!(matches!(
            st.apply(ch, sam, WatchPartyAction::Unqueue { index: 9 }, 0, |_| true),
            Err(Refused::Bad(_))
        ));
        assert_eq!(
            st.apply(ch, sam, WatchPartyAction::Next { from: None }, 0, |_| true),
            Err(Refused::NotAllowed)
        );
        let p = changed(
            st.apply(
                ch,
                alex,
                WatchPartyAction::Next { from: None },
                5_000,
                |_| true,
            )
            .unwrap(),
        );
        assert_eq!(
            (p.video_id.as_str(), p.position_ms, p.playing, p.set_at_ms),
            (W, 0, true, 5_000)
        );
        assert_eq!(p.title.as_deref(), Some("Next"));
        let p = changed(
            st.apply(ch, alex, WatchPartyAction::Unqueue { index: 0 }, 0, |_| {
                true
            })
            .unwrap(),
        );
        assert!(p.queue.is_empty());
        // nothing next: it stops where it got to
        let p = changed(
            st.apply(
                ch,
                alex,
                WatchPartyAction::Next { from: None },
                8_000,
                |_| true,
            )
            .unwrap(),
        );
        assert_eq!(
            (p.video_id.as_str(), p.playing, p.position_ms),
            (W, false, 3_000)
        );
        for _ in 0..MAX_QUEUE {
            st.apply(ch, alex, enqueue(V), 0, |_| true).unwrap();
        }
        assert_eq!(
            st.apply(ch, alex, enqueue(V), 0, |_| true),
            Err(Refused::Bad("the queue is full"))
        );
    }

    #[test]
    fn anyone_in_the_call_adds_and_adders_move_their_own() {
        let st = WatchState::default();
        let (ch, alex, sam, kim) = (
            ChannelId::new(),
            UserId::new(),
            UserId::new(),
            UserId::new(),
        );
        st.start(ch, alex, V, None, 0).unwrap();
        let enqueue = |id: &str| WatchPartyAction::Enqueue {
            video_id: id.into(),
            title: None,
        };
        // kim is in the call but hasn't opened the player: may still add (#464)
        st.apply(ch, kim, enqueue(W), 0, |_| true).unwrap();
        st.apply(ch, alex, enqueue(V), 0, |_| true).unwrap();
        let p = changed(st.apply(ch, sam, enqueue(W), 0, |_| true).unwrap());
        assert_eq!(
            p.queue.iter().map(|i| i.added_by).collect::<Vec<_>>(),
            [kim, alex, sam]
        );
        let mv = |index, to| WatchPartyAction::Move { index, to };
        // sam moves theirs to the front; may not move alex's
        assert_eq!(
            st.apply(ch, sam, mv(1, 0), 0, |_| true),
            Err(Refused::NotAllowed)
        );
        let p = changed(st.apply(ch, sam, mv(2, 0), 0, |_| true).unwrap());
        assert_eq!(
            p.queue.iter().map(|i| i.added_by).collect::<Vec<_>>(),
            [sam, kim, alex]
        );
        // the host moves anyone's; past the end is the end; to where it is changes nothing
        let p = changed(st.apply(ch, alex, mv(0, 99), 0, |_| true).unwrap());
        assert_eq!(
            p.queue.iter().map(|i| i.added_by).collect::<Vec<_>>(),
            [kim, alex, sam]
        );
        assert_eq!(
            st.apply(ch, alex, mv(2, 7), 0, |_| true),
            Ok(Outcome::Unchanged)
        );
        assert!(matches!(
            st.apply(ch, alex, mv(3, 0), 0, |_| true),
            Err(Refused::Bad(_))
        ));
        // play now: controllers only, and it's the adder's video now
        assert_eq!(
            st.apply(ch, sam, WatchPartyAction::PlayNow { index: 2 }, 0, |_| true),
            Err(Refused::NotAllowed)
        );
        let p = changed(
            st.apply(
                ch,
                alex,
                WatchPartyAction::PlayNow { index: 2 },
                7_000,
                |_| true,
            )
            .unwrap(),
        );
        assert_eq!(
            (p.video_id.as_str(), p.added_by, p.queue.len(), p.set_at_ms),
            (W, Some(sam), 2, 7_000)
        );
    }

    #[test]
    fn the_adder_skips_their_own_and_a_stale_next_does_nothing() {
        let st = WatchState::default();
        let (ch, alex, sam) = (ChannelId::new(), UserId::new(), UserId::new());
        let p = st.start(ch, alex, V, None, 0).unwrap();
        assert_eq!(p.added_by, Some(alex));
        let enqueue = |id: &str| WatchPartyAction::Enqueue {
            video_id: id.into(),
            title: None,
        };
        st.apply(ch, sam, enqueue(W), 0, |_| true).unwrap();
        st.apply(ch, alex, enqueue(V), 0, |_| true).unwrap();
        // two players reach the end of V at once: it moves on once
        let next = |from: &str| WatchPartyAction::Next {
            from: Some(from.into()),
        };
        let p = changed(st.apply(ch, alex, next(V), 0, |_| true).unwrap());
        assert_eq!((p.video_id.as_str(), p.added_by), (W, Some(sam)));
        assert_eq!(
            st.apply(ch, alex, next(V), 0, |_| true),
            Ok(Outcome::Unchanged)
        );
        // W is sam's: sam may skip it, though sam doesn't control the party
        let p = changed(st.apply(ch, sam, next(W), 0, |_| true).unwrap());
        assert_eq!((p.video_id.as_str(), p.added_by), (V, Some(alex)));
        assert_eq!(
            st.apply(ch, sam, next(V), 0, |_| true),
            Err(Refused::NotAllowed)
        );
        // the wire form without `from` still reads
        let a: WatchPartyAction = serde_json::from_str(r#"{"action":"next"}"#).unwrap();
        assert_eq!(a, WatchPartyAction::Next { from: None });
    }

    #[test]
    fn a_card_fills_in_what_is_missing() {
        let st = WatchState::default();
        let (ch, alex) = (ChannelId::new(), UserId::new());
        st.start(ch, alex, V, None, 0).unwrap();
        st.apply(
            ch,
            alex,
            WatchPartyAction::Enqueue {
                video_id: W.into(),
                title: Some("Mine".into()),
            },
            0,
            |_| true,
        )
        .unwrap();
        let thumb = EmbedThumb {
            hash: "h".into(),
            width: 1,
            height: 1,
        };
        let p = changed(st.fill_card(ch, V, Some("Song"), Some(&thumb)));
        assert_eq!(p.title.as_deref(), Some("Song"));
        // the queued W keeps its own title, takes the thumbnail
        let p = changed(st.fill_card(ch, W, Some("Other"), Some(&thumb)));
        assert_eq!(p.queue[0].title.as_deref(), Some("Mine"));
        assert_eq!(p.queue[0].thumb, Some(thumb.clone()));
        assert_eq!(
            st.fill_card(ch, W, Some("x"), Some(&thumb)),
            Outcome::Unchanged
        );
        assert_eq!(
            st.fill_card(ChannelId::new(), W, None, None),
            Outcome::Unchanged
        );
    }

    #[test]
    fn the_host_hands_off_and_leaving_passes_it_on() {
        let st = WatchState::default();
        let (ch, alex, sam, kim, jo) = (
            ChannelId::new(),
            UserId::new(),
            UserId::new(),
            UserId::new(),
            UserId::new(),
        );
        st.start(ch, alex, V, None, 0).unwrap();
        st.apply(ch, sam, WatchPartyAction::Join, 0, |_| true)
            .unwrap();
        st.apply(ch, kim, WatchPartyAction::Join, 0, |_| true)
            .unwrap();
        // only to someone in the call, and only by the host
        assert!(matches!(
            st.apply(
                ch,
                alex,
                WatchPartyAction::HandOff { user_id: jo },
                0,
                |u| u != jo
            ),
            Err(Refused::Bad(_))
        ));
        assert_eq!(
            st.apply(
                ch,
                sam,
                WatchPartyAction::HandOff { user_id: sam },
                0,
                |_| true
            ),
            Err(Refused::NotAllowed)
        );
        // jo isn't watching yet: becoming host opens it for them
        let p = changed(
            st.apply(
                ch,
                alex,
                WatchPartyAction::HandOff { user_id: jo },
                0,
                |_| true,
            )
            .unwrap(),
        );
        assert_eq!(p.host_id, jo);
        assert_eq!(p.viewers, [alex, sam, kim, jo]);
        // the host leaves the party (staying in the call): the next viewer in join order takes over
        let p = changed(
            st.apply(ch, jo, WatchPartyAction::Leave, 0, |_| true)
                .unwrap(),
        );
        assert_eq!(p.host_id, alex);
        // a viewer leaving the call: just out
        let p = changed(st.left_call(ch, sam));
        assert_eq!((p.host_id, p.viewers.clone()), (alex, vec![alex, kim]));
        // someone who wasn't watching: nothing to say
        assert_eq!(st.left_call(ch, sam), Outcome::Unchanged);
        assert_eq!(st.left_call(ChannelId::new(), sam), Outcome::Unchanged);
        let p = changed(st.left_call(ch, alex));
        assert_eq!(p.host_id, kim);
        // the last one out ends it
        assert_eq!(st.left_call(ch, kim), Outcome::Ended);
        assert!(st.of(ch).is_none());
        assert_eq!(
            st.apply(ch, kim, WatchPartyAction::Join, 0, |_| true),
            Err(Refused::NoParty)
        );
    }

    #[test]
    fn visible_and_removed_by_channel() {
        let st = WatchState::default();
        let (a, b, alex) = (ChannelId::new(), ChannelId::new(), UserId::new());
        st.start(a, alex, V, None, 0).unwrap();
        st.start(b, alex, W, None, 0).unwrap();
        assert_eq!(st.visible(&[a].into_iter().collect()).len(), 1);
        assert!(st.remove_channel(a));
        assert!(!st.remove_channel(a));
        assert!(st.visible(&[a].into_iter().collect()).is_empty());
    }
}
