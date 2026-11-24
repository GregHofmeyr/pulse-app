//! Lock-free, allocation-free hand-offs out of the audio callbacks (#353).
//!
//! - [`raw_queue`]: the mic callback hands 10 ms blocks to the mic processor through an `rtrb` ring. The block
//!   buffers go round: the processor gives each one back through a second ring once it's used, so a steady call
//!   allocates nothing in the callback.
//! - [`EchoTap`]: what the speakers played, on its way to the echo canceller, through an `rtrb` ring of samples.
//!
//! Each ring has one producer side, but the callbacks that feed it can briefly overlap (a reopen starts the new
//! stream before the old one is gone), so the producer sits behind a `Mutex` the callback only ever `try_lock`s:
//! uncontended it costs an atomic, contended the callback drops its audio instead of waiting.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, TryLockError};
use std::thread::Thread;
use std::time::{Duration, Instant};

use super::devices::relock;

/// (device rate, 10 ms of mono samples, when the device handed it over).
pub type RawBlock = (u32, Vec<f32>, Instant);

/// Blocks the ring holds. The processor drains it every block, and drops audio older than 60 ms anyway: 640 ms
/// of room covers devices with big buffers that hand over many blocks at once.
const RAW_BLOCKS: usize = 64;
/// Buffers made up front: one per ring slot, plus the one each side holds. Room for 10 ms up to 96 kHz; a faster
/// device grows each buffer once, then keeps it.
const SPARES: usize = RAW_BLOCKS + 2;
const SPARE_CAPACITY: usize = 960;

/// Shared by both ends of the raw queue (but not the producer itself: the processor must see it go).
#[derive(Default)]
struct Link {
    /// The processor's thread, woken when a block arrives (registered by its first wait).
    waker: OnceLock<Thread>,
    /// Blocks dropped because the ring was full or another stream held the producer.
    dropped: AtomicU64,
}

impl Link {
    fn wake(&self) {
        if let Some(t) = self.waker.get() {
            t.unpark();
        }
    }
}

struct RawSide {
    blocks: rtrb::Producer<RawBlock>,
    spares: rtrb::Consumer<Vec<f32>>,
}

struct RawShared {
    side: Mutex<RawSide>,
    link: Arc<Link>,
}

impl Drop for RawShared {
    /// The last sender is gone: wake the processor so it sees that now, not at its next timeout.
    fn drop(&mut self) {
        // Close the queue before waking: `drop` runs before the fields are dropped, so the producer would still be
        // alive, and the processor could wake, see the queue open, and sleep on to its timeout (#596).
        let side = self.side.get_mut().unwrap_or_else(PoisonError::into_inner);
        side.blocks = rtrb::RingBuffer::new(0).0;
        self.link.wake();
    }
}

/// The mic callbacks' end of the raw queue. Cheap to clone; the processor runs until every clone is dropped.
#[derive(Clone)]
pub struct RawTx(Arc<RawShared>);

/// The mic processor's end of the raw queue.
pub struct RawRx {
    blocks: rtrb::Consumer<RawBlock>,
    spares: rtrb::Producer<Vec<f32>>,
    link: Arc<Link>,
}

pub fn raw_queue() -> (RawTx, RawRx) {
    let (blocks_tx, blocks_rx) = rtrb::RingBuffer::new(RAW_BLOCKS);
    let (mut spares_tx, spares_rx) = rtrb::RingBuffer::new(SPARES);
    for _ in 0..SPARES {
        let _ = spares_tx.push(Vec::with_capacity(SPARE_CAPACITY));
    }
    let link = Arc::new(Link::default());
    let tx = RawTx(Arc::new(RawShared {
        side: Mutex::new(RawSide {
            blocks: blocks_tx,
            spares: spares_rx,
        }),
        link: link.clone(),
    }));
    let rx = RawRx {
        blocks: blocks_rx,
        spares: spares_tx,
        link,
    };
    (tx, rx)
}

impl RawTx {
    /// Real-time: hand the full `block` over and leave a cleared spare buffer in its place. Never waits and, once
    /// the spares are in use, never allocates. Returns whether it went: a full ring (the processor is stuck) or a
    /// producer held by another stream drops the block (counted) and clears `block` for reuse.
    pub fn hand_over(&self, rate: u32, block: &mut Vec<f32>, at: Instant) -> bool {
        let mut side = match self.0.side.try_lock() {
            Ok(s) => s,
            // a panic mid-push: the ring indices are atomics, carry on with it
            Err(TryLockError::Poisoned(e)) => e.into_inner(),
            Err(TryLockError::WouldBlock) => return self.drop_block(block),
        };
        if side.blocks.is_full() {
            drop(side);
            return self.drop_block(block);
        }
        // Only if the processor is holding every spare (it never does in a steady call).
        let spare = side
            .spares
            .pop()
            .unwrap_or_else(|_| Vec::with_capacity(block.capacity()));
        let full = std::mem::replace(block, spare);
        block.clear();
        // Can't fail: checked above, and we're the only producer while we hold the lock.
        let _ = side.blocks.push((rate, full, at));
        drop(side);
        self.0.link.wake();
        true
    }

    fn drop_block(&self, block: &mut Vec<f32>) -> bool {
        block.clear();
        self.0.link.dropped.fetch_add(1, Ordering::Relaxed);
        false
    }

    /// Not real-time (tests, the headless null device): queue a whole block. `Err` gives it back if the ring is full.
    pub fn send(&self, b: RawBlock) -> Result<(), RawBlock> {
        let r = relock(&self.0.side).blocks.push(b).map_err(|e| match e {
            rtrb::PushError::Full(b) => b,
        });
        self.0.link.wake();
        r
    }
}

impl RawRx {
    /// Wait up to `timeout` for a block. `Disconnected` once every [`RawTx`] is gone and the ring is empty.
    pub fn recv_timeout(&mut self, timeout: Duration) -> Result<RawBlock, RecvTimeoutError> {
        let me = self.link.waker.get_or_init(std::thread::current);
        let mine = me.id() == std::thread::current().id();
        let deadline = Instant::now() + timeout;
        loop {
            if let Ok(b) = self.blocks.pop() {
                return Ok(b);
            }
            if self.blocks.is_abandoned() {
                // a last block may have landed between the pop and the check
                return self
                    .blocks
                    .pop()
                    .map_err(|_| RecvTimeoutError::Disconnected);
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(RecvTimeoutError::Timeout);
            }
            if mine {
                std::thread::park_timeout(deadline - now);
            } else {
                // Another thread registered as the waker (a test moving the receiver): poll.
                std::thread::sleep((deadline - now).min(Duration::from_millis(1)));
            }
        }
    }

    /// A block if one is waiting.
    pub fn try_recv(&mut self) -> Option<RawBlock> {
        self.blocks.pop().ok()
    }

    /// Give a used block's buffer back to the mic callback.
    pub fn recycle(&mut self, mut v: Vec<f32>) {
        v.clear();
        // full only if the callback ever made extra buffers: then this one is surplus
        let _ = self.spares.push(v);
    }

    /// Blocks the callback dropped since the last call.
    pub fn take_dropped(&self) -> u64 {
        self.link.dropped.swap(0, Ordering::Relaxed)
    }
}

/// Echo reference room: 200 ms at 192 kHz, the most the processor ever keeps.
const ECHO_CAPACITY: usize = 192_000 / 5;

/// What the speakers played (mono i16 at the output device rate), on its way to the echo canceller.
pub struct EchoTap {
    /// The output callback's end (`try_lock` only).
    tx: Mutex<rtrb::Producer<i16>>,
    /// The mic processor's end.
    rx: Mutex<rtrb::Consumer<i16>>,
    /// The output device rate the samples are at.
    rate: AtomicU32,
    /// Set by the callback when what's queued is no use any more (the rate changed, or it overflowed because
    /// nothing drained it): the processor throws it away.
    stale: AtomicBool,
}

impl Default for EchoTap {
    fn default() -> Self {
        let (tx, rx) = rtrb::RingBuffer::new(ECHO_CAPACITY);
        Self {
            tx: Mutex::new(tx),
            rx: Mutex::new(rx),
            rate: AtomicU32::new(0),
            stale: AtomicBool::new(false),
        }
    }
}

impl EchoTap {
    /// Real-time: the output callback's end, or `None` if another output stream has it (a reopen overlap: that
    /// block isn't referenced, which the echo canceller rides out).
    pub fn writer(&self, rate: u32) -> Option<EchoWriter<'_>> {
        let tx = match self.tx.try_lock() {
            Ok(t) => t,
            Err(TryLockError::Poisoned(e)) => e.into_inner(),
            Err(TryLockError::WouldBlock) => return None,
        };
        let was = self.rate.swap(rate, Ordering::AcqRel);
        if was != rate && was != 0 {
            // what's queued is at the old rate: throw it away (this block and anything before the
            // processor's next drain go with it, a few ms the echo canceller rides out)
            self.stale.store(true, Ordering::Release);
        }
        Some(EchoWriter { tap: self, tx })
    }

    /// The processor's end: everything played since the last call, in whole 10 ms chunks, at most the newest
    /// 200 ms, into `out` (cleared first). Returns the rate it's at.
    pub fn drain_into(&self, out: &mut Vec<i16>) -> u32 {
        out.clear();
        let mut rx = relock(&self.rx);
        if self.stale.swap(false, Ordering::AcqRel) {
            let n = rx.slots();
            if let Ok(c) = rx.read_chunk(n) {
                c.commit_all();
            }
        }
        let rate = self.rate.load(Ordering::Acquire);
        let chunk = (rate / 100).max(1) as usize;
        let excess = rx.slots().saturating_sub((rate / 5) as usize);
        if let Ok(c) = rx.read_chunk(excess) {
            c.commit_all();
        }
        let whole = rx.slots() / chunk * chunk;
        out.resize(whole, 0);
        let _ = rx.pop_entire_slice(out);
        rate
    }

    /// Samples waiting (tests).
    #[cfg(test)]
    pub fn queued(&self) -> usize {
        relock(&self.rx).slots()
    }
}

/// One output callback's turn at the echo reference.
pub struct EchoWriter<'a> {
    tap: &'a EchoTap,
    tx: MutexGuard<'a, rtrb::Producer<i16>>,
}

impl EchoWriter<'_> {
    pub fn push(&mut self, v: i16) {
        if self.tx.push(v).is_err() {
            // Nothing has drained it for a while (no processor): what's there is old. Start over once it does.
            self.tap.stale.store(true, Ordering::Release);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_go_through_and_their_buffers_come_back() {
        let (tx, mut rx) = raw_queue();
        let mut pending = Vec::with_capacity(480);
        for round in 0..3 * SPARES {
            pending.extend(std::iter::repeat_n(round as f32, 480));
            let had = pending.as_ptr();
            assert!(tx.hand_over(48_000, &mut pending, Instant::now()));
            assert!(pending.is_empty());
            assert_ne!(pending.as_ptr(), had, "a spare took its place");
            let (rate, b, _) = rx.recv_timeout(Duration::from_secs(1)).unwrap();
            assert_eq!((rate, b.len(), b[0]), (48_000, 480, round as f32));
            rx.recycle(b);
        }
        assert_eq!(rx.take_dropped(), 0);
    }

    #[test]
    fn a_full_ring_drops_the_block_instead_of_waiting() {
        let (tx, mut rx) = raw_queue();
        let mut pending = Vec::with_capacity(480);
        for _ in 0..RAW_BLOCKS {
            pending.resize(480, 0.5);
            assert!(tx.hand_over(48_000, &mut pending, Instant::now()));
        }
        pending.resize(480, 0.5);
        assert!(!tx.hand_over(48_000, &mut pending, Instant::now()));
        assert!(pending.is_empty(), "cleared for reuse");
        assert_eq!(rx.take_dropped(), 1);
        assert!(rx.try_recv().is_some());
    }

    #[test]
    fn a_held_producer_drops_the_block_instead_of_waiting() {
        let (tx, rx) = raw_queue();
        let _held = tx.0.side.lock().unwrap();
        let mut pending = vec![0.0; 480];
        assert!(!tx.hand_over(48_000, &mut pending, Instant::now()));
        assert_eq!(rx.take_dropped(), 1);
    }

    #[test]
    fn the_receiver_is_woken_by_a_block_and_by_the_last_sender_going() {
        let (tx, mut rx) = raw_queue();
        let h = std::thread::spawn(move || {
            let first = rx.recv_timeout(Duration::from_secs(5)).map(|b| b.1.len());
            let started = Instant::now();
            let gone = rx.recv_timeout(Duration::from_secs(5));
            (first, gone, started.elapsed())
        });
        std::thread::sleep(Duration::from_millis(50));
        let mut pending = vec![0.0; 480];
        tx.hand_over(48_000, &mut pending, Instant::now());
        std::thread::sleep(Duration::from_millis(50));
        drop(tx);
        let (first, gone, waited) = h.join().unwrap();
        assert_eq!(first, Ok(480));
        assert_eq!(gone, Err(RecvTimeoutError::Disconnected));
        assert!(waited < Duration::from_secs(2), "woken, not timed out");
    }

    #[test]
    fn a_wait_with_nothing_coming_times_out() {
        let (_tx, mut rx) = raw_queue();
        assert_eq!(
            rx.recv_timeout(Duration::from_millis(20)).map(|_| ()),
            Err(RecvTimeoutError::Timeout)
        );
    }

    fn play(tap: &EchoTap, rate: u32, n: usize) {
        let mut w = tap.writer(rate).unwrap();
        for _ in 0..n {
            w.push(1);
        }
    }

    #[test]
    fn echo_drains_whole_chunks_and_keeps_the_rest() {
        let tap = EchoTap::default();
        play(&tap, 48_000, 1200); // 25 ms
        let mut out = Vec::new();
        assert_eq!(tap.drain_into(&mut out), 48_000);
        assert_eq!(out.len(), 960);
        assert_eq!(tap.queued(), 240, "5 ms waits for more");
    }

    #[test]
    fn echo_keeps_at_most_the_newest_200ms() {
        let tap = EchoTap::default();
        play(&tap, 48_000, 19_200); // 400 ms, nothing drained
        let mut out = Vec::new();
        tap.drain_into(&mut out);
        assert_eq!(out.len(), 9_600);
    }

    #[test]
    fn echo_overflow_and_rate_changes_start_over() {
        let tap = EchoTap::default();
        play(&tap, 48_000, ECHO_CAPACITY + 10);
        let mut out = Vec::new();
        tap.drain_into(&mut out);
        assert!(out.is_empty(), "overflowed: all of it is old");
        play(&tap, 48_000, 480);
        tap.drain_into(&mut out);
        assert_eq!(out.len(), 480);

        play(&tap, 48_000, 400);
        play(&tap, 44_100, 441);
        // the 48 kHz leftovers are thrown away with the rate change
        assert_eq!(tap.drain_into(&mut out), 44_100);
        assert!(out.is_empty());
    }

    #[test]
    fn a_second_output_stream_skips_rather_than_waits() {
        let tap = EchoTap::default();
        let _first = tap.writer(48_000).unwrap();
        assert!(tap.writer(48_000).is_none());
    }
}
