# Screen share step zero: per-track voice receive — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Key the client's receive tasks and mixer by track instead of by person, so one person can send two audio
tracks (voice plus stream audio) without one replacing the other.

**Architecture:**
- The mixer's sources are keyed by track SID. Each source remembers its owner (the participant identity) and its kind
  (`Voice` or `Stream`).
- Gain is set per (owner, kind), so the existing per-person voice volume keeps working and stream audio later gets its
  own volume.
- Speaking levels count only `Voice` sources, so a streamed game never lights someone's speaking ring.
- `RxTasks` is keyed by track SID and can drop everything an owner has.
- Voice behaviour is unchanged for today's single-mic case.

**Tech Stack:** Rust; the `livekit` crate 0.9.3; tokio; `cargo test`.

**Spec:** `docs/superpowers/specs/2026-10-15-screenshare-design.md` §2 "Step zero".

## Global Constraints

- No behaviour change a user can notice in voice: the same volumes, speaking rings, latency trimming and diagnostics.
- The mixer runs inside the audio callback. No allocation and no hash lookup per sample in `mix_into`; per-source state
  is read from the source itself.
- Keep the existing style: short doc comments that say why, tests next to the code in `#[cfg(test)] mod tests`.
- Commit messages end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Branch: `screenshare-step0` off `main`. Merging to `main` needs the owner's OK.

## Review Focus

1. **A person's mic and stream audio both play, and stopping the stream leaves the mic playing.** That's the bug being
   fixed. Pinned by Task 1 (mixer), Task 2 (rx) and Task 4 (real LiveKit).
2. **Per-person volume set before that person's track arrives still applies** (today `set_gain` creates the peer
   early). Pinned by Task 1, `gain_set_before_the_source_arrives_applies`.
3. **Stream audio never lights the speaking ring and isn't affected by the voice volume slider.** Pinned by Task 1.
4. **A participant disconnecting removes all their sources and tasks.** Without it, a stuck receive task replays audio
   ("robot voice", FINDINGS rule 1). Pinned by Task 1 `remove_owner…` and Task 2 `drop_owner…`.
5. **A track re-subscribed under the same SID replaces the old receive task rather than doubling the audio.** Pinned by
   Task 2 `replacing_a_track_aborts_the_old_task`.

---

## File Structure

| File | Change |
|---|---|
| `client/src-tauri/src/voice/mixer.rs` | `peers: HashMap<String, Peer>` becomes `sources: HashMap<String, Source>` plus `gains: HashMap<(String, SourceKind), f32>`. New `SourceKind`, `add_source`, `remove_source`, `remove_owner`; `set_gain` takes a kind. |
| `client/src-tauri/src/voice/rx.rs` | Keyed by track SID, with the owner stored beside the handle. `replace(track, owner, h)`, `drop_track`, `drop_owner`. |
| `client/src-tauri/src/voice/mod.rs` | The `TrackSubscribed`, `TrackUnsubscribed` and `ParticipantDisconnected` handlers and `set_peer_volume` use the new API. New `source_kind()`. |
| `client/src-tauri/src/voice/mictest.rs` | Registers its `"self"` source before pushing. |
| `client/src-tauri/tests/voice_it.rs` | A new ignored real-LiveKit test: two tracks from one person. |

---

### Task 1: Mixer sources keyed by track

**Files:**
- Modify: `client/src-tauri/src/voice/mixer.rs`
- Modify: `client/src-tauri/src/voice/mictest.rs:73`
- Test: `client/src-tauri/src/voice/mixer.rs` (`mod tests`)

**Interfaces:**
- Produces:
  - `pub enum SourceKind { Voice, Stream }` (`Clone, Copy, Debug, PartialEq, Eq, Hash`)
  - `Mixer::add_source(&mut self, source: &str, owner: &str, kind: SourceKind)`
  - `Mixer::push(&mut self, source: &str, samples: &[i16])` (an unknown source is ignored)
  - `Mixer::set_gain(&mut self, owner: &str, kind: SourceKind, gain: f32)`
  - `Mixer::remove_source(&mut self, source: &str)`
  - `Mixer::remove_owner(&mut self, owner: &str)`
  - `Mixer::take_peer_levels(&mut self) -> Vec<(String, f32)>`: per owner, `Voice` sources only, the loudest source's
    RMS
  - `Mixer::buffered(&self, source: &str) -> usize`; `max_buffered`, `take_underrun_samples`, `pushed_total` and
    `mix_into` keep their signatures.

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `mixer.rs`:

```rust
    use super::SourceKind::{Stream, Voice};

    #[test]
    fn two_tracks_from_one_person_both_play() {
        let mut m = Mixer::new(48_000, 200);
        m.add_source("TR_mic", "alex", Voice);
        m.add_source("TR_game", "alex", Stream);
        m.push("TR_mic", &[8192; 4]); // 0.25
        m.push("TR_game", &[8192; 4]); // 0.25
        let mut out = [0.0f32; 4];
        m.mix_into(&mut out, 1, 1.0);
        for v in out {
            assert!((v - 0.5).abs() < 1e-3, "both sources mixed: {v}");
        }
    }

    #[test]
    fn removing_one_track_keeps_the_other() {
        let mut m = Mixer::new(48_000, 200);
        m.add_source("TR_mic", "alex", Voice);
        m.add_source("TR_game", "alex", Stream);
        m.remove_source("TR_game");
        m.push("TR_mic", &[8192; 4]);
        m.push("TR_game", &[8192; 4]); // gone: ignored
        let mut out = [0.0f32; 4];
        m.mix_into(&mut out, 1, 1.0);
        assert!((out[0] - 0.25).abs() < 1e-3, "{}", out[0]);
        assert_eq!(m.buffered("TR_mic"), 0);
    }

    #[test]
    fn remove_owner_drops_all_their_sources() {
        let mut m = Mixer::new(48_000, 200);
        m.add_source("TR_mic", "alex", Voice);
        m.add_source("TR_game", "alex", Stream);
        m.add_source("TR_sam", "sam", Voice);
        m.push("TR_mic", &[1; 10]);
        m.push("TR_game", &[1; 10]);
        m.push("TR_sam", &[1; 10]);
        m.remove_owner("alex");
        assert_eq!(m.buffered("TR_mic"), 0);
        assert_eq!(m.buffered("TR_game"), 0);
        assert_eq!(m.buffered("TR_sam"), 10);
    }

    #[test]
    fn voice_volume_does_not_touch_stream_audio() {
        let mut m = Mixer::new(48_000, 200);
        m.add_source("TR_mic", "alex", Voice);
        m.add_source("TR_game", "alex", Stream);
        m.set_gain("alex", Voice, 0.0);
        m.push("TR_mic", &[8192; 4]);
        m.push("TR_game", &[8192; 4]);
        let mut out = [0.0f32; 4];
        m.mix_into(&mut out, 1, 1.0);
        assert!((out[0] - 0.25).abs() < 1e-3, "only the stream plays: {}", out[0]);
    }

    #[test]
    fn gain_set_before_the_source_arrives_applies() {
        // the volume slider is restored before the person's track is subscribed
        let mut m = Mixer::new(48_000, 200);
        m.set_gain("alex", Voice, 0.5);
        m.add_source("TR_mic", "alex", Voice);
        m.push("TR_mic", &[16384; 2]); // 0.5
        let mut out = [0.0f32; 2];
        m.mix_into(&mut out, 1, 1.0);
        assert!((out[0] - 0.25).abs() < 1e-3, "{}", out[0]);
    }

    #[test]
    fn stream_audio_never_lights_the_speaking_ring() {
        let mut m = Mixer::new(48_000, 200);
        m.add_source("TR_mic", "alex", Voice);
        m.add_source("TR_game", "alex", Stream);
        m.add_source("TR_sam_game", "sam", Stream);
        m.push("TR_mic", &[0; 480]);
        m.push("TR_game", &[16384; 480]);
        m.push("TR_sam_game", &[16384; 480]);
        let levels = m.take_peer_levels();
        assert_eq!(levels, vec![("alex".to_string(), 0.0)], "voice owners only, voice level only");
    }

    #[test]
    fn re_adding_a_source_starts_it_empty() {
        let mut m = Mixer::new(48_000, 200);
        m.add_source("TR_mic", "alex", Voice);
        m.push("TR_mic", &[1; 10]);
        m.add_source("TR_mic", "alex", Voice);
        assert_eq!(m.buffered("TR_mic"), 0);
    }
```

Also update the existing tests to register their sources first:
- `steady()`: add `m.add_source("a", "a", Voice);` at the start of every test that calls it, before its first push
  (`a_standing_backlog_drains_to_the_target`, `trimming_a_backlog_does_not_click`,
  `a_small_excess_from_clock_drift_is_left_alone`, `a_short_queue_is_left_alone`).
- `underruns_are_counted_per_peer_sample_missed`, `empty_peer_contributes_silence`, `cap_drops_oldest`,
  `remove_peer_drops_queue`: `m.add_source("a", "a", Voice);` first.
- `mixes_two_peers_with_per_peer_gain`: add `a` and `b` as `Voice` owners `a` and `b`; change
  `m.set_gain("b", 0.0)` to `m.set_gain("b", Voice, 0.0)`.
- `remove_peer_drops_queue`: `m.remove("a")` becomes `m.remove_source("a")`.
- `per_peer_levels_reflect_pushed_audio`: add `loud` and `quiet` as `Voice` sources owned by `loud` and `quiet`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p pulse-client voice::mixer`
Expected: compile errors: `SourceKind` not found, `add_source` / `remove_source` / `remove_owner` not found.

- [ ] **Step 3: Implement**

Replace the `Peer` type, the `peers` field and the per-peer methods in `mixer.rs`:

```rust
/// What a received track is: someone's voice, or the audio of a stream they're sharing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SourceKind {
    Voice,
    Stream,
}

/// One received track. Keyed by track SID: a person can send voice and stream audio at once.
struct Source {
    owner: String,
    kind: SourceKind,
    queue: VecDeque<i16>,
    /// Shortest the queue has been (right after a pull) since the last backlog check.
    low: usize,
    /// Copied from `gains` when added or when the gain changes: no lookup in the audio callback.
    gain: f32,
    /// Level of what we received (drives the speaking ring: lit = audible).
    meter: super::meter::Meter,
}
```

In `Mixer`, replace `peers: HashMap<String, Peer>` with:

```rust
    sources: HashMap<String, Source>,
    /// Volume per (person, kind), kept even while they have no track, so a slider set early applies.
    gains: HashMap<(String, SourceKind), f32>,
```

Initialise both with `HashMap::new()` in `new`. Then replace `peer()`, `push`, `set_gain`, `remove` and
`take_peer_levels`:

```rust
    /// Start playing a track. Re-adding a source starts it empty (a re-subscribe is a new stream).
    pub fn add_source(&mut self, source: &str, owner: &str, kind: SourceKind) {
        let gain = self
            .gains
            .get(&(owner.to_string(), kind))
            .copied()
            .unwrap_or(1.0);
        self.sources.insert(
            source.to_string(),
            Source {
                owner: owner.to_string(),
                kind,
                queue: VecDeque::new(),
                low: usize::MAX,
                gain,
                meter: Default::default(),
            },
        );
    }

    /// Queue received samples (mono i16). Beyond the cap, the oldest are dropped (bounded latency).
    /// Audio for a source that was removed (a late frame from an aborted task) is ignored.
    pub fn push(&mut self, source: &str, samples: &[i16]) {
        let cap = self.cap;
        let Some(s) = self.sources.get_mut(source) else {
            return;
        };
        self.pushed += samples.len() as u64;
        for v in samples {
            s.meter.add(*v as f32 / i16::MAX as f32 * s.gain);
        }
        let q = &mut s.queue;
        q.extend(samples.iter().copied());
        if q.len() > cap {
            let excess = q.len() - cap;
            skip(q, excess);
        }
    }

    pub fn set_gain(&mut self, owner: &str, kind: SourceKind, gain: f32) {
        self.gains.insert((owner.to_string(), kind), gain);
        for s in self.sources.values_mut() {
            if s.owner == owner && s.kind == kind {
                s.gain = gain;
            }
        }
    }

    pub fn remove_source(&mut self, source: &str) {
        self.sources.remove(source);
    }

    /// Everything a person was sending (they left the room).
    pub fn remove_owner(&mut self, owner: &str) {
        self.sources.retain(|_, s| s.owner != owner);
    }

    /// (person, rms since the last call): how each person's *voice* sounded, after their volume.
    /// Stream audio is drained too (so it doesn't build up) but never reported: a loud game isn't talking.
    pub fn take_peer_levels(&mut self) -> Vec<(String, f32)> {
        let mut out: Vec<(String, f32)> = Vec::new();
        for s in self.sources.values_mut() {
            let rms = s.meter.take().0;
            if s.kind != SourceKind::Voice {
                continue;
            }
            match out.iter_mut().find(|(o, _)| *o == s.owner) {
                Some((_, v)) => *v = v.max(rms),
                None => out.push((s.owner.clone(), rms)),
            }
        }
        out
    }
```

In `max_buffered`, `buffered` and `mix_into`, rename `self.peers` to `self.sources` (the bodies are unchanged:
`buffered` now takes a source SID). Keep the `push` diagnostics: count `pushed` only for known sources.

In `mictest.rs`, register the loopback source once, before the push loop starts (next to where `mixer` is cloned,
around line 59):

```rust
mixer.lock().unwrap().add_source("self", "self", super::mixer::SourceKind::Voice);
```

The existing `push("self", &out)` call at line 73 stays.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p pulse-client voice::mixer`
Expected: PASS, all mixer tests including the 7 new ones.

`mod.rs` won't compile yet (it still calls the old `set_gain` and `remove`). That's fixed in Task 3; run only the
mixer tests here.

- [ ] **Step 5: Commit**

```bash
git add client/src-tauri/src/voice/mixer.rs client/src-tauri/src/voice/mictest.rs
git commit -m "refactor(voice): mixer sources keyed by track, gain per person and kind

One person can now send voice and stream audio without one replacing the other.
Stream audio never lights the speaking ring.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: Receive tasks keyed by track

**Files:**
- Modify: `client/src-tauri/src/voice/rx.rs`
- Test: `client/src-tauri/src/voice/rx.rs` (`mod tests`)

**Interfaces:**
- Produces:
  - `RxTasks::replace(&mut self, track: String, owner: String, h: JoinHandle<()>)`
  - `RxTasks::drop_track(&mut self, track: &str)`
  - `RxTasks::drop_owner(&mut self, owner: &str)`
  - `len`, `is_empty` and `clear` are unchanged.

- [ ] **Step 1: Write the failing tests**

Replace the `mod tests` body in `rx.rs`:

```rust
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
        let mic = pending();
        let mic_abort = mic.abort_handle();
        let game = pending();
        let game_abort = game.abort_handle();
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
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p pulse-client voice::rx`
Expected: compile errors: `replace` takes 2 arguments, and `drop_track` / `drop_owner` aren't found.

- [ ] **Step 3: Implement**

Replace the struct and its `impl` in `rx.rs` (keep the `Drop` impl):

```rust
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
            if o == owner {
                h.abort();
                false
            } else {
                true
            }
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
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p pulse-client voice::rx`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add client/src-tauri/src/voice/rx.rs
git commit -m "refactor(voice): receive tasks keyed by track, droppable per person

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: Wire the room events to tracks

**Files:**
- Modify: `client/src-tauri/src/voice/mod.rs` (the room-events task, around lines 549-612; `set_peer_volume`, around
  line 866)
- Test: `client/src-tauri/src/voice/mod.rs` (`mod tests`)

**Interfaces:**
- Consumes: Task 1's `SourceKind`, `add_source`, `set_gain(owner, kind, gain)`, `remove_source` and `remove_owner`;
  Task 2's `replace(track, owner, h)`, `drop_track` and `drop_owner`.
- Produces: `fn source_kind(s: TrackSource) -> SourceKind` (private to `voice`).

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `voice/mod.rs`:

```rust
    #[test]
    fn screen_share_audio_is_stream_everything_else_is_voice() {
        use super::mixer::SourceKind;
        assert_eq!(source_kind(TrackSource::ScreenshareAudio), SourceKind::Stream);
        assert_eq!(source_kind(TrackSource::Microphone), SourceKind::Voice);
        // an unknown source plays like a voice: never silently dropped
        assert_eq!(source_kind(TrackSource::Unknown), SourceKind::Voice);
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p pulse-client voice::tests::screen_share_audio`
Expected: compile errors: `source_kind` not found (plus the call sites from Tasks 1 and 2).

- [ ] **Step 3: Implement**

Add near `mic_publish_options` in `mod.rs`:

```rust
/// Stream audio (a shared game's sound) gets its own volume and never lights the speaking ring.
fn source_kind(s: TrackSource) -> mixer::SourceKind {
    match s {
        TrackSource::ScreenshareAudio => mixer::SourceKind::Stream,
        _ => mixer::SourceKind::Voice,
    }
}
```

Replace the `TrackSubscribed`, `TrackUnsubscribed` and `ParticipantDisconnected` arms in the room-events task:

```rust
                        RoomEvent::TrackSubscribed {
                            track: RemoteTrack::Audio(t),
                            publication,
                            participant,
                        } => {
                            let owner = participant.identity().to_string();
                            let source = publication.sid().to_string();
                            let kind = source_kind(publication.source());
                            {
                                let mut m = mixer.lock().unwrap();
                                if kind == mixer::SourceKind::Voice {
                                    let pct =
                                        volumes.lock().unwrap().get(&owner).copied().unwrap_or(100);
                                    m.set_gain(&owner, kind, percent_to_gain(pct));
                                }
                                m.add_source(&source, &owner, kind);
                            }
                            let (m, src) = (mixer.clone(), source.clone());
                            let subscribed = Instant::now();
                            tracing::info!(
                                peer = %owner,
                                track = %source,
                                ?kind,
                                since_join_ms = started.elapsed().as_millis() as u64,
                                "voice: subscribed"
                            );
                            let peer = owner.clone();
                            let task = tokio::spawn(async move {
                                let mut stream =
                                    NativeAudioStream::new(t.rtc_track(), INTERNAL_RATE as i32, 1);
                                let mut heard = false;
                                while let Some(f) = stream.next().await {
                                    // WebRTC hands over (silent) frames before any packet arrives:
                                    // the first one with sound is when they're really audible
                                    if !heard && f.data.iter().any(|s| *s != 0) {
                                        heard = true;
                                        tracing::info!(
                                            %peer,
                                            since_subscribed_ms =
                                                subscribed.elapsed().as_millis() as u64,
                                            "voice: first sound"
                                        );
                                    }
                                    m.lock().unwrap().push(&src, &f.data);
                                }
                            });
                            rx.lock().unwrap().replace(source, owner, task);
                        }
                        RoomEvent::TrackUnsubscribed { publication, .. } => {
                            let source = publication.sid().to_string();
                            rx.lock().unwrap().drop_track(&source);
                            mixer.lock().unwrap().remove_source(&source);
                        }
                        RoomEvent::ParticipantDisconnected(p) => {
                            let owner = p.identity().to_string();
                            rx.lock().unwrap().drop_owner(&owner);
                            mixer.lock().unwrap().remove_owner(&owner);
                        }
```

In `set_peer_volume`, change the gain call to the voice kind:

```rust
                .set_gain(&user_id, mixer::SourceKind::Voice, percent_to_gain(pct));
```

The local `mixer` binding in the room-events task shadows the `mixer` module. Refer to the type through
`self::mixer::SourceKind` (or import `use mixer::SourceKind;` at the top of `mod.rs` and use `SourceKind::Voice`
everywhere in this task).

- [ ] **Step 4: Run all client tests**

Run: `cargo test -p pulse-client`
Expected: PASS, everything (the ignored real-LiveKit tests are skipped).

Run: `cargo clippy -p pulse-client --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 5: Commit**

```bash
git add client/src-tauri/src/voice/mod.rs
git commit -m "fix(voice): play every audio track a person sends, not only the latest

Receive tasks and mixer sources follow the track SID; leaving drops everything the person sent.
Prerequisite for screen-share audio.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: Real-LiveKit test: two tracks from one person

**Files:**
- Modify: `client/src-tauri/tests/voice_it.rs`
- Modify: `client/src-tauri/src/voice/mod.rs`: add a test-visible accessor next to `rx_len` (around line 925)

**Interfaces:**
- Consumes: Task 3's wiring.
- Produces: `VoiceManager::mixer_buffered_sources(&self) -> usize` (the count of sources with audio queued; for tests).

- [ ] **Step 1: Add the accessor**

In `Mixer` (`mixer.rs`):

```rust
    /// Sources currently registered (diagnostics and tests).
    pub fn source_count(&self) -> usize {
        self.sources.len()
    }
```

In `VoiceManager` (`mod.rs`, next to `rx_len`):

```rust
    pub async fn mixer_sources(&self) -> usize {
        match self.session.lock().await.as_ref() {
            Some(s) => s.shared.mixer.lock().unwrap().source_count(),
            None => 0,
        }
    }
```

- [ ] **Step 2: Write the ignored integration test**

Append to `voice_it.rs`:

```rust
/// One person publishing a mic AND stream audio: the listener plays both, and unpublishing the
/// stream audio leaves the mic playing (screen-share spec §2, step zero).
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn two_tracks_from_one_person_both_play() {
    use livekit::options::TrackPublishOptions;
    use livekit::prelude::*;
    use livekit::webrtc::audio_source::native::NativeAudioSource;
    use livekit::webrtc::audio_source::{AudioSourceOptions, RtcAudioSource};
    use livekit::webrtc::prelude::AudioFrame;

    let mut cfg = testing::test_config();
    cfg.livekit_url = "ws://127.0.0.1:7880".into();
    cfg.livekit_secret = DEV_SECRET.into();
    let app = testing::spawn_with(cfg).await;
    let (_, alex) = testing::register(&app, "alex").await;
    let (_, sam) = testing::register(&app, "sam").await;
    let s = testing::create_server(&app, &alex, "Main").await;
    let api = Api::new(&format!("http://{}", app.addr));
    api.join_server(&sam, s.id).await.unwrap();
    let lounge = api
        .channels(&alex, s.id)
        .await
        .unwrap()
        .into_iter()
        .find(|c| c.name.as_deref() == Some("Lounge"))
        .unwrap();

    // alex listens through Pulse's voice pipeline
    let vm = VoiceManager::new(std::sync::Arc::new(|_| {}));
    vm.join(&api, &alex, lounge.id, AudioConfig::default(), AudioMode::Null(48_000))
        .await
        .unwrap();

    // sam is a bare SDK client publishing two tracks
    let tok = api.voice_token(&sam, lounge.id).await.unwrap();
    let (room, _events) = Room::connect(&tok.url, &tok.token, RoomOptions::default())
        .await
        .unwrap();
    let publish = |name: &'static str, source: TrackSource| {
        let src = NativeAudioSource::new(AudioSourceOptions::default(), 48_000, 1, 100);
        let track =
            LocalAudioTrack::create_audio_track(name, RtcAudioSource::Native(src.clone()));
        (src, track, TrackPublishOptions { source, ..Default::default() })
    };
    let (mic_src, mic, mic_opts) = publish("mic", TrackSource::Microphone);
    let (game_src, game, game_opts) = publish("game", TrackSource::ScreenshareAudio);
    let lp = room.local_participant();
    lp.publish_track(LocalTrack::Audio(mic), mic_opts).await.unwrap();
    let game_pub = lp
        .publish_track(LocalTrack::Audio(game), game_opts)
        .await
        .unwrap();

    // feed both with a tone for 3 s at real time
    let feed = |src: NativeAudioSource| {
        tokio::spawn(async move {
            let tone: Vec<i16> = (0..480)
                .map(|i| ((i as f32 * 0.13).sin() * 8000.0) as i16)
                .collect();
            for _ in 0..300 {
                let frame = AudioFrame {
                    data: tone.clone().into(),
                    sample_rate: 48_000,
                    num_channels: 1,
                    samples_per_channel: 480,
                };
                src.capture_frame(&frame).await.unwrap();
            }
        })
    };
    let (a, b) = (feed(mic_src.clone()), feed(game_src));
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(vm.rx_len().await, 2, "one receive task per track");
    assert_eq!(vm.mixer_sources().await, 2, "both tracks in the mixer");

    lp.unpublish_track(&game_pub.sid()).await.unwrap();
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(vm.rx_len().await, 1, "the mic task survives");
    assert_eq!(vm.mixer_sources().await, 1, "the mic source survives");
    let before = vm.mixer_pushed().await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(vm.mixer_pushed().await > before, "the mic is still playing");

    a.abort();
    b.abort();
    room.close().await.unwrap();
    vm.leave().await;
}
```

If the `livekit` 0.9.3 types differ from the snippet (for example `AudioFrame.data` is `Cow<[i16]>`, or `capture_frame`
returns `Result<(), _>`), adjust to the crate's signatures. The `spikes/voice` code and `voice/mod.rs` already publish
a `NativeAudioSource` and are the local reference.

- [ ] **Step 3: Run it against the dev LiveKit**

Run: `just dev-livekit` (in another terminal), then
`cargo test -p pulse-client --test voice_it two_tracks_from_one_person_both_play -- --ignored --nocapture`
Expected: PASS.

Sanity check that the test can fail: temporarily change `rx.lock().unwrap().replace(source, owner, task)` in
`mod.rs` to key by `owner.clone()` (the old behaviour). The test must fail on "one receive task per track". Revert.

- [ ] **Step 4: Run everything**

Run: `just test` and `just voice-it`
Expected: PASS (all existing real-LiveKit voice tests still pass).

- [ ] **Step 5: Commit**

```bash
git add client/src-tauri/tests/voice_it.rs client/src-tauri/src/voice/mod.rs client/src-tauri/src/voice/mixer.rs
git commit -m "test(voice): a person's mic and stream audio both play; dropping one keeps the other

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

## Done when

- `just test`, `cargo clippy --all-targets -- -D warnings` and `just voice-it` pass.
- A review (Opus) of the branch finds no blocking issues.
- With the owner's OK: merged to `main`. No Windows test is needed on its own: voice behaviour is unchanged, and the
  next Windows test covers it.
