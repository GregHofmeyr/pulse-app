//! Shared by the LiveKit integration tests (voice_it, stream_it).

/// The LiveKit tests in one test binary take turns.
///
/// Every WebRTC connection in a process shares one WebRTC runtime, and the ICE candidates each
/// connection sends grow with the number of connections alive in the process (measured: ~7 for
/// one alone, ~49 each for two, ~60 each for five, up to ~1200 each for ten at once). With a
/// dozen tests joining at once, LiveKit dropped a participant within half a second of its join
/// (`SIGNAL_SOURCE_CLOSE`, "error writing to request sink: stream closed") as the burst arrived;
/// that join then hung or its mic was never published. The same joins from separate processes,
/// or one test at a time, never failed. The app runs a few connections at most (voice and a
/// stream's two; measured with three in one process: no failures), so this gate is about the
/// tests sharing a process, not the app.
static LIVEKIT: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);

/// Take this first in a test and hold it until the test returns. Tasks the test spawned can
/// outlive it briefly (its runtime drops them after it returns), so a closing room may overlap
/// the next test's start; that is a connection or two, not the burst above.
pub async fn livekit_turn() -> tokio::sync::SemaphorePermit<'static> {
    LIVEKIT.acquire().await.expect("never closed")
}
