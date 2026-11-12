//! Spotify on a voice channel's stage (#234): someone in a server voice call shares a Spotify link (a Jam invite, a
//! track, album, playlist, episode, show or artist), and everyone sees its card on the stage until it's stopped, its
//! sharer leaves the call, or it runs out.
//!
//! In memory only, like the voice state it hangs off: a share is as ephemeral as the call it belongs to. A restart
//! drops it (the sharer shares again); keeping it in a table would mean reconciling it with the call LiveKit restores
//! after a restart, for little gain. The card comes from the embed resolver (`embeds::Unfurler`), as a chat link's
//! does: no Spotify API keys, nothing that controls anyone's Spotify (#196).
//!
//! The owner's open choices are the constants below, in one place.

pub mod routes;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ChannelId, UserId};
use pulse_protocol::rest::{EmbedProvider, Listening, StageShare};
use reqwest::Url;

use crate::AppState;
use crate::embeds::{links, providers};

/// How many shares a stage shows at once (pick: one per channel). A new share replaces the sharer's own, then the
/// oldest while the stage is full.
pub const MAX_PER_CHANNEL: usize = 1;

/// The longest a share stays up (pick: 3 hours); `Config::stage_share_ttl` defaults to it.
pub const SHARE_TTL: Duration = Duration::from_secs(3 * 3600);

/// Whether a share goes when its sharer leaves the call (pick: yes).
pub const ENDS_WHEN_SHARER_LEAVES: bool = true;

/// Whether anyone in the call may stop a share for everyone, not just its sharer (pick: yes, so a stale Jam doesn't
/// need its sharer).
pub const ANYONE_IN_CALL_MAY_STOP: bool = true;

/// Every voice channel's shares, oldest first.
#[derive(Clone, Default)]
pub struct StageState {
    shares: Arc<Mutex<HashMap<ChannelId, Vec<StageShare>>>>,
}

impl StageState {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<ChannelId, Vec<StageShare>>> {
        self.shares.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The shares up in `channel`, oldest first.
    pub fn of(&self, channel: ChannelId) -> Vec<StageShare> {
        self.lock().get(&channel).cloned().unwrap_or_default()
    }

    /// Every share up in a channel in `visible`.
    pub fn visible(&self, visible: &HashSet<ChannelId>) -> Vec<StageShare> {
        self.lock()
            .iter()
            .filter(|(c, _)| visible.contains(c))
            .flat_map(|(_, s)| s.iter().cloned())
            .collect()
    }

    /// `user`'s share of `url` in `channel`, if it's up: the same link, whatever its query (Spotify's `?si=`
    /// changes with every copy of a link).
    pub fn same(&self, channel: ChannelId, user: UserId, url: &str) -> Option<StageShare> {
        let bare = |u: &str| u.split('?').next().unwrap_or(u).to_owned();
        self.of(channel)
            .into_iter()
            .find(|s| s.shared_by == user && bare(&s.embed.url) == bare(url))
    }

    /// Put `share` up; returns the shares it pushed out (the sharer's own, then the oldest past
    /// [`MAX_PER_CHANNEL`]).
    pub fn put(&self, share: StageShare) -> Vec<StageShare> {
        let mut map = self.lock();
        let list = map.entry(share.channel_id).or_default();
        let mut out = Vec::new();
        list.retain(|s| {
            let keep = s.shared_by != share.shared_by;
            if !keep {
                out.push(s.clone());
            }
            keep
        });
        while list.len() >= MAX_PER_CHANNEL.max(1) {
            out.push(list.remove(0));
        }
        list.push(share);
        out
    }

    /// Take down the share `id` in `channel`, if it's up.
    pub fn remove(&self, channel: ChannelId, id: &str) -> Option<StageShare> {
        let mut map = self.lock();
        let list = map.get_mut(&channel)?;
        let at = list.iter().position(|s| s.id == id)?;
        let gone = list.remove(at);
        if list.is_empty() {
            map.remove(&channel);
        }
        Some(gone)
    }

    /// Take down `user`'s shares in `channel`.
    pub fn remove_by(&self, channel: ChannelId, user: UserId) -> Vec<StageShare> {
        let mut map = self.lock();
        let Some(list) = map.get_mut(&channel) else {
            return Vec::new();
        };
        let (gone, kept): (Vec<_>, Vec<_>) = list.drain(..).partition(|s| s.shared_by == user);
        if kept.is_empty() {
            map.remove(&channel);
        } else {
            *list = kept;
        }
        gone
    }

    /// Every share `user` has up, anywhere.
    pub fn by(&self, user: UserId) -> Vec<StageShare> {
        self.lock()
            .values()
            .flat_map(|l| l.iter().filter(|s| s.shared_by == user).cloned())
            .collect()
    }

    /// Set the "Now playing" of the share `id` in `channel` (#275); whether it changed.
    pub fn set_now_playing(&self, channel: ChannelId, id: &str, now: Option<Listening>) -> bool {
        let mut map = self.lock();
        let Some(share) = map
            .get_mut(&channel)
            .and_then(|l| l.iter_mut().find(|s| s.id == id))
        else {
            return false;
        };
        if share.now_playing == now {
            return false;
        }
        share.now_playing = now;
        true
    }

    /// Take down everything in `channel`.
    pub fn remove_channel(&self, channel: ChannelId) -> Vec<StageShare> {
        self.lock().remove(&channel).unwrap_or_default()
    }
}

/// `raw` as the link a share stores, when it's a Spotify one: an `open.spotify.com` link (Jam invite or anything
/// #196 unfurls), a Jam's `spotify:socialsession:<id>`, or a `spotify.link` short link (followed by the resolver, which
/// checks where it leads). `None` for anything else, before anything is fetched.
pub fn spotify_link(raw: &str) -> Option<String> {
    let raw = raw.trim();
    // One link, nothing around it: the box takes a link, not a message.
    if raw.is_empty() || raw.chars().any(char::is_whitespace) {
        return None;
    }
    let link = links::extract(raw).into_iter().next()?;
    let url = Url::parse(&link).ok()?;
    match providers::detect(&url) {
        providers::Detection::Known(d) if d.provider == EmbedProvider::Spotify => Some(link),
        providers::Detection::Short if url.host_str() == Some("spotify.link") => Some(link),
        _ => None,
    }
}

/// Whether `share` is a Jam: only a Jam gets a live "Now playing" (#275).
pub fn jam(share: &StageShare) -> bool {
    share
        .embed
        .provider_id
        .as_deref()
        .is_some_and(|id| providers::is_jam(share.embed.provider, id))
}

/// What `user` is listening to changed (or they stopped reading, or disconnected): each Jam of theirs on a stage
/// shows it, whatever their status or "Show my activity" says (#275: they shared the Jam, in a call they're visibly
/// in). Only real changes go out.
pub async fn listening_changed(s: &AppState, user: UserId) {
    let _order = s.hub.in_order().await;
    let now = s.hub.listening_of(user, false);
    for share in s.voice.stage.by(user).into_iter().filter(jam) {
        if s.voice
            .stage
            .set_now_playing(share.channel_id, &share.id, now.clone())
        {
            s.hub
                .publish(
                    &s.db,
                    Event::StageNowPlaying {
                        channel_id: share.channel_id,
                        share_id: share.id,
                        now_playing: now.clone(),
                    },
                )
                .await;
        }
    }
}

/// Tell `channel` each of `gone` came down.
pub(crate) async fn announce_cleared(s: &AppState, gone: Vec<StageShare>) {
    for share in gone {
        s.hub
            .publish(
                &s.db,
                Event::StageShareCleared {
                    channel_id: share.channel_id,
                    share_id: share.id,
                },
            )
            .await;
    }
}

/// `user` left `channel`'s call: their shares there go (when [`ENDS_WHEN_SHARER_LEAVES`]).
pub async fn left_call(s: &AppState, channel: ChannelId, user: UserId) {
    if !ENDS_WHEN_SHARER_LEAVES {
        return;
    }
    let _order = s.hub.in_order().await;
    let gone = s.voice.stage.remove_by(channel, user);
    announce_cleared(s, gone).await;
}

/// `channel` was deleted: its shares go. No event: clients drop the channel's stage on `ChannelDeleted`, and events
/// scoped to a channel that can't be found reach nobody.
pub fn channel_gone(s: &AppState, channel: ChannelId) {
    s.voice.stage.remove_channel(channel);
}

#[cfg(test)]
mod tests {
    use super::*;
    use pulse_protocol::rest::{EmbedKind, LinkEmbed};

    fn share(id: &str, ch: ChannelId, by: UserId, url: &str) -> StageShare {
        StageShare {
            id: id.into(),
            channel_id: ch,
            shared_by: by,
            shared_at: String::new(),
            expires_at: String::new(),
            embed: LinkEmbed {
                url: url.into(),
                provider: EmbedProvider::Spotify,
                kind: EmbedKind::Card,
                provider_id: Some("track:x".into()),
                title: None,
                description: None,
                site_name: None,
                author: None,
                thumb: None,
                start: None,
                vertical: false,
            },
            now_playing: None,
        }
    }

    #[test]
    fn a_new_share_replaces_the_one_up() {
        let st = StageState::default();
        let (ch, alex, sam) = (ChannelId::new(), UserId::new(), UserId::new());
        assert!(st.put(share("1", ch, alex, "a")).is_empty());
        let out = st.put(share("2", ch, sam, "b"));
        assert_eq!(out.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["1"]);
        assert_eq!(st.of(ch).len(), 1);
        assert!(st.same(ch, sam, "b").is_some());
        assert!(st.same(ch, sam, "b?si=other").is_some());
        assert!(st.same(ch, alex, "a").is_none());
        assert!(st.remove(ch, "1").is_none());
        assert_eq!(st.remove(ch, "2").map(|s| s.id), Some("2".into()));
        assert!(st.of(ch).is_empty());
    }

    #[test]
    fn now_playing_only_on_change_and_only_jams_count() {
        let st = StageState::default();
        let (ch, alex) = (ChannelId::new(), UserId::new());
        let mut j = share(
            "1",
            ch,
            alex,
            "https://open.spotify.com/socialsession/abcdefgh",
        );
        j.embed.provider_id = Some("jam:abcdefgh".into());
        assert!(jam(&j));
        assert!(!jam(&share("2", ch, alex, "a")));
        st.put(j);
        let song = Listening {
            title: "Song".into(),
            artist: None,
            album: None,
            spotify_id: None,
            art: None,
        };
        assert_eq!(st.by(alex).len(), 1);
        assert!(st.set_now_playing(ch, "1", Some(song.clone())));
        assert!(
            !st.set_now_playing(ch, "1", Some(song.clone())),
            "no change"
        );
        assert_eq!(st.of(ch)[0].now_playing, Some(song));
        assert!(st.set_now_playing(ch, "1", None));
        assert!(!st.set_now_playing(ch, "nope", None));
        assert!(st.by(UserId::new()).is_empty());
    }

    #[test]
    fn remove_by_takes_only_that_persons() {
        let st = StageState::default();
        let (ch, other, alex, sam) = (
            ChannelId::new(),
            ChannelId::new(),
            UserId::new(),
            UserId::new(),
        );
        st.put(share("1", ch, alex, "a"));
        st.put(share("2", other, alex, "a"));
        assert!(st.remove_by(ch, sam).is_empty());
        assert_eq!(st.remove_by(ch, alex).len(), 1);
        assert_eq!(st.of(other).len(), 1);
        assert_eq!(st.remove_channel(other).len(), 1);
        assert!(st.visible(&[ch, other].into_iter().collect()).is_empty());
    }

    #[test]
    fn spotify_links_only() {
        let t = "https://open.spotify.com/track/4uLU6hMCjMI75M1A2tKUQC";
        assert_eq!(
            spotify_link(&format!("  {t}?si=abc ")).as_deref(),
            Some(&*format!("{t}?si=abc"))
        );
        assert_eq!(
            spotify_link("spotify:socialsession:ABC123abc123").as_deref(),
            Some("https://open.spotify.com/socialsession/ABC123abc123")
        );
        assert!(spotify_link("https://spotify.link/AbCd").is_some());
        assert!(spotify_link("https://open.spotify.com/socialsession/ABC123abc123").is_some());
        for bad in [
            "",
            "https://example.com/track/x",
            "https://open.spotify.com.example.com/track/4uLU6hMCjMI75M1A2tKUQC",
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://open.spotify.com/",
            "look https://open.spotify.com/track/4uLU6hMCjMI75M1A2tKUQC",
            "spotify:track:4uLU6hMCjMI75M1A2tKUQC",
            "ftp://open.spotify.com/track/4uLU6hMCjMI75M1A2tKUQC",
        ] {
            assert_eq!(spotify_link(bad), None, "{bad}");
        }
    }
}
