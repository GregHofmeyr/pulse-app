//! Linux: Spotify's MPRIS player over the session D-Bus (zbus, pure Rust). Event-driven: it listens for the player's
//! `PropertiesChanged` (only from the owner of Spotify's bus name, filtered by the bus) and for that name's owner
//! changing (Spotify starting or quitting), and reads all the player's properties only when the owner changes or a
//! property is invalidated. Nothing is asked on a timer.

use std::collections::HashMap;

use futures::StreamExt;
use tokio::sync::mpsc::UnboundedSender;
use zbus::fdo::{DBusProxy, PropertiesProxy};
use zbus::names::{BusName, InterfaceName, WellKnownName};
use zbus::zvariant::{OwnedValue, Value};

use super::{MprisMeta, Seen, now_playing_mpris};

/// The Spotify app's MPRIS bus name (the desktop app, its Flatpak and Snap alike).
pub const SPOTIFY: &str = "org.mpris.MediaPlayer2.spotify";
const PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";

/// The player's state, as its properties last said.
#[derive(Debug, Default)]
struct Player {
    status: Option<String>,
    meta: MprisMeta,
}

impl Player {
    fn apply(&mut self, name: &str, value: &Value<'_>) {
        match name {
            "PlaybackStatus" => self.status = value.downcast_ref::<String>().ok(),
            "Metadata" => self.meta = metadata(value),
            _ => {}
        }
    }

    fn seen(&self) -> Seen {
        now_playing_mpris(self.status.as_deref(), &self.meta)
    }
}

/// The fields Pulse reads from an MPRIS `Metadata` map (`a{sv}`); anything missing or of another type is absent.
fn metadata(value: &Value<'_>) -> MprisMeta {
    let Ok(map) =
        HashMap::<String, OwnedValue>::try_from(value.try_clone().unwrap_or(Value::Bool(false)))
    else {
        return MprisMeta::default();
    };
    let text = |k: &str| -> Option<String> {
        let v = map.get(k)?;
        // `mpris:trackid` is an object path; the text fields are strings
        v.downcast_ref::<String>().ok().or_else(|| {
            v.downcast_ref::<zbus::zvariant::ObjectPath>()
                .ok()
                .map(|p| p.to_string())
        })
    };
    let artists = map
        .get("xesam:artist")
        .and_then(|v| {
            Vec::<String>::try_from(v.try_clone().ok()?)
                .ok()
                .or_else(|| v.downcast_ref::<String>().ok().map(|s| vec![s]))
        })
        .unwrap_or_default();
    MprisMeta {
        trackid: text("mpris:trackid"),
        url: text("xesam:url"),
        title: text("xesam:title"),
        artists,
        album: text("xesam:album"),
    }
}

/// All the player's properties, fresh (an empty player when it doesn't answer).
async fn read_all(props: &PropertiesProxy<'_>) -> Player {
    let mut p = Player::default();
    let Ok(iface) = InterfaceName::try_from(PLAYER) else {
        return p;
    };
    match props.get_all(iface).await {
        Ok(all) => {
            for (k, v) in &all {
                p.apply(k, v);
            }
        }
        Err(e) => tracing::debug!(error = %e, "media activity: player didn't answer"),
    }
    p
}

/// Watch the MPRIS player at bus name `name`, sending what it plays to `tx` whenever that changes (first: what it
/// plays now). Runs until `tx` is closed or the bus goes; fails if there's no session bus.
pub async fn watch(name: &str, tx: UnboundedSender<Seen>) -> zbus::Result<()> {
    let conn = zbus::Connection::session().await?;
    let dbus = DBusProxy::new(&conn).await?;
    let wk = WellKnownName::try_from(name)?;
    let mut owners = dbus
        .receive_name_owner_changed_with_args(&[(0, name)])
        .await?;
    let props = PropertiesProxy::builder(&conn)
        .destination(wk.clone())?
        .path(PATH)?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await?;
    let mut changes = props.receive_properties_changed().await?;
    let mut player = if dbus.name_has_owner(BusName::from(wk.clone())).await? {
        read_all(&props).await
    } else {
        Player::default()
    };
    let mut last = player.seen();
    if tx.send(last.clone()).is_err() {
        return Ok(());
    }
    loop {
        tokio::select! {
            o = owners.next() => {
                let Some(sig) = o else { return Ok(()) };
                let args = sig.args()?;
                player = if args.new_owner().is_some() { read_all(&props).await } else { Player::default() };
            }
            c = changes.next() => {
                let Some(sig) = c else { return Ok(()) };
                let args = sig.args()?;
                if args.interface_name().as_str() != PLAYER {
                    continue;
                }
                for (k, v) in args.changed_properties() {
                    player.apply(k, v);
                }
                if !args.invalidated_properties().is_empty() {
                    player = read_all(&props).await;
                }
            }
            _ = tx.closed() => return Ok(()),
        }
        let seen = player.seen();
        if seen != last {
            if tx.send(seen.clone()).is_err() {
                return Ok(());
            }
            last = seen;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activity::NowPlaying;
    use std::time::Duration;
    use tokio::sync::mpsc;
    use zbus::object_server::SignalEmitter;

    const ID: &str = "4uLU6hMCjMI75M1A2tKUQC";

    /// A fake MPRIS player: what a real one says about itself, changed by the test.
    struct Fake {
        status: String,
        meta: HashMap<String, OwnedValue>,
    }

    #[zbus::interface(name = "org.mpris.MediaPlayer2.Player")]
    impl Fake {
        #[zbus(property)]
        fn playback_status(&self) -> String {
            self.status.clone()
        }
        #[zbus(property)]
        fn metadata(&self) -> HashMap<String, OwnedValue> {
            self.meta
                .iter()
                .map(|(k, v)| (k.clone(), v.try_clone().unwrap()))
                .collect()
        }
    }

    fn meta(title: &str, id: &str) -> HashMap<String, OwnedValue> {
        let path =
            zbus::zvariant::ObjectPath::try_from(format!("/com/spotify/track/{id}")).unwrap();
        let mut m = HashMap::new();
        m.insert(
            "mpris:trackid".into(),
            OwnedValue::try_from(Value::from(path)).unwrap(),
        );
        m.insert(
            "xesam:title".into(),
            OwnedValue::try_from(Value::from(title)).unwrap(),
        );
        m.insert(
            "xesam:artist".into(),
            OwnedValue::try_from(Value::from(vec!["Alex".to_owned()])).unwrap(),
        );
        m.insert(
            "xesam:album".into(),
            OwnedValue::try_from(Value::from("Album")).unwrap(),
        );
        m
    }

    /// A name no real player has, so the test never meets one (nor Spotify).
    fn name() -> String {
        format!("org.mpris.MediaPlayer2.pulsetest_{}", std::process::id())
    }

    async fn serve(name: &str, status: &str, title: &str) -> zbus::Connection {
        zbus::connection::Builder::session()
            .unwrap()
            .name(name.to_owned())
            .unwrap()
            .serve_at(
                PATH,
                Fake {
                    status: status.into(),
                    meta: meta(title, ID),
                },
            )
            .unwrap()
            .build()
            .await
            .unwrap()
    }

    async fn next(rx: &mut mpsc::UnboundedReceiver<Seen>) -> Seen {
        tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("nothing seen")
            .expect("watcher ended")
    }

    /// Against a real session bus (skipped where there's none, as in CI containers): a player that starts, changes
    /// song and status, and quits is followed, and another player's signals on the same path are ignored.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn follows_a_player_on_the_session_bus() {
        if zbus::Connection::session().await.is_err() {
            eprintln!("no session bus: skipped");
            return;
        }
        let name = name();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let n = name.clone();
        let task = tokio::spawn(async move { watch(&n, tx).await });
        assert_eq!(next(&mut rx).await, None, "nothing running yet");

        // the player starts, playing
        let player = serve(&name, "Playing", "First").await;
        let first = next(&mut rx).await.expect("playing");
        assert_eq!(
            first,
            NowPlaying {
                title: "First".into(),
                artist: Some("Alex".into()),
                album: Some("Album".into()),
                spotify_id: Some(format!("track:{ID}")),
            }
        );

        // another player (not the one watched) changing on the same path is ignored
        let other = serve(&format!("{name}_other"), "Playing", "Other").await;
        {
            let iface = other
                .object_server()
                .interface::<_, Fake>(PATH)
                .await
                .unwrap();
            iface.get_mut().await.meta = meta("Other song", ID);
            let e: &SignalEmitter<'_> = iface.signal_emitter();
            iface.get().await.metadata_changed(e).await.unwrap();
        }

        // the watched one changes song, then pauses
        let iface = player
            .object_server()
            .interface::<_, Fake>(PATH)
            .await
            .unwrap();
        iface.get_mut().await.meta = meta("Second", ID);
        iface
            .get()
            .await
            .metadata_changed(iface.signal_emitter())
            .await
            .unwrap();
        assert_eq!(next(&mut rx).await.map(|p| p.title), Some("Second".into()));
        iface.get_mut().await.status = "Paused".into();
        iface
            .get()
            .await
            .playback_status_changed(iface.signal_emitter())
            .await
            .unwrap();
        assert_eq!(next(&mut rx).await, None);
        iface.get_mut().await.status = "Playing".into();
        iface
            .get()
            .await
            .playback_status_changed(iface.signal_emitter())
            .await
            .unwrap();
        assert_eq!(next(&mut rx).await.map(|p| p.title), Some("Second".into()));

        // it quits: nothing plays
        drop(iface);
        drop(player);
        assert_eq!(next(&mut rx).await, None);
        drop(other);
        drop(rx);
        // the watcher ends with its receiver
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("watcher still running")
            .unwrap()
            .unwrap();
    }

    #[test]
    fn metadata_reads_what_it_knows_and_skips_the_rest() {
        let m = meta("Song", ID);
        let v = Value::from(
            m.iter()
                .map(|(k, v)| (k.clone(), v.try_clone().unwrap()))
                .collect::<HashMap<String, OwnedValue>>(),
        );
        let got = metadata(&v);
        assert_eq!(got.title.as_deref(), Some("Song"));
        assert_eq!(got.trackid, Some(format!("/com/spotify/track/{ID}")));
        assert_eq!(got.artists, vec!["Alex".to_owned()]);
        // not a map at all
        assert_eq!(metadata(&Value::from(3u32)), MprisMeta::default());
        // a title that's a number is no title
        let mut odd: HashMap<String, OwnedValue> = HashMap::new();
        odd.insert("xesam:title".into(), OwnedValue::from(7u32));
        assert_eq!(metadata(&Value::from(odd)).title, None);
    }

    /// This process's CPU time (user + system) and resident memory, from /proc.
    fn usage() -> (Duration, u64, u64) {
        let stat = std::fs::read_to_string("/proc/self/stat").unwrap();
        let after = &stat[stat.rfind(')').unwrap() + 2..];
        let f: Vec<&str> = after.split_whitespace().collect();
        // fields 14 and 15 (utime, stime), counted from the state at index 0 = field 3
        let ticks: u64 = f[11].parse::<u64>().unwrap() + f[12].parse::<u64>().unwrap();
        let hz = 100; // USER_HZ on Linux
        let status = std::fs::read_to_string("/proc/self/status").unwrap();
        let field = |k: &str| -> u64 {
            status
                .lines()
                .find(|l| l.starts_with(k))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|v| v.parse().ok())
                .unwrap_or(0)
        };
        (
            Duration::from_millis(ticks * 1000 / hz),
            field("VmRSS:"),
            field("Threads:"),
        )
    }

    /// The fake player as its own process (so its work isn't counted): plays `PULSE_FAKE_PLAYER_CHANGES` songs,
    /// one every 100 ms, then quits. Run by `cost_with_reading_on_and_off` only.
    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    #[ignore = "a child of cost_with_reading_on_and_off"]
    async fn fake_player_process() {
        let Ok(name) = std::env::var("PULSE_FAKE_PLAYER") else {
            return;
        };
        let changes: u32 = std::env::var("PULSE_FAKE_PLAYER_CHANGES")
            .unwrap()
            .parse()
            .unwrap();
        let player = serve(&name, "Playing", "Song 0").await;
        tokio::time::sleep(Duration::from_millis(500)).await;
        let iface = player
            .object_server()
            .interface::<_, Fake>(PATH)
            .await
            .unwrap();
        for i in 1..=changes {
            iface.get_mut().await.meta = meta(&format!("Song {i}"), ID);
            iface
                .get()
                .await
                .metadata_changed(iface.signal_emitter())
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// What reading costs (#275), printed: this process idle with no watcher, idle with the watcher on (Spotify not
    /// running, then a player playing), and while the player changes song every 100 ms, then after the watcher is
    /// dropped. `cargo test -p pulse-client --lib cost_with_reading_on_and_off -- --ignored --nocapture`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "a measurement, not a check: needs a session bus and ~2 minutes"]
    async fn cost_with_reading_on_and_off() {
        let idle = Duration::from_secs(30);
        let line = |what: &str,
                    a: (Duration, u64, u64),
                    b: (Duration, u64, u64),
                    over: Duration| {
            println!(
                "{what:<44} cpu {:>6.1} ms over {:>3} s ({:.3}%)  rss {:>7} kB → {:>7} kB  threads {} → {}",
                (b.0 - a.0).as_secs_f64() * 1000.0,
                over.as_secs(),
                (b.0 - a.0).as_secs_f64() / over.as_secs_f64() * 100.0,
                a.1,
                b.1,
                a.2,
                b.2
            );
        };
        let name = format!("org.mpris.MediaPlayer2.pulsecost_{}", std::process::id());
        let a = usage();
        tokio::time::sleep(idle).await;
        let b = usage();
        line("reading off (no watcher), idle", a, b, idle);

        let (tx, rx) = mpsc::unbounded_channel();
        let seen = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let n = name.clone();
        let watcher = tokio::spawn(async move { watch(&n, tx).await });
        let s = seen.clone();
        let settle = tokio::spawn(crate::activity::settle(rx, move |_| {
            s.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }));
        tokio::time::sleep(Duration::from_secs(1)).await;
        let a = usage();
        tokio::time::sleep(idle).await;
        let b = usage();
        line("reading on, no player, idle", a, b, idle);

        let changes = 100;
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "activity::mpris::tests::fake_player_process",
                "--ignored",
                "--quiet",
            ])
            .env("PULSE_FAKE_PLAYER", &name)
            .env("PULSE_FAKE_PLAYER_CHANGES", changes.to_string())
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let a = usage();
        let burst = Duration::from_secs(12);
        tokio::time::sleep(burst).await;
        let b = usage();
        line(
            &format!("reading on, {changes} song changes in 10 s"),
            a,
            b,
            burst,
        );
        assert!(child.wait().unwrap().success());
        println!(
            "settled sends during the burst: {}",
            seen.load(std::sync::atomic::Ordering::Relaxed)
        );

        let a = usage();
        tokio::time::sleep(idle).await;
        let b = usage();
        line("reading on, player gone, idle", a, b, idle);

        watcher.abort();
        settle.abort();
        tokio::time::sleep(Duration::from_secs(2)).await;
        let a = usage();
        tokio::time::sleep(idle).await;
        let b = usage();
        line("reading off again (watcher dropped), idle", a, b, idle);
    }
}
