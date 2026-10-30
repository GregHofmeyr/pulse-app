# OS Notifications Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A Windows toast / Linux desktop notification for the messages that already ding, while the Pulse window is unfocused; a click brings Pulse up on that conversation.

**Architecture:** The UI decides (pure functions next to `messageSound`, plus a per-conversation throttle) and builds `{title, body, channelId, threadId?}`. A Tauri command `notify_message` hands it to one notifier worker thread in the Rust core, which shows it with the platform API (WinRT toast with tag/group on Windows, `notify-rust` over zbus on Linux, nothing elsewhere). A click focuses the window and emits `pulse://notification-click`; the UI navigates.

**Tech Stack:** Svelte 5 + vitest (UI), Rust/Tauri 2 (core), `windows` 0.62 WinRT (`UI_Notifications`, `Data_Xml_Dom`), `notify-rust` 4.18 (`z` feature only).

**Spec:** `docs/superpowers/specs/2026-10-30-os-notifications-design.md`

## Global Constraints

- Notify only for what `messageSound` would ding for, ignoring its 2 s gap, and only while the window is unfocused.
- Throttle: first message per conversation at once, then a 4 s window; at its end one replacement "newest message, and N more", which opens a new window. Focus or sign-out cancels pending replacements and clears windows.
- Title: author's display name, + ` in #channel (Server)` for a server channel, + ` in Group name` for a group, nothing for a DM.
- Body: one line, cut at 140 characters, no author prefix; custom emoji `:name:`; mentions `@name`; no text → "sent a clip" / "shared a quote" / "posted the Quote of the Year card". Text off → "New message" / "Mentioned you". `previewText` unchanged.
- Settings → Notifications: "Desktop notifications" (on), "Show message text" (on), per device + per profile (localStorage, which is per profile already: each profile has its own webview data dir).
- Click: unminimise, show, focus; open the conversation; a reply opens its quote's replies panel; unknown conversation or signed out → just the window.
- Failure: one warning per session, otherwise quiet. Never block the UI thread or the async runtime.
- AUMID: the NSIS installer sets the bundle identifier `app.pulse.client` on its shortcuts (tauri-bundler `SetLnkAppUserModelId "${BUNDLEID}"`); debug builds use PowerShell's AUMID.
- Public repo: test users alex/sam, no IPs/emails/real names.

## Review Focus

1. A busy group chat at 10 msg/s for a minute: one OS entry per conversation, one toast per 4 s at most (throttle test "chains windows and accumulates N").
2. A notification clicked after it was replaced several times on Linux: exactly one click delivered, for the newest payload, and no waiter thread per replacement (Replacer test "a replacement reuses the waiter").
3. Message text containing `<`, `&`, quotes or a 300-char emoji-heavy line: valid toast XML, no split surrogate pair (toast_xml escape test; body cut test with emoji).
4. "Show message text" off: no message text reaches the payload in any branch, including clips/quotes and the "and N more" replacement (body tests with showText false).
5. Focus regained while a replacement is pending: no toast after focus (store wiring test clears the throttle on focus).

---

### Task 1: Split "should this alert" out of `messageSound`

**Files:** Modify `client/ui/src/lib/conversations.ts`, `client/ui/src/lib/conversations.test.ts`

**Interfaces:**
- Produces: `export type AlertInput = { message: Message; me: string; channel: Channel; open: boolean; openThread: string | null; focused: boolean; muted: boolean }` and `export function shouldAlert(v: AlertInput): boolean` (everything `messageSound` checks except the gap). `messageSound(v)` = `shouldAlert(v) && gap`.

- [ ] Step 1: test `shouldAlert` with every existing `messageSound` case except the gap (a 9 s-ago sound still alerts), plus: deleted message → false. Run `pnpm test conversations` → FAIL (not exported).
- [ ] Step 2: implement; add `if (m.deleted) return false` (spec: never deleted ones; a live MessageCreated is never deleted, so the sound is unchanged).
- [ ] Step 3: `pnpm test conversations` → all pass, old `messageSound` tests untouched.
- [ ] Step 4: commit `refactor(ui): "should this alert" is its own function; the sound adds its gap`.

### Task 2: Notification text and the decision

**Files:** Create `client/ui/src/lib/notify.ts`, `client/ui/src/lib/notify.test.ts`

**Interfaces:**
- Consumes: `shouldAlert`, `AlertInput` (Task 1); `displayName` (selectors.ts); `conversationName`.
- Produces:
  - `export type Notice = { title: string; body: string; channelId: string; threadId?: string }`
  - `export type NotifyPrefs = { enabled: boolean; showText: boolean }`
  - `export const BODY_MAX = 140`
  - `export function notificationBody(m: Message, me: string, showText: boolean): string`
  - `export function notificationTitle(s: AppState, m: Message, channel: Channel, more: number): string`
  - `export function notifyMessage(s: AppState, v: AlertInput, prefs: NotifyPrefs, more = 0): Notice | null` — null unless `prefs.enabled && !v.focused && shouldAlert(v)`. `threadId` = `message.thread_root_id` when set.

- [ ] Step 1: tests (users alex = ME, sam = U1):
  - decision: DM from sam unfocused → Notice; focused (even not open) → null; disabled → null; server channel without mention → null; with mention → Notice; muted group → null, mention in it → Notice; own / system / deleted → null.
  - title: DM `sam`; group named `Friends` → `sam in Friends`; unnamed group → `sam in sam, jo`; server text `sam in #general (Main)`; member nickname used in a server; `more = 3` → `sam and 3 more in #general (Main)`.
  - body: whitespace collapsed to one line; `<:party:E1>` → `:party:`; `@alex` kept; 200-char text → 140 chars ending `…`; 200 emoji (`😀`) never split (`Array.from(body).length === 140`); empty clip → `sent a clip`; clip with caption → caption; quote → its content; empty quote → `shared a quote`; qoty_card → `posted the Quote of the Year card`; showText false → `New message`, and `Mentioned you` when it mentions ME.
  - threadId set for a reply, absent otherwise.
- [ ] Step 2: run → FAIL. Step 3: implement. Step 4: run → PASS. Step 5: commit `feat(ui): notification title, body and decision`.

### Task 3: The throttle

**Files:** Modify `client/ui/src/lib/notify.ts`, `client/ui/src/lib/notify.test.ts`

**Interfaces:**
- Produces: `export const NOTIFY_WINDOW_MS = 4000`; `export class NotifyThrottle { constructor(send: (n: Notice) => void, windowMs = NOTIFY_WINDOW_MS); offer(key: string, build: (more: number) => Notice): void; clear(): void }`.
- Algorithm: no window for `key` → `send(build(0))`, open a window (`shown = 1`). Inside a window → keep `build` as the newest, `pending++`. Window end: `pending > 0` → `send(newest(shown + pending - 1))`, `shown += pending`, new window; else forget `key`. `clear()` cancels every timer and forgets every key. (N counts every message in the chain not shown: the replaced ones too, since the OS entry is replaced.)

- [ ] Step 1: tests with `vi.useFakeTimers()`: single message → 1 send, nothing at 4 s; burst of 10 within 4 s → first at once, at 4 s one send built with more = 9, nothing at 8 s; arrivals in the second window → more accumulates (2 then +2 → 3 then 5); two conversations are independent; `clear()` during a window → no replacement, and the next message sends at once.
- [ ] Step 2: FAIL. Step 3: implement. Step 4: PASS. Step 5: commit `feat(ui): notification throttle: one per conversation per 4 s, "and N more"`.

### Task 4: Rust core: payload, command, notifier worker, click

**Files:** Create `client/src-tauri/src/notify.rs`; modify `client/src-tauri/src/lib.rs` (module, manage, register command, `bring_to_front` shared with the single-instance handler).

**Interfaces:**
- Produces:
  - `#[derive(Deserialize)] #[serde(rename_all = "camelCase")] pub struct Notice { title, body, channel_id: String, thread_id: Option<String> }`
  - `#[derive(Serialize, Clone, PartialEq, Debug)] #[serde(rename_all = "camelCase")] pub struct Click { channel_id: String, thread_id: Option<String> }`
  - `pub const CLICK_EVENT: &str = "pulse://notification-click"`
  - `pub struct Notifier` — `Notifier::new(app_id: String, on_click: Arc<dyn Fn(Click) + Send + Sync>)` starts thread `pulse-notify`; `fn send(&self, n: Notice)` never blocks.
  - `#[tauri::command] pub async fn notify_message(notifier: State<'_, Notifier>, notice: Notice)`
  - `fn clean(n: Notice) -> Option<Notice>`: one line, title ≤ 200 bytes, body ≤ 400 bytes (`logging::one_line`), channel/thread ids ≤ 64 chars; empty channel id → None.
  - `pub fn app_id(identifier: &str, debug: bool) -> String`: debug → PowerShell's AUMID, else the identifier.
  - `pub(crate) fn bring_to_front(app: &AppHandle)` in lib.rs: unminimise, show, set_focus.
  - Failures: `warn_once` (an `AtomicBool`): first failure logs a warning, the rest nothing.
- [ ] Step 1: tests: `Notice` deserialises from `{"title","body","channelId","threadId"}` and without `threadId`; `Click` serialises to `{"channelId":..,"threadId":null}`; `clean` cuts and one-lines, drops an empty channel; `app_id` debug/release; `warn_once` true once. Run `cargo test -p pulse-client --lib notify` → FAIL.
- [ ] Step 2: implement (platform `show` is a no-op stub here), register the command, manage `Notifier` in setup with `app.config().identifier`, click → `bring_to_front` + emit.
- [ ] Step 3: tests PASS; `cargo clippy` clean. Step 4: commit `feat(core): notify_message command, notifier thread and click event`.

### Task 5: Linux backend (notify-rust)

**Files:** Modify `client/src-tauri/Cargo.toml` (`[target.'cfg(target_os = "linux")'.dependencies] notify-rust = { version = "4.18", default-features = false, features = ["z"] }` — zbus with async-io, the same zbus 5 / async-io already in the tree via the single-instance plugin, no C libraries), `client/src-tauri/src/notify.rs`.

**Interfaces:**
- Produces (pure, tested on every platform): `struct Replacer` with `fn replaces_id(&self, channel: &str) -> Option<u32>`, `fn shown(&mut self, id: u32, click: Click) -> bool` (true = start a waiter for `id`), `fn ended(&mut self, id: u32, clicked: bool) -> Option<Click>`.
- Show: appname "Pulse", summary title, body, `action("default", "Open")`, category hint `im.received`, `.id(replaces)` when known; on `shown(..) == true` spawn thread `pulse-notify-wait` running `wait_for_action`: `"default"` → clicked, `"__closed"` → closed.
- [ ] Step 1: Replacer tests: first show → no replaces-id, start a waiter; second show for the same channel → replaces-id is the first id; same id back while waiting → no new waiter and a click returns the newest payload; closed → entry gone (next show has no replaces-id); a click is delivered once; another channel is independent. FAIL → implement → PASS.
- [ ] Step 2: add the dependency and the Linux `show`. `cargo clippy` clean, `cargo deny check` clean.
- [ ] Step 3: `#[ignore]` test `linux_desktop_notification_shows` that sends one "Pulse test notification" through the real `show`; run once by hand.
- [ ] Step 4: commit `feat(core): Linux desktop notifications over D-Bus, replaced per conversation`.

### Task 6: Windows backend (WinRT toast)

**Files:** Modify `client/src-tauri/Cargo.toml` (`windows` features `UI_Notifications`, `Data_Xml_Dom`, `Foundation`), `client/src-tauri/src/notify.rs`.

**Decision:** `tauri-winrt-notification` 0.8.1 has no tag/group API (its tag is only reachable through a progress bar), and the spec's one-entry-per-conversation needs a tag. Its whole toast path is ~40 lines over the `windows` crate, which is already a direct dependency at the same version (0.62), so the toast is built on `windows` directly: no new crate.

**Interfaces:**
- Produces (pure, tested everywhere): `fn toast_xml(title: &str, body: &str) -> String` — `<toast><visual><binding template="ToastGeneric"><text>…</text><text>…</text></binding></visual></toast>`, XML-escaped (`& < > " '`).
- Show: `XmlDocument::LoadXml`, `ToastNotification::CreateToastNotification`, `SetTag(channel_id)`, `SetGroup("pulse")`, `Activated` handler calling `on_click(click)`, `CreateToastNotifierWithId(app_id).Show`. The last toast per channel is kept so its Activated handler lives as long as it is on screen.
- [ ] Step 1: toast_xml test (escapes, two text lines) FAIL → implement → PASS.
- [ ] Step 2: Windows `show`; `cargo xwin clippy --target x86_64-pc-windows-msvc` clean.
- [ ] Step 3: commit `feat(core): Windows toasts, one per conversation (tag), click opens it`.

### Task 7: Settings → Notifications

**Files:** Create `client/ui/src/lib/notifyPrefs.svelte.ts`, `client/ui/src/components/NotificationSettings.svelte`, `client/ui/src/components/NotificationSettings.test.ts`; modify `client/ui/src/components/Settings.svelte`.

**Interfaces:**
- Produces: `export const notifyPrefs = $state<NotifyPrefs>(...)`, `setNotificationsEnabled(on: boolean)`, `setNotificationText(on: boolean)`; keys `pulse.notifications`, `pulse.notificationText` (stored `'false'` = off; anything else on). Section key `'notifications'`, label "Notifications", between Chat and Advanced.
- [ ] Step 1: tests: both switches on by default; toggling stores `'false'`/`'true'` and updates `notifyPrefs`; "Show message text" is disabled while notifications are off; the Settings nav has "Notifications". FAIL → implement → PASS.
- [ ] Step 2: commit `feat(ui): Settings → Notifications`.

### Task 8: Wiring: send, clear, click

**Files:** Modify `client/ui/src/lib/tauri.ts` (`notifyMessage: (notice: Notice) => invoke<void>('notify_message', { notice }).catch(() => {})`), `client/ui/src/lib/store.svelte.ts`, `client/ui/src/components/TextChannel.svelte`; tests in `client/ui/src/lib/notifyWiring.test.ts`.

**Interfaces:**
- `maybeSound` becomes `maybeAlert`: same `v`, then `notifyMessage(app.state, v, notifyPrefs)`; a Notice → `throttle.offer(channelId, (more) => notifyMessage(snapshot, v, notifyPrefs, more) ?? notice)`.
- Throttle cleared when `windowFocus.focused` turns true, in `resetState()`, and when notifications are switched off.
- `handleNotificationClick(c: { channelId: string; threadId: string | null })` (exported from store): unknown channel → nothing; a thread in a quotes channel → `ui.openReplies = { channelId, rootId }` and `jumpTo(channelId, rootId)`; else `ui.goTo = channelId`. Listened on `pulse://notification-click` in `startListening`.
- `ui.openReplies: { channelId: string; rootId: string } | null`; TextChannel opens the panel once the root is in its list, then clears it; cleared on unmount like `ui.jump`.
- [ ] Step 1: tests (mock `@tauri-apps/api/core` invoke): an unfocused DM event invokes `notify_message` with the Notice; a focused one doesn't; a burst sends one more after 4 s; focus during the window → nothing at 4 s; notifications off → nothing; click → `ui.goTo`; click with thread → `ui.openReplies` set; click for unknown channel → nothing. FAIL → implement → PASS.
- [ ] Step 2: `pnpm check` 0/0, `pnpm test` all pass. Commit `feat(ui): message notifications go out; a click opens the conversation`.

### Task 9: Checks and report

- [ ] `cargo fmt --all`, both clippies, `cargo test -p pulse-client --lib`, `cargo deny check`, `pnpm install --frozen-lockfile && pnpm check && pnpm test`.
- [ ] THIRD_PARTY.md lists no Rust crates (assets, fonts, ffmpeg only): leave it.
- [ ] Report at `.superpowers/sdd/2026-10-30-os-notifications/report.md`.
