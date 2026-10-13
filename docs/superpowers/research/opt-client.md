# Pulse client: footprint and responsiveness findings (excluding audio DSP internals)

Researcher: client layer (Rust core outside the DSP, Tauri/WebView2, Svelte UI). Read-only pass, 2026-10-13.
Nothing was compiled or bundled. Every number below comes from artifacts already on disk, from `cargo tree --offline --locked` (which only resolves metadata) or from reading the sources.
Note: the tree changed while I was reading it (Settings/keybinds were being refactored). Line numbers are as of this pass, so re-grep before editing.

## Baseline measured from existing artifacts

| Artifact | Size | Notes |
|---|---|---|
| `target/x86_64-pc-windows-msvc/release/pulse-app.exe` (Oct 6) | 67,150,336 B | `.text` 47.2 MB, `.rdata` 18.1 MB, `.pdata` 1.6 MB, `.reloc` 0.18 MB |
| Same exe, `xz -6` (a proxy for the NSIS LZMA installer) | **22.5 MB** | the 8 MB DFN `.tar.gz` inside it is already compressed: about **35% of the installer** |
| `target/release/pulse-app` (Linux, Oct 12) | 90,834,800 B | **not stripped**: loaded sections are 70.5 MB, so about 20 MB is symbols |
| `client/ui/dist` | 1.1 MB | JS 220,847 B (72,695 B gzipped); CSS 53,973 B (8,892 B gzipped) |
| Fonts in dist | 48 files, 582,704 B | woff 325,896 B plus woff2 256,808 B: 4 weights × 6 subsets × 2 formats |
| `dist/sounds/*.wav` | 47,928 B | also `include_bytes!`'d into the Rust binary (`sounds.rs:11-21`), so shipped twice |

Code size by crate, from `nm --size-sort` on the unstripped Linux release binary (48.8 MB of text symbols):
ndarray 4.9 · core generics 4.9 · webrtc C++ 4.1 · **rustfft 3.6** · tract_core 3.1 · tokio 1.7 · std 1.6 · tauri 1.2 · tract_data 1.1 · smallvec 1.1 · pulse_client 0.5 · rustls 0.4 · plain C about 10.5, of which **about 5 MB is video codecs** (dav1d, aom/av1, vpx, ffmpeg, openh264 `WelsEnc`) that a voice-only app never calls.
=> The DFN stack (tract*, ndarray, rustfft) is about **13–14 MB of code plus the 8 MB model**, which is about a third of the exe.

Release profile: **there is no `[profile.release]` anywhere** (root `Cargo.toml` has only `profile.dev.package.*`). Release builds therefore use Cargo's defaults: `lto = false`, `codegen-units = 16`, `opt-level = 3`, `panic = "unwind"`, and on Linux only `strip = "debuginfo"` (symbols kept).

---

## HIGH impact

### H1. Every gateway event re-renders the markdown of every loaded message
- **Where:** `client/ui/src/components/TextChannel.svelte:95-102` builds `mentionables` and then `mentionNames`, and both read `app.state` (the top-level signal). They go into every `<MessageItem>` at `:235-244`. `MessageItem.svelte:80` has `{@html renderMarkdown(message.content, mentionNames)}`, which runs `marked.parse`, then `DOMPurify.sanitize`, then a `<template>` re-parse for mention pills (`lib/markdown.ts:40-81`). The same items also call `nameOf()` → `displayName(app.state…)`, a linear scan over members, and `app.state.me?.id`.
- **Why it re-runs:** `store.svelte.ts:55-58` does `app.state = applyEvent(...)` on **every** event: Typing (every 3 s per typer), PresenceChanged, VoiceJoined/Left/StateChanged, ReadState, and messages in *other* channels. Each one gives `app.state` a new top-level object. `mentionables` re-runs and returns a new array, so `mentionNames` gets a new identity, and every MessageItem re-runs its `{@html}` expression. The Svelte html block skips the DOM write when the string is identical (`svelte/src/internal/client/dom/blocks/html.js:75`), but the marked + DOMPurify work for all N messages is already done by then.
- **The same cost on every channel switch:** `Shell.svelte:266` `{#key activeChannel.id}` remounts TextChannel, so every message in the retained history is rendered again from scratch.
- **Current cost (reasoned):** DOMPurify parses each string into a DOM, typically 0.05–0.3 ms per short message in Chromium. With 200 loaded messages that is about 10–60 ms of main-thread scripting **per gateway event**. That is a dropped frame or more every time someone types or a presence changes, and it gets worse as more history is paged in.
- **Change:**
  1. Memoise rendering: keep a module-level `Map<string, string>` keyed by `message.id + (edited_at ?? '') + mentionKey`, where `mentionKey = mentionNames.join('\u0000')`. Bound it with an LRU of about 2–5k entries.
  2. Make `mentionNames` stable: return the previous array when the joined key is unchanged, or derive it from `app.state.people` / `app.state.dmMembers[channelId]` slices only (see H2).
  3. Precompute `name`, `mine` and `grouped` per message in one `$derived` array over `messages`, so the each-block does not read `app.state` per item.
- **Gain:** the work per event drops from O(N × markdown) to O(changed messages). Channel switching becomes nearly free after the first visit.
- **Risk:** low. The cache must be invalidated on edits (edited_at is in the key) and on deletes (deleted messages do not render markdown anyway).
- **Measure:** use a dev build (`pnpm tauri dev`; devtools are not compiled into release) and open a channel with 200+ messages. Have a second profile (`PULSE_PROFILE=b`) type for 10 s and record a DevTools Performance trace. Metrics: scripting ms per `pulse://event`, and the count of `renderMarkdown` calls (temporary counter). Target: about 0 calls per Typing event, and channel-switch time (click to `requestAnimationFrame`) under 16 ms on a revisit.
- **Sources:** Svelte `{@html}` implementation (local `node_modules/svelte@5.57.2/src/internal/client/dom/blocks/html.js:75`); Svelte docs for `$derived` / `$state` <https://svelte.dev/docs/svelte/$state>.

### H2. The store is a deep `$state` proxy wrapped around an immutable reducer. Use `$state.raw` and slice-level deriveds.
- **Where:** `client/ui/src/lib/store.svelte.ts:11` has `export const app = $state<{ state: AppState }>(...)`. The reducer in `lib/state.ts` is already pure and structurally shared. A grep found no in-place mutation of `app.state.*`: every write is `app.state = …`.
- **Current cost:** each `app.state = next` wraps `next` in a new Proxy. Reading it lazily creates a `source` signal and a child Proxy per property touched (`svelte/src/internal/client/proxy.js:170-200`; there is no object-to-proxy cache). The reducer is called *with the proxy*, so every `{...s}` and `[...cur, m]` spread runs through ownKeys/get traps. `upsertMessage` copies the whole channel array and re-sorts it on every MessageCreated (`state.ts:81-89`). The `{#each}` then creates per-index signals for the new array. Every message object carries proxy and signal overhead, which is roughly hundreds of bytes per message per property read.
- **Change:**
  - `export const app = $state.raw({ state: emptyState() })`. Alternatively `let state = $state.raw(emptyState())` behind getter/setter functions, because a raw *field* needs the wrapper object itself to stay reactive. The simplest form is `$state({ state: $state.raw(...) })`; Svelte allows a raw value inside reactive state.
  - Then expose slice deriveds, for example `export const channels = $derived(app.state.channels)` (or inside components). Selectors such as `channelsFor`, `homeBadge`, `serverBadge` (called per server *inside the template* at `Shell.svelte:191`), `conversations` and `voiceOccupants` should take slices, not the whole `AppState`. With structural sharing, an unchanged slice keeps its identity and `$derived` stops propagation.
  - In `upsertMessage`, append without sort when `m.id > last.id` (the common case).
- **Gain:** removes all proxy and signal allocation for messages, people and channels. Each event then only re-runs what depends on the slice that changed. Expect scripting per event to drop from milliseconds to tens of microseconds, plus a few MB less JS heap with large histories.
- **Risk:** medium-low. Any future in-place mutation (`app.state.x.push(...)`) would fail silently. Add a lint or comment, and keep `Object.freeze` in dev builds to catch it.
- **Measure:** JS heap (DevTools Memory: heap snapshot, filter "Proxy") after loading 1000 messages, before and after. Also scripting time per event, with the same setup as H1.
- **Sources:** Svelte docs: "$state.raw … can improve performance with large arrays and objects that you weren't planning to mutate anyway" <https://svelte.dev/docs/svelte/$state>.

### H3. Add a release profile (LTO, one codegen unit, strip)
- **Where:** root `Cargo.toml`, which has no `[profile.release]`.
- **Current cost:** a 67.15 MB exe and about 22.5 MB LZMA. About 32 MB of the text is Rust code compiled as 16 CGUs without LTO. Heavy generics (ndarray 4.9 MB, `core` instantiations 4.9 MB, smallvec 1.1 MB) are duplicated across CGUs. The Linux binary also carries about 20 MB of symbols.
- **Change:**
  ```toml
  [profile.release]
  lto = "fat"          # or "thin" if CI time matters (~10 min Windows build today)
  codegen-units = 1
  strip = true         # Linux: drops ~20 MB of symbols; Windows: no PDB emitted (exe unchanged)
  # keep opt-level = 3: tract/rustfft are real-time hot paths (Tauri's "s" advice is for UI-only apps)
  # panic = "abort"    # optional, see risk
  ```
- **Gain (estimate):** 10–20% of the Rust part of `.text`, so about 3–6 MB of exe and 1–2 MB of installer. Typically a few % CPU improvement in Rust hot paths. Linux: 90.8 → about 65 MB on disk from `strip` alone. LTO does not touch prebuilt libwebrtc C++ (a static archive), but the linker's `/OPT:REF` already applies to it.
- **Risk:** low for `lto`, `codegen-units` and `strip`, though CI build time rises (fat LTO over tract/livekit could add several minutes). `panic = "abort"` is **medium**: today a panic in a tokio task (gateway, outbox, voice tasks) is contained by the JoinHandle; with abort it kills the app. The crash hook in `logging.rs` still runs first, so reports survive. Gain is about 5–10% more size (landing pads, `.pdata`). Do it only once you are confident tasks don't panic.
- **Measure:** `ls -l target/x86_64-pc-windows-msvc/release/pulse-app.exe` plus the NSIS installer size from the CI artifact. Run `cargo bloat --release --crates -n 30` once on the dev box (when no other build is running). For runtime: CPU% of `pulse-app.exe` in a 5-minute call with Strong suppression, from Process Explorer / `typeperf "\Process(pulse-app)\% Processor Time"`.
- **Sources:** Tauri "App Size" (updated 2025-11-04): recommends `codegen-units = 1`, `lto = true`, `opt-level = "s"` ("use 3 if you prefer speed"), `panic = "abort"`, `strip = true` <https://v2.tauri.app/concept/size/>.

### H4. WebView2 memory: drop to the `Low` memory target when the app is backgrounded
- **Where:** `client/src-tauri/src/lib.rs:22-43` (`create_main_window`). There is a Linux-only `with_webview` hook already; add a Windows one, plus a window-event handler.
- **Current cost:** WebView2 runs about 6–7 processes (browser, GPU, renderer, network/utility, crashpad and so on). Earlier measurement: in-call PSS about 310 MB, of which about 200 MB is outside the Rust core, mostly WebView2. Pulse runs all day minimised or in the background.
- **Change:** on `WindowEvent::Focused(false)`, sustained for about 2–5 minutes, or on minimise, call `ICoreWebView2_19::SetMemoryUsageTargetLevel(COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW)`. Set NORMAL on focus. Sketch:
  ```rust
  #[cfg(windows)]
  fn mem_target(win: &tauri::WebviewWindow, low: bool) {
      use webview2_com::Microsoft::Web::WebView2::Win32::*;
      use windows::core::Interface;
      let _ = win.with_webview(move |wv| unsafe {
          if let Ok(c) = wv.controller().CoreWebView2() {
              if let Ok(c19) = c.cast::<ICoreWebView2_19>() {
                  let _ = c19.SetMemoryUsageTargetLevel(if low { COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW } else { COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL });
              }
          }
      });
  }
  ```
  Add `webview2-com = "0.39"` and `windows` at the **same versions already in Cargo.lock** (webview2-com 0.39.1 / windows-core 0.62). Another Tauri 2.12 app hit E0308 type mismatches when these were mismatched.
- **Gain:** a real-world report from a Tauri 2.12 app on Win11 (2026-10-05) measured private memory falling from **121.9 MB to 36.3 MB** after the Low target. Expect a similar 60–90 MB cut in Pulse's background footprint.
- **Risk:** low-medium. Microsoft notes that pages may be swapped out, so the first interaction after returning can stutter briefly. Events that keep arriving in the background (message sound logic in JS) page memory back in, which is why the delay matters. Do not combine with `TrySuspend`: suspending would stop JS from handling `pulse://event`, which breaks message sounds and unread counts.
- **Measure:** on Windows, minimise Pulse for 5 min, then sum "Private Bytes" for `pulse-app.exe` plus all child `msedgewebview2.exe` (Process Explorer tree, or `Get-Process msedgewebview2 | ? {$_.Parent.Id -eq <pid>}`), before and after. Also check time-to-interactive after restore.
- **Sources:** Microsoft, "Performance best practices for WebView2 apps" <https://learn.microsoft.com/microsoft-edge/webview2/concepts/performance> ("set the level to Low on inactive WebViews… don't mix with TrySuspend"); miklol/Muna PR #76 (2026-10-05), 121.9 → 36.3 MB <https://github.com/miklol/Muna/pull/76>.

### H5. Installer size: the DeepFilterNet model is 8 MB of incompressible payload (about 35% of the installer)
- **Where:** `client/src-tauri/Cargo.toml:41` (`deep_filter … features = ["tract","default-model"]`), which embeds `DeepFilterNet3_onnx.tar.gz` (7,983,136 B) via `include_bytes!` (`libDF/src/tract.rs:86` in the git checkout).
- **Current cost:** disk and installer only. Embedded `.rdata` pages are not touched until Strong is selected, and the model is already loaded lazily: `denoise.rs:141-145` starts on RNNoise until DFN is ready. So it costs **no RAM** unless Strong is used. It is still about 8 of the 22.5 MB LZMA payload.
- **Change (optional):** drop `default-model`. On first selection of "Strong", download the tar.gz (pinned SHA-256) into `app_local_data_dir`, then call `DfParams::from_path` / `from_bytes`. Strong already has a "loading / fell back" state (`voice.ns.note`).
- **Gain:** installer about 22.5 → 14.5 MB (−35%) and exe −8 MB. The code for tract, ndarray and rustfft (about 13 MB) would remain.
- **Risk:** medium. Strong needs network on first use, plus a hosting URL, a hash check and an offline fallback (RNNoise). Not worth it if installer size is not a pain point for friends. **Fine to keep as is** if not.
- **Measure:** NSIS artifact size before and after; Strong time-to-active on first use.

---

## MEDIUM impact

### M1. Voice `Levels` events go to the webview 10× per second for the whole call, even though only the Settings meter uses them
- **Where:** `client/src-tauri/src/voice/mod.rs:615-646` emits `VoiceEvent::Levels` every 100 ms. `lib.rs:105-112` sends `handle.emit("voice://event")`, which goes through `webview.eval` (`tauri-2.12.1/src/manager/webview.rs:696`, then WebView2 `ExecuteScript`). The JS side (`lib/voice.svelte.ts:50-53`) assigns `voice.levels` and `voice.gateOpen`, and only the voice settings meter reads them.
- **Current cost:** 10 JSON serialisations, 10 ExecuteScript calls and 10 JS tasks plus Svelte flushes per second, for the full length of every call. That keeps the renderer and browser processes awake. The same channel carries `Speaking`, which is only sent on change, so that one is fine.
- **Change:** add an atomic `meters_wanted` flag, set by a command when the voice settings are open or the mic test runs, and emit `Levels` only while it is set. Better still, move meter traffic to a Tauri `Channel` passed in by the Settings page.
- **Gain:** removes about 36k IPC evals per hour of call. Expect a measurable drop in renderer and browser CPU and wakeups in a call: roughly 0.5–2% of a core, to be measured.
- **Risk:** low.
- **Measure:** Windows Performance Recorder / Process Explorer: context switches/s and CPU of `msedgewebview2.exe` (renderer) during a 5-minute call with Settings closed, before and after. Linux: `pidstat -w -u -p <WebKitWebProcess pid> 1`.
- **Sources:** Tauri docs, "Calling the frontend from Rust": "The event system is not designed for low latency or high throughput situations… it directly evaluates JavaScript code"; "Channels are designed to be fast and deliver ordered data" <https://v2.tauri.app/develop/calling-frontend/>.

### M2. libwebrtc's log sink is registered at `LS_VERBOSE`, so every RTC_LOG line is formatted in C++ and then thrown away in Rust
- **Where:** `~/.cargo/registry/src/*/webrtc-sys-0.3.47/src/webrtc.cpp:161-163` (`AddLogToStream(this, LS_VERBOSE)`) and `libwebrtc-0.3.50/src/native/peer_connection_factory.rs:38-46` (forwards every line as `log::debug!` with target `libwebrtc`). Pulse's filter is `info,livekit=warn,libwebrtc=warn` (`client/src-tauri/src/logging.rs:21-22`).
- **Current cost:** because a stream is registered at VERBOSE, `RTC_LOG(LS_VERBOSE|LS_INFO)` is never a no-op. Each line is built with ostringstream, copied into a Rust `String` across the cxx FFI and dropped by `log`'s max-level check. The volume in a call is unknown; WebRTC is chatty at VERBOSE (stats, ICE, audio device, NetEq), so it is likely tens to hundreds per second.
- **Change:** first measure (see the "measure first" list). If it is significant, use a `[patch.crates-io]` fork of webrtc-sys with `LS_WARNING`, or file an upstream issue asking LiveKit to make the sink severity follow `log::max_level()`.
- **Gain:** unknown until measured. Possibly 1–3% of a core in a call.
- **Risk:** low (logging only). Maintaining a patch fork costs effort.
- **Measure:** `PULSE_LOG=info,libwebrtc=trace` in a 2-minute call, then `grep -c libwebrtc pulse.log*`, divided by 120 s, gives lines/s. Optional: `perf record -g` on Linux and look for `rtc::LogMessage` / `new_log_sink` closures.

### M3. Startup does two serial TLS handshakes before any data, and the UI waits on the first one
- **Where:** `client/ui/src/App.svelte:21-35` (boot: `listen` ×6, then `restore_session`). `client/src-tauri/src/commands.rs:185-204`: `restore_session` reads the keyring, then `Api::new` (builds a reqwest client and loads the OS root store, `api.rs:121-128`), then `GET /me` on a new TLS connection, and only then `activate()`, which starts the WebSocket (a **second** TCP+TLS handshake, because tokio-tungstenite does not share reqwest's pool, `gateway.rs:169-170`). The window shows a blank `.boot` div until `/me` returns. The Shell then renders empty until `Ready` arrives.
- **Current cost:** first meaningful paint = WebView2 init + JS parse + 1 RTT for TCP + 1–2 RTT for TLS + 1 RTT for /me. Data (Ready) needs about 3–4 more RTTs. On home internet to `pulsechat.co.za`, that is probably 150–400 ms of avoidable wait.
- **Change:**
  - (a) Start the gateway from Rust in `setup()` as soon as the stored token loads, in parallel with WebView2 startup. Keep the last `Ready` in `Core`, add a `current_ready` command the UI calls after attaching its listeners, and let `Ready.me` stand in for `/me`. A 4001 close already means logged out.
  - (b) If that is too much, at least run `/me` and the WS connect concurrently (`tokio::join!`).
- **Gain:** about 1–2 RTTs plus a TLS handshake off time-to-data, and the WS handshake overlaps webview startup, which often takes 200+ ms on cold WebView2.
- **Risk:** medium. Race between Ready and the UI listener (solved by `current_ready`); the logic for an expired token moves to the 4001 path, which already exists.
- **Measure:** add `tracing::info!` timestamps for process start, `setup` done, `restore_session` entered and returned, WS connected and Ready. In JS, `performance.now()` at `mount` and at first Shell render. Metric: process start → Ready rendered, median of 10 cold launches (`PULSE_PROFILE` test profile, after reboot for a true cold start).

### M4. Fonts: 48 font files (583 KB) are embedded, but only latin woff2 is ever used
- **Where:** `client/ui/src/main.ts:3-6` imports `@fontsource/onest/{400,500,600,700}.css`. Each one pulls 6 unicode-range subsets (cyrillic, cyrillic-ext, latin, latin-ext, math, symbols) in both woff2 and woff.
- **Current cost:** 582,704 B in the binary. Tauri brotli-compresses assets, but woff/woff2 barely compress. At runtime WebView2 only fetches latin woff2 (unicode-range), so RAM is unaffected.
- **Change:** import only `@fontsource/onest/latin-{400,500,600,700}.css` (plus `latin-ext` if friends use accented names), or switch to `@fontsource-variable/onest` (one latin woff2 with a weight axis, likely 30–40 KB). Drop the woff fallbacks, since WebView2 and WebKitGTK both support woff2.
- **Gain:** about −500 KB in the exe (small, but free), and fewer @font-face rules for the style engine.
- **Risk:** very low. Check that Cyrillic/emoji names still fall back to system-ui.
- **Measure:** `du -cb client/ui/dist/assets/*.woff*` and the exe size.

### M5. A 1 Hz interval runs whenever any text channel is open, which is almost always
- **Where:** `client/ui/src/components/TextChannel.svelte:39-42`, `setInterval(() => now = Date.now(), 1000)`. It feeds only `typers` (`:29`).
- **Current cost:** 3,600 renderer wakeups/hour plus a Svelte flush, in an app that sits idle all day. It also stops Chromium's timer throttling from settling.
- **Change:** schedule a single `setTimeout` for the earliest typing expiry, only while `app.state.typing[channelId]` is non-empty, and recompute on Typing events.
- **Gain:** idle renderer wakeups from this source go from 1/s to 0. Small CPU and power gain, which matters on laptops.
- **Risk:** very low.
- **Measure:** Windows: Process Explorer, `msedgewebview2.exe` (renderer) context-switch delta over 60 s idle. Linux: `pidstat -w -p <WebKitWebProcess> 10`.

### M6. Message history only ever grows, with no virtualisation or trimming
- **Where:** `lib/state.ts:46-71` keeps `messages` across Ready; `addHistory` pages older messages in without limit (`TextChannel.svelte:78-92`). Every message is a full DOM subtree, plus its proxy cost until H2 lands.
- **Current cost:** linear in session length. A long-running app that scrolls back in a few channels holds thousands of messages, their DOM and their sanitised HTML.
- **Change:** when leaving a channel, keep only the newest page (about 50–100) for non-open channels and reset `history[c].start` so scrolling up refetches. A full virtual list is probably unnecessary at this group size. An easy CSS win: `content-visibility: auto; contain-intrinsic-size: auto 48px` on `.msg`, which skips layout and paint for off-screen messages.
- **Gain:** a bounded JS heap and DOM. Faster channel switches together with H1.
- **Risk:** low, provided the scroll anchoring in `maybeLoadOlder` keeps working. Test `content-visibility` on WebKitGTK as well.
- **Measure:** DevTools Memory, JS heap plus `document.querySelectorAll('*').length` after scrolling 1000 messages back and switching away.

---

## LOW impact

### L1. Duplicate crates in the client graph (Windows target)
- `cargo tree -d` shows: **tokio-tungstenite/tungstenite 0.27 (ours, `Cargo.toml:27`) vs 0.29 (livekit-net)**. Bumping ours to 0.29 removes one copy (about 100–200 KB) and is a trivial API change. Also rubato 0.14 (from deep_filter) vs 5 (ours), prost 0.11 / rand 0.8 / itertools (via tract 0.21.4, pinned), windows 0.54 (cpal 0.15) vs 0.62. These are not fixable without upgrading upstream.
- TLS: reqwest's `rustls-tls` (`Cargo.toml:26`) means **webpki-roots** (an embedded Mozilla bundle). livekit also enables `rustls-tls-native-roots` on the same reqwest, so both root stores are compiled in. Switching ours to `rustls-tls-native-roots` drops webpki-roots (about 0.2–0.3 MB). ring is the only crypto provider (no aws-lc), which is good.
- `tokio = { features = ["full"] }` (root `Cargo.toml:17`) mostly costs compile time; unused features are dead-stripped. Nothing to gain at runtime.
- **Measure:** `cargo tree --offline -p pulse-client -e normal --target x86_64-pc-windows-msvc -d`, plus the exe size.

### L2. Video codecs (about 5 MB+ of code) ship inside the prebuilt libwebrtc
- dav1d, aom, vpx, ffmpeg and openh264 symbols together come to about 5 MB, plus webrtc video classes. LiveKit's `webrtc-sys` downloads a prebuilt static lib, and there is no audio-only build feature. **Not actionable** short of building libwebrtc yourself. Noted so nobody chases it.

### L3. Sounds embedded twice
- `client/ui/public/sounds/*.wav` (48 KB) are copied into `dist/` and embedded as webview assets, and are also `include_bytes!`'d in `sounds.rs:13-19`. Nothing in `ui/src` references `/sounds/`. Move them out of `public/` (for example to `client/src-tauri/sounds/`). Saves 48 KB.

### L4. Tauri runtime threads
- Tauri's default tokio runtime starts one worker per logical CPU (16–32 threads on modern boxes), plus libwebrtc's own threads, cpal and so on. On Windows the heap is shared, so the cost is mostly stacks and kernel objects. On Linux (glibc) each thread can get its own malloc arena, which inflates RSS. Option: `tauri::async_runtime::set(tokio::runtime::Builder::new_multi_thread().worker_threads(4).enable_all().build()?.handle().clone())` before `Builder::default()`.
- **Measure:** thread count in Process Explorer, plus private bytes. On Linux also try `MALLOC_ARENA_MAX=2` as a quick A/B test of the arena effect before changing any code.

### L5. Logging cost is fine; small tidy-ups
- `logging.rs:13-28`: a non_blocking worker thread blocks on a channel (no periodic flush timer) and the filter is static, so `debug!` callsites cost one cached-interest check. Optional: `tracing = { features = ["release_max_level_info"] }` compiles debug/trace callsites out of release (tiny size and CPU gain). The `Capped` writer passes each line straight to the file (one `write` per line), which is fine at info volume.

### L6. Other idle timers (all OK)
- Rust: gateway heartbeat every 30 s with a 75 s dead-man (`gateway.rs:14-18,194-205`); outbox retry every 15 s (`commands.rs:66-78`; it could become event-driven, but 4 wakeups/min is negligible); IPC socket accept (Linux, event-driven); in-call 100 ms levels loop and 10 ms send-loop tick (`voice/mod.rs:471`). The 10 ms tick is 100 wakeups/s for the whole call; it is part of the DSP send path, so I'm handing it to the DSP researcher. The mic test runs a 100 ms loop only while testing. **Voice loops run only in a call**, confirmed by the `alive` flag and the per-session task list.
- UI: `clock` every 30 s (`store.svelte.ts:18`, fine); tooltip uses delegated listeners and a 350 ms timeout on hover (fine; no observers anywhere).

### L7. CSS / layout
- No `backdrop-filter`, no `filter`, no infinite animations in the shipped CSS. 18 static `box-shadow`s, none on scroll containers. Two nits: the meter `transition: width .1s linear` animates `width` 10×/s while the meter is visible (use `transform: scaleX()`), and one `transition: left .12s` (toggle knob, use `transform`). Negligible.
- Avatars are CSS initials (no images), so there is no image decode or memory cost today.

### L8. JS bundle
- 220 KB min / 73 KB gzipped. marked (~40 KB min) and DOMPurify (~22 KB min) are the largest libraries; the rest is the Svelte runtime and app code. Loaded once from embedded assets, so parse time is a few ms. Not worth replacing. `removeUnusedCommands: true` in `tauri.conf.json > build` (Tauri ≥ 2.4) trims plugin/core commands the ACL never allows; a small gain.

### L9. WebView2 browser args (experimental; measure before adopting)
- wry passes `--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection` by default (`wry-0.57.0/src/webview2/mod.rs:300-304`). **Calling `.additional_browser_args()` replaces that string**, so any experiment must re-include those three.
- Candidates for A/B testing only: `--disable-gpu` (saves the GPU process, about 30–60 MB, but moves compositing onto the CPU; probably a net loss with smooth scrolling); `--js-flags=--max-lazy` / `--lite-mode` (smaller V8 heap, slower JS). I found **no published numbers** for these in WebView2, so treat them as unverified. H4 is the supported lever.
- Single-instance, the frameless window (`decorations(false)`) and the dark `background_color` (no white flash) are all cheap. There is no tray, and devtools are not in release (the tauri `devtools` feature is off). Assets are embedded and brotli-compressed (tauri's default `compression` feature), not served through the asset protocol. All of that is already optimal.

---

## Measure first (before changing anything)
1. **Per-event UI cost (H1/H2):** a dev build with a 200-message channel and a second profile typing. Record a DevTools Performance trace and count `renderMarkdown` calls per `pulse://event`. This decides whether H1/H2 are the top UI items (I expect they are).
2. **Windows memory breakdown:** Process Explorer tree with private bytes per process (`pulse-app.exe` plus every `msedgewebview2.exe` child, labelled by `--type=`) at idle in the foreground, idle minimised for 5 min, and in a call. Use the same breakdown before and after H4.
3. **libwebrtc log volume (M2):** `PULSE_LOG=info,libwebrtc=trace` for a 2-minute call, then lines/s. If < 5/s, drop M2.
4. **Startup timeline (M3):** add tracing timestamps (process start, setup done, restore entered/returned, WS open, Ready), plus JS `performance.now()` at mount and first Shell paint. Take the median of 10 cold launches.
5. **Idle wakeups:** context switches/s for `pulse-app.exe` and the renderer over 60 s idle (Process Explorer / `pidstat -w`), before and after M5. Also in a call with Settings closed, before and after M1.
6. **Size:** one `cargo bloat --release --crates` and `cargo bloat --release -n 50` on the Windows target when nobody else is building, to confirm the nm-based breakdown above (DFN stack about 13–14 MB of code, libwebrtc C/C++ about 15 MB).
7. **Installer:** record the current NSIS `*-setup.exe` size from the last CI artifact (my xz estimate is 22.5 MB).

## Sources
- Tauri, "App Size", updated 2025-11-04: <https://v2.tauri.app/concept/size/>
- Tauri, "Calling the Frontend from Rust" (events evaluate JS; use Channels for throughput): <https://v2.tauri.app/develop/calling-frontend/>
- Microsoft, "Performance best practices for WebView2 apps" (MemoryUsageTargetLevel Low on inactive WebViews; don't mix with TrySuspend): <https://learn.microsoft.com/microsoft-edge/webview2/concepts/performance>
- miklol/Muna PR #76, Tauri 2.12 + webview2-com 0.39, 2026-10-05 (private memory 121.9 → 36.3 MB after a Low target trim; E0308 when webview2-com versions mismatch): <https://github.com/miklol/Muna/pull/76>
- tauri-apps/tauri issue #5889, opened 2022-12-21, closed 2024-05-17 (Tauri/WebView2 about 399 MB vs Electron 318 MB on Win10 for the same site; measure PSS/USS, not RSS): <https://github.com/tauri-apps/tauri/issues/5889>
- Svelte docs, `$state` / `$state.raw`: <https://svelte.dev/docs/svelte/$state>
- Local sources read: `svelte@5.57.2` `proxy.js` and `blocks/html.js`; `wry-0.57.0/src/webview2/mod.rs`; `tauri-2.12.1` Cargo features and `manager/webview.rs`; `webrtc-sys-0.3.47/src/webrtc.cpp`; `libwebrtc-0.3.50/src/native/peer_connection_factory.rs`; DeepFilterNet `libDF/src/tract.rs` at rev d375b2d.
