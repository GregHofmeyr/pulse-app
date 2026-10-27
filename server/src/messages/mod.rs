pub mod mentions;
pub(crate) mod nonce;
pub mod reactions;
pub mod routes;
pub mod system;
pub mod threads;

/// Longest message (or quote line), in characters.
pub(crate) const MAX_LEN: usize = 4000;
