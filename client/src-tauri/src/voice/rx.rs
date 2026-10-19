//! One receive task per remote audio track (FINDINGS rule 1).

use std::collections::HashMap;

use tokio::task::JoinHandle;

/// A `NativeAudioStream` does NOT end when its participant leaves: it stays attached and keeps
/// receiving later audio. So each track's receive task is tracked here and aborted explicitly on
/// unsubscribe/leave and before re-subscribing — otherwise rejoins multiply the audio ("robot voice").
/// Keyed by track SID, with the owner kept so everything a person sent can be dropped at once.
#[derive(Default)]
pub struct RxTasks(HashMap<String, (String, JoinHandle<()>)>);

impl RxTasks {
    pub fn replace(&mut self, track: String, owner: String, h: JoinHandle<()>) {
        if let Some((_, old)) = self.0.insert(track, (owner, h)) {
            old.abort();
        }
    }

    pub fn drop_track(&mut self, track: &str) {
        if let Some((_, h)) = self.0.remove(track) {
            h.abort();
        }
    }

    pub fn drop_owner(&mut self, owner: &str) {
        self.0.retain(|_, (o, h)| {
            let keep = o != owner;
            if !keep {
                h.abort();
            }
            keep
        });
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn clear(&mut self) {
        for (_, (_, h)) in self.0.drain() {
            h.abort();
        }
    }
}

impl Drop for RxTasks {
    fn drop(&mut self) {
        self.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending() -> tokio::task::JoinHandle<()> {
        tokio::spawn(std::future::pending())
    }

    #[tokio::test]
    async fn replacing_a_track_aborts_the_old_task() {
        let mut rx = RxTasks::default();
        let first = pending();
        let first_abort = first.abort_handle();
        rx.replace("TR_mic".into(), "alex".into(), first);
        rx.replace("TR_mic".into(), "alex".into(), pending());
        tokio::task::yield_now().await;
        assert!(first_abort.is_finished());
        assert_eq!(rx.len(), 1);
    }

    #[tokio::test]
    async fn two_tracks_from_one_person_both_run() {
        let mut rx = RxTasks::default();
        let mic = pending();
        let mic_abort = mic.abort_handle();
        rx.replace("TR_mic".into(), "alex".into(), mic);
        rx.replace("TR_game".into(), "alex".into(), pending());
        tokio::task::yield_now().await;
        assert!(!mic_abort.is_finished(), "the stream track must not replace the mic");
        assert_eq!(rx.len(), 2);
    }

    #[tokio::test]
    async fn drop_track_leaves_the_persons_other_track() {
        let mut rx = RxTasks::default();
        let (mic, game) = (pending(), pending());
        let (mic_abort, game_abort) = (mic.abort_handle(), game.abort_handle());
        rx.replace("TR_mic".into(), "alex".into(), mic);
        rx.replace("TR_game".into(), "alex".into(), game);
        rx.drop_track("TR_game");
        tokio::task::yield_now().await;
        assert!(game_abort.is_finished());
        assert!(!mic_abort.is_finished());
        assert_eq!(rx.len(), 1);
    }

    #[tokio::test]
    async fn drop_owner_aborts_everything_they_sent() {
        let mut rx = RxTasks::default();
        let (a, b) = (pending(), pending());
        let (a_ab, b_ab) = (a.abort_handle(), b.abort_handle());
        rx.replace("TR_mic".into(), "alex".into(), a);
        rx.replace("TR_game".into(), "alex".into(), b);
        rx.replace("TR_sam".into(), "sam".into(), pending());
        rx.drop_owner("alex");
        tokio::task::yield_now().await;
        assert!(a_ab.is_finished() && b_ab.is_finished());
        assert_eq!(rx.len(), 1);
        rx.clear();
        assert_eq!(rx.len(), 0);
    }
}
