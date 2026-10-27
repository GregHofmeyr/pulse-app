//! Screen-share streams: in-memory state, the routes, the `~s` room webhook, and the check that
//! ends streams LiveKit no longer has a video connection for.

pub mod liveness;
pub mod routes;
pub mod state;
pub mod webhook;
