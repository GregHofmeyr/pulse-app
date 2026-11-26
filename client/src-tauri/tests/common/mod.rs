//! Shared by the LiveKit integration tests (voice_it, stream_it, server_mute_it, voice_bitrate_it).

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

/// Each person's loudest voice level over `window`. `VoiceManager::mixer_levels()` races the
/// session's ring tick, which drains the same meters every 70 ms (#651), so one read covers at
/// most that long: poll faster than the tick and keep the max.
#[allow(dead_code)] // not every test binary listens for sound
pub async fn levels_over(
    vm: &pulse_client::voice::VoiceManager,
    window: std::time::Duration,
) -> Vec<(String, f32)> {
    let deadline = tokio::time::Instant::now() + window;
    let mut loudest: Vec<(String, f32)> = Vec::new();
    loop {
        for (id, l) in vm.mixer_levels().await {
            match loudest.iter_mut().find(|(o, _)| *o == id) {
                Some((_, v)) => *v = v.max(l),
                None => loudest.push((id, l)),
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return loudest;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}
