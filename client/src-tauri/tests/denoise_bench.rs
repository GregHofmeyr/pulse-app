//! Measures the denoisers (run: `cargo test --release --test denoise_bench -- --ignored --nocapture`,
//! and again with `--features dfn-ll`). Results go into spikes/voice/FINDINGS.md.

use std::time::{Duration, Instant};

const FRAME: usize = 480;

fn speech() -> Vec<f32> {
    include_bytes!("fixtures/speech_48k_mono_s16le.raw")
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0)
        .collect()
}

/// 60 s: the speech clip looped, plus deterministic noise and a click every 150 ms.
fn workload() -> Vec<f32> {
    let s = speech();
    let mut x: u32 = 1;
    (0..48_000 * 60)
        .map(|i| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            let noise = (x as f32 / u32::MAX as f32 - 0.5) * 0.02;
            let click = if i % 7200 < 96 {
                0.4 * (1.0 - (i % 7200) as f32 / 96.0)
            } else {
                0.0
            };
            s[i % s.len()] * 0.5 + noise + click
        })
        .collect()
}

fn rss_mb() -> f64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmRSS:"))
                .map(str::to_owned)
        })
        .and_then(|l| l.split_whitespace().nth(1)?.parse::<f64>().ok())
        .map_or(0.0, |kb| kb / 1024.0)
}

fn report(name: &str, mut times: Vec<Duration>) {
    times.sort();
    let mean = times.iter().sum::<Duration>() / times.len() as u32;
    let p99 = times[times.len() * 99 / 100];
    let max = *times.last().unwrap();
    println!(
        "{name}: per 10 ms block mean {mean:?} p99 {p99:?} max {max:?} ({:.1}% of budget)",
        mean.as_secs_f64() * 100.0 / 0.010
    );
}

/// Lag (ms) maximising the cross-correlation of output against input over 0..100 ms.
fn delay_ms(input: &[f32], output: &[f32]) -> f32 {
    let n = input.len().min(output.len());
    let best = (0..4800)
        .max_by(|&a, &b| {
            let c = |lag: usize| {
                (0..n - lag)
                    .step_by(4)
                    .map(|i| input[i] * output[i + lag])
                    .sum::<f32>()
            };
            c(a).total_cmp(&c(b))
        })
        .unwrap();
    best as f32 / 48.0
}

#[test]
#[ignore]
fn bench_denoisers() {
    let w = workload();

    let mut st = nnnoiseless::DenoiseState::new();
    let (mut inb, mut outb) = (vec![0f32; FRAME], vec![0f32; FRAME]);
    let mut times = Vec::new();
    let mut rn_out = Vec::with_capacity(w.len());
    for block in w.as_chunks::<FRAME>().0 {
        inb.iter_mut()
            .zip(block)
            .for_each(|(d, s)| *d = s * 32767.0);
        let t = Instant::now();
        st.process_frame(&mut outb, &inb);
        times.push(t.elapsed());
        rn_out.extend(outb.iter().map(|v| v / 32767.0));
    }
    report("rnnoise", times);
    println!(
        "rnnoise: delay {:.1} ms",
        delay_ms(&w[..48_000 * 3], &rn_out[..48_000 * 3])
    );

    let r0 = rss_mb();
    let t = Instant::now();
    let mut df = df::tract::DfTract::new(
        df::tract::DfParams::default(),
        &df::tract::RuntimeParams::default_with_ch(1),
    )
    .expect("load DeepFilterNet");
    println!(
        "deepfilter ({}): load {:?}, rss +{:.1} MB, hop {}, lookahead {}, fft {}",
        if cfg!(feature = "dfn-ll") {
            "low-latency"
        } else {
            "normal"
        },
        t.elapsed(),
        rss_mb() - r0,
        df.hop_size,
        df.lookahead,
        df.fft_size
    );
    assert_eq!(df.hop_size, FRAME);
    let mut noisy = ndarray::Array2::<f32>::zeros((1, FRAME));
    let mut enh = ndarray::Array2::<f32>::zeros((1, FRAME));
    let mut times = Vec::new();
    let mut df_out = Vec::with_capacity(w.len());
    for block in w.as_chunks::<FRAME>().0 {
        noisy
            .row_mut(0)
            .iter_mut()
            .zip(block)
            .for_each(|(d, s)| *d = *s);
        let t = Instant::now();
        df.process(noisy.view(), enh.view_mut())
            .expect("df process");
        times.push(t.elapsed());
        df_out.extend(enh.row(0).iter().copied());
    }
    report("deepfilter", times);
    println!(
        "deepfilter: delay {:.1} ms",
        delay_ms(&w[..48_000 * 3], &df_out[..48_000 * 3])
    );
}
