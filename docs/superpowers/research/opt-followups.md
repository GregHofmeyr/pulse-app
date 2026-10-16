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

## Deferred minors (open)

- Task 1: shutdown.rs wait-for-Ready loop has no timeout / spins on None (plan-mandated)
- Task 1: spawn_binary nulls child stdout/stderr → no diagnostics when the binary fails to start
- Task 1: free-port bind-then-release race in harness
- Task 2: create_dm takes the write lock even on the existing-DM path (brief-mandated)
- Task 2: no test of atomic rollback (message+mentions / remove_member)
- Task 2: remove_member returns 500 if post-commit reads fail though removal succeeded (pre-existing)
- Task 3: dedupe lookup not channel-scoped: reused nonce in another channel returns the other channel's message (UUID nonces make it practically unreachable) — add comment or 409
- Task 3: same nonce + different content returns original silently (undocumented); no test for empty-nonce 400
- Task 4: retry path — apply commits then voice.restore; a late webhook join between can create 2 open rows (call restore before commit)
- Task 4: retry path stale snapshot — participant_left between fetch and apply re-adds a ghost until next restart
- Task 4: one deleted-user identity → FK INSERT fails → whole restore Errs + all retries fail; skip unknown users — **fixed in the final fix wave**
- Task 4: LiveKit joined_at=0 inserts a 1970 row; treat <=0 as now — **fixed in the final fix wave**
- Task 4: PULSE_LIVEKIT_URL read twice in config.rs; no DM-path / on_startup-success / concurrent-webhook tests; dev-deps not justified in report
- Task 5: cpal 0.18 Xrun + StreamInvalidated now logged at warn (throttled) — consider debug/info so bug reports don't read as regressions
- Task 5: later MMCSS refusal unlogged (first_time per kind) — log refusals always
- Task 5: cpal maps packed 24-bit WASAPI to I24 but assumes a 32-bit container
- Task 6: gate_open_pct counts fade-out blocks as open (doc it); take() swaps ns_in/ns_out separately (≤1 block skew); no processor-level muted-window test; SILENT_ENERGY comment dB value loose
- Task 7: no test for went-backwards branch of `since`; stale doc comment on a_new_stream_counts_from_zero; Totals keyed by SSRC alone + audio_in fixture uses SSRC 0; note wording (netem qdisc vs one-way, bare Verdict line, row (a) repro command, "about" the stall length)
- Task 8: decide skips soft_limit (moot for manual after ruling); no test for set_preroll / mid-stream Standard↔Strong switch / mute→unmute after switch; dead-StrongWorker block passes raw with stale level (comment); Rnnoise computes unused sqrt-RMS in Standard; switch drops/inserts 10 ms; no before/after latency numbers yet (X1)
- Task 8: no test for auto↔manual toggle mid-stream
- Task 9: refill level ignores pull size (2048 pulls can take 2 start-up gaps; refill = 480+pull fixes); `low` updates while rebuffering → one full-budget repeat after refill; no fade at refill-gap edges (audible click candidate — triage at final review); QUIET checked before peer gain; drop checks only the played sample — **fixed in the final fix wave**
- Task 9: deadband guard test is 48 kHz only (rate scaling by inspection)
- Task 10: GapFill silence is bursty (5-6 frames every ~50 ms during a device stall), not a steady 10 ms cadence — per Task 7 that jitter can raise the far end's NetEq target; pre-existing — **fixed in the final fix wave**
- Task 10: backlog::drop_stale + ToInternal + MicChunk now dead outside tests — delete in cleanup
- Task 10: set_mic_sink drops the old sink under the lock (swap then drop outside); stale send_queue_ms doc; mic_queue assumes 10 ms blocks; fill_gap counts refused frames as silence_fill
- Task 11: full receive ring keeps the OLDEST audio (SPSC producer can't evict) → after a >400 ms speaker stall, ~200 ms of stale audio then a jump; comment/test name "keep newest" wrong (plan-mandated) — fix: consumer discards ring on long pull gap
- Task 11: loose overflow assertion (<1.5); never-waits test doesn't assert audio mixed / cmds path; cmds unbounded if speakers stop pulling; poisoned mixer mutex → silent playback forever (log it)
- Task 12: no retry on a stale pooled connection (h1 fallback: fails after ≤15 s); h2 ping eviction untested; deploy Caddy before/with client release; tokio-util gains libc (feature unification)
- Task 13: no tests for Tauri-side wiring (forward hold, restore mapping, suspend); setup+restore keychain read + std Mutex in async cmd; conn/deactivate held until UI attaches (comment); crash notice task waits forever if UI never attaches
- Task 13: stale restore_session from the pre-reload page can time out first and suspend the fresh session (reload inside slow 25 s startup); old forward task may feed stale updates into new boot; reload re-runs crash::send_pending (verify it consumes); worst-case startup 25+5 s; reload path untested (needs tauri mock runtime)
- Task 14: probe ack deadline flat 3 s ignores RTT → alt-tabbing on a bufferbloated link (gaming PC uploading / future screenshare) drops a live gateway; use max(probe_ack, 3×rtt) — **fixed in the final fix wave**
- Task 14: unbiased select can drop a live link after a client stall (add `biased;` frame-first); queued probes cancel several backoff steps (drain with try_recv); Brief sessions reset n → accept-then-drop retried every ~1 s forever (plan-mandated); no jitter on Healthy→0 reconnect; no e2e Brief test / probe-coalescing test; reconnect_now/Cmd::ReconnectNow unused; long module doc line
- Task 15: no test that the hook re-reads flags per Ready (constant closure in test); failed-send-after-Ready path untested; comment cosmetic
- Task 16: no hover debounce (sweeping rows → N POSTs; A,B,A → 3); failed prefetch occupies the slot for 120 s; click waits on a slow in-flight prefetch up to 15 s; in-flight test weak; duplicated guard in ChannelList handlers
- Task 17: Leave waits on settle() before room.close (up to 6 s; close first, settle only when reopening); SETTLE_WAIT 6 s < worst-case ~12 s (fix comment/value); Disconnected arm drops io without waiting (leave io for leave_locked(false) to close); Rejected (4xx) retried 6× (stop on it) + rejoin 401 bypasses core.check; connect reads current epoch (pass it in); no committed test for real hand-over / join(B) during rejoin / 401 stop / leave_locked→settle — **fixed in the final fix wave**
- Task 17: reconnect_give_up field never shortened in a manager-level test; unpause assert in Drop can abort on double panic; stale room kept until rejoin leave_locked
- Task 18: device failure now waits for room connect + others see a brief join/leave (plan-mandated); no (Err,Err) test; Bluetooth A2DP→HFP parallel-open risk → manual BT join in Windows protocol; connect wiring only covered by voice_it
- Task 18: OpenRoom Drop close is fire-and-forget (leave may return before ghost gone); cancel during Room::connect itself still has nothing to close (upstream); SlowNull could be #[doc(hidden)]
- Task 19: runtime test fragile to parallel holders + doesn't cover warm_runtime; warm_runtime spawns a blocking task per hover even when warm (early return); gain unmeasured (X1)
- Task 20: teardown closure duplicates lost's 3 lines (shared fn); no UI store test for `mic` case / micLive unused (no visual); benign alive check-then-emit race (comment); possible duplicate Disconnected on failure (SDK-dependent)
- Task 20: retries reset before alive check (late Ok from dead session); stray Reconnecting possible before a leave's Disconnected during close_room await; join() reset untested; micLive stays true during Reconnecting
- Task 21: switch drops old AudioIo without waiting → device-close race on reopen more likely (Task 22 keeps the pipeline across switches, which removes the reopen); stray Disconnected if A's real Disconnected lands between alive=false and drop (emit only when was_alive); switch can wait ≤6 s on settle (not a regression); CloseRoom discards close errors (log + channel); failed-switch test can't catch the ordering race (switch test does)
- Task 22: stall reported before a switch isn't re-reported in the new room (UI clears stalled on join; StallPolicy reports once) — re-emit DeviceStalled on reuse or reset policy
- Task 22: built counter increments before build succeeds; usable_for ignores Null rate (test-only); no test for adopt-on-switch / switch during reopen; Pipeline Drop closes AudioIo on an async worker (not a regression)
- Task 22: worst-case switch wait 2×SETTLE_WAIT (guard released before alive cleared — clear alive under the guard); stale stall banner race if Recovered lands between flag load and re-emit; stall re-emit untested (Null mode)
- Task 23: store tests share the singleton without afterEach reset; no-in-place-write invariant enforced only by comment
- Task 24: render cache key omits content (relies on edited_at changing per edit — add length/hash); keys embed full mention-name list (intern it); lastNames side effect in $derived (non-idiomatic); no combined rename/new-member test or Typing zero-render component check
- Task 25: Shell leave-effect untested (also runs on logout teardown); addOlder head-id invariant comment; just check rc missing from report
- Task 25: page size 50 duplicated (state.ts / TextChannel.svelte:57)
- Task 26: no reversed-order expiry assertion; nextTypingExpiry doesn't exclude self (possible extra wakeup); timer re-arm untested at component level
- Task 27: stale levels/gateOpen shown ≤100 ms on meter reopen; watchMeters(true) failure swallowed (brief)
- Task 28: wake replay (8-9 denoiser calls) lands in one tick → one-off latency/chain spike on unmute (brief design); first live outputs are the denoiser's last 10-30 ms of muted-time audio (same as before; latency effect); no tests for run idle-branch wiring or waking into meter-only; mem::take drops ring capacity
- Task 29: µs race at the 120 s mark can leave webview Low while focused (re-check policy inside with_webview); policy low flag set before COM success; old-runtime warn per application (AtomicBool once)
- Task 30: libwebrtc warnings/errors only ever reach our logs at DEBUG (libwebrtc maps all to log::debug) → bug reports never contain them; consider a sink-level mapping follow-up; document that verbose libwebrtc logs now need an unpatched rebuild; vendor/** -text in .gitattributes
- Task 31: font test loose (assert weight×subset matrix, ranges per weight, src existence); fonts.css comment says 48 files, baseline was 56
- Task 32: guard test only checks public/sounds dir absence; old plan docs mention public/sounds (historical)
- Task 34: comment claims "slightly faster" + "Windows writes no PDB" unmeasured; strip removes native symbolication (nothing consumes it today)
Task 35 part 1: review → Needs fixes (2 Important: Strong ns+gate 'before' cell contradicts methods note; window counts don't reconcile). minor (deferred): README line length; NO_HEAD removes rclone post-upload check → note restore-drill in README; raw command output not attached
