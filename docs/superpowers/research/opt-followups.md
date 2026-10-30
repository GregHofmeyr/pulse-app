# Optimisation run: rulings and follow-ups

Decisions taken during the subagent-driven run of `plans/2026-10-16-optimisation-run.md` (each with what it costs if wrong), and the minor findings that were deferred rather than fixed. Triage from the final whole-branch review: the fix-before-test items were fixed in the final fix wave; everything below is open unless marked otherwise.

## Rulings

- P1: BlockOut keeps R4's ns_in/ns_out/judged; L6 adds in_level = sqrt(ns_in/len) (no second pass); L6's Coordination ns_atten formula is ignored (R4 + header win); F11 `idle` returns every field and its wake-replay goes first in process — spec+R4 telemetry is the measured goal — cost if wrong: one rework of the gate stats.
- P2: L8's new spawn keeps `let _rt = super::rtprio::promote("pulse-mic");` — R3's MMCSS is spec A7 — cost: none.
- P3: R4's test processor_counts_denoiser_energy_and_gate_verdicts is converted to collecting/wait_frames in L8 — keeps R4 coverage — cost: none.
- P4: F11's idle branch fills i16buf with zeros and calls deliver(&shared,&mut gap,&i16buf) (mic_tx is gone after L8) — cost: small.
- P5: R2 keeps L8's shared.set_mic_sink(None) in leave_locked and the Disconnected arm — else a dead room keeps receiving frames — cost: none.
- P6: tests use Shared::new(&cfg, controls, PEER_BUFFER_MS) and shared.mix_in.max_buffered() (post-L7a API); F6 re-anchors its levels loop on the post-L7a code — cost: none.
- P7: user joins call detach_locked (and settle the reopener); R2 rejoins keep leave_locked(false) with settle + waited io.close() — preserves the BT-reopen fix (Review Focus 3) — cost: two close paths to maintain.
- P8: L4b keeps leave_locked(announce) -> bool, settles pipeline.reopener, closes pipeline.io with a wait when !announce, passes `kept` through join_inner — cost: none.
- P9: multiple test filters run as `cargo test -p <crate> --lib -- a b` — cargo rejects the plan's form — cost: none.
- P10: R3 runs `XWIN_ACCEPT_LICENSE=1 cargo xwin clippy -p pulse-client --target x86_64-pc-windows-msvc -- -D warnings` (cargo-xwin is installed) — Global Constraints — cost: none.
- P11: a_detached_room_gets_no_more_frames must exercise detach_locked (not call set_mic_sink itself) — a test that bypasses the code under test proves nothing — cost: a slightly harder test.
- P12: R1 and F2 window-event handlers stay separate (F2 is a per-window listener); header's "merge" note withdrawn — cost: none.
- P13: before asking the owner for the deploy, say a dispatched release-server run moves :latest (release-server.yml:85); prefer a run that only tags sha, or redeploy main's image after — cost: prod `latest` points at an unmerged branch until merge.
- Task 8: manual-threshold mode keeps the old semantics — decide on the same level the meter shows (out.level), with the full PREROLL_BLOCKS pre-roll; auto mode keeps the new undelayed decision — spec says the change is timing-only and the slider sits on the meter (foolproof UX); the plan's choice silently changed manual behaviour — cost if wrong: manual-mode users don't get the 10–20 ms (auto is the default).
- Task 9: add a drop-side deadband of one push frame (rate/100) — keep repeats whenever low < target so underrun protection is unchanged; plus a guard test (2048-frame pulls, 0 ppm → adjusted < 10/window) — spec's goal is "no drift clicks", continuous micro-edits defeat it — cost if wrong: mean queue +1–7 ms.
- Task 13: on a non-Ready/non-auth boot outcome, probe /me once (short timeout) to classify — Unauthorized → clear + Ok(None), Server → while_restoring, else CONNECT_FAILED — spec wants /me off the HAPPY path only; error messages must stay truthful — cost if wrong: one extra request on failure only.
- Task 17: fix both in this task (not a follow-up) — spec A4 "voice comes back by itself" must hold for a black hole too; owner prefers complete — RECONNECT_GIVE_UP ≈ 60 s armed on Reconnecting, disarmed on Reconnected, firing the shared Disconnected hand-over body — cost if wrong: a slow-but-recovering link gets a fresh rejoin at 60 s instead of resuming.
- Task 18: fix now with a drop guard (OpenRoom(Option<Room>) whose Drop spawns close() under CLOSE_TIMEOUT), wrapping the room from Room::connect until the session is stored — covers the new device window AND the pre-existing publish window; ghost users in channels are user-visible — cost if wrong: one extra spawned close on cancel.
- Task 20: a failed publish on a REJOIN session hands back to the rejoin loop (send epoch on dropped + Reconnecting, like lost(true)), bounded by a VoiceManager publish_retries counter (max 2; reset on Mic{live:true} and on any user join); user's own join keeps Mic error + Disconnected — spec A4 "voice comes back by itself" — cost if wrong: up to 2 extra rejoin cycles (~seconds) before the call ends.
- Task 21: P11 satisfied via Session::detach (detach_locked is a 3-line take-and-call wrapper; voice_it covers it end-to-end through join) — test exercises the real teardown body — cost if wrong: none.
- Task 22: folded the stall re-report minor into fix round 1 (same code area, user-visible banner) — cost if wrong: slightly larger fix diff to re-review.
- Task 25: skip markHistory when addOlder dropped the page (plan's wiring is wrong; spec forbids losing history) — cost if wrong: none.
- Task 30: vendoring webrtc-sys (one-line LS_WARNING change, 501→1.7 lines/s) accepted — no runtime severity API exists in webrtc-sys/libwebrtc/livekit 0.9.3; measured cost is real CPU on gaming PCs — cost if wrong: 6.7 MB in the public repo + manual upgrade on livekit bumps.
- Task 34: keep fat LTO + cgu=1 + strip as the spec says despite Windows saving only 2.8 MB (Linux −33 MB) and ~3× longer release builds — runtime code-quality benefit, release builds are infrequent (dispatch-only) — cost if wrong: ~4 extra CI minutes per Windows kit/server image; switch to lto="thin" if it hurts.
- Task 35 part 1: fix round 2/5 (2 addressed, 0 open — "46 of 50" wording + counted n with command; commit 7dc41ea). Ruling: controller verified the 3-line doc-only diff directly instead of dispatching a re-reviewer — diff is two wording changes matching the findings verbatim — cost if wrong: a doc typo.
- Final fix wave: item 7 uses rtt.map_or(probe_ack, |r| (r*3).max(probe_ack)) instead of probe_ack.max(ack_wait()) — the literal formula floors at 5 s so the 3 s probe never applies; implementer's form keeps 3 s on normal links and 3×RTT on slow ones — cost if wrong: none. Residual: rejoin publish-failure teardown has the same un-waited device drop as item 1.
- Final fix wave: real, rare, self-healing (failed open retries after backoff); follow-up — cost if wrong: a BT rejoin attempt fails once more.

## Deferred minors: triaged 2026-10-30

Every point of the old open list was checked against the code of 2026-10-30, by area, and fixed (test first where testable), confirmed already fixed, found stale, or left with a reason. Commit hashes are on main.

### Server

| Bullet | Point | Verdict | Commit / reason |
|---|---|---|---|
| Task 1 | shutdown.rs wait-for-Ready loop has no timeout, spins on `None` | FIX | `49b9666`: 10 s timeout; a socket that ends before Ready panics (with the server log) |
| Task 1 | spawn_binary nulls stdout/stderr, so no diagnostics | FIX | `49b9666`: child output goes to a temp file (`ServerProcess::log()`), shown in the "exited early" / "didn't come up" panics. New test `two_factor_startup::a_binary_that_fails_to_start_shows_its_output` (failed first, passes now) |
| Task 1 | free-port bind-then-release race in the harness | FIX | `49b9666`: the binary binds `127.0.0.1:0`; `server/src/lib.rs` now logs `listener.local_addr()` (not the configured address), and the harness reads the port from that line. `run_binary` also binds :0; `free_addr` is gone |
| Task 2 | create_dm takes the write lock even on the existing-DM path | WONTFIX | That path writes too (reopening a closed DM deletes its `dm_hidden` row), so it needs a write anyway. The plan mandated it, and 1:1 opens are rare |
| Task 2 | no test of atomic rollback (message+mentions / remove_member) | FIX | `3620a74`: `sqlite_tx.rs` tests inject a failing trigger: `a_send_whose_mentions_fail_leaves_no_message`, `a_removal_that_fails_part_way_keeps_the_member` (they cover behaviour that already worked, so they passed on the first run) |
| Task 2 | remove_member answers 500 if reads after the commit fail, though the removal happened | FIX | `3620a74`: events and system line moved to `announce_removal`; a failure there is logged at warn and the answer is 204. Test `a_committed_removal_is_a_success_even_if_its_announcement_fails` (500 before, 204 now) |
| Task 3 | dedupe lookup isn't channel-scoped: a reused nonce in another channel returns that channel's message | FIX | `73740ef`: `nonce::existing` takes the channel and answers 409 "That request was already used." for a different one, the same as clip confirm. Used by send, quotes and clip confirm. Test `same_nonce_in_another_channel_is_409` (200 before) |
| Task 3 | same nonce + different content silently returns the original (not documented) | FIX (doc + test) | `73740ef`: doc comment on `nonce::existing`; test `same_nonce_with_other_content_returns_the_original` |
| Task 3 | no test for empty-nonce 400 | FIX | `73740ef`: `empty_nonce_is_400` |
| Task 4 | retry path: apply commits, then voice.restore; a webhook join in between makes 2 open rows | FIX | `a0be966`: people go into VoiceState inside the transaction. Only people this restore added get a new row, because a webhook that placed them writes its own. A failed transaction takes the added presence back. Test `a_restore_opens_no_row_for_someone_a_webhook_placed` (failed before) |
| Task 4 | retry path stale snapshot: a participant_left between fetch and apply brings back a ghost | FIX | `a0be966`: `VoiceState::snapshot()` guard (taken in `sync` before asking LiveKit) records leave sids while a restore is in flight; `restore` skips them. Test `a_leave_after_the_snapshot_is_not_undone_by_the_restore` (failed before) |
| Task 4 | deleted-user identity: FK INSERT fails the whole restore (marked fixed in the final fix wave) | FIXED-ALREADY | `voice/restore.rs` `apply` keeps only known users; test `an_unknown_user_is_skipped_without_failing_the_restore` |
| Task 4 | LiveKit joined_at=0 inserts a 1970 row (marked fixed in the final fix wave) | FIXED-ALREADY | `voice/restore.rs` `fetch` treats <=0 as now; test `an_unset_livekit_join_time_is_treated_as_now` |
| Task 4 | PULSE_LIVEKIT_URL read twice in config.rs | FIX | `7b7f98a` |
| Task 4 | no DM-path test | FIX | `a0be966`: `a_dm_call_is_restored_without_a_session_row` |
| Task 4 | no on_startup-success test | FIX | `a0be966`: `livekit_up_at_boot_restores_without_the_fallback` |
| Task 4 | no concurrent-webhook tests | FIX | `a0be966`: the two race tests above (and the existing `a_webhook_join_that_beat_the_restore_wins`) |
| Task 4 | dev-deps not justified in report | STALE | This was about the old Task 4 report, not code. `prost` and `livekit-protocol` dev-deps are used by `tests/common/livekit.rs` (the fake RoomService encodes protobuf) |

### Client core (gateway, session, webview)

| Bullet | Point | Verdict | Commit / reason |
|---|---|---|---|
| Task 12 | No retry on a stale pooled connection (h1 fallback fails after ≤15 s) | WONTFIX | Production is h2 behind Caddy, and h2 pings (25 s / 10 s) evict dead connections. h1 is only a fallback. hyper already retries requests cancelled before they were sent. The outbox retries sends. A blind retry of non-idempotent POSTs could double-post. |
| Task 12 | h2 ping eviction untested | WONTFIX | That is hyper's keepalive. Testing it needs a fake h2 server that stops answering pings; the cost is higher than the value. |
| Task 12 | Deploy Caddy before/with the client release | STALE (ops) | `deploy/Caddyfile` has `idle 60m` (since 6ecbd9e). Worth a one-time manual check that the deployed Caddy runs it. No code change. |
| Task 12 | tokio-util gains libc (feature unification) | WONTFIX | Harmless: libc is already in the graph through tokio/mio. |
| Task 13 | No tests for the Tauri-side wiring (forward hold, restore mapping, suspend) | WONTFIX | Needs the tauri `test` mock runtime plus a keychain-backed `session::Store` in tests. The new superseded-restore guard is a small pure `Generation` type, which is unit-tested. |
| Task 13 | setup + restore keychain read, std Mutex in an async command | FIX | 32b46ee: `restore_session` runs `start_stored` on the blocking pool, as setup already does. |
| Task 13 | conn/deactivate held until the UI attaches (comment) | FIX | 32b46ee: comment added on `forward`'s wait. |
| Task 13 | Crash notice task waits forever if the UI never attaches | WONTFIX | It is a parked future that costs nothing. If the UI never attaches, the app is broken anyway. `ui_attached` stays true across reloads. |
| Task 13 | A stale `restore_session` from the pre-reload page can time out and suspend the fresh session | FIX | 32b46ee: `Core::restores` numbers the calls. A superseded call skips the give-up/suspend branch and returns a plain connect error to the dead page. |
| Task 13 | The old forward task may feed stale updates into the new boot | FIX | 32b46ee: `Core::epoch` is bumped by `activate`, `suspend` and `deactivate`. Each `forward` stops once its epoch is no longer current. |
| Task 13 | A reload re-runs `crash::send_pending` (verify it consumes) | Verified / WONTFIX | Sent, Dropped and OtherServer clear the file. Held stays, and it re-notifies the new page (wanted). Residual risk: a Keep (e.g. timeout), or a reload while the first send is in flight, can send the report twice. That needs crash + reload within about 2 s, so it is rare. |
| Task 13 | Worst-case startup 25 + 5 s | WONTFIX | By design: `BOOT_WAIT` 25 s, plus a `/me` probe of at most 5 s that runs only off the happy path. |
| Task 13 | Reload path untested (needs tauri mock runtime) | WONTFIX | Same reason as the wiring tests above. |
| Task 14 | Probe ack ignores RTT | FIXED-ALREADY | `Liveness::probe_wait` (gateway.rs) uses `rtt.map_or(probe_ack, \|r\| (r*3).max(probe_ack))`, with the test `a_probe_waits_at_least_three_round_trips`. |
| Task 14 | Unbiased select can drop a live link after a stall | FIX | 7ff09c6: `biased;` in this order: frame, cmd, heartbeat, deadline, ack-due. No test: the random select is not deterministically reproducible. |
| Task 14 | Queued probes cancel several backoff steps | FIX | 7ff09c6: drained with `try_recv`. The test `probes_queued_in_one_wait_cause_one_retry` failed before the fix and passes after. |
| Task 14 | Brief sessions reset n → accept-then-drop retried every ~1 s forever | WONTFIX | Plan-mandated. |
| Task 14 | No jitter on the Healthy→0 reconnect | WONTFIX | Immediacy is the point. The server is a small self-hosted one, so there is no real stampede, and failed tries fall back to the jittered schedule. |
| Task 14 | No e2e Brief test / probe-coalescing test | FIX | 7ff09c6: added `a_brief_session_is_retried_after_the_first_backoff_step` and the coalescing test. |
| Task 14 | `reconnect_now` / `Cmd::ReconnectNow` unused | FIX | 7ff09c6: deleted. |
| Task 14 | Long module doc line | FIX | 7ff09c6: wrapped. |
| Task 15 | No test that the hook re-reads flags per Ready | FIX | 7ff09c6: added `after_ready_frames_are_read_afresh_on_each_ready`. The flag flips between connects. Coverage only; the behaviour was already right. |
| Task 15 | Failed-send-after-Ready path untested | WONTFIX | Needs the socket to die between Ready and the first write, which can't be made deterministic. It is the same `End::Retry(Brief)` as any other lost socket. |
| Task 15 | Comment cosmetic | STALE | The review doesn't say which comment. The after-Ready comments in gateway.rs and commands.rs read fine. |
| Task 29 | µs race at the 120 s mark can leave the webview Low while focused | FIX | 294db2d: the target is read from the policy inside `with_webview`, when it is applied, so the last call applies the latest decision. Test: `the_target_follows_the_latest_decision` (new `MemPolicy::target`). |
| Task 29 | Policy low flag set before COM success | WONTFIX | The flag only picks what to ask for next. A transient failure is retried on the next change, and an old runtime now stops trying (next row). |
| Task 29 | Old-runtime warn on every application (AtomicBool once) | FIX | 294db2d: a failed `ICoreWebView2_19` cast sets `UNSUPPORTED`, logs once, and is never retried. |
| Task 34 | Comment claims "slightly faster" + "Windows writes no PDB" without measuring | FIX | 87c34a2: the comment now states only the measured sizes (Linux −33 MB, Windows −2.8 MB) and about 3× the build time. |
| Task 34 | strip removes native symbolication | WONTFIX (documented) | 87c34a2: the comment notes that nothing reads backtrace symbols. The crash marker carries the panic message and file:line, which strip does not remove. |

### Voice core

| Bullet | Point | Verdict | Commit / reason |
|---|---|---|---|
| Task 5 | Xrun + StreamInvalidated logged at warn | FIX | 0f9039f: both now info (`routine_stream_error`, tested); other kinds (RealtimeDenied, DeviceNotAvailable, ...) stay warn |
| Task 5 | later MMCSS refusal unlogged (first_time per kind) | FIX | 0f9039f: success logged once per kind, refusal every time |
| Task 5 | cpal maps packed 24-bit WASAPI to I24 assuming a 32-bit container | WONTFIX (upstream) | cpal 0.18.2 `device.rs` maps `wBitsPerSample == 24` to I24 and `stream.rs` shifts i32s. Shared-mode mix formats (what `default_*_config` returns) are float in practice. Comment added at the I24 arm (0f9039f) |
| Task 6 | gate_open_pct counts fade-out blocks as open | FIX (doc) | 08b93f8: `Summary::gate_open_pct` doc says the 100 ms fade counts as open |
| Task 6 | take() swaps ns_in/ns_out separately (≤1 block skew) | WONTFIX (commented) | ≤1 block of ~500 per window in a diagnostic; one snapshot would need a lock on the audio side. Comment in `take()` (08b93f8) |
| Task 6 | no processor-level muted-window test | FIX | 08b93f8: `a_muted_window_has_no_ns_or_gate_figures` |
| Task 6 | SILENT_ENERGY comment dB value loose | FIX (doc) | 08b93f8: 1e-6 over a fully open 5 s window is ≈ −114 dBFS, not −100 |
| Task 7 | no test for went-backwards branch of `since` | FIX | b16586b: `a_stream_whose_counters_went_backwards_counts_in_full` |
| Task 7 | stale doc comment on a_new_stream_counts_from_zero | FIX (doc) | b16586b: the parenthesis now says it was the OLD summed totals that shrank |
| Task 7 | Totals keyed by SSRC alone; audio_in fixture uses SSRC 0 | WONTFIX | Inbound audio only comes from the subscriber PC; a 32-bit random SSRC clash between two peers is negligible for a 5 s diagnostic window. The SSRC-0 fixture is single-stream, so the value doesn't matter |
| Task 7 | note wording (netem qdisc vs one-way, bare Verdict line, row (a) repro, "about" the stall length) | FIX (doc) | b16586b: `docs/superpowers/research/opt-neteq-buffer.md`: header "netem qdisc delay ms (each lo crossing)", "For the playout task" line says 40 per crossing = 80 sender to receiver, verdict names the Step 6 rule, row (a) gives `PROBE_PATTERNS=bursts,continuous` (+ `PROBE_STALL_MS=500` on today's probe), "by about the stall length" |
| Task 8 | decide skips soft_limit (moot for manual) | WONTFIX | Auto compares the newest level against the room floor + 6 dB; soft_limit only bends samples above the 0.8 knee, so it can't move that decision. Manual already judges the post-limit level |
| Task 8 | no test for set_preroll / mid-stream Standard↔Strong switch / mute→unmute after switch | FIXED-ALREADY + FIX | Switch + preroll resize: `switching_engines_mid_stream_settles_to_each_engines_lag` (already there). Mute→unmute after a switch: new `mute_and_unmute_after_an_engine_switch` (08b93f8) |
| Task 8 | dead-StrongWorker block passes raw with stale level | FIX | 08b93f8: a dead worker clears its level, so that one raw block is gated on its own level. Test `dead_strong_worker_forgets_its_last_level` failed first |
| Task 8 | Rnnoise computes unused sqrt-RMS in Standard | WONTFIX | 480 mult-adds + 1 sqrt per 10 ms. Moving it would touch Strong's decision path for no measurable gain |
| Task 8 | switch drops/inserts 10 ms | WONTFIX | Comes with engines of different delay (10 vs 30 ms). One-off on a user or overload switch |
| Task 8 | no before/after latency numbers yet (X1) | WONTFIX here | Needs the X1 measurement run on the owner's Windows machine, not a code change |
| Task 8 | no test for auto↔manual toggle mid-stream | FIX | 08b93f8: `switching_auto_and_manual_mid_stream_settles_to_each_modes_lag` |
| Task 9 | (fixed in the final fix wave) refill ignores pull size | FIXED-ALREADY | abd1a19: `refill = target + largest recent pull` |
| Task 9 | (same bullet) `low` updates while rebuffering; no fade at refill-gap edges; QUIET before peer gain; drop checks only the played sample | NOT actually changed: WONTFIX | The code still does all four (see Concerns). Left alone: each is a sub-ms effect at most, and changing playout by ear-risk isn't worth it. `low`: `judge` skips a peer still dry at window end, but a window where it refilled mid-way still repeats up to its 0.5 % budget (2.4 ms over 250 ms), which only adds cushion. The refill gap edge is silence → audio at a non-zero sample, a possible click on a sender stall (manual check below) |
| Task 9 | deadband guard test is 48 kHz only | WONTFIX (moot) | The mixer only ever runs at INTERNAL_RATE (devices.rs `Shared::new` → `mixer(INTERNAL_RATE, ..)`); device rates are handled by `Playout` after it |
| Task 10 | (fixed in the final fix wave) bursty GapFill silence | FIXED-ALREADY | backlog.rs `GapFill::filling` + `a_long_stall_is_bridged_one_frame_per_tick_after_the_first_fill` |
| Task 10 | backlog::drop_stale + ToInternal + MicChunk dead outside tests | FIX | 32e304f (drop_stale, ToInternal + their tests), 885f1e9 (MicChunk and its stray doc line) |
| Task 10 | set_mic_sink drops the old sink under the lock | FIX | 885f1e9: swapped under the lock, dropped after. Test `the_old_mic_sink_is_dropped_outside_the_lock` failed first |
| Task 10 | stale send_queue_ms doc | FIX (doc) | 885f1e9: it's the processor's queue (frames go straight to WebRTC after it) |
| Task 10 | mic_queue assumes 10 ms blocks | WONTFIX (commented) | True by construction: `on_input` hands over `rate / 100` samples at any rate. Comment says so (885f1e9) |
| Task 10 | fill_gap counts refused frames as silence_fill | FIX | 885f1e9: refused silence counts as send drops. Test `refused_silence_counts_as_send_drops_not_fill` failed first |
| Task 11 | full receive ring keeps the OLDEST audio; "keep newest" comment/test name wrong; fix: consumer discards on long pull gap | FIX (doc + test), discard WONTFIX | 885f1e9: the `push_in` comment says what happens; the old test is renamed `an_oversized_push_keeps_its_newest_audio_bounded` (that IS what it tests) and a new test pins the full-ring case. The discard is not done: it needs overflow tracking in the real-time path for a rare case (speakers stalled > 2 × cap, i.e. an output reopen), and the stale audio is bounded by the cap and cut at the next window |
| Task 11 | loose overflow assertion (<1.5) | FIX | 885f1e9: rounds and accepts exactly 22 or 23 (f32 rounding of 0.1×14+0.9×24 decides which) |
| Task 11 | never-waits test doesn't assert audio mixed / cmds path | FIX | 885f1e9: `playback_never_waits_on_the_network_side` now checks the peer's samples were played; new `a_held_command_list_does_not_stop_playback` in mixer.rs |
| Task 11 | cmds unbounded if speakers stop pulling | WONTFIX | Commands come from joins, leaves and volume changes only: a few dozen bytes each, drained on the next pull |
| Task 11 | poisoned mixer mutex → silent playback forever (log it) | FIX | 885f1e9: on poison, `clear_poison` + error log + mix with the inner guard. Test `a_poisoned_mixer_still_plays` failed first |
| Task 17 | (fixed in the final fix wave) leave/settle order, SETTLE_WAIT, Disconnected arm, 4xx, epoch, tests | FIXED-ALREADY | `Session::end(announce)` closes the room before `settle`; `rejoin_attempts` gives up on `ApiError::Unauthorized`; tests `leave_closes_the_room_before_waiting_out_a_reopen`, `a_rejoin_opens_devices_only_after_the_dropped_calls_have_closed` |
| Task 17 | reconnect_give_up field never shortened in a manager-level test | FIX | 43288eb: `#[doc(hidden)] with_reconnect_give_up`; `voice_outage_it` uses 5 s (also ~55 s faster) |
| Task 17 | unpause assert in Drop can abort on double panic | FIX | 43288eb: Drop prints and only panics when not already panicking |
| Task 17 | stale room kept until rejoin leave_locked | WONTFIX | The rejoin's first step (`leave_locked(false)`) closes it, after a backoff of seconds. Its audio is already stopped by `drop_call`. Closing it earlier would duplicate the rejoin teardown in the events task |
| Task 18 | device failure waits for room connect + brief join/leave | WONTFIX (plan-mandated) | Accepted cost, documented in `connect_detached` |
| Task 18 | no (Err,Err) test | FIXED-ALREADY | `both_failing_reports_the_room_error` |
| Task 18 | Bluetooth A2DP→HFP parallel-open risk | WONTFIX (manual) | Manual BT join check below |
| Task 18 | connect wiring only covered by voice_it | WONTFIX | Needs a real LiveKit; voice_it covers it |
| Task 18 | OpenRoom Drop close is fire-and-forget | FIXED-ALREADY | The guard holds its `InFlight` until the background close ends, and the next join waits on `connecting` |
| Task 18 | cancel during Room::connect has nothing to close | FIXED-ALREADY / upstream | `connect_detached` lets the connect run on and closes the room once connected. The residual (timeout drops `Room::connect` mid-way) is upstream, commented in `connect` |
| Task 18 | SlowNull could be #[doc(hidden)] | FIX | 43288eb |
| Task 19 | runtime test fragile to parallel holders + doesn't cover warm_runtime | FIX | 43288eb: the test now checks our handle is the live runtime (`Arc::ptr_eq`), which other tests' rooms can't fake; new `hovering_warms_and_keeps_the_webrtc_runtime` |
| Task 19 | warm_runtime spawns a blocking task per hover even when warm | FIX | 43288eb: early return once warm |
| Task 19 | gain unmeasured (X1) | WONTFIX here | Measurement on the owner's machine |
| Task 20 | teardown closure duplicates lost's 3 lines | FIXED-ALREADY | Both go through `stop_audio` / `stop_audio_keep_devices` |
| Task 20 | no UI store test for `mic` case / micLive unused | not mine | UI side (other agent) |
| Task 20 | benign alive check-then-emit race | FIX (comment) | 43288eb, in `report_publish` |
| Task 20 | possible duplicate Disconnected on failure (SDK-dependent) | FIX | 43288eb: `drop_call` returns `Dropped::{HandedOver, Ended, AlreadyOver}`; the events task reports nothing for `AlreadyOver` (whoever ended the session reports). Test `dropping_a_session_already_ended_does_nothing` |
| Task 20 | retries reset before alive check | FIX | 43288eb: reset only for a live session. Test `a_dead_sessions_late_publish_leaves_the_retries_alone` failed first |
| Task 20 | stray Reconnecting possible before a leave's Disconnected | WONTFIX | It lands before the leave's "disconnected", so the UI ends right. The leave bumps the epoch, so the rejoin loop ignores the hand-back |
| Task 20 | join() reset untested | FIX | 43288eb: `a_users_join_resets_the_publish_retries` |
| Task 20 | micLive stays true during Reconnecting | not mine | UI store state |
| Task 21 | switch drops old AudioIo without waiting | STALE | Task 22 keeps the pipeline across switches (`Session::detach` hands it back), so there's no reopen to race |
| Task 21 | stray Disconnected if A's real Disconnected lands between alive=false and drop | FIX | Same `Dropped::AlreadyOver` change as Task 20 |
| Task 21 | switch can wait ≤6 s on settle | WONTFIX | Not a regression (reviewer's own note) |
| Task 21 | CloseRoom discards close errors | FIX | 43288eb: logged at warn with the room name |
| Task 21 | failed-switch test can't catch the ordering race | WONTFIX | The switch test covers the ordering |
| Task 22 | stall reported before a switch not re-reported in the new room | FIXED-ALREADY | `join_inner` re-emits `DeviceStalled` from `Pipeline::stalled` |
| Task 22 | built counter increments before build succeeds | FIX | 43288eb |
| Task 22 | usable_for ignores Null rate (test-only) | WONTFIX | Null is test-only; no test switches Null rates |
| Task 22 | no test for adopt-on-switch / switch during reopen | FIXED-ALREADY + FIX | Switch during reopen: `a_switch_waits_out_a_reopen_in_flight_and_keeps_the_pipeline`. Adopt: new `a_kept_pipeline_adopts_the_joins_settings` |
| Task 22 | Pipeline Drop closes AudioIo on an async worker | WONTFIX | Not a regression (reviewer's own note) |
| Task 22 | worst-case switch wait 2×SETTLE_WAIT (clear alive under the guard) | FIX | 43288eb: `keep_if_usable` clears `alive` before releasing the turn. No deterministic test: making a reopen queue behind the turn needs a device open |
| Task 22 | stale stall banner race (Recovered between flag load and re-emit) | FIX | 43288eb: the flag is read right before the emit, after the `session` await. A µs window remains (watchdog on another thread), commented |
| Task 22 | stall re-emit untested (Null mode) | WONTFIX | Needs a full join (LiveKit) plus a hook to set the stall flag from an integration test |
| Task 28 | wake replay lands in one tick → one-off spike on unmute | WONTFIX (by design) | Brief's design. 8 denoiser calls once per unmute |
| Task 28 | first live outputs are the denoiser's last 10-30 ms of muted-time audio | WONTFIX | Same as before idling existed (muted with meter open behaves the same). The gate's pre-roll already sends 20 ms from before it opens |
| Task 28 | no tests for run idle-branch wiring or waking into meter-only | FIX | 08b93f8: `a_muted_window_has_no_ns_or_gate_figures` (the processor takes the idle branch only with no meter open) and `waking_into_meter_only_replays_once_and_sends_nothing` |
| Task 28 | mem::take drops ring capacity | FIX | 08b93f8: replays in place and `clear()`s |
| Task 30 | libwebrtc warnings/errors only reach logs at DEBUG | WONTFIX (upstream) | The libwebrtc crate (0.3.50, not vendored) passes every line to `log::debug!` and ignores the severity. Fixing it means vendoring a third crate |
| Task 30 | document that verbose libwebrtc logs need an unpatched rebuild | FIX (doc) | 78918e0: root Cargo.toml `[patch]` comment |
| Task 30 | vendor/** -text in .gitattributes | WONTFIX | `git ls-files --eol vendor`: every file is LF in the index and LF in the work tree, except the one .cmd (CRLF on purpose) and a .jar (binary). `-text` would only matter for re-vendoring a CRLF upstream file |

### UI

| Bullet | Point | Verdict | Commit / reason |
|---|---|---|---|
| Task 16 | no hover debounce (sweep → N POSTs; A,B,A → 3) | FIXED-ALREADY | `lib/hoverPrefetch.ts` (f5a2617, e2b86ae): 200 ms rest delay plus a freshness window. A,B,A with a rest on each still gives 3, which is correct: the core keeps one token (`voice/tokens.rs`), so B replaced A |
| Task 16 | failed prefetch occupies the slot for 120 s | FIX | voice/tokens.rs: a finished, failed prefetch no longer fits, so the next hover fetches again (tested) |
| Task 16 | click waits up to 15 s on a slow in-flight prefetch | FIX | voice/tokens.rs: the click waits at most PREFETCH_WAIT (2 s), then fetches normally (tested) |
| Task 16 | in-flight test weak | FIXED-ALREADY | `hoverPrefetch.test.ts` "a prefetch still in flight is not asked for again" checks the de-duplication while in flight and after it finishes (e2b86ae) |
| Task 16 | duplicated guard in ChannelList handlers | FIX | e360b9f (`restOn` helper) |
| Task 20 (UI) | no UI store test for the `mic` case | FIX | 1f34971: the error is surfaced, the stage moves on to connected, and a live mic leaves the error alone |
| Task 20 (UI) | micLive unused (no visual) | FIX | 320bc35: dead state removed (nothing read it; the `live` field stays in the event type) |
| Task 20 (UI) | micLive stays true during Reconnecting | STALE | Fixed by 320bc35: the field no longer exists |
| Task 23 | store tests share the singleton without a reset | FIX | 4369855: the `app store` describe now has `beforeEach(resetState)`. The other describes already had one |
| Task 23 | no-in-place-write invariant enforced only by a comment | FIX | 7fba9f4: `lib/storeWrites.test.ts` scans every non-test .ts/.svelte file for `app.state.x = …`, `+=`, `++`, `.push/.splice/.sort/.set/.delete/…`. I planted a violation and confirmed the scan catches it. It doesn't follow aliases (`const s = app.state; s.x.push()`) |
| Task 24 | render cache key omits content | FIX | 4514db3: the key now has the content length plus an FNV-1a hash (the emoji-refs cache too). The test failed first |
| Task 24 | keys embed the full mention-name list | FIX | 4514db3: the joined list is interned to a small number (`namesIds`) |
| Task 24 | lastNames side effect inside `$derived` | FIX | ecbe675: `keepStable()` memo helper in selectors.ts (unit-tested), used by TextChannel and RepliesPanel |
| Task 24 | no combined rename/new-member test or Typing zero-render check | FIX, plus a real bug | 94754d5: new `TextChannel.render.test.ts`. The zero-render test **failed**: MessageItem read `app.state.customEmoji` straight in the template, so every state update (each Typing event) re-ran `renderMessage` for every visible message. Each run was a cache hit, but still built a key. It now goes through a `$derived` slice. The rename + new-member pill test passes |
| Task 25 | Shell leave-effect untested (also runs on logout teardown) | FIX, plus a real bug | d86f423: see Concerns. The test for the sign-out sequence failed: the teardown wrote the old session back over `resetState()`. The trim now runs when the next channel opens, never in a teardown. Tests cover the trim on leave and sign-out |
| Task 25 | addOlder head-id invariant comment | FIXED-ALREADY | `state.ts` `addOlder`: "Only prepends and trims change messages[0]…" |
| Task 25 | `just check` rc missing from the report | WONTFIX | It was about the Oct report itself; nothing to change in code |
| Task 25 | page size 50 duplicated | FIXED-ALREADY | `PAGE` is exported from state.ts and imported in TextChannel.svelte |
| Task 26 | no reversed-order expiry assertion | FIX | d7c7e67: the typers are now added out of expiry order |
| Task 26 | nextTypingExpiry doesn't exclude self | FIX | d7c7e67: own typing is filtered out (as typingNames does). The test failed first |
| Task 26 | timer re-arm untested at component level | FIX | 4507f28: fake timers; two typers fade one after the other, and no timer is left at the end |
| Task 27 | stale levels/gateOpen shown ≤100 ms on meter reopen | FIX | 82604bc: `voiceApi.watchMeters(true)` zeroes levels and gateOpen. VoiceSettings test failed first |
| Task 27 | watchMeters(true) failure swallowed | WONTFIX | `watch_voice_meters` returns `()` and can't fail. Only a broken IPC would reject, and then nothing else works either (brief-mandated) |
| Task 31 | font test loose (matrix, per-weight ranges, src existence) | FIX | 441e12e: exact weight × subset matrix, each URL names its own weight, every file resolves in the package, ranges are compared per weight against that weight's upstream CSS |
| Task 31 | fonts.css says 48 files, baseline was 56 | FIX | 441e12e: both numbers were right. 56 files are in the package (7 subsets × 4 weights × woff2+woff); 48 / 583 KB was measured in the build (plan Task 31). The comment now says both |
| Task 32 | guard test only checks the public/sounds dir is absent | FIX (UI side) | c51065a: `src/noAudioAssets.test.ts` scans all of `public/` and `src/` for audio files. It also checks that src-tauri/sounds still has them, so the scan can't pass when there's nothing to find. Planted a file: caught. The Rust test stays as it is |
| Task 32 | old plan docs mention public/sounds | WONTFIX | Historical plans are a record; editing them rewrites history |
| Task 35 p1 | README line length | FIX | 7e7680a: wrapped the 186-char restore-drill line in deploy/README.md §8 |
| Task 35 p1 | NO_HEAD note → restore-drill in README | FIXED-ALREADY | 5548e43, deploy/README.md §8 step 6 |
| Task 35 p1 | raw command output not attached | WONTFIX | That measurement session is over; nothing in the repo to attach it to |
