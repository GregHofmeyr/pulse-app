//! Badges (#49): the server awards every badge from per-user counters (`badge_stats`) bumped where
//! things already happen. Badges are global per user; only server activity counts, never DMs or
//! groups. A tiered badge's tiers are separate unlocks, and the first person to reach each tier
//! keeps that forever.

pub mod catalogue;
pub mod engine;
pub mod stats;
