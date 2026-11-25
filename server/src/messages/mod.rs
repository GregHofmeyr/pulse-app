pub mod history;
pub mod mentions;
pub(crate) mod nonce;
pub mod reactions;
pub mod routes;
pub mod system;
pub mod threads;

/// Longest message (or quote line), in characters.
pub(crate) const MAX_LEN: usize = 4000;

/// `m` as everyone sees it, for an event: no one's reactions, seen points or stars.
pub fn neutral(m: &mut pulse_protocol::rest::Message) {
    for r in &mut m.reactions {
        r.me = false;
    }
    if let Some(q) = &mut m.quote {
        q.replies_seen = None;
    }
    if let Some(c) = &mut m.clip {
        c.replies_seen = None;
    }
    if let Some(b) = &mut m.build {
        b.replies_seen = None;
        b.favourite = false;
    }
}
