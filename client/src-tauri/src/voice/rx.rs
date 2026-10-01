//! One receive task per remote participant (FINDINGS rule 1).

use std::collections::HashMap;

use tokio::task::JoinHandle;

/// A `NativeAudioStream` does NOT end when its participant leaves: it stays attached and keeps
/// receiving later audio. So each peer's receive task is tracked here and aborted explicitly on
/// unsubscribe/leave and before re-subscribing — otherwise rejoins multiply the audio ("robot voice").
#[derive(Default)]
pub struct RxTasks(HashMap<String, JoinHandle<()>>);

impl RxTasks {
    pub fn replace(&mut self, id: String, h: JoinHandle<()>) {
        if let Some(old) = self.0.insert(id, h) {
            old.abort();
        }
    }

    pub fn drop_for(&mut self, id: &str) {
        if let Some(h) = self.0.remove(id) {
            h.abort();
        }
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn clear(&mut self) {
        for (_, h) in self.0.drain() {
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
    async fn replacing_peer_drops_old_stream() {
        let mut rx = RxTasks::default();
        let first = pending();
        let first_abort = first.abort_handle();
        rx.replace("a".into(), first);
        rx.replace("a".into(), pending());
        tokio::task::yield_now().await;
        assert!(first_abort.is_finished());
        assert_eq!(rx.len(), 1);
    }

    #[tokio::test]
    async fn drop_for_and_clear_abort() {
        let mut rx = RxTasks::default();
        let h = pending();
        let ab = h.abort_handle();
        rx.replace("a".into(), h);
        rx.replace("b".into(), pending());
        rx.drop_for("a");
        tokio::task::yield_now().await;
        assert!(ab.is_finished());
        assert_eq!(rx.len(), 1);
        rx.clear();
        assert_eq!(rx.len(), 0);
    }
}
