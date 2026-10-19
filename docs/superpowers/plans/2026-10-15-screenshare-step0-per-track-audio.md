# Screen share step zero: per-track voice receive — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

> **Revised 2026-10-19 against bd62296** (after the optimisation run). Changes from the original:
> - The mixer is now two halves: `MixerInput` (network side, locked `sources` map) and `Mixer` (speaker callback,
>   fed by `Cmd`s and lock-free rings). All the new per-track API lives on `MixerInput` and takes `&self`. The callback
>   side only gains a `gain` field on `Cmd::Add`; it never learns about owners or kinds (L7a: no wait on network locks;
>   L7b: drift-compensated playout untouched).
> - Room-scoped calls (`add_source`, `remove_source`, `remove_owner`) take the room's generation, like `push_in`, so a
>   handler of the last room still running across a `clear` can't add or remove anything in the new one.
> - A push for a source that isn't registered is now ignored instead of creating a peer. This also stops a late frame
>   from an aborted receive task (or a volume set for someone absent) from leaving a phantom peer that counts underrun.
> - Gains per (owner, kind) are kept across `clear`: a volume belongs to the person, not the room.
> - Each task now leaves the crate compiling and every test green: Tasks 1 and 2 move today's call sites (still keyed
>   by identity) to the new API, and Task 3 switches them to track SIDs. The original plan ran `voice::mixer` tests
>   while `mod.rs` didn't compile, which can't work in one crate.
> - Tests use `mixer(..)` / `MixerInput` / `Mixer`, and existing tests register their sources through a `mixer_with`
>   helper. Two more tests need a registration: `devices::tests::playback_never_waits_on_the_network_side` and
>   `voice::tests::a_detached_room_gets_no_more_frames`. The mic test's `"self"` source is registered by a new
>   `Loopback::new`, with a test.
> - `mod.rs` line references updated (handlers around line 1339, `set_peer_volume` around line 1587). The accessor
>   for Task 4 is `MixerInput::source_count`.
> - Baseline at bd62296: `cargo test -p pulse-client --lib voice::mixer` passes 21 tests, `voice::rx` passes 2.

**Goal:** Key the client's receive tasks and mixer by track instead of by person, so one person can send two audio
tracks (voice plus stream audio) without one replacing the other.

**Architecture:**
- The mixer's sources are keyed by track SID. On the network side (`MixerInput`) each source remembers its owner
  (the participant identity) and its kind (`Voice` or `Stream`). The speaker side (`Mixer`) keys its working queues by
  the same SID and knows only a gain per source.
- Gain is set per (owner, kind), so the existing per-person voice volume keeps working and stream audio later gets its
  own volume. `MixerInput` turns a (owner, kind) gain into one `Cmd::Gain` per matching source.
- Speaking levels count only `Voice` sources, so a streamed game never lights someone's speaking ring.
- `RxTasks` is keyed by track SID and can drop everything an owner has.
- Voice behaviour is unchanged for today's single-mic case.

**Tech Stack:** Rust; the `livekit` crate 0.9.3 (libwebrtc 0.3.50); tokio; `rtrb`; `cargo test`.

**Spec:** `docs/superpowers/specs/2026-10-15-screenshare-design.md` §2 "Step zero".

## Global Constraints

- No behaviour change a user can notice in voice: the same volumes, speaking rings, latency trimming and diagnostics.
- The speaker callback (`Mixer::mix_into` and `intake`) never waits on a network-side lock (L7a: commands are taken
  with `try_lock` only) and its drift-compensated playout (L7b, about 20 ms) is untouched. No allocation and no hash
  lookup per sample: per-source state (gain) is read from the source itself. Owners and kinds stay on the network side.
- Keep the existing style: short doc comments that say why, tests next to the code in `#[cfg(test)] mod tests`.
- Commit messages end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Branch: `screenshare-step0` off `main`. Merging to `main` needs the owner's OK.

## Review Focus

1. **A person's mic and stream audio both play, and stopping the stream leaves the mic playing.** That's the bug being
   fixed. Pinned by Task 1 (mixer), Task 2 (rx) and Task 4 (real LiveKit).
2. **Per-person volume set before that person's track arrives still applies**, and survives a channel switch's
   `clear`. Today `set_gain` creates the peer early; now the gain waits in `gains`. Pinned by Task 1
   `gain_set_before_the_source_arrives_applies` and `a_volume_survives_a_clear`.
3. **Stream audio never lights the speaking ring and isn't affected by the voice volume slider.** Pinned by Task 1.
4. **A participant disconnecting removes all their sources and tasks.** Without it, a stuck receive task replays audio
   ("robot voice", FINDINGS rule 1). Pinned by Task 1 `remove_owner_drops_all_their_sources` and Task 2
   `drop_owner_aborts_everything_they_sent`.
5. **A track re-subscribed under the same SID replaces the old receive task rather than doubling the audio.** Pinned by
   Task 2 `replacing_a_track_aborts_the_old_task`.
6. **The generation rules still hold**: nothing from the last room (push, add or remove) lands after a `clear`, and an
   unregistered source never becomes a peer. Pinned by Task 1 `a_push_for_an_unknown_source_is_ignored`,
   `a_source_added_for_the_last_room_is_ignored`, `a_remove_for_the_last_room_leaves_the_new_one_alone` and the
   rewritten `a_push_from_before_a_clear_is_ignored`.

---

## File Structure

| File | Change |
|---|---|
| `client/src-tauri/src/voice/mixer.rs` | New `SourceKind`. `MixerInput.peers: Mutex<HashMap<String, PeerIn>>` becomes `sources: Mutex<HashMap<String, SourceIn>>` plus `gains: Mutex<HashMap<(String, SourceKind), f32>>`. New `add_source`, `remove_source`, `remove_owner`, `source_count`; `set_gain` takes a kind; `remove` and the auto-creating `peer()` go. `Cmd::Add` carries the starting gain. `Mixer` (callback side) otherwise unchanged. |
| `client/src-tauri/src/voice/rx.rs` | Keyed by track SID, with the owner stored beside the handle. `replace(track, owner, h)`, `drop_track`, `drop_owner`; `drop_for` goes. |
| `client/src-tauri/src/voice/mod.rs` | The `TrackSubscribed`, `TrackUnsubscribed` and `ParticipantDisconnected` arms (around line 1339) and `set_peer_volume` (around line 1587) use the new API. New `source_kind()` and `VoiceManager::mixer_sources()`. The test `a_detached_room_gets_no_more_frames` registers its source. |
| `client/src-tauri/src/voice/devices.rs` | Test `playback_never_waits_on_the_network_side` (line 1275) registers its source and holds `mix_in.sources`. |
| `client/src-tauri/src/voice/mictest.rs` | `Loopback::new` registers the `"self"` source; a test pins it. |
| `client/src-tauri/tests/voice_it.rs` | A new ignored real-LiveKit test: two tracks from one person. |

---

### Task 1: Mixer sources keyed by track

**Files:**
- Modify: `client/src-tauri/src/voice/mixer.rs`
- Modify: `client/src-tauri/src/voice/mictest.rs` (`Loopback`, lines 18-30; `MicTest::start`, line 72)
- Modify: `client/src-tauri/src/voice/mod.rs` (call sites only, still keyed by identity: lines ~41, ~1346, ~1378,
  ~1383, ~1596; test `a_detached_room_gets_no_more_frames`, line ~1861)
- Modify: `client/src-tauri/src/voice/devices.rs` (test at line 1275)
- Test: `mixer.rs` and `mictest.rs` (`mod tests`)

**Interfaces:**
- Produces (all on `MixerInput`, all `&self`):
  - `pub enum SourceKind { Voice, Stream }` (`Clone, Copy, Debug, PartialEq, Eq, Hash`)
  - `add_source(&self, generation: u64, source: &str, owner: &str, kind: SourceKind)`: registers a track; a stale
    `generation` is ignored; re-adding a SID starts it empty.
  - `push(&self, source: &str, samples: &[i16])` and `push_in(&self, generation: u64, source: &str, samples: &[i16])`:
    signatures unchanged; an unregistered source is ignored (and not counted in `pushed_total`).
  - `set_gain(&self, owner: &str, kind: SourceKind, gain: f32)`: not room-scoped, no generation.
  - `remove_source(&self, generation: u64, source: &str)`
  - `remove_owner(&self, generation: u64, owner: &str)`
  - `take_peer_levels(&self) -> Vec<(String, f32)>`: per owner, `Voice` sources only, the loudest source's RMS.
  - `clear`, `generation`, `take_underrun_samples`, `take_adjusted_samples`, `pushed_total`, `max_buffered`: unchanged
    signatures. `clear` keeps `gains`.
  - Removed: `MixerInput::remove`.
- `Mixer::buffered(&mut self, peer: &str)` and `Mixer::mix_into` keep their signatures (`buffered` now takes a SID).

- [ ] **Step 1: Write the failing tests**

At the top of `mod tests` in `mixer.rs`, add the imports and two helpers:

```rust
    use super::SourceKind::{Stream, Voice};

    /// A mixer with each of `ids` added as a voice source owned by itself (most tests need one).
    fn mixer_with(rate: u32, cap_ms: u32, ids: &[&str]) -> (MixerInput, Mixer) {
        let (input, m) = mixer(rate, cap_ms);
        for id in ids {
            input.add_source(input.generation(), id, id, Voice);
        }
        (input, m)
    }

    /// One person sending two tracks: their mic and the audio of a stream they share.
    fn alex_mic_and_game() -> (MixerInput, Mixer) {
        let (input, m) = mixer(48_000, 200);
        let g = input.generation();
        input.add_source(g, "TR_mic", "alex", Voice);
        input.add_source(g, "TR_game", "alex", Stream);
        (input, m)
    }
```

Add these tests:

```rust
    #[test]
    fn two_tracks_from_one_person_both_play() {
        let (input, mut m) = alex_mic_and_game();
        input.push("TR_mic", &[8192; 4]); // 0.25
        input.push("TR_game", &[8192; 4]); // 0.25
        let mut out = [0.0f32; 4];
        m.mix_into(&mut out, 1, 1.0);
        for v in out {
            assert!((v - 0.5).abs() < 1e-3, "both sources mixed: {v}");
        }
    }

    #[test]
    fn removing_one_track_keeps_the_other() {
        let (input, mut m) = alex_mic_and_game();
        input.remove_source(input.generation(), "TR_game");
        input.push("TR_mic", &[8192; 4]);
        input.push("TR_game", &[8192; 4]); // gone: ignored
        let mut out = [0.0f32; 4];
        m.mix_into(&mut out, 1, 1.0);
        assert!((out[0] - 0.25).abs() < 1e-3, "{}", out[0]);
        assert_eq!(input.pushed_total(), 4, "the removed track's push isn't counted");
        assert_eq!(m.buffered("TR_game"), 0);
    }

    #[test]
    fn remove_owner_drops_all_their_sources() {
        let (input, mut m) = alex_mic_and_game();
        input.add_source(input.generation(), "TR_sam", "sam", Voice);
        for s in ["TR_mic", "TR_game", "TR_sam"] {
            input.push(s, &[1; 10]);
        }
        input.remove_owner(input.generation(), "alex");
        assert_eq!(m.buffered("TR_mic"), 0);
        assert_eq!(m.buffered("TR_game"), 0);
        assert_eq!(m.buffered("TR_sam"), 10);
        assert_eq!(input.take_peer_levels().len(), 1, "only sam is left");
    }

    #[test]
    fn voice_volume_does_not_touch_stream_audio() {
        let (input, mut m) = alex_mic_and_game();
        input.set_gain("alex", Voice, 0.0);
        input.push("TR_mic", &[8192; 4]);
        input.push("TR_game", &[8192; 4]);
        let mut out = [0.0f32; 4];
        m.mix_into(&mut out, 1, 1.0);
        assert!((out[0] - 0.25).abs() < 1e-3, "only the stream plays: {}", out[0]);
    }

    #[test]
    fn gain_set_before_the_source_arrives_applies() {
        // the volume slider is restored before the person's track is subscribed
        let (input, mut m) = mixer(48_000, 200);
        input.set_gain("alex", Voice, 0.5);
        input.add_source(input.generation(), "TR_mic", "alex", Voice);
        input.push("TR_mic", &[16384; 2]); // 0.5
        let mut out = [0.0f32; 2];
        m.mix_into(&mut out, 1, 1.0);
        assert!((out[0] - 0.25).abs() < 1e-3, "{}", out[0]);
    }

    #[test]
    fn a_volume_survives_a_clear() {
        // a channel switch clears the room's people, not the person's volume
        let (input, mut m) = mixer(48_000, 200);
        input.set_gain("alex", Voice, 0.5);
        input.clear();
        input.add_source(input.generation(), "TR_mic", "alex", Voice);
        input.push("TR_mic", &[16384; 2]);
        let mut out = [0.0f32; 2];
        m.mix_into(&mut out, 1, 1.0);
        assert!((out[0] - 0.25).abs() < 1e-3, "{}", out[0]);
    }

    #[test]
    fn stream_audio_never_lights_the_speaking_ring() {
        let (input, _m) = alex_mic_and_game();
        input.add_source(input.generation(), "TR_sam_game", "sam", Stream);
        input.push("TR_mic", &[0; 480]);
        input.push("TR_game", &[16384; 480]);
        input.push("TR_sam_game", &[16384; 480]);
        assert_eq!(
            input.take_peer_levels(),
            vec![("alex".to_string(), 0.0)],
            "voice owners only, voice level only"
        );
    }

    #[test]
    fn re_adding_a_source_starts_it_empty() {
        let (input, mut m) = mixer_with(48_000, 200, &["a"]);
        input.push("a", &[1; 10]);
        input.add_source(input.generation(), "a", "a", Voice);
        assert_eq!(m.buffered("a"), 0);
    }

    /// A late frame from a receive task aborted mid-push (or a source never added) must not
    /// become a peer nobody feeds: it would count underrun for the rest of the call.
    #[test]
    fn a_push_for_an_unknown_source_is_ignored() {
        let (input, mut m) = mixer(48_000, 200);
        input.push("TR_gone", &[100; 480]);
        let mut out = vec![0.0f32; 480];
        m.mix_into(&mut out, 1, 1.0);
        assert_eq!(input.pushed_total(), 0);
        assert_eq!(input.take_underrun_samples(), 0);
        assert!(input.take_peer_levels().is_empty());
    }

    /// The last room's events task, still mid-handler when a switch clears the mixer.
    #[test]
    fn a_source_added_for_the_last_room_is_ignored() {
        let (input, mut m) = mixer(48_000, 200);
        let old = input.generation();
        input.clear();
        input.add_source(old, "TR_mic", "alex", Voice);
        input.push_in(old, "TR_mic", &[100; 480]);
        let mut out = vec![0.0f32; 480];
        m.mix_into(&mut out, 1, 1.0);
        assert_eq!(input.take_underrun_samples(), 0, "a source nobody feeds was added");
        assert!(input.take_peer_levels().is_empty());
        assert_eq!(m.buffered("TR_mic"), 0);
    }

    /// The same person in both rooms of a switch: the old room's "left" isn't about the new one.
    #[test]
    fn a_remove_for_the_last_room_leaves_the_new_one_alone() {
        let (input, mut m) = mixer(48_000, 200);
        let old = input.generation();
        input.clear();
        input.add_source(input.generation(), "TR_mic2", "alex", Voice);
        input.push("TR_mic2", &[1; 10]);
        input.remove_owner(old, "alex");
        input.remove_source(old, "TR_mic2");
        assert_eq!(m.buffered("TR_mic2"), 10);
    }
```

Update the existing tests so they register their sources:
- Every test that builds with `mixer(…)` and pushes to `"a"` builds with `mixer_with(…, &["a"])` instead (same rate and
  cap): `a_standing_backlog_drains_to_the_target`, `trimming_a_backlog_does_not_click`,
  `underruns_are_counted_per_peer_sample_missed`, `a_queue_at_the_target_is_left_alone`,
  `empty_peer_contributes_silence`, `overflow_keeps_the_newest_audio_bounded` (`mixer_with(1000, 10, &["a"])`),
  `remove_peer_drops_queue`, `pushes_from_another_thread_reach_the_speakers`, `clear_forgets_the_last_rooms_peers`,
  `a_fast_sender_is_held_near_the_target`, `a_slow_sender_never_underruns_once_settled`,
  `after_an_underrun_playback_resumes_with_a_cushion`, `a_small_excess_is_eased_out_without_a_cut`,
  `big_device_callbacks_underrun_once_then_hold`, `a_stall_with_big_callbacks_is_one_gap`,
  `big_callbacks_without_drift_are_barely_adjusted`.
- `mixes_two_peers_with_per_peer_gain`: `mixer_with(48_000, 200, &["a", "b"])`; `input.set_gain("b", 0.0)` becomes
  `input.set_gain("b", Voice, 0.0)`.
- `remove_peer_drops_queue`: `input.remove("a")` becomes `input.remove_source(input.generation(), "a")`.
- `per_peer_levels_reflect_pushed_audio`: `mixer_with(48_000, 200, &["loud", "quiet"])`.
- `a_push_from_before_a_clear_is_ignored`: build with `mixer_with(48_000, 200, &["a"])`, and register the new room's
  source before its push:

```rust
        input.add_source(input.generation(), "b", "b", Voice); // the new room's
        input.push_in(input.generation(), "b", &[100; 480]);
        assert_eq!(m.buffered("b"), 480);
```

In `mictest.rs` `mod tests`, add:

```rust
    /// The loopback registers its source: an unregistered push is dropped (silence).
    #[test]
    fn the_loopback_is_heard() {
        use super::super::processor::FrameSink;
        let input = Arc::new(super::super::mixer::mixer(48_000, 200).0);
        let mut lb = Loopback::new(input.clone());
        lb.send(&[100; 480]);
        assert_eq!(input.pushed_total(), 480);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p pulse-client --lib voice::mixer`
Expected: compile errors: `SourceKind` not found; no method `add_source` / `remove_source` / `remove_owner`;
`set_gain` takes 2 arguments; `Loopback::new` not found.

- [ ] **Step 3: Implement**

In `mixer.rs`:

```rust
/// What a received track is: someone's voice, or the audio of a stream they're sharing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SourceKind {
    Voice,
    Stream,
}
```

`Cmd::Add` gains `gain: f32`; `Mixer::intake` uses it for the new `Peer` instead of `1.0`. Nothing else on the callback
side changes: `Mixer.peers` is now keyed by track SID, which it treats as an opaque id.

Rename `PeerIn` to `SourceIn` and add `owner: String` and `kind: SourceKind`. In `MixerInput`, replace
`pub(crate) peers` with:

```rust
    /// Keyed by track SID: a person can send voice and stream audio at once.
    pub(crate) sources: Mutex<HashMap<String, SourceIn>>,
    /// Volume per (person, kind), kept with no track and across `clear`, so a slider set early applies.
    gains: Mutex<HashMap<(String, SourceKind), f32>>,
```

Lock order is `sources`, then `gains`, then `cmds`. Commands are pushed while `sources` is held, as today, so they
reach the speakers in the order the sources changed. Delete `fn peer()` (sources are no longer created by a push or a
gain) and `remove`. Bodies where the tests leave room:

```rust
    /// Start playing a track (before its receive task starts: earlier pushes are dropped).
    /// Re-adding a SID starts it empty. One for an older room is ignored, as in `push_in`.
    pub fn add_source(&self, generation: u64, source: &str, owner: &str, kind: SourceKind) {
        let mut sources = self.sources.lock().unwrap();
        if generation != self.generation.load(Relaxed) {
            return;
        }
        let gain = self
            .gains
            .lock()
            .unwrap()
            .get(&(owner.to_string(), kind))
            .copied()
            .unwrap_or(1.0);
        // twice the cap: a burst while the speakers reopen is trimmed on their side
        let (tx, rx) = rtrb::RingBuffer::new(2 * self.cap);
        let filled = Arc::new(AtomicUsize::new(0));
        self.cmds.lock().unwrap().push(Cmd::Add {
            id: source.to_string(),
            ring: rx,
            filled: filled.clone(),
            gain,
        });
        sources.insert(
            source.to_string(),
            SourceIn {
                owner: owner.to_string(),
                kind,
                ring: tx,
                gain,
                meter: Meter::default(),
                filled,
            },
        );
    }

    pub fn set_gain(&self, owner: &str, kind: SourceKind, gain: f32) {
        let mut sources = self.sources.lock().unwrap();
        self.gains
            .lock()
            .unwrap()
            .insert((owner.to_string(), kind), gain);
        let mut cmds = self.cmds.lock().unwrap();
        for (id, s) in sources.iter_mut() {
            if s.owner == owner && s.kind == kind {
                s.gain = gain;
                cmds.push(Cmd::Gain {
                    id: id.clone(),
                    gain,
                });
            }
        }
    }

    /// (person, rms since the last call): how each person's *voice* sounded, after their volume.
    /// Stream meters are drained too but never reported: a loud game isn't talking.
    pub fn take_peer_levels(&self) -> Vec<(String, f32)> {
        let mut out: Vec<(String, f32)> = Vec::new();
        for s in self.sources.lock().unwrap().values_mut() {
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

- `push_in`: after the generation check, `let Some(p) = sources.get_mut(source) else { return };` (a removed track's
  late frame, or one never added), then count `pushed` and continue as today.
- `remove_source` / `remove_owner`: lock `sources`, return on a stale generation, remove the SID (or `retain` the
  other owners' sources) and push a `Cmd::Remove { id }` for each SID actually removed.
- `clear`: unchanged apart from the field name (it leaves `gains` alone).
- `max_buffered`: rename `peers` to `sources`.

In `mictest.rs`, import `use super::mixer::{MixerInput, SourceKind};` (line 13), give `Loopback` a constructor that
registers the source, and use it in `MicTest::start` (line 72, `Loopback::new(shared.mix_in.clone())`):

```rust
impl Loopback {
    fn new(mixer: Arc<MixerInput>) -> Self {
        // a push to an unregistered source is dropped
        mixer.add_source(mixer.generation(), "self", "self", SourceKind::Voice);
        Self {
            delay: DelayLine::new((INTERNAL_RATE * DELAY_MS / 1000) as usize),
            mixer,
        }
    }
}
```

Move today's call sites in `mod.rs` to the new API, still keyed by identity (Task 3 switches them to SIDs):
- line 41: `use mixer::{SourceKind, percent_to_gain};`
- `TrackSubscribed`: `mix_in.set_gain(&id, SourceKind::Voice, percent_to_gain(pct));` then
  `mix_in.add_source(mix_gen, &id, &id, SourceKind::Voice);` before the task is spawned.
- `TrackUnsubscribed`: `mix_in.remove(&id)` becomes `mix_in.remove_source(mix_gen, &id)`.
- `ParticipantDisconnected`: `mix_in.remove(&id)` becomes `mix_in.remove_owner(mix_gen, &id)`.
- `set_peer_volume`: `.set_gain(&user_id, SourceKind::Voice, percent_to_gain(pct))`.
- test `a_detached_room_gets_no_more_frames`: before `shared.mix_in.push("peer", …)` add
  `shared.mix_in.add_source(shared.mix_in.generation(), "peer", "peer", SourceKind::Voice);` (without it the
  "old room's people are still in the mix" assertion passes vacuously).

In `devices.rs` test `playback_never_waits_on_the_network_side`: register `"a"` the same way before the push, and
hold `s.mix_in.sources.lock()` instead of `peers`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p pulse-client --lib voice::mixer`
Expected: `test result: ok. 32 passed` (21 existing + 11 new).

Run: `cargo test -p pulse-client --lib`
Expected: PASS, everything (including `voice::mictest::tests::the_loopback_is_heard`,
`voice::devices::tests::playback_never_waits_on_the_network_side` and `voice::tests::a_detached_room_gets_no_more_frames`).

- [ ] **Step 5: Commit**

```bash
git add client/src-tauri/src/voice/mixer.rs client/src-tauri/src/voice/mictest.rs \
        client/src-tauri/src/voice/mod.rs client/src-tauri/src/voice/devices.rs
git commit -m "refactor(voice): mixer sources with an owner and kind, gain per person and kind

One person can now send voice and stream audio without one replacing the other.
Stream audio never lights the speaking ring. A push for an unregistered source is
dropped rather than leaving a phantom peer. Call sites still key by identity.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: Receive tasks keyed by track

**Files:**
- Modify: `client/src-tauri/src/voice/rx.rs`
- Modify: `client/src-tauri/src/voice/mod.rs` (call sites only, lines ~1373, ~1377, ~1382)
- Test: `client/src-tauri/src/voice/rx.rs` (`mod tests`)

**Interfaces:**
- Produces:
  - `RxTasks::replace(&mut self, track: String, owner: String, h: JoinHandle<()>)`
  - `RxTasks::drop_track(&mut self, track: &str)`
  - `RxTasks::drop_owner(&mut self, owner: &str)`
  - `len`, `is_empty`, `clear` and the `Drop` impl are unchanged. Removed: `drop_for`.

- [ ] **Step 1: Write the failing tests**

Replace the two tests in `rx.rs` (`replacing_peer_drops_old_stream`, `drop_for_and_clear_abort`); keep `pending()`:

```rust
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
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p pulse-client --lib voice::rx`
Expected: compile errors: `replace` takes 2 arguments; no method `drop_track` / `drop_owner`.

- [ ] **Step 3: Implement**

The map becomes `HashMap<String, (String, JoinHandle<()>)>` (track SID → owner, handle). Update the module doc to
"One receive task per remote audio track (FINDINGS rule 1)" and add to the struct's doc: "Keyed by track SID, with
the owner kept so everything a person sent can be dropped at once." `replace` aborts the handle it displaces;
`drop_track` removes and aborts one; `drop_owner` aborts and removes every entry with that owner (`retain`); `clear`
drains and aborts all.

Move the `mod.rs` call sites, still keyed by identity:
- `TrackSubscribed`: `rx.lock().unwrap().replace(id.clone(), id, task);`
- `TrackUnsubscribed` and `ParticipantDisconnected`: `drop_for(&id)` becomes `drop_owner(&id)`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p pulse-client --lib voice::rx`
Expected: `test result: ok. 4 passed`.

Run: `cargo test -p pulse-client --lib`
Expected: PASS, everything.

- [ ] **Step 5: Commit**

```bash
git add client/src-tauri/src/voice/rx.rs client/src-tauri/src/voice/mod.rs
git commit -m "refactor(voice): receive tasks keyed by track, droppable per person

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: Wire the room events to tracks

**Files:**
- Modify: `client/src-tauri/src/voice/mod.rs` (the room-events arms around lines 1339-1384; `source_kind` next to
  `mic_publish_options`, line ~1656)
- Test: `client/src-tauri/src/voice/mod.rs` (`mod tests`, line ~1669)

**Interfaces:**
- Consumes: Task 1's `SourceKind`, `add_source`, `set_gain`, `remove_source`, `remove_owner`; Task 2's `replace`,
  `drop_track`, `drop_owner`.
- Produces: `fn source_kind(s: TrackSource) -> SourceKind` (private to `voice`).

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `voice/mod.rs`:

```rust
    #[test]
    fn screen_share_audio_is_stream_everything_else_is_voice() {
        assert_eq!(source_kind(TrackSource::ScreenshareAudio), SourceKind::Stream);
        assert_eq!(source_kind(TrackSource::Microphone), SourceKind::Voice);
        // an unknown source plays like a voice: never silently dropped
        assert_eq!(source_kind(TrackSource::Unknown), SourceKind::Voice);
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p pulse-client --lib voice::tests::screen_share_audio_is_stream`
Expected: compile error: cannot find function `source_kind`.

- [ ] **Step 3: Implement**

Add after `mic_publish_options`:

```rust
/// Stream audio (a shared game's sound) gets its own volume and never lights the speaking ring.
fn source_kind(s: TrackSource) -> SourceKind {
    match s {
        TrackSource::ScreenshareAudio => SourceKind::Stream,
        _ => SourceKind::Voice,
    }
}
```

Replace the three arms. The receive task body (the `NativeAudioStream` loop and the "first sound" log) is unchanged
apart from pushing to the track's SID:

```rust
                        RoomEvent::TrackSubscribed {
                            track: RemoteTrack::Audio(t),
                            publication,
                            participant,
                        } => {
                            let owner = participant.identity().to_string();
                            let source = publication.sid().to_string();
                            let kind = source_kind(publication.source());
                            if kind == SourceKind::Voice {
                                let pct =
                                    volumes.lock().unwrap().get(&owner).copied().unwrap_or(100);
                                mix_in.set_gain(&owner, kind, percent_to_gain(pct));
                            }
                            // before the task starts: a push for an unregistered source is dropped
                            mix_in.add_source(mix_gen, &source, &owner, kind);
                            let (m, src, peer) = (mix_in.clone(), source.clone(), owner.clone());
                            let subscribed = Instant::now();
                            tracing::info!(
                                peer = %owner,
                                track = %source,
                                ?kind,
                                since_join_ms = started.elapsed().as_millis() as u64,
                                "voice: subscribed"
                            );
                            let task = tokio::spawn(async move {
                                // … unchanged, except:
                                    m.push_in(mix_gen, &src, &f.data);
                            });
                            rx.lock().unwrap().replace(source, owner, task);
                        }
                        RoomEvent::TrackUnsubscribed { publication, .. } => {
                            let source = publication.sid().to_string();
                            rx.lock().unwrap().drop_track(&source);
                            mix_in.remove_source(mix_gen, &source);
                        }
                        RoomEvent::ParticipantDisconnected(p) => {
                            let owner = p.identity().to_string();
                            rx.lock().unwrap().drop_owner(&owner);
                            mix_in.remove_owner(mix_gen, &owner);
                        }
```

`set_peer_volume` already sets the `Voice` kind (Task 1). The task's mixer binding is `mix_in`, so it doesn't shadow
the `mixer` module.

- [ ] **Step 4: Run all client tests**

Run: `cargo test -p pulse-client`
Expected: PASS, everything (the ignored real-LiveKit tests are skipped).

Run: `cargo clippy -p pulse-client --all-targets -- -D warnings` and `cargo fmt --all -- --check`
Expected: no warnings, no diff.

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
- Modify: `client/src-tauri/src/voice/mixer.rs` (`source_count`)
- Modify: `client/src-tauri/src/voice/mod.rs` (`mixer_sources`, next to `rx_len`, line ~1645)
- Modify: `client/src-tauri/tests/voice_it.rs`

**Interfaces:**
- Consumes: Task 3's wiring.
- Produces: `MixerInput::source_count(&self) -> usize` (sources registered); `VoiceManager::mixer_sources(&self) -> usize`
  (async; 0 with no session).

- [ ] **Step 1: Add the accessors, test-first**

In `mixer.rs` `mod tests`:

```rust
    #[test]
    fn source_count_follows_adds_and_removes() {
        let (input, _m) = alex_mic_and_game();
        assert_eq!(input.source_count(), 2);
        input.remove_source(input.generation(), "TR_game");
        assert_eq!(input.source_count(), 1);
        input.clear();
        assert_eq!(input.source_count(), 0);
    }
```

Run `cargo test -p pulse-client --lib voice::mixer::tests::source_count`: compile error (no `source_count`). Then add
`pub fn source_count(&self) -> usize` (the `sources` map's length, for diagnostics and tests), and in `VoiceManager`:

```rust
    pub async fn mixer_sources(&self) -> usize {
        match self.session.lock().await.as_ref() {
            Some(s) => s.pipeline.shared.mix_in.source_count(),
            None => 0,
        }
    }
```

Run `cargo test -p pulse-client --lib voice::mixer`: `33 passed`.

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
    use livekit::webrtc::audio_frame::AudioFrame;
    use livekit::webrtc::audio_source::native::NativeAudioSource;
    use livekit::webrtc::audio_source::{AudioSourceOptions, RtcAudioSource};

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
    let vt = api.voice_token(&sam, lounge.id).await.unwrap();
    let (room, _events) = Room::connect(&vt.url, &vt.token, RoomOptions::default())
        .await
        .unwrap();
    let publish = |name: &str, source: TrackSource| {
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
    let (a, b) = (feed(mic_src), feed(game_src));
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

Checked against the locked crates: `AudioFrame.data` is `Cow<'_, [i16]>`; `capture_frame` returns
`Result<(), RtcError>`; `NativeAudioSource::new` needs a queue size that is a multiple of 10; `unpublish_track` takes
`&TrackSid`; `TrackSubscribed`/`TrackUnsubscribed` carry `publication: RemoteTrackPublication`. If an import path
differs (for example `AudioFrame`'s module), follow `voice/mod.rs` and `tests/neteq_probe.rs`, which already publish a
`NativeAudioSource`.

- [ ] **Step 3: Run it against the dev LiveKit**

Run: `just dev-livekit`, then
`cargo test -p pulse-client --test voice_it two_tracks_from_one_person_both_play -- --ignored --nocapture`
Expected: PASS.

Sanity check that the test can fail: temporarily change `rx.lock().unwrap().replace(source, owner, task)` in
`mod.rs` to `replace(owner.clone(), owner, task)` (the old keying). The test must fail on "one receive task per
track". Revert.

- [ ] **Step 4: Run everything**

Run: `just test`, `just clippy` and `just voice-it`
Expected: PASS (all existing real-LiveKit voice tests still pass).

- [ ] **Step 5: Commit**

```bash
git add client/src-tauri/tests/voice_it.rs client/src-tauri/src/voice/mod.rs client/src-tauri/src/voice/mixer.rs
git commit -m "test(voice): a person's mic and stream audio both play; dropping one keeps the other

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

## Notes for later steps (not done here)

- The spec's voice-room hardening (`can_publish_sources: [microphone]`) would reject the `ScreenshareAudio` publish in
  Task 4's test. When it lands, that test moves to the stream room (or publishes a second microphone-source track).
- In the spec, stream audio arrives through a second Rust connection to the stream room as `<uid>#a`. When it is
  wired into this mixer, its owner should be `<uid>` (strip the suffix) so per-person settings line up, and it must
  not be cleared by the voice room's `clear`/generation. That is the stream step's decision, not this one's.

## Done when

- `just test`, `just clippy` and `just voice-it` pass.
- A review (Opus) of the branch finds no blocking issues.
- With the owner's OK: merged to `main`. No Windows test is needed on its own: voice behaviour is unchanged, and the
  next Windows test covers it.
