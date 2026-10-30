# NetEq "buffer" vs measured lag (R5)

Date: 2026-10-16. Build: ba9ad36. livekit-server v1.13 (dev, host network). libwebrtc 0.3.50.

Verdict: **standing delay** (Step 6's "standing-delay" rule, not "artifact"). `buffer` tracks the measured lag within 3-10 ms, so it is audio people hear, not a
statistic. On the bare path the excess over `target` is only about 10 ms (accel 0). DTX gaps and a realistic gate
pattern do not inflate it. The field's 80-100 over 20-40 did not reproduce here. What does inflate it: a send stall with no frames raises NetEq's `target` and `buffer`
by about the stall length and keeps them there (stall 200 ms → target 200, 500 ms → 520, still there 30 s later,
accel 0). In the probe, the "Bursts 520 ms" result came from exactly this. RTP timestamps count samples, so a
send that stops and restarts makes every later packet look late by the gap, and NetEq's inter-arrival estimate
takes that as the network's delay.

## How the path line computes it
Per inbound stream (SSRC), per 5 s window: Δ jitterBufferDelay / Δ jitterBufferEmittedCount (buffer),
Δ jitterBufferTargetDelay / Δ emitted (target). Our own queues are not in it (that's `recv`).

## Runs
The four Step 6 runs. Probe at 2a79a40 (Continuous, then Bursts, on one stream).

| pattern | netem qdisc delay ms (each lo crossing) | lag p50 | lag p90 | jb from lag | buffer | target | accel ms/5s | silent % | onsets |
|---|---|---|---|---|---|---|---|---|---|
| Continuous | 0 | 50 | 50 | 40 | 30 | 20 | 0 | 0 | 30 |
| Bursts | 0 | 528 | 528 | 518 | 488 | 520 | 0 | 0 | 14 |
| Continuous | 40 | 124 | 124 | 34 | 30 | 20 | 0 | 0 | 30 |
| Bursts | 40 | 607 | 607 | 517 | 480 | 520 | 0 | 0 | 14 |

Corrections:
- Under netem the audio crosses lo twice (sender→SFU→receiver), so `jb from lag` = lag − 2 × 40 − 10. The
  probe printed 74 and 557 (one × 40); the table shows the corrected values. The probe now subtracts 2×.
- Both Bursts rows are **invalid**. In that version the probe sent no frames for about 500 ms between patterns,
  and the second pattern on the stream inherited a 500 ms higher target, whichever pattern it was (see (a)
  below). A repeat no-delay run read 47/37/30/20 and 523/513/481/520.

Experiments, no netem. Probe at ba9ad36 unless marked; every test run is a fresh room and stream.

| run | pattern | lag p50 | lag p90 | jb from lag | buffer | target | accel ms/5s | silent % | send pkt/s | onsets |
|---|---|---|---|---|---|---|---|---|---|---|
| (a) order swapped (`PROBE_PATTERNS=bursts,continuous`), old probe with its 500 ms stall between (now `PROBE_STALL_MS=500`) | Bursts | 48 | 48 | 38 | 30 | 20 | 0 | 0 | 28 | 14 |
| | Continuous | 447 | 528 | 437 | 433 | 520 | 0 | 0 | 50 | 30 |
| (b) gate-like: talk 1-2 s, gaps 0.3-2 s seeded, 100 ms fade | Gated | 43 | 43 | 33 | 30 | 20 | 0 | 0 | 34 | 11 |
| (c) −60 dBFS noise in the gaps, not zeros | Noise | 50 | 50 | 40 | 30 | 20 | 0 | 0 | 37 | 14 |
| fixed probe, no stall | Continuous | 50 | 50 | 40 | 30 | 20 | 0 | 0 | 50 | 30 |
| | Bursts | 47 | 47 | 37 | 30 | 20 | 0 | 0 | 28 | 14 |
| `PROBE_STALL_MS=200` | Continuous | 46 | 46 | 36 | 30 | 20 | 0 | 0 | 50 | 30 |
| | Continuous (after 200 ms stall) | 214 | 214 | 204 | 200 | 200 | 0 | 0 | 50 | 30 |
| `PROBE_STALL_MS=500` | Continuous | 45 | 45 | 35 | 30 | 20 | 0 | 0 | 50 | 30 |
| | Continuous (after 500 ms stall) | 381 | 521 | 371 | 415 | 520 | 0 | 0 | 50 | 30 |

Notes:
- `send pkt/s` is the sender's packet rate. 28 for Bursts against 50 for Continuous shows DTX engaging in the
  zero gaps. −60 dBFS noise only partly stops it (37).
- `silent %` read 0 in every run, DTX included: libwebrtc's `silentConcealedSamples` does not count DTX/CNG
  playout, so `silent` can't flag DTX. The sender's packet rate can.

## Reading
Applying R5 Step 6's rule to the netem run (Continuous, which comes first on the stream, so it is uncontaminated):
- `jb from lag` is 34, within 15 ms of `buffer` (30). That is the **standing-delay** rule.
- The **artifact** rule needs `buffer − jb from lag ≥ 30`, and here it is −4.

Every clean run agrees: `buffer` sits within 3-10 ms of the measured jitter-buffer time, and `target` sits about
10 ms under it.

DTX is not the cause. Bursts (b) and (c) all read 20/30 on a fresh, unstalled stream, so Step 7 does not apply.
The 520 ms Bursts target came only from the probe's own inter-pattern send stall. Neither the hard 1 s on/off
pattern nor the gate-like irregular pattern (b) inflates it.

The baseline's 80-100 ms `buffer` over a 20-40 ms `target` did not reproduce on the bare path here. The
excess was 10 ms, with accel 0.

## For the playout task
- Standing excess over target on the bare path: about 10 ms (Continuous, netem 40 ms per lo crossing: 80 ms sender to receiver), accel 0 ms per 5 s.
- Real and sticky: any send stall that lets RTP timestamps fall behind wall-clock time adds its full length to
  NetEq's target and buffer (200 → +180 ms, 500 → +500 ms). Accel stays 0, so NetEq never drains it. It also lasts
  at least 30 s.
- The app's send path covers stalls over 50 ms with zero frames (`GapFill` in `voice/backlog.rs`). That keeps
  timestamps level with wall-clock time and should prevent this. But anything that makes the sent sample count
  fall behind real time would show up exactly like this: stalls under the limit that pile up, or a capture
  clock running slow (the H4 question).
- The stall signature is `target` and `buffer` rising **together**. The baseline's signature (`buffer` 80-100
  over `target` 20-40) is different, and it reproduced neither on the bare path nor with stalls. Its cause is
  still open. Candidates: the old cross-peer summing (fixed here), or something on the field route or the
  Windows build. Re-read the field path line now that the window is per stream and shows `accel`: if the gap is
  still there, compare `buffer` with a measured lag on that route before changing playout.
