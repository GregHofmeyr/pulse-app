//! One-click Join (#381): the strict checks on what Steam says can be joined, and the `steam://` URL built only from
//! parts that pass them. The server checks what it reads from Steam; the client checks again before opening anything.

use std::net::{Ipv4Addr, SocketAddrV4};

use crate::rest::SteamJoin;

/// A SteamID as Pulse stores a linked one: exactly 17 ASCII digits.
pub fn valid_steam_id(raw: &str) -> bool {
    raw.len() == 17 && raw.bytes().all(|b| b.is_ascii_digit())
}

/// A lobby's SteamID: ASCII digits only (no sign, no leading zero), a u64 in the public universe (1) of the chat
/// account type (8) with the lobby instance flag, and a non-zero account number.
pub fn valid_lobby(raw: &str) -> bool {
    if raw.is_empty()
        || raw.len() > 20
        || raw.starts_with('0')
        || !raw.bytes().all(|b| b.is_ascii_digit())
    {
        return false;
    }
    let Ok(id) = raw.parse::<u64>() else {
        return false;
    };
    const LOBBY_FLAG: u64 = 0x4_0000;
    let universe = id >> 56;
    let kind = (id >> 52) & 0xF;
    let instance = (id >> 32) & 0xF_FFFF;
    universe == 1 && kind == 8 && instance & LOBBY_FLAG != 0 && id & 0xFFFF_FFFF != 0
}

/// A game server's `ip:port`, rebuilt from its parts: dotted-quad IPv4 (no leading zeros) that could be reached
/// (not unspecified, loopback, link-local, broadcast or multicast) and a port from 1 to 65535 (digits only, no
/// leading zero). `None` for anything else.
pub fn valid_server(raw: &str) -> Option<String> {
    let (ip, port) = raw.split_once(':')?;
    if port.is_empty()
        || port.len() > 5
        || port.starts_with('0')
        || !port.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let port: u16 = port.parse().ok().filter(|p| *p != 0)?;
    // the standard parser takes exactly four decimal octets and refuses leading zeros
    let ip: Ipv4Addr = ip.parse().ok()?;
    if ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_multicast()
    {
        return None;
    }
    Some(SocketAddrV4::new(ip, port).to_string())
}

/// A Steam app id worth naming: 1 to 2^31 - 1.
fn valid_app(id: u32) -> bool {
    (1..=i32::MAX as u32).contains(&id)
}

impl SteamJoin {
    /// `steam://joinlobby/<app>/<lobby>/<steamid>` or `steam://connect/<ip:port>`, when every part checks out.
    pub fn url(&self) -> Option<String> {
        match self {
            Self::Lobby {
                app_id,
                lobby_id,
                steam_id,
            } => (valid_app(*app_id) && valid_lobby(lobby_id) && valid_steam_id(steam_id))
                .then(|| format!("steam://joinlobby/{app_id}/{lobby_id}/{steam_id}")),
            Self::Server { app_id, address } => {
                let addr = valid_server(address).filter(|a| a == address && valid_app(*app_id))?;
                Some(format!("steam://connect/{addr}"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Obviously fake: lobby account 1 and 2.
    const LOBBY: &str = "109775240917155841";
    const ALEX: &str = "76500000000000001";

    #[test]
    fn urls_only_from_parts_that_check_out() {
        let lobby = |app: u32, lobby: &str, id: &str| SteamJoin::Lobby {
            app_id: app,
            lobby_id: lobby.into(),
            steam_id: id.into(),
        };
        let server = |app: u32, a: &str| SteamJoin::Server {
            app_id: app,
            address: a.into(),
        };
        assert_eq!(
            lobby(440, LOBBY, ALEX).url().as_deref(),
            Some("steam://joinlobby/440/109775240917155841/76500000000000001")
        );
        assert_eq!(
            server(440, "203.0.113.7:27015").url().as_deref(),
            Some("steam://connect/203.0.113.7:27015")
        );
        assert_eq!(lobby(0, LOBBY, ALEX).url(), None);
        assert_eq!(lobby(440, "1/../../x", ALEX).url(), None);
        assert_eq!(lobby(440, LOBBY, "7650000000000000a").url(), None);
        assert_eq!(lobby(440, LOBBY, "765000000000000011").url(), None);
        assert_eq!(server(440, "203.0.113.7:27015/+quit").url(), None);
        assert_eq!(server(440, "127.0.0.1:27015").url(), None);
        assert_eq!(server(u32::MAX, "203.0.113.7:27015").url(), None);
    }

    #[test]
    fn lobby_ids_are_checked_strictly() {
        assert!(valid_lobby(LOBBY));
        assert!(valid_lobby("109775240917155842"));
        for bad in [
            "",
            "0109775240917155841",
            "+109775240917155841",
            "109775240917155841 ",
            "1097752409171558o1",
            // a person's SteamID (individual account type), not a lobby
            "76561197960265729",
            // a chat room without the lobby flag
            "108086391056891905",
            // account number 0
            "109775240917155840",
            "99999999999999999999",
            "18446744073709551616",
        ] {
            assert!(!valid_lobby(bad), "{bad}");
        }
    }

    #[test]
    fn server_addresses_are_checked_strictly() {
        assert_eq!(
            valid_server("203.0.113.7:27015").as_deref(),
            Some("203.0.113.7:27015")
        );
        assert_eq!(
            valid_server("192.168.1.20:27016").as_deref(),
            Some("192.168.1.20:27016")
        );
        for bad in [
            "",
            "203.0.113.7",
            "203.0.113.7:",
            "203.0.113.7:0",
            "203.0.113.7:65536",
            "203.0.113.7:027015",
            "203.0.113.7:+27015",
            "203.0.113.7:27015:1",
            "203.0.113.007:27015",
            "203.0.113:27015",
            "0.0.0.0:27015",
            "127.0.0.1:27015",
            "169.254.1.1:27015",
            "255.255.255.255:27015",
            "224.0.0.1:27015",
            "[2001:db8::1]:27015",
            "example.com:27015",
            "203.0.113.7:27015/../x",
            "203.0.113.7:27015 ",
        ] {
            assert_eq!(valid_server(bad), None, "{bad}");
        }
    }
}
