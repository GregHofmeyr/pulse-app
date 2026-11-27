//! Linux idle sources (#374), over D-Bus with the zbus already in the tree and over Wayland; the first that answers
//! is kept
//! ([`super::FirstThatAnswers`]):
//!
//! 1. `org.freedesktop.ScreenSaver.GetSessionIdleTime` on the session bus (KDE): milliseconds since the last input.
//! 2. `org.gnome.Mutter.IdleMonitor.GetIdletime` on the session bus (GNOME): milliseconds since the last input.
//! 3. The Wayland `ext-idle-notify-v1` protocol (wlroots compositors, Hyprland, sway, niri, KDE): not D-Bus, our
//!    own connection to the compositor ([`Wayland`]), asking to be told after [`super::THRESHOLD`] without input.
//!    It says idle or not, not how long, so it reads as exactly the threshold or zero.
//! 4. logind's `IdleHint` / `IdleSinceHint` for this session, then for the whole seat manager, on the system bus.
//!    Whatever runs the desktop's idle timeout sets the hint; logind always answers, so it comes last. Where
//!    nothing ever sets it the hint stays false and the device stays not idle, as before.
//!
//! The ScreenSaver idle time is read as milliseconds (what KDE returns). Were a service to return seconds, the
//! 30 minutes would never be reached: the device would stay not idle, never wrongly idle.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use zbus::blocking::Connection;
use zbus::zvariant::OwnedValue;

use super::{FirstThatAnswers, InputSource};

/// The Linux input source: the sources above, in that order.
pub fn source() -> FirstThatAnswers {
    let session = Bus::new(false);
    let system = Bus::new(true);
    FirstThatAnswers::new(vec![
        Box::new(OnBus {
            bus: session.clone(),
            read: screensaver,
        }),
        Box::new(OnBus {
            bus: session,
            read: mutter,
        }),
        Box::new(Wayland::new(super::THRESHOLD)),
        Box::new(OnBus {
            bus: system.clone(),
            read: logind_session,
        }),
        Box::new(OnBus {
            bus: system,
            read: logind_manager,
        }),
    ])
}

/// What logind's hint says about the time since the last input (pure). Not idle: input within the desktop's own
/// timeout, so as good as now. Idle since a time: at least that long ago (the input was earlier still, by the
/// desktop's timeout, so this errs towards not idle). Idle with no time: unknown.
pub fn from_logind(idle_hint: bool, idle_since_usec: u64, now: SystemTime) -> Option<Duration> {
    if !idle_hint {
        return Some(Duration::ZERO);
    }
    if idle_since_usec == 0 {
        return None;
    }
    let since = UNIX_EPOCH + Duration::from_micros(idle_since_usec);
    Some(now.duration_since(since).unwrap_or(Duration::ZERO))
}

/// One bus connection, opened on first use and shared by the sources on that bus (retried on later reads if it
/// couldn't be opened).
#[derive(Clone)]
struct Bus {
    system: bool,
    conn: Arc<Mutex<Option<Connection>>>,
}

impl Bus {
    fn new(system: bool) -> Self {
        Self {
            system,
            conn: Arc::default(),
        }
    }

    fn get(&self) -> Option<Connection> {
        let mut conn = self.conn.lock().ok()?;
        if conn.is_none() {
            *conn = if self.system {
                Connection::system()
            } else {
                Connection::session()
            }
            .ok();
        }
        conn.clone()
    }
}

struct OnBus {
    bus: Bus,
    read: fn(&Connection) -> zbus::Result<Option<Duration>>,
}

impl InputSource for OnBus {
    fn idle_for(&mut self) -> Option<Duration> {
        (self.read)(&self.bus.get()?).ok().flatten()
    }
}

fn screensaver(conn: &Connection) -> zbus::Result<Option<Duration>> {
    let reply = conn.call_method(
        Some("org.freedesktop.ScreenSaver"),
        "/org/freedesktop/ScreenSaver",
        Some("org.freedesktop.ScreenSaver"),
        "GetSessionIdleTime",
        &(),
    )?;
    let ms: u32 = reply.body().deserialize()?;
    Ok(Some(Duration::from_millis(u64::from(ms))))
}

fn mutter(conn: &Connection) -> zbus::Result<Option<Duration>> {
    let reply = conn.call_method(
        Some("org.gnome.Mutter.IdleMonitor"),
        "/org/gnome/Mutter/IdleMonitor/Core",
        Some("org.gnome.Mutter.IdleMonitor"),
        "GetIdletime",
        &(),
    )?;
    let ms: u64 = reply.body().deserialize()?;
    Ok(Some(Duration::from_millis(ms)))
}

fn logind_session(conn: &Connection) -> zbus::Result<Option<Duration>> {
    logind(
        conn,
        "/org/freedesktop/login1/session/auto",
        "org.freedesktop.login1.Session",
    )
}

fn logind_manager(conn: &Connection) -> zbus::Result<Option<Duration>> {
    logind(
        conn,
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    )
}

fn logind(conn: &Connection, path: &str, iface: &str) -> zbus::Result<Option<Duration>> {
    let get = |name: &str| -> zbus::Result<OwnedValue> {
        let reply = conn.call_method(
            Some("org.freedesktop.login1"),
            path,
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &(iface, name),
        )?;
        reply.body().deserialize()
    };
    let hint = bool::try_from(get("IdleHint")?)?;
    let since = u64::try_from(get("IdleSinceHint")?)?;
    Ok(from_logind(hint, since, SystemTime::now()))
}

/// The `ext-idle-notify-v1` source: connects on its first read (from the poller's blocking thread), then a thread of
/// its own waits for the compositor's idled / resumed events. `None` where there is no Wayland compositor or it
/// doesn't offer the protocol (the next source is tried), and again if the connection is lost later.
pub struct Wayland {
    after: Duration,
    tried: bool,
    state: Option<Arc<AtomicU8>>,
}

const ACTIVE: u8 = 0;
const IDLE: u8 = 1;
const GONE: u8 = 2;

impl Wayland {
    pub fn new(after: Duration) -> Self {
        Self {
            after,
            tried: false,
            state: None,
        }
    }
}

impl InputSource for Wayland {
    fn idle_for(&mut self) -> Option<Duration> {
        if !self.tried {
            self.tried = true;
            self.state = wayland::watch(self.after);
        }
        from_wayland(self.state.as_ref()?.load(Ordering::Relaxed), self.after)
    }
}

/// What the compositor last said, as a time since input (pure): idle reads as exactly the threshold asked for.
fn from_wayland(state: u8, after: Duration) -> Option<Duration> {
    match state {
        ACTIVE => Some(Duration::ZERO),
        IDLE => Some(after),
        _ => None,
    }
}

mod wayland {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU8, Ordering};
    use std::time::Duration;

    use wayland_client::globals::{GlobalListContents, registry_queue_init};
    use wayland_client::protocol::{wl_registry, wl_seat};
    use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
    use wayland_protocols::ext::idle_notify::v1::client::ext_idle_notification_v1::{
        self, ExtIdleNotificationV1,
    };
    use wayland_protocols::ext::idle_notify::v1::client::ext_idle_notifier_v1::ExtIdleNotifierV1;

    use super::{ACTIVE, GONE, IDLE};

    struct State(Arc<AtomicU8>);

    impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
        fn event(
            _: &mut Self,
            _: &wl_registry::WlRegistry,
            _: wl_registry::Event,
            _: &GlobalListContents,
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    impl Dispatch<wl_seat::WlSeat, ()> for State {
        fn event(
            _: &mut Self,
            _: &wl_seat::WlSeat,
            _: wl_seat::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    impl Dispatch<ExtIdleNotifierV1, ()> for State {
        fn event(
            _: &mut Self,
            _: &ExtIdleNotifierV1,
            _: <ExtIdleNotifierV1 as Proxy>::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    impl Dispatch<ExtIdleNotificationV1, ()> for State {
        fn event(
            state: &mut Self,
            _: &ExtIdleNotificationV1,
            event: ext_idle_notification_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            match event {
                ext_idle_notification_v1::Event::Idled => state.0.store(IDLE, Ordering::Relaxed),
                ext_idle_notification_v1::Event::Resumed => {
                    state.0.store(ACTIVE, Ordering::Relaxed)
                }
                _ => {}
            }
        }
    }

    /// Asks the compositor (`WAYLAND_DISPLAY`) to say when there's been no input for `after`, and follows its answers
    /// on a thread of its own. `None` without a compositor that offers `ext-idle-notify-v1` and a seat. Version 1
    /// compositors only have the request that idle inhibitors hold off, which is used there.
    pub fn watch(after: Duration) -> Option<Arc<AtomicU8>> {
        let conn = Connection::connect_to_env().ok()?;
        let (globals, mut queue) = registry_queue_init::<State>(&conn).ok()?;
        let qh = queue.handle();
        let notifier: ExtIdleNotifierV1 = globals.bind(&qh, 1..=2, ()).ok()?;
        let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=1, ()).ok()?;
        let ms = u32::try_from(after.as_millis()).unwrap_or(u32::MAX);
        // version 2's input-only request where offered: like Windows' last-input time, it counts input alone, so an
        // idle inhibitor (a playing video, a browser tab holding one) doesn't keep someone who walked away active
        let notification = if notifier.version() >= 2 {
            notifier.get_input_idle_notification(ms, &seat, &qh, ())
        } else {
            notifier.get_idle_notification(ms, &seat, &qh, ())
        };
        let state = Arc::new(AtomicU8::new(ACTIVE));
        let mut handler = State(state.clone());
        queue.roundtrip(&mut handler).ok()?;
        std::thread::Builder::new()
            .name("wayland-idle".into())
            .spawn(move || {
                // the objects live as long as the thread that hears about them
                let _keep = (conn, notifier, seat, notification);
                while queue.blocking_dispatch(&mut handler).is_ok() {}
                handler.0.store(GONE, Ordering::Relaxed);
            })
            .ok()?;
        Some(state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: Duration = Duration::from_secs(60);

    #[test]
    fn logind_hint_reads_as_time_since_input() {
        let now = UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        let usec = |t: SystemTime| t.duration_since(UNIX_EPOCH).unwrap().as_micros() as u64;
        // not idle (whatever the stale "since"): input as good as now
        assert_eq!(from_logind(false, 0, now), Some(Duration::ZERO));
        assert_eq!(
            from_logind(false, usec(now - 40 * MIN), now),
            Some(Duration::ZERO)
        );
        // idle since 31 minutes ago
        assert_eq!(from_logind(true, usec(now - 31 * MIN), now), Some(31 * MIN));
        // idle with no time: unknown, which changes nothing
        assert_eq!(from_logind(true, 0, now), None);
        // a "since" in the future (clock moved back): just went idle
        assert_eq!(
            from_logind(true, usec(now + MIN), now),
            Some(Duration::ZERO)
        );
    }

    #[test]
    fn wayland_answers_read_as_the_threshold_or_zero() {
        assert_eq!(from_wayland(ACTIVE, 30 * MIN), Some(Duration::ZERO));
        assert_eq!(from_wayland(IDLE, 30 * MIN), Some(30 * MIN));
        // the connection went: unknown, which changes nothing
        assert_eq!(from_wayland(GONE, 30 * MIN), None);
    }

    /// Against this machine's compositor, with nobody touching it for 2 s: run by hand on a wlroots/Hyprland/sway
    /// desktop (`cargo test -p pulse-client --lib idle::linux -- --ignored`), hands off the keyboard and mouse.
    #[test]
    #[ignore = "needs a real wlroots compositor and hands off the keyboard and mouse: run by hand"]
    fn wayland_says_idle_after_the_timeout_on_a_real_compositor() {
        let mut source = Wayland::new(Duration::from_secs(2));
        assert_eq!(
            source.idle_for(),
            Some(Duration::ZERO),
            "no ext-idle-notify-v1 here"
        );
        std::thread::sleep(Duration::from_secs(4));
        assert_eq!(source.idle_for(), Some(Duration::from_secs(2)));
    }
}
