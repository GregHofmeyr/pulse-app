//! Steam sign-in with OpenID 2.0 (#379), the parts that are pure: the sign-in URL, the return URL, checking what
//! Steam sent back, and the `check_authentication` request and answer. Steam proves the SteamID; Pulse never sees a
//! password and needs no key for it.

use reqwest::Url;

const NS: &str = "http://specs.openid.net/auth/2.0";
const IDENTIFIER_SELECT: &str = "http://specs.openid.net/auth/2.0/identifier_select";
/// A Steam claimed id is exactly this followed by the 64-bit SteamID's 17 digits.
const CLAIMED_PREFIX: &str = "https://steamcommunity.com/openid/id/";

/// Where Steam sends the browser back: `<public>/steam/openid/return?state=<state>`.
pub fn return_to(public: &Url, state: &str) -> String {
    let mut u = public.join("/steam/openid/return").expect("a fixed path");
    u.query_pairs_mut().append_pair("state", state);
    u.to_string()
}

/// The `checkid_setup` URL at `endpoint` that signs in to Steam and comes back to `return_to`.
pub fn login_url(endpoint: &str, public: &Url, return_to: &str) -> Option<String> {
    let mut u = Url::parse(endpoint).ok()?;
    let realm = public.origin().ascii_serialization();
    u.query_pairs_mut()
        .append_pair("openid.ns", NS)
        .append_pair("openid.mode", "checkid_setup")
        .append_pair("openid.return_to", return_to)
        .append_pair("openid.realm", &realm)
        .append_pair("openid.identity", IDENTIFIER_SELECT)
        .append_pair("openid.claimed_id", IDENTIFIER_SELECT);
    Some(u.to_string())
}

/// The SteamID in a claimed id: `https://steamcommunity.com/openid/id/` and exactly 17 ASCII digits (a 64-bit
/// SteamID in decimal), nothing before or after (the regex `^https://steamcommunity\.com/openid/id/(\d{17})$`).
pub fn steam_id_from_claimed(claimed: &str) -> Option<String> {
    let id = claimed.strip_prefix(CLAIMED_PREFIX)?;
    (id.len() == 17 && id.bytes().all(|b| b.is_ascii_digit())).then(|| id.to_string())
}

/// Why a return was refused (shown on the page, so plain words).
#[derive(Debug, PartialEq, Eq)]
pub enum Refused {
    NotSignedIn,
    Mismatch,
    BadId,
}

/// Check what Steam sent back (every `openid.*` field) against what Pulse asked for: a positive assertion from
/// `endpoint`, for exactly `expected_return_to`, whose claimed id and identity are the same Steam id. The SteamID if
/// so; Steam still has to vouch for the signature ([`check_body`]).
pub fn verify(
    params: &[(String, String)],
    endpoint: &str,
    expected_return_to: &str,
) -> Result<String, Refused> {
    let get = |k: &str| params.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
    if get("openid.mode") != Some("id_res") {
        return Err(Refused::NotSignedIn);
    }
    if get("openid.ns") != Some(NS)
        || get("openid.op_endpoint") != Some(endpoint)
        || get("openid.return_to") != Some(expected_return_to)
        || get("openid.sig").is_none_or(str::is_empty)
        || get("openid.signed").is_none_or(str::is_empty)
    {
        return Err(Refused::Mismatch);
    }
    // the signed fields must cover what Pulse relies on
    let signed: Vec<&str> = get("openid.signed").unwrap_or("").split(',').collect();
    for need in [
        "claimed_id",
        "identity",
        "return_to",
        "op_endpoint",
        "response_nonce",
    ] {
        if !signed.contains(&need) {
            return Err(Refused::Mismatch);
        }
    }
    let (Some(claimed), Some(identity)) = (get("openid.claimed_id"), get("openid.identity")) else {
        return Err(Refused::BadId);
    };
    if claimed != identity {
        return Err(Refused::BadId);
    }
    steam_id_from_claimed(claimed).ok_or(Refused::BadId)
}

/// The `check_authentication` form: every `openid.*` field as sent, with the mode swapped.
pub fn check_body(params: &[(String, String)]) -> String {
    let mut u = Url::parse("http://x/").expect("a fixed URL");
    {
        let mut q = u.query_pairs_mut();
        for (k, v) in params.iter().filter(|(k, _)| k.starts_with("openid.")) {
            if k == "openid.mode" {
                q.append_pair(k, "check_authentication");
            } else {
                q.append_pair(k, v);
            }
        }
    }
    u.query().unwrap_or("").to_string()
}

/// Whether Steam's key-value answer says `is_valid:true`.
pub fn is_valid(body: &str) -> bool {
    body.lines().any(|l| l.trim() == "is_valid:true")
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENDPOINT: &str = "https://steamcommunity.com/openid/login";
    const ID: &str = "76500000000000001";

    fn public() -> Url {
        Url::parse("https://pulse.example.test").unwrap()
    }

    fn good(ret: &str) -> Vec<(String, String)> {
        let claimed = format!("{CLAIMED_PREFIX}{ID}");
        [
            ("openid.ns", NS),
            ("openid.mode", "id_res"),
            ("openid.op_endpoint", ENDPOINT),
            ("openid.claimed_id", claimed.as_str()),
            ("openid.identity", claimed.as_str()),
            ("openid.return_to", ret),
            ("openid.response_nonce", "2026-11-16T10:00:00Zabc"),
            ("openid.assoc_handle", "1234567890"),
            (
                "openid.signed",
                "signed,op_endpoint,claimed_id,identity,return_to,response_nonce,assoc_handle",
            ),
            ("openid.sig", "c2ln"),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
    }

    fn with(mut p: Vec<(String, String)>, k: &str, v: &str) -> Vec<(String, String)> {
        p.iter_mut().find(|(n, _)| n == k).unwrap().1 = v.into();
        p
    }

    #[test]
    fn the_login_url_asks_for_an_identifier_and_comes_back_here() {
        let ret = return_to(&public(), "abc");
        assert_eq!(
            ret,
            "https://pulse.example.test/steam/openid/return?state=abc"
        );
        let u = Url::parse(&login_url(ENDPOINT, &public(), &ret).unwrap()).unwrap();
        let q: std::collections::HashMap<_, _> = u.query_pairs().into_owned().collect();
        assert_eq!(q["openid.mode"], "checkid_setup");
        assert_eq!(q["openid.return_to"], ret);
        assert_eq!(q["openid.realm"], "https://pulse.example.test");
        assert_eq!(q["openid.claimed_id"], IDENTIFIER_SELECT);
    }

    #[test]
    fn claimed_ids_are_strict() {
        assert_eq!(
            steam_id_from_claimed(&format!("{CLAIMED_PREFIX}{ID}")).as_deref(),
            Some(ID)
        );
        for bad in [
            format!("{CLAIMED_PREFIX}7650000000000000"),
            format!("{CLAIMED_PREFIX}765000000000000012"),
            format!("{CLAIMED_PREFIX}7650000000000000a"),
            format!("{CLAIMED_PREFIX}{ID}/"),
            format!("{CLAIMED_PREFIX}{ID}?x=1"),
            format!("http://steamcommunity.com/openid/id/{ID}"),
            format!("https://steamcommunity.com.evil.test/openid/id/{ID}"),
            format!("https://evil.test/?https://steamcommunity.com/openid/id/{ID}"),
            format!(" {CLAIMED_PREFIX}{ID}"),
            format!("{CLAIMED_PREFIX}７6500000000000001"),
        ] {
            assert_eq!(steam_id_from_claimed(&bad), None, "{bad}");
        }
    }

    #[test]
    fn a_good_return_gives_the_steam_id() {
        let ret = return_to(&public(), "abc");
        assert_eq!(verify(&good(&ret), ENDPOINT, &ret).as_deref(), Ok(ID));
    }

    #[test]
    fn anything_off_is_refused() {
        let ret = return_to(&public(), "abc");
        let p = good(&ret);
        assert_eq!(
            verify(&with(p.clone(), "openid.mode", "cancel"), ENDPOINT, &ret),
            Err(Refused::NotSignedIn)
        );
        for (k, v) in [
            ("openid.op_endpoint", "https://evil.test/openid/login"),
            (
                "openid.return_to",
                "https://pulse.example.test/steam/openid/return?state=other",
            ),
            ("openid.sig", ""),
            ("openid.signed", "signed,claimed_id,identity"),
        ] {
            assert_eq!(
                verify(&with(p.clone(), k, v), ENDPOINT, &ret),
                Err(Refused::Mismatch),
                "{k}"
            );
        }
        let other = format!("{CLAIMED_PREFIX}76500000000000002");
        assert_eq!(
            verify(&with(p.clone(), "openid.identity", &other), ENDPOINT, &ret),
            Err(Refused::BadId)
        );
        let bad = "https://evil.test/openid/id/76500000000000001";
        let p2 = with(with(p, "openid.identity", bad), "openid.claimed_id", bad);
        assert_eq!(verify(&p2, ENDPOINT, &ret), Err(Refused::BadId));
    }

    #[test]
    fn the_check_swaps_the_mode_and_keeps_the_rest() {
        let ret = return_to(&public(), "abc");
        let mut p = good(&ret);
        p.push(("state".into(), "abc".into()));
        let body = check_body(&p);
        assert!(body.contains("openid.mode=check_authentication"));
        assert!(body.contains("openid.sig=c2ln"));
        assert!(!body.contains("state="), "only openid fields");
        assert!(is_valid(
            "ns:http://specs.openid.net/auth/2.0\nis_valid:true\n"
        ));
        assert!(!is_valid(
            "ns:http://specs.openid.net/auth/2.0\nis_valid:false\n"
        ));
        assert!(!is_valid("is_valid:truex"));
    }
}
