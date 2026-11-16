//! Linux idle sources (#374), all over D-Bus with the zbus already in the tree; the first that answers is kept
//! ([`super::FirstThatAnswers`]):
//!
//! 1. `org.freedesktop.ScreenSaver.GetSessionIdleTime` on the session bus (KDE): milliseconds since the last input.
//! 2. `org.gnome.Mutter.IdleMonitor.GetIdletime` on the session bus (GNOME): milliseconds since the last input.
//! 3. logind's `IdleHint` / `IdleSinceHint` for this session, then for the whole seat manager, on the system bus.
//!    Whatever runs the desktop's idle timeout sets the hint; logind always answers, so it comes last. Where
//!    nothing ever sets it the hint stays false and the device stays not idle, as before.
//!
//! The ScreenSaver idle time is read as milliseconds (what KDE returns). Were a service to return seconds, the
//! 30 minutes would never be reached: the device would stay not idle, never wrongly idle.

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
}
