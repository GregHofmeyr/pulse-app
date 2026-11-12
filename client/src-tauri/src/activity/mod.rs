//! "Listening to…" (#275): what the local Spotify app is playing, read from the OS media session (MPRIS over D-Bus
//! on Linux, the system media transport controls on Windows), sent to the server as `ClientFrame::Listening`.
//!
//! Only while "Read media activity" is on ([`MediaActivity::set`]): off, there is no watcher at all (no D-Bus
//! connection, no thread, no task, nothing sent). On, the watcher is event-driven, never polling: it wakes on the
//! player's own change signals, filtered to Spotify's player. What it sees is settled ([`Settle`]) before it's sent:
//! a track change sends a few signals at once, a pause or stop clears only after a short delay, and sends are spaced.
//!
//! Spotify's rules (#196): this only reads what the OS already shows; nothing here controls anyone's Spotify.

#[cfg(windows)]
mod gsmtc;
#[cfg(target_os = "linux")]
pub mod mpris;
mod parse;

pub use parse::{MprisMeta, is_spotify_app, now_playing_gsmtc, now_playing_mpris, spotify_id};

use std::sync::{Arc, Mutex};

use pulse_protocol::gateway::ClientFrame;
use pulse_protocol::rest::ListeningUpdate;
use serde::Serialize;
use tokio::sync::mpsc;
use tokio::time::{Duration, Instant};

/// A track change sends several signals within moments (metadata, then status): what's seen is sent once it has
/// held this long.
pub const SETTLE: Duration = Duration::from_millis(750);
/// A pause or stop clears the activity only once it has lasted this long (a skip or a quick pause doesn't flicker).
pub const CLEAR_AFTER: Duration = Duration::from_secs(8);
/// The least time between two sends; the latest is sent when it's up (the server's limit is looser).
pub const MIN_GAP: Duration = Duration::from_secs(5);

/// What Spotify is playing, as the media session tells it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct NowPlaying {
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    /// `track:<id>` or `episode:<id>`, when the session gave one (MPRIS does; Windows' doesn't).
    pub spotify_id: Option<String>,
}

impl NowPlaying {
    fn update(&self) -> ListeningUpdate {
        ListeningUpdate {
            title: self.title.clone(),
            artist: self.artist.clone(),
            album: self.album.clone(),
            spotify_id: self.spotify_id.clone(),
        }
    }
}

/// What a watcher sees: what Spotify plays, or `None` (paused, stopped, closed, an ad, nothing).
pub type Seen = Option<NowPlaying>;

/// Decides when what's seen is sent (pure: the clock is passed in).
#[derive(Debug)]
pub struct Settle {
    /// What was last sent (the server holds it).
    sent: Seen,
    last_send: Option<Instant>,
    /// What's seen now, and since when.
    want: Seen,
    since: Instant,
}

impl Settle {
    pub fn new(now: Instant) -> Self {
        Settle {
            sent: None,
            last_send: None,
            want: None,
            since: now,
        }
    }

    /// The watcher saw `seen` at `now`.
    pub fn seen(&mut self, seen: Seen, now: Instant) {
        if seen != self.want {
            self.want = seen;
            self.since = now;
        }
    }

    /// When to send next, if something is waiting.
    pub fn due(&self) -> Option<Instant> {
        if self.want == self.sent {
            return None;
        }
        let hold = if self.want.is_some() {
            SETTLE
        } else {
            CLEAR_AFTER
        };
        let at = self.since + hold;
        Some(match self.last_send {
            Some(t) => at.max(t + MIN_GAP),
            None => at,
        })
    }

    /// What to send at `now`, if it's due.
    pub fn take(&mut self, now: Instant) -> Option<Seen> {
        if self.due()? > now {
            return None;
        }
        self.sent = self.want.clone();
        self.last_send = Some(now);
        Some(self.sent.clone())
    }
}

/// Feeds what `rx` sees through a [`Settle`] and calls `send` with each settled change. Ends when the watcher does
/// (its sender dropped). Sleeps only while a send is waiting.
pub async fn settle(mut rx: mpsc::UnboundedReceiver<Seen>, send: impl Fn(Seen)) {
    let mut st = Settle::new(Instant::now());
    loop {
        let due = st.due();
        tokio::select! {
            seen = rx.recv() => match seen {
                Some(s) => st.seen(s, Instant::now()),
                None => return,
            },
            _ = tokio::time::sleep_until(due.unwrap_or_else(Instant::now)), if due.is_some() => {
                if let Some(out) = st.take(Instant::now()) {
                    send(out);
                }
            }
        }
    }
}

/// A running watcher: dropping it stops it, and everything it holds (the D-Bus connection, the thread) goes.
pub struct Watcher {
    #[cfg(target_os = "linux")]
    task: tauri::async_runtime::JoinHandle<()>,
    #[cfg(windows)]
    _thread: gsmtc::Handle,
    settle: tauri::async_runtime::JoinHandle<()>,
}

impl Drop for Watcher {
    fn drop(&mut self) {
        #[cfg(target_os = "linux")]
        self.task.abort();
        self.settle.abort();
    }
}

/// Start watching Spotify's media session, calling `send` with each settled change. `None` where there's no media
/// session to read (macOS).
fn watch(send: Box<dyn Fn(Seen) + Send>) -> Option<Watcher> {
    #[cfg(any(target_os = "linux", windows))]
    {
        let (tx, rx) = mpsc::unbounded_channel();
        #[cfg(target_os = "linux")]
        let task = tauri::async_runtime::spawn(async move {
            if let Err(e) = mpris::watch(mpris::SPOTIFY, tx).await {
                tracing::info!(error = %e, "media activity: can't read the media session");
            }
        });
        #[cfg(windows)]
        let thread = gsmtc::spawn(tx);
        let settle = tauri::async_runtime::spawn(settle(rx, send));
        Some(Watcher {
            #[cfg(target_os = "linux")]
            task,
            #[cfg(windows)]
            _thread: thread,
            settle,
        })
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        let _ = send;
        None
    }
}

/// What Settings shows (`pulse://media-activity`): whether Pulse reads, and what it last read.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MediaState {
    pub reading: bool,
    pub playing: Option<NowPlaying>,
}

type SendFrame = Arc<dyn Fn(ClientFrame) + Send + Sync>;
type Notify = Arc<dyn Fn(MediaState) + Send + Sync>;
/// Starts a watcher that calls the given function with each settled change (the real one: [`watch`]).
type Start = Arc<dyn Fn(Box<dyn Fn(Seen) + Send>) -> Option<Watcher> + Send + Sync>;

#[derive(Default)]
struct Inner {
    read: bool,
    show: bool,
    /// What was last sent, while reading.
    playing: Seen,
    watcher: Option<Watcher>,
    /// Bumped on every start and stop: a settled change from a watcher already stopped is dropped.
    epoch: u64,
}

/// The two Settings switches (#275) and the watcher they decide. Managed Tauri state.
pub struct MediaActivity {
    inner: Arc<Mutex<Inner>>,
    send: SendFrame,
    notify: Notify,
    start: Start,
}

impl MediaActivity {
    pub fn new(send: SendFrame, notify: Notify) -> Self {
        Self::with_start(send, notify, Arc::new(watch))
    }

    fn with_start(send: SendFrame, notify: Notify, start: Start) -> Self {
        MediaActivity {
            inner: Arc::default(),
            send,
            notify,
            start,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// "Read media activity" (`read`) and "Show my activity" (`show`). Reading off stops and drops the watcher and
    /// clears what the server holds; on starts one if none runs. `show` changing re-sends what's playing with it.
    pub fn set(&self, read: bool, show: bool) {
        let mut g = self.lock();
        let was = (g.read, g.show);
        g.read = read;
        g.show = show;
        if !read {
            if g.watcher.take().is_some() {
                g.epoch += 1;
            }
            if g.playing.take().is_some() {
                (self.send)(frame(None, show));
            }
        } else if g.watcher.is_none() {
            g.epoch += 1;
            let epoch = g.epoch;
            let (inner, send, notify) =
                (self.inner.clone(), self.send.clone(), self.notify.clone());
            g.watcher = (self.start)(Box::new(move |seen: Seen| {
                let mut g = inner.lock().unwrap_or_else(|e| e.into_inner());
                if g.epoch != epoch || !g.read {
                    return;
                }
                g.playing = seen.clone();
                send(frame(seen.as_ref(), g.show));
                notify(MediaState {
                    reading: true,
                    playing: seen,
                });
            }));
        } else if was.1 != show && g.playing.is_some() {
            (self.send)(frame(g.playing.as_ref(), show));
        }
        if was.0 != read {
            (self.notify)(MediaState {
                reading: read,
                playing: g.playing.clone(),
            });
        }
    }

    /// Whether a watcher runs right now.
    pub fn watching(&self) -> bool {
        self.lock().watcher.is_some()
    }

    pub fn state(&self) -> MediaState {
        let g = self.lock();
        MediaState {
            reading: g.read,
            playing: g.playing.clone(),
        }
    }

    /// Sent after every Ready: the server forgets with the connection. Nothing when nothing plays.
    pub fn after_ready(&self) -> Option<ClientFrame> {
        let g = self.lock();
        g.playing
            .as_ref()
            .filter(|_| g.read)
            .map(|p| frame(Some(p), g.show))
    }
}

fn frame(playing: Option<&NowPlaying>, show: bool) -> ClientFrame {
    ClientFrame::Listening {
        listening: playing.map(NowPlaying::update),
        show,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(t: &str) -> NowPlaying {
        NowPlaying {
            title: t.into(),
            artist: Some("The Band".into()),
            album: None,
            spotify_id: None,
        }
    }

    #[test]
    fn a_new_song_goes_once_it_settles() {
        let t0 = Instant::now();
        let mut st = Settle::new(t0);
        assert_eq!(
            st.due(),
            None,
            "nothing playing, nothing sent: nothing to do"
        );
        st.seen(Some(song("A")), t0);
        // the status signal right after the metadata one doesn't restart the wait
        st.seen(Some(song("A")), t0 + Duration::from_millis(100));
        assert_eq!(st.due(), Some(t0 + SETTLE));
        assert_eq!(st.take(t0 + Duration::from_millis(500)), None);
        assert_eq!(st.take(t0 + SETTLE), Some(Some(song("A"))));
        assert_eq!(st.due(), None);
    }

    #[test]
    fn a_pause_clears_only_after_a_while_and_a_quick_one_not_at_all() {
        let t0 = Instant::now();
        let mut st = Settle::new(t0);
        st.seen(Some(song("A")), t0);
        st.take(t0 + SETTLE).unwrap();
        let p = t0 + Duration::from_secs(60);
        st.seen(None, p);
        assert_eq!(st.due(), Some(p + CLEAR_AFTER));
        // back within the delay: nothing to send
        st.seen(Some(song("A")), p + Duration::from_secs(3));
        assert_eq!(st.due(), None);
        st.seen(None, p + Duration::from_secs(4));
        assert_eq!(
            st.take(p + Duration::from_secs(4) + CLEAR_AFTER),
            Some(None)
        );
    }

    #[test]
    fn sends_are_spaced_and_the_latest_wins() {
        let t0 = Instant::now();
        let mut st = Settle::new(t0);
        st.seen(Some(song("A")), t0);
        st.take(t0 + SETTLE).unwrap();
        // skipping through B and C quickly: only C goes, MIN_GAP after A
        st.seen(Some(song("B")), t0 + Duration::from_secs(1));
        st.seen(Some(song("C")), t0 + Duration::from_secs(2));
        assert_eq!(st.due(), Some(t0 + SETTLE + MIN_GAP));
        assert_eq!(
            st.take(t0 + SETTLE + MIN_GAP),
            Some(Some(song("C"))),
            "B was never sent"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn the_settle_loop_sends_settled_changes_and_ends_with_its_watcher() {
        let (tx, rx) = mpsc::unbounded_channel();
        let got = Arc::new(Mutex::new(Vec::new()));
        let g = got.clone();
        let task = tokio::spawn(settle(rx, move |s| g.lock().unwrap().push(s)));
        tx.send(Some(song("A"))).unwrap();
        tokio::time::sleep(SETTLE + Duration::from_millis(10)).await;
        assert_eq!(*got.lock().unwrap(), vec![Some(song("A"))]);
        tx.send(None).unwrap();
        tokio::time::sleep(CLEAR_AFTER + Duration::from_millis(10)).await;
        assert_eq!(got.lock().unwrap().len(), 2);
        drop(tx);
        task.await.unwrap();
    }

    type Feed = Arc<Mutex<Option<Box<dyn Fn(Seen) + Send>>>>;
    type Faked = (
        MediaActivity,
        Arc<Mutex<Vec<ClientFrame>>>,
        Feed,
        Arc<Mutex<u32>>,
    );

    /// A `MediaActivity` whose watcher is a hand-fed function; the frames it sends; how many watchers started.
    fn fake() -> Faked {
        let frames = Arc::new(Mutex::new(Vec::new()));
        let f = frames.clone();
        let feed: Feed = Arc::default();
        let starts = Arc::new(Mutex::new(0));
        let (fd, st) = (feed.clone(), starts.clone());
        let m = MediaActivity::with_start(
            Arc::new(move |fr| f.lock().unwrap().push(fr)),
            Arc::new(|_| {}),
            Arc::new(move |send| {
                *st.lock().unwrap() += 1;
                *fd.lock().unwrap() = Some(send);
                // a watcher that does nothing on its own: its settle task is a no-op
                Some(Watcher {
                    #[cfg(target_os = "linux")]
                    task: tauri::async_runtime::spawn(async {}),
                    #[cfg(windows)]
                    _thread: gsmtc::Handle::none(),
                    settle: tauri::async_runtime::spawn(async {}),
                })
            }),
        );
        (m, frames, feed, starts)
    }

    fn listening(f: &ClientFrame) -> (Option<String>, bool) {
        match f {
            ClientFrame::Listening { listening, show } => {
                (listening.as_ref().map(|l| l.title.clone()), *show)
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn reading_off_means_no_watcher_at_all() {
        let (m, frames, _, starts) = fake();
        m.set(false, true);
        m.set(false, false);
        assert!(!m.watching());
        assert_eq!(*starts.lock().unwrap(), 0, "never started");
        assert!(frames.lock().unwrap().is_empty());
        assert_eq!(m.after_ready().map(|f| listening(&f)), None);
    }

    #[test]
    fn reading_sends_what_plays_and_stopping_clears_it() {
        let (m, frames, feed, starts) = fake();
        m.set(true, true);
        m.set(true, true);
        assert_eq!(*starts.lock().unwrap(), 1, "one watcher");
        (feed.lock().unwrap().as_ref().unwrap())(Some(song("A")));
        assert_eq!(
            listening(&frames.lock().unwrap()[0]),
            (Some("A".into()), true)
        );
        assert_eq!(
            m.after_ready().map(|f| listening(&f)),
            Some((Some("A".into()), true))
        );
        // Show off: the same song again, not to be shown
        m.set(true, false);
        assert_eq!(
            listening(&frames.lock().unwrap()[1]),
            (Some("A".into()), false)
        );
        // reading off: the watcher goes and the server is told nothing plays
        m.set(false, false);
        assert!(!m.watching());
        assert_eq!(listening(&frames.lock().unwrap()[2]), (None, false));
        assert_eq!(m.after_ready().map(|f| listening(&f)), None);
        // a late change from the stopped watcher is dropped
        (feed.lock().unwrap().as_ref().unwrap())(Some(song("B")));
        assert_eq!(frames.lock().unwrap().len(), 3);
        assert_eq!(m.state().playing, None);
    }
}
