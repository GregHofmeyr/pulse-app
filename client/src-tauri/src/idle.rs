//! Idle (#53): after [`THRESHOLD`] without any system-wide input, this device tells the server it's idle
//! (`ClientFrame::Idle`), and that it isn't once input resumes. The server shows the person Idle only while every
//! device of theirs says so.
//!
//! Windows only: the input source is `GetLastInputInfo`, read every [`POLL`] (one cheap call, no hook). Linux has
//! no idle source, so there the poller never starts and the device never says it's idle.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use pulse_protocol::gateway::ClientFrame;

/// How long without input before this device counts as idle (owner, #53).
pub const THRESHOLD: Duration = Duration::from_secs(30 * 60);
/// How often the input source is read: input resuming clears Idle within this.
pub const POLL: Duration = Duration::from_secs(20);

/// Where the time since the last system-wide input comes from (injectable for tests). `None`: it couldn't be read
/// this time, which changes nothing.
pub trait InputSource: Send + 'static {
    fn idle_for(&mut self) -> Option<Duration>;
}

/// Decides this device's idle state from what the input source says (pure).
#[derive(Debug, Default)]
pub struct Tracker {
    idle: bool,
}

impl Tracker {
    /// The source says the last input was `idle_for` ago: the new state, if it changed.
    pub fn seen(&mut self, idle_for: Option<Duration>) -> Option<bool> {
        let now = idle_for? >= THRESHOLD;
        (now != self.idle).then(|| {
            self.idle = now;
            now
        })
    }
}

/// This device's idle state, held for the app: the poller updates it and sends each change, and the gateway
/// re-sends it after every Ready (the server forgets with the connection).
#[derive(Clone, Default)]
pub struct Idle {
    idle: Arc<AtomicBool>,
}

impl Idle {
    /// Start polling on Windows; on other platforms this holds "not idle" forever and runs nothing.
    pub fn start(send: Arc<dyn Fn(ClientFrame) + Send + Sync>) -> Self {
        let me = Self::default();
        #[cfg(windows)]
        me.poll(LastInput, POLL, send);
        #[cfg(not(windows))]
        let _ = send;
        me
    }

    /// Read `source` every `every` and send each change (a background task for the app's lifetime).
    pub fn poll(
        &self,
        mut source: impl InputSource,
        every: Duration,
        send: Arc<dyn Fn(ClientFrame) + Send + Sync>,
    ) {
        let idle = self.idle.clone();
        tauri::async_runtime::spawn(async move {
            let mut tracker = Tracker::default();
            let mut tick = tokio::time::interval(every);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tick.tick().await;
                if let Some(now) = tracker.seen(source.idle_for()) {
                    idle.store(now, Ordering::Relaxed);
                    send(ClientFrame::Idle { idle: now });
                }
            }
        });
    }

    /// Sent after every Ready: only when idle, since a new connection starts out not idle.
    pub fn after_ready(&self) -> Option<ClientFrame> {
        self.idle
            .load(Ordering::Relaxed)
            .then_some(ClientFrame::Idle { idle: true })
    }
}

/// The Windows input source: milliseconds since the last input anywhere in the session.
#[cfg(windows)]
struct LastInput;

#[cfg(windows)]
impl InputSource for LastInput {
    fn idle_for(&mut self) -> Option<Duration> {
        use windows::Win32::System::SystemInformation::GetTickCount;
        use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
        let mut info = LASTINPUTINFO {
            cbSize: size_of::<LASTINPUTINFO>() as u32,
            dwTime: 0,
        };
        // SAFETY: info is a valid LASTINPUTINFO with cbSize set.
        if !unsafe { GetLastInputInfo(&mut info) }.as_bool() {
            return None;
        }
        // SAFETY: no arguments; reads the tick count. Both are 32-bit millisecond ticks, so this wraps correctly.
        let now = unsafe { GetTickCount() };
        Some(Duration::from_millis(u64::from(
            now.wrapping_sub(info.dwTime),
        )))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    const MIN: Duration = Duration::from_secs(60);

    #[test]
    fn idle_after_thirty_minutes_without_input_and_back_on_input() {
        let mut t = Tracker::default();
        assert_eq!(t.seen(Some(Duration::ZERO)), None);
        assert_eq!(t.seen(Some(29 * MIN + Duration::from_secs(59))), None);
        assert_eq!(t.seen(Some(30 * MIN)), Some(true));
        // still idle: nothing new to say
        assert_eq!(t.seen(Some(45 * MIN)), None);
        // a source that couldn't be read changes nothing
        assert_eq!(t.seen(None), None);
        // any input clears it
        assert_eq!(t.seen(Some(Duration::from_secs(3))), Some(false));
        assert_eq!(t.seen(Some(Duration::from_secs(23))), None);
    }

    /// A fake source: the time since the last input on a fake clock that each read moves on by one poll.
    struct Fake {
        clock: Duration,
        last_input: Arc<Mutex<Duration>>,
    }

    impl InputSource for Fake {
        fn idle_for(&mut self) -> Option<Duration> {
            self.clock += POLL;
            Some(self.clock.saturating_sub(*self.last_input.lock().unwrap()))
        }
    }

    #[tokio::test]
    async fn the_poller_sends_each_change_once_and_after_ready_follows_it() {
        let last_input = Arc::new(Mutex::new(Duration::ZERO));
        let sent = Arc::new(Mutex::new(Vec::new()));
        let idle = Idle::default();
        let s = sent.clone();
        idle.poll(
            Fake {
                clock: Duration::ZERO,
                last_input: last_input.clone(),
            },
            Duration::from_millis(1),
            Arc::new(move |f| {
                if let ClientFrame::Idle { idle } = f {
                    s.lock().unwrap().push(idle);
                }
            }),
        );
        assert!(idle.after_ready().is_none());
        // 30 minutes of 20 s polls go by without input
        let wait = |want: usize| {
            let sent = sent.clone();
            async move {
                for _ in 0..2000 {
                    if sent.lock().unwrap().len() >= want {
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
                panic!("no change sent");
            }
        };
        wait(1).await;
        assert_eq!(*sent.lock().unwrap(), vec![true]);
        assert!(matches!(
            idle.after_ready(),
            Some(ClientFrame::Idle { idle: true })
        ));
        // input: back at the next poll
        *last_input.lock().unwrap() = Duration::from_secs(u64::MAX / 4);
        wait(2).await;
        assert_eq!(*sent.lock().unwrap(), vec![true, false]);
        assert!(idle.after_ready().is_none());
    }
}
