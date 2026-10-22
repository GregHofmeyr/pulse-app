//! Stream audio capture: one WASAPI process-loopback capture per `Capture`, summed into one stream.
//!
//! Each capture runs on its own thread and pushes into a shared `Summer`; a sender thread pops the
//! summed 10 ms chunks into the sink. The plumbing is platform-independent; only `wasapi_capture`
//! is Windows-only.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use serde_json::json;

use super::plan::{Capture, plan_diff};
use super::summer::Summer;

/// How far one capture may trail the others before it is zero-filled (see `Summer::new`).
const LAG_CAP_MS: u32 = 30;
/// The sender wakes this often and sends every chunk that is ready.
const SEND_EVERY: Duration = Duration::from_millis(10);

/// Runs one capture until `stop` is set or it fails, handing over interleaved stereo i16.
type Source =
    Arc<dyn Fn(Capture, &AtomicBool, &mut dyn FnMut(Vec<i16>)) -> Result<(), String> + Send + Sync>;

fn log(v: serde_json::Value) {
    tracing::info!(target: "share", "share: {}", v);
}

struct Worker {
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

impl Worker {
    fn join(self) {
        let _ = self.thread.join();
    }
}

pub struct Engine {
    source: Source,
    summer: Arc<Mutex<Summer>>,
    captures: Mutex<HashMap<Capture, Worker>>,
    sending: Arc<AtomicBool>,
    sender: Option<JoinHandle<()>>,
}

impl Engine {
    /// Starts one capture per entry of `plan`; the summed audio goes to `sink`, 10 ms at a time,
    /// from a thread of its own.
    pub fn start(plan: Vec<Capture>, sink: impl FnMut(Vec<i16>) + Send + 'static) -> Engine {
        Self::with_source(Arc::new(wasapi_capture), plan, sink)
    }

    fn with_source(
        source: Source,
        plan: Vec<Capture>,
        mut sink: impl FnMut(Vec<i16>) + Send + 'static,
    ) -> Engine {
        let plan = dedup(plan);
        log(json!({"ev": "capture_started", "plan": plan}));
        let summer = Arc::new(Mutex::new(Summer::new(LAG_CAP_MS)));
        let sending = Arc::new(AtomicBool::new(true));
        let sender = {
            let (summer, sending) = (summer.clone(), sending.clone());
            std::thread::Builder::new()
                .name("share-sender".into())
                .spawn(move || {
                    let mut ready = Vec::new();
                    while sending.load(Ordering::Relaxed) {
                        std::thread::sleep(SEND_EVERY);
                        {
                            let mut s = summer.lock().unwrap();
                            // None: nothing is due yet. Wait for the next tick, send no silence.
                            while let Some(c) = s.pop() {
                                ready.push(c);
                            }
                        }
                        for c in ready.drain(..) {
                            sink(c);
                        }
                    }
                })
                .expect("spawn the stream-audio sender")
        };
        let engine = Engine {
            source,
            summer,
            captures: Mutex::new(HashMap::new()),
            sending,
            sender: Some(sender),
        };
        {
            let mut caps = engine.captures.lock().unwrap();
            for c in plan {
                caps.insert(c, engine.spawn(c));
            }
        }
        engine
    }

    /// Moves to `plan`: starts what is new, stops what is gone. A capture whose thread already
    /// ended (target exited, error) counts as gone, so it is started again if `plan` still has
    /// it. Blocks while stopped captures wind down (up to one WASAPI event wait, 200 ms).
    pub fn apply(&self, plan: Vec<Capture>) {
        let plan = dedup(plan);
        let mut caps = self.captures.lock().unwrap();
        let ended: Vec<Capture> = caps
            .iter()
            .filter(|(_, w)| w.thread.is_finished())
            .map(|(c, _)| *c)
            .collect();
        for c in ended {
            if let Some(w) = caps.remove(&c) {
                w.join();
            }
        }
        let current: Vec<Capture> = caps.keys().copied().collect();
        let (start, stop) = plan_diff(&current, &plan);
        if start.is_empty() && stop.is_empty() {
            return;
        }
        log(json!({"ev": "plan_changed", "start": start, "stop": stop}));
        let stopping: Vec<Worker> = stop.iter().filter_map(|c| caps.remove(c)).collect();
        stop_all(stopping);
        for c in start {
            caps.insert(c, self.spawn(c));
        }
    }

    /// Stops every capture and the sender, and waits for all their threads.
    pub fn stop(self) {
        // Drop does it.
    }

    /// The captures whose threads are still running.
    #[cfg(test)]
    fn running(&self) -> Vec<Capture> {
        let caps = self.captures.lock().unwrap();
        caps.iter()
            .filter(|(_, w)| !w.thread.is_finished())
            .map(|(c, _)| *c)
            .collect()
    }

    fn spawn(&self, c: Capture) -> Worker {
        let stop = Arc::new(AtomicBool::new(false));
        let (source, summer, stop2) = (self.source.clone(), self.summer.clone(), stop.clone());
        let thread = std::thread::Builder::new()
            .name("share-capture".into())
            .spawn(move || {
                let r = source(c, &stop2, &mut |chunk| {
                    summer.lock().unwrap().push(c, &chunk);
                });
                // However it ended: a capture that is gone must not hold the others back.
                summer.lock().unwrap().remove(c);
                log(json!({"ev": "capture_ended", "capture": c, "error": r.err()}));
            })
            .expect("spawn a stream-audio capture");
        Worker { stop, thread }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        let workers: Vec<Worker> = self
            .captures
            .get_mut()
            .unwrap_or_else(|e| e.into_inner())
            .drain()
            .map(|(_, w)| w)
            .collect();
        stop_all(workers);
        self.sending.store(false, Ordering::Relaxed);
        if let Some(t) = self.sender.take() {
            let _ = t.join();
        }
    }
}

/// Signals every worker first, then joins them, so they wind down in parallel.
fn stop_all(workers: Vec<Worker>) {
    for w in &workers {
        w.stop.store(true, Ordering::Relaxed);
    }
    for w in workers {
        w.join();
    }
}

fn dedup(plan: Vec<Capture>) -> Vec<Capture> {
    let mut out = Vec::with_capacity(plan.len());
    for c in plan {
        if !out.contains(&c) {
            out.push(c);
        }
    }
    out
}

#[cfg(not(windows))]
fn wasapi_capture(
    _target: Capture,
    _stop: &AtomicBool,
    _sink: &mut dyn FnMut(Vec<i16>),
) -> Result<(), String> {
    Err("process loopback is Windows-only".into())
}

/// Captures `target` (INCLUDE: only that process tree; EXCLUDE: everything but it) as 48 kHz
/// stereo i16 until `stop` is set. An INCLUDE target that exits ends the capture with an error.
#[cfg(windows)]
fn wasapi_capture(
    target: Capture,
    stop: &AtomicBool,
    sink: &mut dyn FnMut(Vec<i16>),
) -> Result<(), String> {
    use super::summer::CHUNK_SAMPLES;
    let (pid, include) = match target {
        Capture::Include(p) => (p, true),
        Capture::Exclude(p) => (p, false),
    };
    wasapi::initialize_mta()
        .ok()
        .map_err(|e| format!("{e:?}"))?;
    let mut client = wasapi::AudioClient::new_application_loopback_client(pid, include)
        .map_err(|e| format!("activate: {e}"))?;
    let fmt = wasapi::WaveFormat::new(16, 16, &wasapi::SampleType::Int, 48_000, 2, None);
    let mode = wasapi::StreamMode::EventsShared {
        autoconvert: true,
        buffer_duration_hns: 200_000,
    };
    client
        .initialize_client(&fmt, &wasapi::Direction::Capture, &mode)
        .map_err(|e| format!("init: {e}"))?;
    let ev = client
        .set_get_eventhandle()
        .map_err(|e| format!("event: {e}"))?;
    let cap = client
        .get_audiocaptureclient()
        .map_err(|e| format!("capture client: {e}"))?;
    client.start_stream().map_err(|e| format!("start: {e}"))?;
    let mut q = std::collections::VecDeque::<u8>::new();
    while !stop.load(Ordering::Relaxed) {
        if include && !process_alive(pid) {
            let _ = client.stop_stream();
            return Err(format!("target process {pid} exited"));
        }
        if ev.wait_for_event(200).is_err() {
            continue;
        }
        cap.read_from_device_to_deque(&mut q)
            .map_err(|e| format!("read: {e}"))?;
        while q.len() >= CHUNK_SAMPLES * 2 {
            let bytes: Vec<u8> = q.drain(..CHUNK_SAMPLES * 2).collect();
            sink(
                bytes
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|b| i16::from_le_bytes(*b))
                    .collect(),
            );
        }
    }
    let _ = client.stop_stream();
    Ok(())
}

#[cfg(windows)]
fn process_alive(pid: u32) -> bool {
    use windows::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return false;
        };
        let mut code = 0u32;
        let ok = GetExitCodeProcess(h, &mut code).is_ok();
        let _ = windows::Win32::Foundation::CloseHandle(h);
        ok && code == 259 // STILL_ACTIVE
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    use super::super::summer::CHUNK_SAMPLES;
    use super::*;
    use Capture::*;

    /// A fake capture. `script` gives (value, n, ends, len): it pushes `n` chunks of `len` samples
    /// of `value`, one per ms (forever when `n` < 0), then idles until stopped, or returns an
    /// error right away when `ends`.
    fn fake(script: fn(Capture) -> (i16, i32, bool, usize)) -> Source {
        Arc::new(move |c, stop, sink| {
            let (value, n, ends, len) = script(c);
            let mut sent = 0;
            while !stop.load(Ordering::Relaxed) {
                if n < 0 || sent < n {
                    sink(vec![value; len]);
                    sent += 1;
                } else if ends {
                    return Err("target process exited".into());
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            Ok(())
        })
    }

    type Got = Arc<Mutex<Vec<Vec<i16>>>>;

    fn collector() -> (Got, impl FnMut(Vec<i16>) + Send + 'static) {
        let got = Arc::new(Mutex::new(Vec::new()));
        let got2 = got.clone();
        (got, move |c| got2.lock().unwrap().push(c))
    }

    fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
        let end = Instant::now() + Duration::from_secs(5);
        while !cond() {
            assert!(Instant::now() < end, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn two_captures_are_summed_into_the_sink_and_stop_joins_them() {
        let source = fake(|c| match c {
            Include(p) | Exclude(p) => (p as i16, -1, false, CHUNK_SAMPLES),
        });
        let (got, sink) = collector();
        let engine = Engine::with_source(source, vec![Include(1), Include(2)], sink);
        wait_until("a summed chunk", || {
            got.lock()
                .unwrap()
                .iter()
                .any(|c| c == &vec![3; CHUNK_SAMPLES])
        });
        engine.stop();
        let n = got.lock().unwrap().len();
        std::thread::sleep(Duration::from_millis(30));
        assert_eq!(got.lock().unwrap().len(), n, "nothing is sent after stop");
    }

    #[test]
    fn a_capture_that_ends_on_its_own_leaves_the_summer() {
        // Include(1) leaves half a chunk behind and ends; Include(2) then sends 10 chunks. Were
        // Include(1)'s buffer still registered, it would hold the last few of them back.
        let source = fake(|c| match c {
            Include(1) => (1, 1, true, CHUNK_SAMPLES / 2),
            _ => (5, 10, false, CHUNK_SAMPLES),
        });
        let (got, sink) = collector();
        let engine = Engine::with_source(source, vec![Include(1)], sink);
        wait_until("the ended capture to be reaped", || {
            engine.running().is_empty()
        });
        engine.apply(vec![Include(2)]);
        wait_until("all ten chunks", || {
            got.lock().unwrap().iter().filter(|c| c[0] == 5).count() == 10
        });
        assert!(
            got.lock()
                .unwrap()
                .iter()
                .all(|c| c == &vec![5; CHUNK_SAMPLES])
        );
        engine.stop();
    }

    #[test]
    fn apply_starts_and_stops_by_plan_diff() {
        let source = fake(|_| (1, 0, false, CHUNK_SAMPLES));
        let (_got, sink) = collector();
        let engine = Engine::with_source(source, vec![Include(1), Include(2)], sink);
        engine.apply(vec![Include(2), Include(3), Include(3)]);
        let mut now = engine.running();
        now.sort_by_key(|c| format!("{c:?}"));
        assert_eq!(now, vec![Include(2), Include(3)]);
        engine.stop();
    }

    #[test]
    fn a_dead_capture_still_in_the_plan_is_restarted() {
        let starts = Arc::new(AtomicUsize::new(0));
        let s2 = starts.clone();
        let source: Source = Arc::new(move |_, _, _| {
            s2.fetch_add(1, Ordering::Relaxed);
            Err("activate: failed".into())
        });
        let (_got, sink) = collector();
        let engine = Engine::with_source(source, vec![Exclude(7)], sink);
        wait_until("the capture to end", || engine.running().is_empty());
        engine.apply(vec![Exclude(7)]);
        wait_until("a second start", || starts.load(Ordering::Relaxed) == 2);
        engine.stop();
    }

    #[cfg(not(windows))]
    #[test]
    fn process_loopback_says_it_is_windows_only_elsewhere() {
        let stop = AtomicBool::new(false);
        let r = wasapi_capture(Include(1), &stop, &mut |_| {});
        assert!(r.unwrap_err().contains("Windows-only"));
    }

    #[cfg(windows)]
    #[test]
    fn capture_ends_cleanly_when_the_process_exits() {
        let mut child = std::process::Command::new("cmd")
            .args(["/C", "ping -n 3 127.0.0.1 >NUL"])
            .spawn()
            .unwrap();
        let stop = AtomicBool::new(false);
        let r = wasapi_capture(Include(child.id()), &stop, &mut |_| {});
        let _ = child.wait();
        assert!(r.unwrap_err().contains("exited"));
    }
}
