//! Linux: the allowlisted players' MPRIS sessions over the session D-Bus (zbus, pure Rust). Event-driven: it listens
//! for `PropertiesChanged` on the MPRIS object path (filtered by the bus, and kept only from a player the allowlist
//! knows) and for MPRIS names' owners changing (a player starting or quitting), and reads a player's properties only
//! when it appears or a property is invalidated. Nothing is asked on a timer. Of several playing at once, the
//! preferred one is shown ([`best`]).

use std::collections::{BTreeMap, HashMap};

use futures::StreamExt;
use tokio::sync::mpsc::UnboundedSender;
use zbus::fdo::{DBusProxy, NameOwnerChanged, PropertiesChanged, PropertiesProxy};
use zbus::names::{BusName, InterfaceName, WellKnownName};
use zbus::zvariant::{OwnedValue, Value};
use zbus::{MatchRule, MessageStream};

use super::{MPRIS_PREFIX, MprisMeta, Seen, Source, best, now_playing_mpris};

const PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";

/// A player's state, as its properties last said.
#[derive(Debug)]
struct Player {
    source: Source,
    status: Option<String>,
    meta: MprisMeta,
}

impl Player {
    fn new(source: Source) -> Self {
        Player {
            source,
            status: None,
            meta: MprisMeta::default(),
        }
    }

    fn apply(&mut self, name: &str, value: &Value<'_>) {
        match name {
            "PlaybackStatus" => self.status = value.downcast_ref::<String>().ok(),
            "Metadata" => self.meta = metadata(value),
            _ => {}
        }
    }

    fn seen(&self) -> Seen {
        now_playing_mpris(self.source, self.status.as_deref(), &self.meta)
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

/// All of player `name`'s properties, fresh (an empty player when it doesn't answer).
async fn read_all(conn: &zbus::Connection, name: &str, source: Source) -> Player {
    let mut p = Player::new(source);
    let Ok(iface) = InterfaceName::try_from(PLAYER) else {
        return p;
    };
    let props = async {
        PropertiesProxy::builder(conn)
            .destination(WellKnownName::try_from(name.to_owned())?)?
            .path(PATH)?
            .cache_properties(zbus::proxy::CacheProperties::No)
            .build()
            .await
    };
    let all = match props.await {
        Ok(props) => props.get_all(iface).await,
        Err(e) => Err(e.into()),
    };
    match all {
        Ok(all) => {
            for (k, v) in &all {
                p.apply(k, v);
            }
        }
        Err(e) => tracing::debug!(error = %e, "media activity: player didn't answer"),
    }
    p
}

/// The players followed: by well-known name (sorted, so "the first of equals" is stable), and which unique name owns
/// each (the sender of its signals).
#[derive(Default)]
struct Players {
    by_name: BTreeMap<String, Player>,
    owner: HashMap<String, String>,
}

impl Players {
    fn seen(&self) -> Seen {
        best(self.by_name.values().map(Player::seen))
    }

    fn remove(&mut self, name: &str) {
        self.by_name.remove(name);
        self.owner.retain(|_, n| n != name);
    }
}

/// Watch the MPRIS players whose bus names `allow` lets in (and names the [`Source`] of), sending what the preferred
/// one plays to `tx` whenever that changes (first: what plays now). Runs until `tx` is closed or the bus goes; fails
/// if there's no session bus.
pub async fn watch(
    allow: impl Fn(&str) -> Option<Source>,
    tx: UnboundedSender<Seen>,
) -> zbus::Result<()> {
    let conn = zbus::Connection::session().await?;
    let dbus = DBusProxy::new(&conn).await?;
    // only MPRIS names' owner changes, filtered by the bus
    let owners_rule = MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender("org.freedesktop.DBus")?
        .interface("org.freedesktop.DBus")?
        .member("NameOwnerChanged")?
        .arg0ns("org.mpris.MediaPlayer2")?
        .build();
    let mut owners = MessageStream::for_match_rule(owners_rule, &conn, None).await?;
    let changes_rule = MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface("org.freedesktop.DBus.Properties")?
        .member("PropertiesChanged")?
        .path(PATH)?
        .build();
    let mut changes = MessageStream::for_match_rule(changes_rule, &conn, None).await?;

    let mut players = Players::default();
    for name in dbus.list_names().await? {
        let name = name.as_str();
        let Some(source) = name.strip_prefix(MPRIS_PREFIX).and(allow(name)) else {
            continue;
        };
        let Ok(owner) = dbus.get_name_owner(BusName::try_from(name)?).await else {
            continue;
        };
        players.owner.insert(owner.to_string(), name.to_owned());
        players
            .by_name
            .insert(name.to_owned(), read_all(&conn, name, source).await);
    }
    let mut last = players.seen();
    if tx.send(last.clone()).is_err() {
        return Ok(());
    }
    loop {
        tokio::select! {
            o = owners.next() => {
                let Some(msg) = o else { return Ok(()) };
                let Some(sig) = msg.ok().and_then(NameOwnerChanged::from_message) else { continue };
                let args = sig.args()?;
                let name = args.name().as_str().to_owned();
                let Some(source) = allow(&name) else { continue };
                players.remove(&name);
                if let Some(owner) = args.new_owner().as_ref() {
                    players.owner.insert(owner.to_string(), name.clone());
                    let p = read_all(&conn, &name, source).await;
                    players.by_name.insert(name, p);
                }
            }
            c = changes.next() => {
                let Some(msg) = c else { return Ok(()) };
                let Ok(msg) = msg else { continue };
                // only from a player followed (any other app's signals on the same path are ignored)
                let Some(name) = msg
                    .header()
                    .sender()
                    .and_then(|s| players.owner.get(s.as_str()))
                    .cloned()
                else {
                    continue;
                };
                let Some(sig) = PropertiesChanged::from_message(msg) else { continue };
                let args = sig.args()?;
                if args.interface_name().as_str() != PLAYER {
                    continue;
                }
                let Some(p) = players.by_name.get_mut(&name) else { continue };
                for (k, v) in args.changed_properties() {
                    p.apply(k, v);
                }
                if !args.invalidated_properties().is_empty() {
                    let source = p.source;
                    let fresh = read_all(&conn, &name, source).await;
                    players.by_name.insert(name, fresh);
                }
            }
            _ = tx.closed() => return Ok(()),
        }
        let seen = players.seen();
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

    /// An allowlist of just `name`, as Spotify.
    fn only(name: String) -> impl Fn(&str) -> Option<Source> {
        move |n: &str| {
            (n == name).then_some(Source {
                player: pulse_protocol::rest::MusicPlayer::Spotify,
                group: crate::activity::Group::Spotify,
            })
        }
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
        let task = tokio::spawn(async move { watch(only(n), tx).await });
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
                player: pulse_protocol::rest::MusicPlayer::Spotify,
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
        let watcher = tokio::spawn(async move { watch(only(n), tx).await });
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
