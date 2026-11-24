//! The network path's shape, masked (#605): for bug reports and netwatch's log, so problems can be matched to a
//! network without saying which one. What leaves this module:
//!
//! - a **label** like `network a3f9`: the default gateway's MAC address hashed with a random salt kept in this
//!   install's profile folder, cut to 16 bits. Stable for one install, meaningless anywhere else, not reversible;
//! - the **kind** of interface carrying the route (`wifi`, `ethernet`, `vpn`, `mobile`, `virtual`, `other`) and
//!   whether any VPN interface is up;
//! - each address family's **shape** (`private`, `cgnat`, `public`, ...), never the address.
//!
//! The MAC, addresses and interface names are read here and dropped here: [`Raw`] never leaves this file.

use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::Path;
use std::sync::{Mutex, OnceLock, PoisonError};

use serde::Serialize;
use sha2::{Digest, Sha256};

use super::Route;

/// The salt's file in the profile folder.
const SALT_FILE: &str = "network-salt";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Wifi,
    Ethernet,
    Vpn,
    Mobile,
    Virtual,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum V4Shape {
    /// 10/8, 172.16/12, 192.168/16: behind a home or office NAT.
    Private,
    /// 100.64/10: the ISP's carrier-grade NAT (TURN/relay trouble likelier).
    Cgnat,
    Public,
    LinkLocal,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum V6Shape {
    Global,
    /// fc00::/7, unique local.
    Ula,
    LinkLocal,
    Other,
}

/// What a report and the log may say about the path.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct PathInfo {
    /// `network xxxx`, or none (no gateway seen, or no salt).
    pub network: Option<String>,
    pub kind: Option<Kind>,
    pub vpn_active: bool,
    /// None: no route of that family.
    pub ipv4: Option<V4Shape>,
    pub ipv6: Option<V6Shape>,
}

/// What the OS told us, raw. Private to this file: only [`PathInfo`] goes anywhere.
#[derive(Debug, Default)]
struct Raw {
    kind: Option<Kind>,
    gateway_mac: Option<[u8; 6]>,
    vpn_active: bool,
}

pub fn v4_shape(a: Ipv4Addr) -> V4Shape {
    let o = a.octets();
    if a.is_private() {
        V4Shape::Private
    } else if o[0] == 100 && (o[1] & 0xc0) == 64 {
        V4Shape::Cgnat
    } else if a.is_link_local() {
        V4Shape::LinkLocal
    } else if a.is_loopback()
        || a.is_unspecified()
        || a.is_broadcast()
        || a.is_multicast()
        || o[0] == 0
    {
        V4Shape::Other
    } else {
        V4Shape::Public
    }
}

pub fn v6_shape(a: Ipv6Addr) -> V6Shape {
    let s0 = a.segments()[0];
    if s0 & 0xfe00 == 0xfc00 {
        V6Shape::Ula
    } else if s0 & 0xffc0 == 0xfe80 {
        V6Shape::LinkLocal
    } else if s0 & 0xe000 == 0x2000 {
        V6Shape::Global
    } else {
        V6Shape::Other
    }
}

// ---- the label ----

static SALT: OnceLock<[u8; 16]> = OnceLock::new();

/// Read this install's salt from `dir`, making it the first time. Called once at startup; without it there are no
/// labels (never an unsalted hash).
pub fn load_salt(dir: &Path) {
    match salt_in(dir) {
        Ok(s) => {
            let _ = SALT.set(s);
        }
        Err(e) => {
            tracing::warn!(error = %e, "netwatch: no network salt; reports carry no network label")
        }
    }
}

fn salt_in(dir: &Path) -> std::io::Result<[u8; 16]> {
    let path = dir.join(SALT_FILE);
    if let Ok(text) = std::fs::read_to_string(&path)
        && let Some(s) = parse_salt(&text)
    {
        return Ok(s);
    }
    let s: [u8; 16] = rand::random();
    std::fs::create_dir_all(dir)?;
    crate::reports::write_atomic(dir, SALT_FILE, hex(&s).as_bytes())?;
    Ok(s)
}

fn parse_salt(text: &str) -> Option<[u8; 16]> {
    let t = text.trim();
    if t.len() != 32 {
        return None;
    }
    let mut s = [0u8; 16];
    for (i, b) in s.iter_mut().enumerate() {
        *b = u8::from_str_radix(t.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(s)
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|b| format!("{b:02x}")).collect()
}

/// `network xxxx` for `id` (a gateway MAC) under `salt`: 16 bits of SHA-256(salt ‖ id).
pub fn label_with(salt: &[u8; 16], id: &[u8]) -> String {
    let h = Sha256::new().chain_update(salt).chain_update(id).finalize();
    format!("network {}", hex(&h[..2]))
}

fn from_raw(route: &Route, raw: Raw, salt: Option<&[u8; 16]>) -> PathInfo {
    PathInfo {
        network: raw
            .gateway_mac
            .filter(|m| m.iter().any(|b| *b != 0))
            .zip(salt)
            .map(|(m, s)| label_with(s, &m)),
        kind: raw.kind,
        vpn_active: raw.vpn_active,
        ipv4: route.v4.map(v4_shape),
        ipv6: route.v6.map(v6_shape),
    }
}

/// The path as it is now, for `route` (blocking: a few small reads or calls).
pub fn now(route: &Route) -> PathInfo {
    from_raw(route, raw(route), SALT.get())
}

/// The last path seen (netwatch refreshes it on every change): for tagging reconnects without asking the OS.
static CURRENT: Mutex<Option<PathInfo>> = Mutex::new(None);

/// Read the path now, remember it, and log it (masked). Blocking.
pub fn refresh(route: &Route) -> PathInfo {
    let p = now(route);
    log_path(&p);
    *CURRENT.lock().unwrap_or_else(PoisonError::into_inner) = Some(p.clone());
    p
}

pub fn current_network() -> Option<String> {
    CURRENT
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
        .and_then(|p| p.network.clone())
}

fn log_path(p: &PathInfo) {
    tracing::info!(
        network = p.network.as_deref().unwrap_or("unknown"),
        kind = ?p.kind,
        vpn = p.vpn_active,
        ipv4 = ?p.ipv4,
        ipv6 = ?p.ipv6,
        "netwatch: path"
    );
}

// ---- Linux: /proc/net and /sys/class/net ----

#[cfg(target_os = "linux")]
fn raw(_route: &Route) -> Raw {
    let sys = Path::new("/sys/class/net");
    let routes = std::fs::read_to_string("/proc/net/route")
        .map(|t| default_routes(&t))
        .unwrap_or_default();
    // the main table's best default route: under a VPN that keeps it (wg-quick, most clients) it's still the
    // physical network, which is what the label should name; `vpn_active` says a VPN is up
    let best = routes.iter().min_by_key(|r| r.2);
    let kind = best.map(|r| kind_of(sys, &r.0));
    let gateway_mac = routes
        .iter()
        .filter(|r| matches!(kind_of(sys, &r.0), Kind::Wifi | Kind::Ethernet))
        .min_by_key(|r| r.2)
        .and_then(|r| {
            let arp = std::fs::read_to_string("/proc/net/arp").ok()?;
            arp_mac(&arp, r.1, &r.0)
        });
    let vpn_active = std::fs::read_dir(sys)
        .map(|d| {
            d.flatten().any(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                kind_of(sys, &name) == Kind::Vpn && is_up(sys, &name)
            })
        })
        .unwrap_or(false);
    Raw {
        kind,
        gateway_mac,
        vpn_active,
    }
}

#[cfg(target_os = "linux")]
fn is_up(sys: &Path, iface: &str) -> bool {
    // tun and WireGuard say "unknown" while up
    std::fs::read_to_string(sys.join(iface).join("operstate")).is_ok_and(|s| s.trim() != "down")
}

/// `/proc/net/route`'s default routes: (interface, gateway, metric).
#[cfg(any(target_os = "linux", test))]
fn default_routes(text: &str) -> Vec<(String, Ipv4Addr, u32)> {
    text.lines()
        .skip(1)
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            // Iface Destination Gateway Flags RefCnt Use Metric Mask ...
            if f.len() < 8 || f[1] != "00000000" || f[7] != "00000000" {
                return None;
            }
            let flags = u32::from_str_radix(f[3], 16).ok()?;
            // RTF_UP
            if flags & 1 == 0 {
                return None;
            }
            // little-endian hex, as the kernel prints it
            let gw = u32::from_str_radix(f[2], 16).ok()?;
            Some((
                f[0].to_string(),
                Ipv4Addr::from(gw.to_le_bytes()),
                f[6].parse().ok()?,
            ))
        })
        .collect()
}

/// `/proc/net/arp`'s MAC for `ip` on `iface`.
#[cfg(any(target_os = "linux", test))]
fn arp_mac(text: &str, ip: Ipv4Addr, iface: &str) -> Option<[u8; 6]> {
    let ip = ip.to_string();
    let line = text.lines().skip(1).find(|l| {
        let f: Vec<&str> = l.split_whitespace().collect();
        f.len() >= 6 && f[0] == ip && f[5] == iface
    })?;
    parse_mac(line.split_whitespace().nth(3)?)
}

#[cfg(any(target_os = "linux", test))]
fn parse_mac(s: &str) -> Option<[u8; 6]> {
    let mut m = [0u8; 6];
    let mut parts = s.split(':');
    for b in m.iter_mut() {
        *b = u8::from_str_radix(parts.next()?, 16).ok()?;
    }
    parts.next().is_none().then_some(m)
}

/// What kind of interface `iface` is, from sysfs under `sys`.
#[cfg(any(target_os = "linux", test))]
fn kind_of(sys: &Path, iface: &str) -> Kind {
    let d = sys.join(iface);
    let ty = std::fs::read_to_string(d.join("type")).unwrap_or_default();
    if d.join("wireless").exists() || d.join("phy80211").exists() {
        Kind::Wifi
    } else if d.join("tun_flags").exists()
        || ty.trim() == "65534"
        || ["wg", "tun", "tap", "tailscale", "zt"]
            .iter()
            .any(|p| iface.starts_with(p))
    {
        // tun/tap, WireGuard (ARPHRD_NONE), Tailscale, ZeroTier
        Kind::Vpn
    } else if ty.trim() == "772" {
        // loopback
        Kind::Other
    } else if iface.starts_with("wwan") || iface.starts_with("rmnet") {
        Kind::Mobile
    } else if !d.join("device").exists() {
        // no hardware behind it: a bridge, veth, Docker, ...
        Kind::Virtual
    } else if ty.trim() == "1" {
        Kind::Ethernet
    } else {
        Kind::Other
    }
}

// ---- Windows: GetAdaptersAddresses + the neighbour table ----

/// An adapter's `IfType` as a [`Kind`].
#[cfg(any(windows, test))]
fn kind_of_iftype(t: u32) -> Kind {
    match t {
        71 => Kind::Wifi,
        6 => Kind::Ethernet,
        // IF_TYPE_PROP_VIRTUAL (WireGuard, most VPN clients), IF_TYPE_TUNNEL, PPP
        53 | 131 | 23 => Kind::Vpn,
        243 | 244 => Kind::Mobile,
        _ => Kind::Other,
    }
}

#[cfg(windows)]
fn raw(route: &Route) -> Raw {
    win::raw(route)
}

#[cfg(windows)]
mod win {
    use std::net::Ipv4Addr;

    use windows::Win32::Foundation::{ERROR_BUFFER_OVERFLOW, NO_ERROR};
    use windows::Win32::NetworkManagement::IpHelper::{
        FreeMibTable, GAA_FLAG_INCLUDE_GATEWAYS, GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER,
        GAA_FLAG_SKIP_MULTICAST, GetAdaptersAddresses, GetIpNetTable2, IP_ADAPTER_ADDRESSES_LH,
        MIB_IPNET_TABLE2,
    };
    use windows::Win32::NetworkManagement::Ndis::IfOperStatusUp;
    use windows::Win32::Networking::WinSock::{AF_INET, AF_UNSPEC, SOCKADDR_IN, SOCKET_ADDRESS};

    use super::{Kind, Raw, Route, kind_of_iftype};

    struct Adapter {
        index: u32,
        kind: Kind,
        metric: u32,
        v4: Vec<Ipv4Addr>,
        gateway: Option<Ipv4Addr>,
    }

    /// SAFETY: `a` comes from GetAdaptersAddresses' list, which is valid while its buffer lives.
    unsafe fn v4_of(a: &SOCKET_ADDRESS) -> Option<Ipv4Addr> {
        if a.lpSockaddr.is_null() || (a.iSockaddrLength as usize) < size_of::<SOCKADDR_IN>() {
            return None;
        }
        // SAFETY: non-null and long enough for a SOCKADDR_IN; its family says whether it is one.
        unsafe {
            if (*a.lpSockaddr).sa_family != AF_INET {
                return None;
            }
            let sin = &*(a.lpSockaddr as *const SOCKADDR_IN);
            Some(Ipv4Addr::from(sin.sin_addr.S_un.S_addr.to_ne_bytes()))
        }
    }

    fn adapters() -> Vec<Adapter> {
        let flags = GAA_FLAG_INCLUDE_GATEWAYS
            | GAA_FLAG_SKIP_ANYCAST
            | GAA_FLAG_SKIP_MULTICAST
            | GAA_FLAG_SKIP_DNS_SERVER;
        // u64s: the list needs 8-byte alignment
        let mut buf: Vec<u64> = vec![0; 16 * 1024 / 8];
        let mut size = (buf.len() * 8) as u32;
        let mut r = ERROR_BUFFER_OVERFLOW.0;
        for _ in 0..3 {
            // SAFETY: `buf` holds `size` bytes, aligned for IP_ADAPTER_ADDRESSES_LH.
            r = unsafe {
                GetAdaptersAddresses(
                    u32::from(AF_UNSPEC.0),
                    flags,
                    None,
                    Some(buf.as_mut_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>()),
                    &mut size,
                )
            };
            if r != ERROR_BUFFER_OVERFLOW.0 {
                break;
            }
            buf = vec![0; (size as usize).div_ceil(8)];
        }
        if r != NO_ERROR.0 {
            return Vec::new();
        }
        let mut out = Vec::new();
        let mut p = buf.as_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
        // SAFETY: a linked list inside `buf`, which outlives this loop.
        unsafe {
            while !p.is_null() {
                let a = &*p;
                if a.OperStatus == IfOperStatusUp && a.IfType != 24 {
                    let mut v4 = Vec::new();
                    let mut u = a.FirstUnicastAddress;
                    while !u.is_null() {
                        v4.extend(v4_of(&(*u).Address));
                        u = (*u).Next;
                    }
                    let mut gateway = None;
                    let mut g = a.FirstGatewayAddress;
                    while !g.is_null() && gateway.is_none() {
                        gateway = v4_of(&(*g).Address);
                        g = (*g).Next;
                    }
                    out.push(Adapter {
                        index: a.Anonymous1.Anonymous.IfIndex,
                        kind: kind_of_iftype(a.IfType),
                        metric: a.Ipv4Metric,
                        v4,
                        gateway,
                    });
                }
                p = a.Next;
            }
        }
        out
    }

    /// The neighbour table's MAC for `ip` on interface `index`.
    fn neighbour_mac(ip: Ipv4Addr, index: u32) -> Option<[u8; 6]> {
        let mut table: *mut MIB_IPNET_TABLE2 = std::ptr::null_mut();
        // SAFETY: the call allocates the table; it is freed below with FreeMibTable.
        if unsafe { GetIpNetTable2(AF_INET, &mut table) } != NO_ERROR || table.is_null() {
            return None;
        }
        let mut found = None;
        // SAFETY: `NumEntries` rows follow the header, valid until FreeMibTable.
        unsafe {
            let rows =
                std::slice::from_raw_parts((*table).Table.as_ptr(), (*table).NumEntries as usize);
            for row in rows {
                if row.InterfaceIndex != index
                    || row.Address.si_family != AF_INET
                    || row.PhysicalAddressLength != 6
                {
                    continue;
                }
                if Ipv4Addr::from(row.Address.Ipv4.sin_addr.S_un.S_addr.to_ne_bytes()) == ip {
                    let mut m = [0u8; 6];
                    m.copy_from_slice(&row.PhysicalAddress[..6]);
                    found = Some(m);
                    break;
                }
            }
            FreeMibTable(table as *const _);
        }
        found
    }

    pub(super) fn raw(route: &Route) -> Raw {
        let all = adapters();
        let kind = route
            .v4
            .and_then(|src| all.iter().find(|a| a.v4.contains(&src)))
            .map(|a| a.kind);
        // the label names the physical network, even under a VPN
        let gateway_mac = all
            .iter()
            .filter(|a| matches!(a.kind, Kind::Wifi | Kind::Ethernet) && a.gateway.is_some())
            .min_by_key(|a| a.metric)
            .and_then(|a| neighbour_mac(a.gateway?, a.index));
        Raw {
            kind,
            gateway_mac,
            vpn_active: all.iter().any(|a| a.kind == Kind::Vpn),
        }
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
fn raw(_route: &Route) -> Raw {
    Raw::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAC: [u8; 6] = [0x3c, 0x84, 0x6a, 0x12, 0xab, 0xef];

    fn raw_values_absent(s: &str) {
        for bad in [
            "3c:84:6a",
            "3c846a",
            "12:ab:ef",
            "192.168.1.23",
            "100.72.0.9",
            "wlp2s0",
            "Home-WiFi",
        ] {
            assert!(
                !s.to_lowercase().contains(&bad.to_lowercase()),
                "{bad} in {s}"
            );
        }
    }

    fn route() -> Route {
        Route {
            v4: Some(Ipv4Addr::new(192, 168, 1, 23)),
            v6: Some("2001:db8:1:2::5".parse().unwrap()),
        }
    }

    #[test]
    fn shapes_never_the_address() {
        assert_eq!(v4_shape(Ipv4Addr::new(192, 168, 1, 23)), V4Shape::Private);
        assert_eq!(v4_shape(Ipv4Addr::new(10, 1, 2, 3)), V4Shape::Private);
        assert_eq!(v4_shape(Ipv4Addr::new(172, 20, 0, 1)), V4Shape::Private);
        assert_eq!(v4_shape(Ipv4Addr::new(100, 64, 0, 1)), V4Shape::Cgnat);
        assert_eq!(v4_shape(Ipv4Addr::new(100, 127, 255, 254)), V4Shape::Cgnat);
        assert_eq!(v4_shape(Ipv4Addr::new(100, 128, 0, 1)), V4Shape::Public);
        assert_eq!(v4_shape(Ipv4Addr::new(41, 0, 0, 1)), V4Shape::Public);
        assert_eq!(v4_shape(Ipv4Addr::new(169, 254, 3, 4)), V4Shape::LinkLocal);
        assert_eq!(v6_shape("2001:db8::1".parse().unwrap()), V6Shape::Global);
        assert_eq!(v6_shape("fd12:3456::1".parse().unwrap()), V6Shape::Ula);
        assert_eq!(v6_shape("fe80::1".parse().unwrap()), V6Shape::LinkLocal);
    }

    #[test]
    fn the_label_is_stable_in_an_install_and_differs_between_installs() {
        let dir = tempfile::tempdir().unwrap();
        let a = salt_in(dir.path()).unwrap();
        let b = salt_in(dir.path()).unwrap();
        assert_eq!(a, b, "the salt is kept");
        assert_eq!(label_with(&a, &MAC), label_with(&b, &MAC));
        let other = tempfile::tempdir().unwrap();
        let c = salt_in(other.path()).unwrap();
        assert_ne!(a, c);
        // 16 bits: a different install almost surely says something else (this pair is fixed by the salts)
        let l = label_with(&a, &MAC);
        assert!(
            l.starts_with("network ") && l.len() == "network ".len() + 4,
            "{l}"
        );
        assert_ne!(label_with(&[0; 16], &MAC), label_with(&[1; 16], &MAC));
        // a bad salt file is replaced, not trusted
        std::fs::write(dir.path().join(SALT_FILE), "nope").unwrap();
        assert_ne!(salt_in(dir.path()).unwrap(), a);
    }

    #[test]
    fn no_salt_or_no_gateway_means_no_label() {
        let r = || Raw {
            kind: Some(Kind::Wifi),
            gateway_mac: Some(MAC),
            vpn_active: false,
        };
        assert_eq!(from_raw(&route(), r(), None).network, None);
        let zero = Raw {
            gateway_mac: Some([0; 6]),
            ..r()
        };
        assert_eq!(from_raw(&route(), zero, Some(&[7; 16])).network, None);
        assert!(from_raw(&route(), r(), Some(&[7; 16])).network.is_some());
    }

    #[test]
    fn the_report_shape_carries_no_raw_values() {
        let p = from_raw(
            &route(),
            Raw {
                kind: Some(Kind::Wifi),
                gateway_mac: Some(MAC),
                vpn_active: true,
            },
            Some(&[9; 16]),
        );
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["kind"], "wifi");
        assert_eq!(v["ipv4"], "private");
        assert_eq!(v["ipv6"], "global");
        assert_eq!(v["vpn_active"], true);
        raw_values_absent(&v.to_string());
        raw_values_absent(&format!("{p:?}"));
    }

    #[test]
    fn the_log_line_carries_no_raw_values() {
        use std::sync::{Arc, Mutex};
        use tracing_subscriber::layer::SubscriberExt;
        #[derive(Clone, Default)]
        struct Buf(Arc<Mutex<Vec<u8>>>);
        impl std::io::Write for Buf {
            fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(b);
                Ok(b.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let buf = Buf::default();
        let w = buf.clone();
        let sub = tracing_subscriber::registry().with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(move || w.clone()),
        );
        let p = from_raw(
            &route(),
            Raw {
                kind: Some(Kind::Ethernet),
                gateway_mac: Some(MAC),
                vpn_active: false,
            },
            Some(&[3; 16]),
        );
        tracing::subscriber::with_default(sub, || log_path(&p));
        let out = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
        assert!(
            out.contains("netwatch: path") && out.contains(p.network.as_deref().unwrap()),
            "{out}"
        );
        assert!(out.contains("Ethernet"), "{out}");
        raw_values_absent(&out);
    }

    #[test]
    fn proc_net_route_and_arp() {
        let route = "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT\n\
                     wlp2s0\t00000000\t0101A8C0\t0003\t0\t0\t600\t00000000\t0\t0\t0\n\
                     wlp2s0\t0001A8C0\t00000000\t0001\t0\t0\t600\t00FFFFFF\t0\t0\t0\n\
                     enp0s31f6\t00000000\t0100000A\t0003\t0\t0\t100\t00000000\t0\t0\t0\n";
        let d = default_routes(route);
        assert_eq!(
            d,
            vec![
                ("wlp2s0".to_string(), Ipv4Addr::new(192, 168, 1, 1), 600),
                ("enp0s31f6".to_string(), Ipv4Addr::new(10, 0, 0, 1), 100),
            ]
        );
        let arp = "IP address       HW type     Flags       HW address            Mask     Device\n\
                   192.168.1.1      0x1         0x2         3c:84:6a:12:ab:ef     *        wlp2s0\n\
                   192.168.1.1      0x1         0x2         aa:aa:aa:aa:aa:aa     *        docker0\n";
        assert_eq!(
            arp_mac(arp, Ipv4Addr::new(192, 168, 1, 1), "wlp2s0"),
            Some(MAC)
        );
        assert_eq!(arp_mac(arp, Ipv4Addr::new(10, 0, 0, 1), "wlp2s0"), None);
        assert_eq!(parse_mac("00:00:00:00:00"), None);
    }

    #[test]
    fn sysfs_kinds() {
        let sys = tempfile::tempdir().unwrap();
        let mk = |name: &str, ty: &str, extra: &[&str]| {
            let d = sys.path().join(name);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("type"), ty).unwrap();
            for e in extra {
                std::fs::create_dir_all(d.join(e)).unwrap();
            }
        };
        mk("wlp2s0", "1\n", &["device", "wireless"]);
        mk("enp0s31f6", "1\n", &["device"]);
        mk("wg0", "65534\n", &[]);
        mk("tun0", "65534\n", &[]);
        mk("docker0", "1\n", &[]);
        mk("lo", "772\n", &[]);
        let k = |n| kind_of(sys.path(), n);
        assert_eq!(k("wlp2s0"), Kind::Wifi);
        assert_eq!(k("enp0s31f6"), Kind::Ethernet);
        assert_eq!(k("wg0"), Kind::Vpn);
        assert_eq!(k("tun0"), Kind::Vpn);
        assert_eq!(k("docker0"), Kind::Virtual);
        assert_eq!(k("lo"), Kind::Other);
    }

    #[test]
    fn windows_interface_types() {
        assert_eq!(kind_of_iftype(71), Kind::Wifi);
        assert_eq!(kind_of_iftype(6), Kind::Ethernet);
        assert_eq!(kind_of_iftype(53), Kind::Vpn);
        assert_eq!(kind_of_iftype(131), Kind::Vpn);
        assert_eq!(kind_of_iftype(243), Kind::Mobile);
        assert_eq!(kind_of_iftype(1), Kind::Other);
    }

    #[test]
    fn this_machines_path_reads_without_panicking() {
        let p = now(&Route::now());
        raw_values_absent(&serde_json::to_string(&p).unwrap());
    }
}
