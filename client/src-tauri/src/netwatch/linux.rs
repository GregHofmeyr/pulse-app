//! Linux wake-ups: rtnetlink link/address/route events (libc, already in the tree) and logind's `PrepareForSleep`
//! on the system bus (the zbus already in the tree). Container and VM bridges are skipped at the source (they can't
//! move the default route); VPN interfaces (tun, wg, tailscale) are not: when they carry the route it's a real change.

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::mpsc::Sender;

use super::Wake;

/// Starts both sources; true if network notifications are live (otherwise the watcher polls).
pub fn start(tx: Sender<Wake>) -> bool {
    let sleep_tx = tx.clone();
    let _ = std::thread::Builder::new()
        .name("pulse-netwatch-sleep".into())
        .spawn(move || {
            if let Err(e) = watch_sleep(&sleep_tx) {
                tracing::info!(error = %e, "netwatch: no resume notifications (logind)");
            }
        });
    let fd = match open() {
        Ok(fd) => fd,
        Err(e) => {
            tracing::warn!(error = %e, "netwatch: no netlink; polling");
            return false;
        }
    };
    std::thread::Builder::new()
        .name("pulse-netwatch-nl".into())
        .spawn(move || listen(&fd, &tx))
        .is_ok()
}

fn open() -> std::io::Result<OwnedFd> {
    // SAFETY: plain socket calls; the fd is owned from the moment it exists, and `sa` is a zeroed sockaddr_nl of
    // the size passed.
    unsafe {
        let fd = libc::socket(
            libc::AF_NETLINK,
            libc::SOCK_RAW | libc::SOCK_CLOEXEC,
            libc::NETLINK_ROUTE,
        );
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let fd = OwnedFd::from_raw_fd(fd);
        let mut sa: libc::sockaddr_nl = std::mem::zeroed();
        sa.nl_family = libc::AF_NETLINK as libc::sa_family_t;
        sa.nl_groups = (libc::RTMGRP_LINK
            | libc::RTMGRP_IPV4_IFADDR
            | libc::RTMGRP_IPV6_IFADDR
            | libc::RTMGRP_IPV4_ROUTE
            | libc::RTMGRP_IPV6_ROUTE) as u32;
        let r = libc::bind(
            fd.as_raw_fd(),
            &sa as *const libc::sockaddr_nl as *const libc::sockaddr,
            std::mem::size_of::<libc::sockaddr_nl>() as libc::socklen_t,
        );
        if r < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(fd)
    }
}

fn listen(fd: &OwnedFd, tx: &Sender<Wake>) {
    let mut buf = vec![0u8; 32 * 1024];
    loop {
        // SAFETY: reads into our own buffer, at most its length.
        let n = unsafe { libc::recv(fd.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len(), 0) };
        if n < 0 {
            let e = std::io::Error::last_os_error();
            match e.raw_os_error() {
                Some(libc::EINTR) => continue,
                // the kernel dropped events: a lot happened, so look
                Some(libc::ENOBUFS) => {
                    let _ = tx.send(Wake::Network);
                    continue;
                }
                _ => {
                    tracing::warn!(error = %e, "netwatch: netlink read failed");
                    let _ = tx.send(Wake::SourceDown);
                    return;
                }
            }
        }
        if worth_waking(&buf[..n as usize], name_of) && tx.send(Wake::Network).is_err() {
            return;
        }
    }
}

fn name_of(index: u32) -> Option<String> {
    let mut buf = [0 as libc::c_char; libc::IF_NAMESIZE];
    // SAFETY: if_indextoname writes at most IF_NAMESIZE bytes, NUL-terminated, into `buf`.
    let p = unsafe { libc::if_indextoname(index, buf.as_mut_ptr()) };
    if p.is_null() {
        return None;
    }
    // SAFETY: on success `buf` holds a NUL-terminated name.
    let name = unsafe { std::ffi::CStr::from_ptr(buf.as_ptr()) };
    Some(name.to_string_lossy().into_owned())
}

/// Container/VM plumbing: bridges and veths that never carry the default route.
pub fn is_virtual(name: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "lo", "docker", "br-", "veth", "virbr", "vnet", "vboxnet", "vmnet", "podman", "cni",
        "flannel", "lxc", "lxd", "cali", "kube", "weave",
    ];
    PREFIXES.iter().any(|p| name.starts_with(p))
}

const NLMSG_HDR: usize = 16;
const RTA_OIF: u16 = 4;
const NLMSG_NOOP: u16 = libc::NLMSG_NOOP as u16;
const NLMSG_DONE: u16 = libc::NLMSG_DONE as u16;

/// Whether a netlink datagram has anything worth a look: any message that isn't about a virtual interface.
/// `name_of`: an interface index's name (None: gone already, which counts).
pub fn worth_waking(mut msgs: &[u8], name_of: impl Fn(u32) -> Option<String>) -> bool {
    let virt = |index: u32| name_of(index).is_some_and(|n| is_virtual(&n));
    while msgs.len() >= NLMSG_HDR {
        let len = u32::from_ne_bytes(msgs[0..4].try_into().unwrap()) as usize;
        let kind = u16::from_ne_bytes(msgs[4..6].try_into().unwrap());
        if len < NLMSG_HDR || len > msgs.len() {
            return true; // malformed: look anyway
        }
        let body = &msgs[NLMSG_HDR..len];
        let index = match kind {
            // ifinfomsg / ifaddrmsg: the interface index at offset 4
            libc::RTM_NEWLINK | libc::RTM_DELLINK | libc::RTM_NEWADDR | libc::RTM_DELADDR => body
                .get(4..8)
                .map(|b| u32::from_ne_bytes(b.try_into().unwrap())),
            // rtmsg (12 bytes), then attributes: the outgoing interface, if named
            libc::RTM_NEWROUTE | libc::RTM_DELROUTE => route_oif(body),
            NLMSG_NOOP | NLMSG_DONE => {
                msgs = &msgs[align4(len).min(msgs.len())..];
                continue;
            }
            _ => None,
        };
        match index {
            Some(i) if virt(i) => {}
            _ => return true,
        }
        msgs = &msgs[align4(len).min(msgs.len())..];
    }
    false
}

fn align4(n: usize) -> usize {
    (n + 3) & !3
}

fn route_oif(body: &[u8]) -> Option<u32> {
    let mut attrs = body.get(12..)?;
    while attrs.len() >= 4 {
        let len = u16::from_ne_bytes(attrs[0..2].try_into().unwrap()) as usize;
        let kind = u16::from_ne_bytes(attrs[2..4].try_into().unwrap());
        if len < 4 || len > attrs.len() {
            return None;
        }
        if kind == RTA_OIF && len >= 8 {
            return Some(u32::from_ne_bytes(attrs[4..8].try_into().unwrap()));
        }
        attrs = &attrs[align4(len).min(attrs.len())..];
    }
    None
}

fn watch_sleep(tx: &Sender<Wake>) -> zbus::Result<()> {
    let conn = zbus::blocking::Connection::system()?;
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender("org.freedesktop.login1")?
        .interface("org.freedesktop.login1.Manager")?
        .member("PrepareForSleep")?
        .build();
    for msg in zbus::blocking::MessageIterator::for_match_rule(rule, &conn, None)? {
        let Ok(msg) = msg else { continue };
        // false: the machine just woke up
        if let Ok(false) = msg.body().deserialize::<bool>()
            && tx.send(Wake::Resumed).is_err()
        {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(kind: u16, body: &[u8]) -> Vec<u8> {
        let len = NLMSG_HDR + body.len();
        let mut m = Vec::new();
        m.extend((len as u32).to_ne_bytes());
        m.extend(kind.to_ne_bytes());
        m.extend([0u8; 10]); // flags, seq, pid
        m.extend(body);
        m.resize(align4(len), 0);
        m
    }

    fn addr(index: u32) -> Vec<u8> {
        let mut b = vec![2u8, 24, 0, 0];
        b.extend(index.to_ne_bytes());
        msg(libc::RTM_NEWADDR, &b)
    }

    fn route(oif: Option<u32>) -> Vec<u8> {
        let mut b = vec![0u8; 12];
        if let Some(i) = oif {
            b.extend(8u16.to_ne_bytes());
            b.extend(RTA_OIF.to_ne_bytes());
            b.extend(i.to_ne_bytes());
        }
        msg(libc::RTM_NEWROUTE, &b)
    }

    fn names(i: u32) -> Option<String> {
        match i {
            1 => Some("lo".into()),
            2 => Some("wlan0".into()),
            3 => Some("docker0".into()),
            4 => Some("veth12ab".into()),
            5 => Some("wg0".into()),
            _ => None,
        }
    }

    #[test]
    fn container_plumbing_is_skipped_real_and_vpn_interfaces_are_not() {
        assert!(!worth_waking(&addr(3), names));
        assert!(!worth_waking(
            &[addr(3), addr(4), route(Some(4))].concat(),
            names
        ));
        assert!(worth_waking(&addr(2), names));
        assert!(worth_waking(&addr(5), names), "a VPN can carry the route");
        // one real message among virtual ones is enough
        assert!(worth_waking(&[addr(4), addr(2)].concat(), names));
        // an interface already gone: look
        assert!(worth_waking(&addr(99), names));
    }

    #[test]
    fn routes_without_an_interface_and_odd_messages_wake() {
        assert!(worth_waking(&route(None), names));
        assert!(worth_waking(&route(Some(2)), names));
        assert!(worth_waking(&msg(999, &[]), names));
        // truncated: look anyway
        let mut m = addr(3);
        m[0] = 200;
        assert!(worth_waking(&m, names));
        assert!(!worth_waking(&[], names));
    }

    #[test]
    fn virtual_names() {
        for n in ["docker0", "br-1a2b", "veth9", "virbr0", "vboxnet0", "lo"] {
            assert!(is_virtual(n), "{n}");
        }
        for n in [
            "wlan0",
            "wlp2s0",
            "eth0",
            "enp3s0",
            "wg0",
            "tun0",
            "tailscale0",
        ] {
            assert!(!is_virtual(n), "{n}");
        }
    }
}
