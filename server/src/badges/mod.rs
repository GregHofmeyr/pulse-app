//! Badges (#49): the server awards every badge from per-user counters (`badge_stats`) bumped where
//! things already happen. Badges are global per user; only server activity counts, never DMs or
//! groups. A tiered badge's tiers are separate unlocks, and the first person to reach each tier
//! keeps that forever.

pub mod admin;
pub mod catalogue;
pub mod engine;
pub mod hooks;
pub mod routes;
pub mod stats;
pub mod streams;
pub mod sweep;
pub mod time;
pub mod view;
pub mod voice;

use std::collections::HashMap;

use catalogue::{CATALOGUE, Def, HAND_GIVEN};

/// The masked key `viewer_held` sees for `def`, or `None` when they may see it as it is. A hidden
/// badge is masked as `hidden:<n>` (its place among the hidden badges) unless the viewer holds it
/// themselves; a profile's owner always holds their own. The one masking rule, for the catalogue
/// and profiles alike.
pub fn mask(def: &Def, viewer_held: &HashMap<String, u8>) -> Option<String> {
    if !def.hidden || viewer_held.get(def.key).is_some_and(|t| *t > 0) {
        return None;
    }
    let n = CATALOGUE
        .iter()
        .chain(HAND_GIVEN)
        .filter(|d| d.hidden)
        .position(|d| d.key == def.key)
        .expect("a hidden badge");
    Some(format!("hidden:{n}"))
}
