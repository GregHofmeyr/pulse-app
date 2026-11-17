//! "Playing" (#380): the game this computer runs, sent to the server as `ClientFrame::Playing`. Windows only: Steam's
//! running app (`HKCU\Software\Valve\Steam\RunningAppID`, named from its manifest in whichever library has it) or an
//! exe added by hand in Settings. Elsewhere there's no source and nothing ever runs.
//!
//! Settings → Game activity ([`GameSettings`], all on by default): the master switch (off: no poller, nothing read,
//! nothing sent, and what the server holds is cleared), "Show game art" (off: the Steam app id isn't sent, so the
//! server has nothing to fetch art for) and "Show play time" (off: no elapsed time is sent). A hand-added game's exe
//! path never leaves this computer: only its name does. Polled every [`POLL`]; a frame goes only on a change.

pub mod clipdrop;
mod detect;
pub mod vdf;
#[cfg(windows)]
mod win;

pub use detect::{Detected, ManualGame, Source, SteamNames, detect, match_exe};

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pulse_protocol::gateway::ClientFrame;
use pulse_protocol::rest::PlayingUpdate;
use serde::{Deserialize, Serialize};

/// How often what runs is read (owner: cheap, every 15 s or so).
pub const POLL: Duration = Duration::from_secs(15);

/// The Settings → Game activity switches and the hand-added games, as the page saves them. Later switches are one more
/// field here and on the page, defaulting on like these. Clip drop isn't one: it's its own feature with its own switch
/// ([`clipdrop`], `set_clip_drop`), running with game activity off; an older page's `clips` fields are ignored.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GameSettings {
    /// The master switch.
    pub enabled: bool,
    /// "Show game art": send the Steam app id.
    pub art: bool,
    /// "Show play time": send how long it's been running.
    pub time: bool,
    pub games: Vec<ManualGame>,
    /// A linked Steam account shares achievements (#379; the server's setting, told by the page): the Steam app id
    /// is sent even with "Show game art" off, marked `hide_art` so the server shows it to nobody. Off by default: the
    /// page turns it on only for a linked account.
    pub steam_achievements: bool,
}

impl Default for GameSettings {
    fn default() -> Self {
        GameSettings {
            enabled: true,
            art: true,
            time: true,
            games: Vec::new(),
            steam_achievements: false,
        }
    }
}

/// The game running now, and since when (this device's clock: only the elapsed seconds are sent).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Current {
    pub game: Detected,
    pub since: Instant,
}

/// What a read means against what runs (pure): `None` when nothing changed (the same game keeps its start), else the
/// new current game (`Some(None)`: none now).
pub fn changed(
    current: Option<&Current>,
    seen: Option<Detected>,
    now: Instant,
) -> Option<Option<Current>> {
    if current.map(|c| &c.game) == seen.as_ref() {
        return None;
    }
    Some(seen.map(|game| Current { game, since: now }))
}

/// What leaves this computer for `current`: only what the switches allow. The Steam app id goes with art on, or for
/// shared achievements (then marked `hide_art`: the server keeps it to itself).
pub fn update(current: &Current, s: &GameSettings, now: Instant) -> PlayingUpdate {
    let steam_app_id = current
        .game
        .steam_app_id
        .filter(|_| s.art || s.steam_achievements);
    PlayingUpdate {
        name: current.game.name.clone(),
        steam_app_id,
        elapsed_secs: s
            .time
            .then(|| now.saturating_duration_since(current.since).as_secs()),
        hide_art: steam_app_id.is_some() && !s.art,
    }
}

fn frame(current: Option<&Current>, s: &GameSettings, now: Instant) -> ClientFrame {
    ClientFrame::Playing {
        playing: current.map(|c| update(c, s, now)),
    }
}

/// What Settings shows (`pulse://game-activity`): whether Pulse looks, and the game it sees.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct GameState {
    pub detecting: bool,
    pub playing: Option<String>,
}

type SendFrame = Arc<dyn Fn(ClientFrame) + Send + Sync>;
type Notify = Arc<dyn Fn(GameState) + Send + Sync>;
/// Makes the detection source, `None` where there is none (anything but Windows).
type MakeSource = Arc<dyn Fn() -> Option<Box<dyn Source>> + Send + Sync>;

/// The real source: Windows' registry and process list; none elsewhere.
fn source() -> Option<Box<dyn Source>> {
    #[cfg(windows)]
    return Some(Box::new(win::Windows));
    #[cfg(not(windows))]
    None
}

struct Inner {
    settings: GameSettings,
    /// What was last sent, while enabled.
    current: Option<Current>,
    poller: Option<tauri::async_runtime::JoinHandle<()>>,
    /// Bumped on every start and stop: a read from a poller already stopped is dropped.
    epoch: u64,
}

/// The game activity switches and the poller they decide. Managed Tauri state.
pub struct GameActivity {
    inner: Arc<Mutex<Inner>>,
    send: SendFrame,
    notify: Notify,
    make_source: MakeSource,
    every: Duration,
}

impl GameActivity {
    /// Nothing runs until the page sends its saved switches ([`GameActivity::set`]).
    pub fn new(send: SendFrame, notify: Notify) -> Self {
        Self::with_source(send, notify, Arc::new(source), POLL)
    }

    fn with_source(
        send: SendFrame,
        notify: Notify,
        make_source: MakeSource,
        every: Duration,
    ) -> Self {
        GameActivity {
            inner: Arc::new(Mutex::new(Inner {
                // off until the page says otherwise, so nothing is read before its switches arrive
                settings: GameSettings {
                    enabled: false,
                    ..GameSettings::default()
                },
                current: None,
                poller: None,
                epoch: 0,
            })),
            send,
            notify,
            make_source,
            every,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The page's switches and games, at boot and on every change. Off stops the poller and clears what the server
    /// holds; on starts one if none runs (where there's a source); a changed art or play-time switch re-sends the
    /// current game with it. A changed games list is read on the next poll.
    pub fn set(&self, settings: GameSettings) {
        let mut g = self.lock();
        let was = std::mem::replace(&mut g.settings, settings.clone());
        if !settings.enabled {
            if let Some(p) = g.poller.take() {
                p.abort();
                g.epoch += 1;
            }
            if g.current.take().is_some() {
                (self.send)(frame(None, &settings, Instant::now()));
            }
        } else if g.poller.is_none() {
            if let Some(src) = (self.make_source)() {
                g.epoch += 1;
                g.poller = Some(self.spawn(src, g.epoch));
            }
        } else if (was.art != settings.art || was.time != settings.time) && g.current.is_some() {
            (self.send)(frame(g.current.as_ref(), &settings, Instant::now()));
        }
        if was.enabled != settings.enabled {
            (self.notify)(GameState {
                detecting: g.poller.is_some(),
                playing: g.current.as_ref().map(|c| c.game.name.clone()),
            });
        }
    }

    fn spawn(&self, src: Box<dyn Source>, epoch: u64) -> tauri::async_runtime::JoinHandle<()> {
        let (inner, send, notify, every) = (
            self.inner.clone(),
            self.send.clone(),
            self.notify.clone(),
            self.every,
        );
        tauri::async_runtime::spawn(async move {
            let mut tick = tokio::time::interval(every);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            let mut state = Some((src, SteamNames::default()));
            loop {
                tick.tick().await;
                let games = inner
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .settings
                    .games
                    .clone();
                let Some((mut src, mut names)) = state.take() else {
                    return;
                };
                // off the async workers: registry, files and the process snapshot block
                let Ok((back, seen)) = tokio::task::spawn_blocking(move || {
                    let seen = detect(src.as_mut(), &mut names, &games);
                    ((src, names), seen)
                })
                .await
                else {
                    return;
                };
                state = Some(back);
                let mut g = inner.lock().unwrap_or_else(|e| e.into_inner());
                if g.epoch != epoch || !g.settings.enabled {
                    return;
                }
                let now = Instant::now();
                if let Some(next) = changed(g.current.as_ref(), seen, now) {
                    g.current = next;
                    send(frame(g.current.as_ref(), &g.settings, now));
                    notify(GameState {
                        detecting: true,
                        playing: g.current.as_ref().map(|c| c.game.name.clone()),
                    });
                }
            }
        })
    }

    /// Whether a poller runs right now.
    pub fn detecting(&self) -> bool {
        self.lock().poller.is_some()
    }

    pub fn state(&self) -> GameState {
        let g = self.lock();
        GameState {
            detecting: g.poller.is_some(),
            playing: g.current.as_ref().map(|c| c.game.name.clone()),
        }
    }

    /// Sent after every Ready: the server forgets with the connection. Nothing when nothing runs.
    pub fn after_ready(&self) -> Option<ClientFrame> {
        let g = self.lock();
        g.current
            .as_ref()
            .filter(|_| g.settings.enabled)
            .map(|c| frame(Some(c), &g.settings, Instant::now()))
    }
}

#[cfg(test)]
mod tests {
    use super::detect::tests::Fake;
    use super::*;

    fn steam_game(name: &str, id: u32) -> Detected {
        Detected {
            name: name.into(),
            steam_app_id: Some(id),
        }
    }

    #[test]
    fn only_a_different_game_is_a_change_and_it_starts_afresh() {
        let t0 = Instant::now();
        assert_eq!(changed(None, None, t0), None, "nothing, still nothing");
        let started = changed(None, Some(steam_game("A", 1)), t0)
            .unwrap()
            .unwrap();
        assert_eq!(started.since, t0);
        let later = t0 + Duration::from_secs(60);
        assert_eq!(
            changed(Some(&started), Some(steam_game("A", 1)), later),
            None
        );
        let next = changed(Some(&started), Some(steam_game("B", 2)), later)
            .unwrap()
            .unwrap();
        assert_eq!((next.game.name.as_str(), next.since), ("B", later));
        assert_eq!(changed(Some(&next), None, later), Some(None));
    }

    #[test]
    fn only_what_the_switches_allow_is_sent() {
        let t0 = Instant::now();
        let c = Current {
            game: steam_game("A", 570),
            since: t0,
        };
        let now = t0 + Duration::from_secs(95);
        let all = GameSettings::default();
        assert_eq!(
            update(&c, &all, now),
            PlayingUpdate {
                name: "A".into(),
                steam_app_id: Some(570),
                elapsed_secs: Some(95),
                hide_art: false,
            }
        );
        let bare = GameSettings {
            art: false,
            time: false,
            ..GameSettings::default()
        };
        assert_eq!(
            update(&c, &bare, now),
            PlayingUpdate {
                name: "A".into(),
                steam_app_id: None,
                elapsed_secs: None,
                hide_art: false,
            }
        );
        // art off but achievements shared (#379): the id goes, marked for the server only
        let achievements = GameSettings {
            steam_achievements: true,
            ..bare
        };
        assert_eq!(
            update(&c, &achievements, now),
            PlayingUpdate {
                name: "A".into(),
                steam_app_id: Some(570),
                elapsed_secs: None,
                hide_art: true,
            }
        );
        let manual = Current {
            game: Detected {
                name: "B".into(),
                steam_app_id: None,
            },
            since: t0,
        };
        assert!(!update(&manual, &achievements, now).hide_art);
    }

    #[test]
    fn settings_default_on_and_older_saves_fill_in() {
        let s: GameSettings =
            serde_json::from_str(r#"{"games":[{"name":"Old","exe":"C:\\old.exe"}]}"#).unwrap();
        assert!(s.enabled && s.art && s.time);
        assert_eq!(s.games[0].name, "Old");
        // an older page still sends clip drop's fields here: ignored (clip drop has its own switch now)
        let s: GameSettings =
            serde_json::from_str(r#"{"enabled":false,"clips":true,"clip_folders":["D:\\Clips"]}"#)
                .unwrap();
        assert!(!s.enabled);
    }

    type Faked = (GameActivity, Arc<Mutex<Vec<ClientFrame>>>, Arc<Mutex<u32>>);

    /// A `GameActivity` polling a fake source every 10 ms; the frames it sends; how many sources were made.
    fn fake(app_id: Arc<Mutex<Option<u32>>>, steam: std::path::PathBuf) -> Faked {
        let frames = Arc::new(Mutex::new(Vec::new()));
        let f = frames.clone();
        let made = Arc::new(Mutex::new(0));
        let m = made.clone();
        struct Shared(Arc<Mutex<Option<u32>>>, Fake);
        impl Source for Shared {
            fn running_app_id(&mut self) -> Option<u32> {
                *self.0.lock().unwrap()
            }
            fn steam_path(&mut self) -> Option<std::path::PathBuf> {
                self.1.steam_path()
            }
            fn processes(&mut self) -> Vec<String> {
                self.1.processes()
            }
        }
        let g = GameActivity::with_source(
            Arc::new(move |fr| f.lock().unwrap().push(fr)),
            Arc::new(|_| {}),
            Arc::new(move || {
                *m.lock().unwrap() += 1;
                Some(Box::new(Shared(
                    app_id.clone(),
                    Fake {
                        steam: Some(steam.clone()),
                        ..Fake::default()
                    },
                )) as Box<dyn Source>)
            }),
            Duration::from_millis(10),
        );
        (g, frames, made)
    }

    fn playing(f: &ClientFrame) -> Option<PlayingUpdate> {
        match f {
            ClientFrame::Playing { playing } => playing.clone(),
            other => panic!("{other:?}"),
        }
    }

    async fn wait_for(frames: &Arc<Mutex<Vec<ClientFrame>>>, n: usize) {
        for _ in 0..300 {
            if frames.lock().unwrap().len() >= n {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("only {} frames", frames.lock().unwrap().len());
    }

    #[tokio::test]
    async fn off_means_nothing_read_or_sent_and_on_sends_each_change_once() {
        let dir = tempfile::tempdir().unwrap();
        let apps = dir.path().join("steamapps");
        std::fs::create_dir_all(&apps).unwrap();
        std::fs::write(
            apps.join("appmanifest_570.acf"),
            "\"AppState\" { \"name\" \"Space Game\" }",
        )
        .unwrap();
        let id = Arc::new(Mutex::new(Some(570)));
        let (g, frames, made) = fake(id.clone(), dir.path().to_path_buf());

        // off (as at boot, and when the master switch is off): no source, no poller, nothing sent
        g.set(GameSettings {
            enabled: false,
            ..GameSettings::default()
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!((*made.lock().unwrap(), g.detecting()), (0, false));
        assert!(frames.lock().unwrap().is_empty());
        assert!(g.after_ready().is_none());

        g.set(GameSettings::default());
        wait_for(&frames, 1).await;
        let first = playing(&frames.lock().unwrap()[0]).unwrap();
        assert_eq!(
            (first.name.as_str(), first.steam_app_id),
            ("Space Game", Some(570))
        );
        // the same game on every poll after: nothing more
        tokio::time::sleep(Duration::from_millis(80)).await;
        assert_eq!(frames.lock().unwrap().len(), 1);
        assert!(g.after_ready().is_some());

        // art off: re-sent without the app id
        g.set(GameSettings {
            art: false,
            ..GameSettings::default()
        });
        assert_eq!(
            playing(&frames.lock().unwrap()[1]).unwrap().steam_app_id,
            None
        );

        // the game closes
        *id.lock().unwrap() = Some(0);
        wait_for(&frames, 3).await;
        assert_eq!(playing(&frames.lock().unwrap()[2]), None);
        *id.lock().unwrap() = Some(570);
        wait_for(&frames, 4).await;

        // master off: the poller goes and the server is told nothing runs
        g.set(GameSettings {
            enabled: false,
            ..GameSettings::default()
        });
        assert!(!g.detecting());
        assert_eq!(playing(&frames.lock().unwrap()[4]), None);
        let n = frames.lock().unwrap().len();
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert_eq!(frames.lock().unwrap().len(), n, "nothing after off");
        assert!(g.after_ready().is_none());
        assert_eq!(*made.lock().unwrap(), 1);
    }

    #[test]
    fn no_source_no_poller() {
        let g = GameActivity::with_source(
            Arc::new(|_| panic!("nothing to send")),
            Arc::new(|_| {}),
            Arc::new(|| None),
            POLL,
        );
        g.set(GameSettings::default());
        assert!(!g.detecting());
        assert_eq!(
            g.state(),
            GameState {
                detecting: false,
                playing: None
            }
        );
    }
}
