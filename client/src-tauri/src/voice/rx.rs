//! One receive task per remote audio track (FINDINGS rule 1).

use std::collections::HashMap;
use std::time::Duration;

use futures::{Stream, StreamExt};
use tokio::task::JoinHandle;
use tokio::time::Instant;

/// How long a subscribed track may go without a frame before it is reported. WebRTC hands every
/// track it pulls a frame each 10 ms, silence included, so a gap this long means it isn't pulling
/// it (or nothing reaches it) — not that the person is quiet.
pub const STALL_AFTER: Duration = Duration::from_secs(5);

/// Frames stopping or starting again on one track (#216).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    /// No frame for `STALL_AFTER`, after `frames` of them (0: none since the subscribe).
    Stalled { frames: u64 },
    /// Frames again, `after` the last one (or the subscribe).
    Resumed { after: Duration },
}

/// A track's receive loop: every frame to `on_frame`, and a `Flow` report when they stop for
/// `gap` (once per stall) and when they come back. Ends with the stream.
pub async fn receive<S, T>(
    frames: S,
    gap: Duration,
    mut on_frame: impl FnMut(T),
    mut on_flow: impl FnMut(Flow),
) where
    S: Stream<Item = T>,
{
    let mut frames = std::pin::pin!(frames);
    let (mut count, mut last, mut stalled) = (0u64, Instant::now(), false);
    loop {
        // once reported, a stall waits on with no deadline: one report per stall
        let next = if stalled {
            Ok(frames.next().await)
        } else {
            tokio::time::timeout_at(last + gap, frames.next()).await
        };
        match next {
            Ok(Some(f)) => {
                if stalled {
                    stalled = false;
                    on_flow(Flow::Resumed {
                        after: last.elapsed(),
                    });
                }
                count += 1;
                last = Instant::now();
                on_frame(f);
            }
            Ok(None) => break,
            Err(_) => {
                stalled = true;
                on_flow(Flow::Stalled { frames: count });
            }
        }
    }
}

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
    use futures::channel::mpsc;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    const GAP: Duration = Duration::from_secs(5);

    /// The frames and flow reports a watched loop passed on.
    type Seen = Arc<Mutex<(Vec<u32>, Vec<Flow>)>>;

    /// Runs `receive` over a channel's frames, recording frames and flow reports.
    fn watched() -> (
        mpsc::UnboundedSender<u32>,
        Seen,
        tokio::task::JoinHandle<()>,
    ) {
        let (tx, frames) = mpsc::unbounded();
        let seen = Arc::new(Mutex::new((Vec::new(), Vec::new())));
        let (a, b) = (seen.clone(), seen.clone());
        let h = tokio::spawn(receive(
            frames,
            GAP,
            move |f| a.lock().unwrap().0.push(f),
            move |flow| b.lock().unwrap().1.push(flow),
        ));
        (tx, seen, h)
    }

    async fn wait(d: Duration) {
        tokio::time::sleep(d).await;
        tokio::task::yield_now().await;
    }

    /// #216: a subscribed track WebRTC never hands a single frame (not even silence) is reported.
    #[tokio::test(start_paused = true)]
    async fn a_track_that_never_delivers_is_reported_once() {
        let (_tx, seen, _h) = watched();
        wait(GAP - Duration::from_millis(100)).await;
        assert!(seen.lock().unwrap().1.is_empty(), "not before the gap");
        wait(Duration::from_millis(200)).await;
        assert_eq!(seen.lock().unwrap().1, vec![Flow::Stalled { frames: 0 }]);
        wait(GAP * 3).await;
        assert_eq!(
            seen.lock().unwrap().1.len(),
            1,
            "once per stall, not every gap"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn frames_that_stop_and_resume_are_reported_with_the_gap() {
        let (tx, seen, _h) = watched();
        for i in 0..3 {
            tx.unbounded_send(i).unwrap();
            wait(Duration::from_millis(10)).await;
        }
        assert!(
            seen.lock().unwrap().1.is_empty(),
            "frames flowing: nothing to say"
        );
        wait(GAP + Duration::from_secs(2)).await;
        assert_eq!(seen.lock().unwrap().1, vec![Flow::Stalled { frames: 3 }]);
        tx.unbounded_send(3).unwrap();
        wait(Duration::from_millis(1)).await;
        let s = seen.lock().unwrap();
        assert_eq!(s.0, vec![0, 1, 2, 3], "every frame passed on");
        match s.1.as_slice() {
            [_, Flow::Resumed { after }] => {
                assert!(*after >= GAP + Duration::from_secs(2), "{after:?}")
            }
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn the_loop_ends_with_the_stream() {
        let (tx, _seen, h) = watched();
        drop(tx);
        wait(Duration::from_millis(1)).await;
        assert!(h.is_finished());
    }

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
        assert!(
            !mic_abort.is_finished(),
            "the stream track must not replace the mic"
        );
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
