//! Hostile-packet fuzzing of the soundboard's Opus decoding (#123): every listener decodes files
//! anyone in a server uploaded, and the server only checks the Ogg framing, never the Opus packets
//! inside. Each case is a mutated or random run of packets fed through [`super::decode_packets`]
//! (the production decode loop) on a worker thread, which must neither panic nor take longer than
//! a time budget.
//!
//! `fuzz_decoder_quick` runs a fixed-seed batch with every `cargo test`. `fuzz_decoder_long` is the
//! open-ended run (ignored by default): `PULSE_FUZZ_SECS=600 cargo test -p pulse-client --lib
//! fuzz_decoder_long -- --ignored --nocapture` (optionally `PULSE_FUZZ_SEED=<n>`).

use std::borrow::Cow;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use pulse_protocol::sound::{OggOpus, parse_ogg_opus};

use super::decode::{decode_packets, tests::sine_ogg_secs};

/// Longest one case may take. A real 120 ms packet decodes in well under a millisecond (opt-level
/// 3 even in test builds), and a case is at most a dozen packets.
const BUDGET: Duration = Duration::from_secs(2);

/// xorshift64*: deterministic, no dependency, good enough to drive mutations.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn byte(&mut self) -> u8 {
        self.next() as u8
    }
    fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.byte()).collect()
    }
}

/// Real packets to mutate: a mono and a stereo sine, when the system ffmpeg is there (random
/// packets alone still run without it).
fn corpus() -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for stereo in [false, true] {
        let Some(ogg) = (if stereo {
            stereo_ogg()
        } else {
            sine_ogg_secs(1)
        }) else {
            continue;
        };
        let o = parse_ogg_opus(&ogg).expect("ffmpeg's file parses");
        out.extend(o.packets.iter().map(|p| p.to_vec()));
    }
    out
}

fn stereo_ogg() -> Option<Vec<u8>> {
    let dir = crate::clips::ffmpeg::dev_override().unwrap_or_else(|| "/usr/bin".into());
    let out = std::process::Command::new(dir.join("ffmpeg"))
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=330:duration=1",
        ])
        .args(["-f", "lavfi", "-i", "anoisesrc=d=1:a=0.3"])
        .args(["-filter_complex", "[0][1]amerge=inputs=2"])
        .args([
            "-ar", "48000", "-c:a", "libopus", "-b:a", "96k", "-f", "ogg", "pipe:1",
        ])
        .output()
        .ok()?;
    out.status.success().then_some(out.stdout)
}

/// One hostile packet: a mutation of a real one, or made up.
fn packet(rng: &mut Rng, corpus: &[Vec<u8>]) -> Vec<u8> {
    let real = |rng: &mut Rng| -> Vec<u8> {
        if corpus.is_empty() {
            let n = 1 + rng.below(400);
            rng.bytes(n)
        } else {
            corpus[rng.below(corpus.len())].clone()
        }
    };
    let mut p = real(rng);
    match rng.below(12) {
        // flip a few bits
        0..=2 => {
            for _ in 0..1 + rng.below(8) {
                if !p.is_empty() {
                    let i = rng.below(p.len());
                    p[i] ^= 1 << rng.below(8);
                }
            }
        }
        // overwrite a few bytes
        3 | 4 => {
            for _ in 0..1 + rng.below(16) {
                if !p.is_empty() {
                    let i = rng.below(p.len());
                    p[i] = rng.byte();
                }
            }
        }
        // a random TOC (mode, bandwidth, frame size, channel flag, frame-count code)
        5 => {
            if !p.is_empty() {
                p[0] = rng.byte();
            }
        }
        // cut short
        6 => p.truncate(rng.below(p.len() + 1)),
        // random tail
        7 => {
            let n = rng.below(2000);
            p.extend(rng.bytes(n));
        }
        // a real TOC over random payload
        8 => {
            let n = rng.below(1500);
            p.truncate(1);
            p.extend(rng.bytes(n));
        }
        // fully random, any length a 256 KiB file could carry (an Ogg packet isn't capped at 1275)
        9 => {
            let n = match rng.below(4) {
                0 => rng.below(4),
                1 => rng.below(1300),
                2 => rng.below(8000),
                _ => rng.below(70_000),
            };
            p = rng.bytes(n);
        }
        // code-3 packet (many frames) with hostile frame count / padding bytes
        10 => {
            let toc = (rng.byte() & !0b11) | 0b11;
            let mut q = vec![toc, rng.byte()];
            for _ in 0..rng.below(8) {
                q.push(if rng.below(2) == 0 { 255 } else { rng.byte() });
            }
            let n = rng.below(3000);
            q.extend(rng.bytes(n));
            p = q;
        }
        // empty (packet loss concealment path)
        _ => p.clear(),
    }
    p
}

/// A case: a channel count and a run of packets decoded on one decoder (its state carries).
fn case(rng: &mut Rng, corpus: &[Vec<u8>]) -> (u8, Vec<Vec<u8>>) {
    let channels = 1 + rng.below(2) as u8;
    let n = 1 + rng.below(12);
    let mut packets = Vec::with_capacity(n);
    for _ in 0..n {
        // some real, unmutated packets in between, so the decoder's state is "warm"
        if !corpus.is_empty() && rng.below(3) == 0 {
            packets.push(corpus[rng.below(corpus.len())].clone());
        } else {
            packets.push(packet(rng, corpus));
        }
    }
    (channels, packets)
}

/// Run `cases` (or until `until`) from `seed`, each on the worker under [`BUDGET`]. Returns the
/// findings, the cases run, the slowest case, and the case to replay for each finding.
fn run(seed: u64, cases: usize, until: Option<Instant>) -> Report {
    let corpus = corpus();
    let mut rng = Rng(seed | 1);
    let mut rep = Report::default();
    let (to_worker, jobs) = mpsc::channel::<(u8, Vec<Vec<u8>>)>();
    let (done_tx, done) = mpsc::channel::<Result<(), String>>();
    let mut worker = Some(spawn_worker(jobs, done_tx.clone()));
    let mut to_worker = Some(to_worker);
    for i in 0..cases {
        if until.is_some_and(|u| Instant::now() >= u) {
            break;
        }
        let (channels, packets) = case(&mut rng, &corpus);
        rep.packets += packets.len();
        let t = Instant::now();
        to_worker
            .as_ref()
            .unwrap()
            .send((channels, packets.clone()))
            .unwrap();
        match done.recv_timeout(BUDGET) {
            Ok(Ok(())) => {}
            Ok(Err(msg)) => {
                rep.replay.push((channels, packets));
                rep.findings.push(format!("case {i}: panic: {msg}"));
            }
            Err(_) => {
                rep.replay.push((channels, packets));
                rep.findings.push(format!("case {i}: took over {BUDGET:?}"));
                // the stuck worker can't be stopped: abandon it and start another
                let (tx, jobs) = mpsc::channel();
                to_worker = Some(tx);
                worker = Some(spawn_worker(jobs, done_tx.clone()));
                // a late answer from the stuck one would be misread as the next case's
                while done.try_recv().is_ok() {}
            }
        }
        rep.slowest = rep.slowest.max(t.elapsed());
        rep.cases = i + 1;
    }
    drop(to_worker);
    drop(worker);
    rep
}

#[derive(Default)]
struct Report {
    cases: usize,
    packets: usize,
    slowest: Duration,
    /// What went wrong, one line per case.
    findings: Vec<String>,
    replay: Vec<(u8, Vec<Vec<u8>>)>,
}

fn spawn_worker(
    jobs: mpsc::Receiver<(u8, Vec<Vec<u8>>)>,
    done: mpsc::Sender<Result<(), String>>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        for (channels, packets) in jobs {
            let r = std::panic::catch_unwind(|| {
                let ogg = OggOpus {
                    channels,
                    pre_skip: 0,
                    packets: packets.iter().map(|p| Cow::Borrowed(&p[..])).collect(),
                    duration_ms: 6000,
                };
                let mut total = 0usize;
                // errors are fine (a bad file is refused); only panics and hangs are findings
                let _ = decode_packets(&ogg, |mono| {
                    assert!(mono.iter().all(|v| v.is_finite()), "non-finite sample");
                    total += mono.len();
                    true
                });
                total
            });
            let r = r.map(|_| ()).map_err(|e| {
                e.downcast_ref::<String>()
                    .cloned()
                    .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_else(|| "panic".into())
            });
            if done.send(r).is_err() {
                return;
            }
        }
    })
}

fn hex(p: &[u8]) -> String {
    p.iter().take(64).map(|b| format!("{b:02x}")).collect()
}

fn print(rep: &Report, seed: u64) {
    eprintln!(
        "opus fuzz: seed {seed}, {} cases, {} packets, slowest case {:.1} ms, {} findings",
        rep.cases,
        rep.packets,
        rep.slowest.as_secs_f64() * 1000.0,
        rep.findings.len()
    );
    for (f, (ch, packets)) in rep.findings.iter().zip(&rep.replay).take(10) {
        eprintln!("  {f} (channels {ch})");
        for p in packets {
            eprintln!("    {} bytes: {}", p.len(), hex(p));
        }
    }
}

#[test]
fn fuzz_decoder_quick() {
    let seed = 0x5eed_0123;
    let rep = run(seed, 500, None);
    print(&rep, seed);
    assert!(rep.findings.is_empty(), "{:?}", rep.findings);
}

#[test]
#[ignore = "open-ended fuzz run: PULSE_FUZZ_SECS=<n> ... -- --ignored --nocapture"]
fn fuzz_decoder_long() {
    let secs: u64 = std::env::var("PULSE_FUZZ_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(60);
    let seed: u64 = std::env::var("PULSE_FUZZ_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos() as u64
        });
    let rep = run(
        seed,
        usize::MAX,
        Some(Instant::now() + Duration::from_secs(secs)),
    );
    print(&rep, seed);
    assert!(rep.findings.is_empty(), "{:?}", rep.findings);
}
