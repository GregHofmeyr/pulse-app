//! Network changes (#355): a Wi-Fi switch, a VPN going up or down, a cable pulled, a resume from sleep. Voice
//! reacts at once (`voice::netreact`) instead of waiting out LiveKit's ~45 s reconnect and our 60 s give-up.
//!
//! The OS only wakes us (Windows: `NotifyUnicastIpAddressChange` / `NotifyIpInterfaceChange` and the suspend/resume
//! notification; Linux: rtnetlink link/address/route events and logind's `PrepareForSleep`). Whether the path really
//! moved is decided from the [`Route`]: the source address the OS would use to reach the internet, read with a
//! connected UDP socket (no packet is sent). A virtual adapter coming and going (Hyper-V, WSL, Docker, a Tailscale
//! that doesn't carry the default route) wakes us but changes nothing there, so it is ignored; a VPN that takes the
//! default route does change it, and counts. Wake-ups are debounced ([`DEBOUNCE`]) into one [`Change`]; a route that
//! moved at any point in the burst counts even if it ends where it began (a Wi-Fi reassociation that got the same
//! address back still broke the call's path). Where the OS source can't be opened, the route is polled instead.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket};
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[cfg(target_os = "linux")]
mod linux;
pub mod path;
#[cfg(windows)]
mod windows;

/// How long wake-ups must stop before a burst is judged: a Wi-Fi switch or a VPN coming up is a flurry of address,
/// interface and route events over a second or so.
pub const DEBOUNCE: Duration = Duration::from_millis(1500);
/// The fallback's poll, where the OS won't tell us.
const POLL: Duration = Duration::from_secs(3);

/// What woke the watcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wake {
    /// An address, interface or route changed somewhere (maybe only on a virtual adapter).
    Network,
    /// The machine woke from sleep: whatever the route says, the call's connections are stale.
    Resumed,
    /// The OS source stopped working: poll from now on.
    SourceDown,
}

/// The source addresses the OS would use towards the internet right now (`None`: no route of that family).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Route {
    pub v4: Option<Ipv4Addr>,
    pub v6: Option<Ipv6Addr>,
}

/// Public addresses, only ever `connect`ed to (UDP: that picks a route and sends nothing).
const PROBE_V4: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)), 53);
const PROBE_V6: SocketAddr = SocketAddr::new(
    IpAddr::V6(Ipv6Addr::new(0x2606, 0x4700, 0x4700, 0, 0, 0, 0, 0x1111)),
    53,
);

fn source_for(probe: SocketAddr) -> Option<IpAddr> {
    let bind: SocketAddr = match probe {
        SocketAddr::V4(_) => (Ipv4Addr::UNSPECIFIED, 0).into(),
        SocketAddr::V6(_) => (Ipv6Addr::UNSPECIFIED, 0).into(),
    };
    let s = UdpSocket::bind(bind).ok()?;
    s.connect(probe).ok()?;
    let ip = s.local_addr().ok()?.ip();
    (!ip.is_unspecified()).then_some(ip)
}

impl Route {
    /// The route as it is now (two syscalls a family; cheap enough for every wake-up).
    pub fn now() -> Self {
        let v4 = match source_for(PROBE_V4) {
            Some(IpAddr::V4(a)) => Some(a),
            _ => None,
        };
        let v6 = match source_for(PROBE_V6) {
            Some(IpAddr::V6(a)) => Some(a),
            _ => None,
        };
        Self { v4, v6 }
    }

    /// Whether `other` is the same path. IPv6 by its /64 only: privacy addresses rotate within the prefix (Windows
    /// does daily) without the network changing.
    pub fn same_path(&self, other: &Route) -> bool {
        self.v4 == other.v4 && self.v6.map(prefix64) == other.v6.map(prefix64)
    }

    pub fn online(&self) -> bool {
        self.v4.is_some() || self.v6.is_some()
    }
}

fn prefix64(a: Ipv6Addr) -> [u16; 4] {
    let s = a.segments();
    [s[0], s[1], s[2], s[3]]
}

/// A real network change, once its burst has gone quiet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Change {
    pub from: Route,
    pub to: Route,
    /// The machine woke from sleep in the burst.
    pub resumed: bool,
}

impl Change {
    pub fn v4_changed(&self) -> bool {
        self.from.v4 != self.to.v4
    }

    pub fn v6_changed(&self) -> bool {
        self.from.v6.map(prefix64) != self.to.v6.map(prefix64)
    }
}

/// Folds bursts of wake-ups into one [`Change`] (pure: time and routes are handed in).
pub struct Debounce {
    baseline: Route,
    burst: Option<Burst>,
}

struct Burst {
    quiet_at: Instant,
    moved: bool,
    resumed: bool,
}

impl Debounce {
    pub fn new(baseline: Route) -> Self {
        Self {
            baseline,
            burst: None,
        }
    }

    pub fn baseline(&self) -> Route {
        self.baseline
    }

    /// A wake-up at `now`, with the route as it is at that moment.
    pub fn wake(&mut self, wake: Wake, route: Route, now: Instant) {
        let b = self.burst.get_or_insert(Burst {
            quiet_at: now,
            moved: false,
            resumed: false,
        });
        b.quiet_at = now + DEBOUNCE;
        b.moved |= !route.same_path(&self.baseline);
        b.resumed |= wake == Wake::Resumed;
    }

    /// When the burst in progress goes quiet (None: none in progress).
    pub fn deadline(&self) -> Option<Instant> {
        self.burst.as_ref().map(|b| b.quiet_at)
    }

    /// Once the burst has gone quiet: the change it made, if any. `route` (as it is now) becomes the baseline.
    pub fn settle(&mut self, route: Route, now: Instant) -> Option<Change> {
        if self.burst.as_ref()?.quiet_at > now {
            return None;
        }
        let b = self.burst.take()?;
        let from = std::mem::replace(&mut self.baseline, route);
        let moved = b.moved || !route.same_path(&from);
        (moved || b.resumed).then_some(Change {
            from,
            to: route,
            resumed: b.resumed,
        })
    }
}

/// Watches for network changes for the app's life, calling `on_change` (on the watcher's own thread) once per real
/// change. Never fails: without the OS's notifications it polls.
pub fn spawn(on_change: impl Fn(Change) + Send + 'static) {
    let (tx, rx) = mpsc::channel();
    let notified = start_sources(tx.clone());
    let started = std::thread::Builder::new()
        .name("pulse-netwatch".into())
        .spawn(move || {
            // kept so the channel stays open whatever the sources do
            let _tx = tx;
            run(&rx, notified, Route::now, on_change)
        });
    if let Err(e) = started {
        tracing::warn!(error = %e, "netwatch: couldn't start");
    }
}

#[cfg(target_os = "linux")]
fn start_sources(tx: mpsc::Sender<Wake>) -> bool {
    linux::start(tx)
}

#[cfg(windows)]
fn start_sources(tx: mpsc::Sender<Wake>) -> bool {
    windows::start(tx)
}

#[cfg(not(any(target_os = "linux", windows)))]
fn start_sources(_tx: mpsc::Sender<Wake>) -> bool {
    false
}

fn run(
    rx: &mpsc::Receiver<Wake>,
    mut notified: bool,
    route: impl Fn() -> Route,
    on_change: impl Fn(Change),
) {
    let mut d = Debounce::new(route());
    tracing::info!(
        notified,
        online = d.baseline().online(),
        "netwatch: watching"
    );
    loop {
        let wait = match d.deadline() {
            Some(at) => at.saturating_duration_since(Instant::now()),
            None if notified => Duration::from_secs(3600),
            None => POLL,
        };
        match rx.recv_timeout(wait) {
            Ok(Wake::SourceDown) => {
                tracing::warn!("netwatch: OS notifications stopped; polling");
                notified = false;
            }
            Ok(w) => d.wake(w, route(), Instant::now()),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let (now, r) = (Instant::now(), route());
                if d.deadline().is_some() {
                    if let Some(c) = d.settle(r, now) {
                        tracing::info!(
                            resumed = c.resumed,
                            v4_changed = c.v4_changed(),
                            v6_changed = c.v6_changed(),
                            online = c.to.online(),
                            "netwatch: network changed"
                        );
                        on_change(c);
                    }
                } else if !notified && !r.same_path(&d.baseline()) {
                    d.wake(Wake::Network, r, now);
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v4(last: u8) -> Route {
        Route {
            v4: Some(Ipv4Addr::new(192, 168, 1, last)),
            v6: None,
        }
    }

    fn v6(prefix: u16, host: u16) -> Option<Ipv6Addr> {
        Some(Ipv6Addr::new(0x2001, 0xdb8, prefix, 1, 0, 0, 0, host))
    }

    #[test]
    fn a_rotated_privacy_address_is_the_same_path_a_new_prefix_is_not() {
        let a = Route {
            v4: None,
            v6: v6(1, 1),
        };
        let rotated = Route {
            v4: None,
            v6: v6(1, 2),
        };
        let moved = Route {
            v4: None,
            v6: v6(2, 1),
        };
        assert!(a.same_path(&rotated));
        assert!(!a.same_path(&moved));
        assert!(!a.same_path(&Route::default()));
    }

    #[test]
    fn a_burst_is_one_change_once_it_goes_quiet() {
        let t0 = Instant::now();
        let mut d = Debounce::new(v4(10));
        d.wake(Wake::Network, v4(10), t0);
        d.wake(
            Wake::Network,
            Route::default(),
            t0 + Duration::from_millis(500),
        );
        d.wake(Wake::Network, v4(20), t0 + Duration::from_millis(1000));
        // still noisy: the quiet period runs from the last wake-up
        assert_eq!(
            d.deadline(),
            Some(t0 + Duration::from_millis(1000) + DEBOUNCE)
        );
        assert_eq!(d.settle(v4(20), t0 + Duration::from_millis(2000)), None);
        let c = d
            .settle(v4(20), t0 + Duration::from_millis(2500))
            .expect("a change");
        assert_eq!((c.from, c.to, c.resumed), (v4(10), v4(20), false));
        assert!(c.v4_changed() && !c.v6_changed());
        assert_eq!(d.baseline(), v4(20));
        assert_eq!(d.deadline(), None);
        // nothing pending: nothing more to settle
        assert_eq!(d.settle(v4(20), t0 + Duration::from_secs(10)), None);
    }

    #[test]
    fn churn_that_never_moves_the_route_is_ignored() {
        // a virtual adapter (Hyper-V, WSL, Docker) coming and going: the route stays put
        let t0 = Instant::now();
        let mut d = Debounce::new(v4(10));
        for i in 0..5 {
            d.wake(Wake::Network, v4(10), t0 + Duration::from_millis(100 * i));
        }
        assert_eq!(d.settle(v4(10), t0 + Duration::from_secs(5)), None);
        assert_eq!(d.deadline(), None);
    }

    #[test]
    fn a_route_that_dropped_and_came_back_still_counts() {
        // a Wi-Fi reassociation that got the same address back: the call's path broke meanwhile
        let t0 = Instant::now();
        let mut d = Debounce::new(v4(10));
        d.wake(Wake::Network, Route::default(), t0);
        d.wake(Wake::Network, v4(10), t0 + Duration::from_millis(800));
        let c = d
            .settle(v4(10), t0 + Duration::from_secs(3))
            .expect("a change");
        assert_eq!((c.from, c.to), (v4(10), v4(10)));
    }

    #[test]
    fn a_resume_counts_even_on_the_same_route() {
        let t0 = Instant::now();
        let mut d = Debounce::new(v4(10));
        d.wake(Wake::Resumed, v4(10), t0);
        d.wake(Wake::Network, v4(10), t0 + Duration::from_millis(200));
        let c = d
            .settle(v4(10), t0 + Duration::from_secs(3))
            .expect("a change");
        assert!(c.resumed);
    }

    #[test]
    fn a_change_that_settles_after_the_last_wake_up_counts() {
        // the address arrives after the last event we saw (DHCP finishing quietly)
        let t0 = Instant::now();
        let mut d = Debounce::new(v4(10));
        d.wake(Wake::Network, v4(10), t0);
        let c = d
            .settle(v4(30), t0 + Duration::from_secs(3))
            .expect("a change");
        assert_eq!(c.to, v4(30));
    }

    #[test]
    fn the_watch_loop_reports_one_change_per_burst() {
        let (tx, rx) = mpsc::channel();
        let routes = std::sync::Mutex::new(vec![v4(10), v4(20), v4(20), v4(20)]);
        let route = || {
            let mut r = routes.lock().unwrap();
            if r.len() > 1 { r.remove(0) } else { r[0] }
        };
        let seen = std::sync::Mutex::new(Vec::new());
        tx.send(Wake::Network).unwrap();
        tx.send(Wake::Network).unwrap();
        // ends the loop once the burst has been judged
        let closer = std::thread::spawn(move || {
            std::thread::sleep(DEBOUNCE + Duration::from_millis(500));
            drop(tx);
        });
        run(&rx, true, route, |c| seen.lock().unwrap().push(c));
        closer.join().unwrap();
        let seen = seen.into_inner().unwrap();
        assert_eq!(seen.len(), 1, "{seen:?}");
        assert_eq!((seen[0].from, seen[0].to), (v4(10), v4(20)));
    }

    /// The real probe on this machine: never panics; an address it finds is a usable source.
    #[test]
    fn the_route_probe_runs() {
        let r = Route::now();
        if let Some(a) = r.v4 {
            assert!(!a.is_unspecified());
        }
    }
}
