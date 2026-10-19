# Keybinds v2 (pass-through) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Windows binds that never swallow their key: any single key, a lone modifier (Right Ctrl) or a mouse button (Mouse 3/4/5) can toggle mute or deafen, and it keeps working in the focused app.

**Architecture:**
- A pure, platform-neutral `bindmatch.rs` holds the scan-code → W3C-code table and a `Matcher`. It decides from key/button downs and ups which action fires, and is tested on Linux.
- On Windows, `hotkeys.rs` runs `WH_KEYBOARD_LL` + `WH_MOUSE_LL` hooks on one thread that feeds the `Matcher` and always passes events on. This replaces tauri-plugin-global-shortcut.
- `keybinds.rs` validation shrinks to "known code, not Escape/Mouse1/Mouse2, no duplicates".
- The Svelte recorder learns lone modifiers and mouse buttons.

**Tech Stack:** Rust (Tauri 2, `windows` 0.62 crate), Svelte 5 + vitest.

**Spec:** `docs/superpowers/specs/2026-10-19-keybinds-passthrough-design.md`

## Global Constraints

- Branch `keybinds-v2` in worktree `~/Projects/pulse-app-keybinds` (cut from `optimisation-run`). Never commit to or push `main`; merging needs the owner's OK and comes after `optimisation-run` is merged.
- PUBLIC repo: no names, IPs, emails or secrets in code, comments, docs or commit messages.
- Only two actions: toggle mute, toggle deafen. No push-to-talk.
- Never bindable: `Escape`, `Mouse1`, `Mouse2`. Everything else in the code table is bindable, alone or with modifiers (Win included).
- Mouse codes are exactly `Mouse3` (wheel click), `Mouse4` (XBUTTON1), `Mouse5` (XBUTTON2).
- The hook procedure always calls `CallNextHookEx` (never swallows) and never blocks. Real work happens off the hook thread.
- Injected input (`LLKHF_INJECTED` / `LLMHF_INJECTED`) is treated like real input.
- Binds are always on, paused only while recording. Paused suppresses firing only; held-key tracking continues.
- Linux behaviour is unchanged (compositor binds + `pulse-app --toggle-*`).
- Windows-only code must pass `XWIN_ACCEPT_LICENSE=1 cargo xwin clippy -p pulse-client --target x86_64-pc-windows-msvc -- -D warnings`.
- Commit messages end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

1. **Gaming-mouse software delivers Mouse 4/5 as injected events:** the bind still fires. → Task 1: no injected flag exists in the `Matcher`'s input, and Task 3's hook must not filter on it (reviewer check).
2. **A key looks stuck after the lock screen or UAC** (key-up missed): its next real press still fires. → Task 1, `a_missed_key_up_doesnt_eat_the_next_press`.
3. **Recording ends while the new bind's key is still held:** its auto-repeat doesn't fire it. → Task 1, `a_key_held_through_unpause_doesnt_fire`.
4. **An existing v1 `keybinds.json`** (e.g. Ctrl+Shift+M, Ctrl+Alt+F13) still loads and is matched. → Task 2, `old_binds_still_load`.
5. **Right Ctrl = deafen and Ctrl+Shift+M = mute, user presses Right Ctrl+Shift+M:** deafen fires on Right Ctrl and mute on M (Right Ctrl satisfies `ctrl`). → Task 1, `a_lone_modifier_bind_also_counts_as_ctrl_for_combos`.

---

### Task 1: `bindmatch.rs`, the code table and the Matcher

**Files:**
- Create: `client/src-tauri/src/bindmatch.rs` (+ `pub mod bindmatch;` in `lib.rs`)
- Test: unit tests in the same file

**Interfaces:**
- Consumes: `keybinds::{Action, Bind, Keybinds}` (`Keybinds::bound()`).
- Produces:
  - `pub const MOUSE_CODES: [&str; 3] = ["Mouse3", "Mouse4", "Mouse5"]`
  - `pub fn code_for_scan(scan: u32, extended: bool) -> Option<&'static str>`: PC set-1 scan codes as Windows low-level hooks report them.
  - `pub fn is_known(code: &str) -> bool`: true for every code the table can produce, plus `MOUSE_CODES`, `Mouse1`, `Mouse2`.
  - `pub struct Matcher` with:
    - `Matcher::default()`
    - `set_binds(&mut self, b: &Keybinds)`
    - `set_paused(&mut self, p: bool)`
    - `down(&mut self, code: &str, at: std::time::Instant) -> Vec<Action>`
    - `up(&mut self, code: &str)`

**Behaviour the tests pin:**
- **Held state.** "Held" = codes in the down-set (removed on `up`).
- **Repeats.** A `down` for a code already held is a repeat (returns nothing, refreshes its time) only if it comes within **1 s** of that code's previous down. Otherwise it's a fresh press.
- **What matches.** A bind matches when its `code` is the code going down and every modifier flag it sets is satisfied by a held modifier other than the key going down. `ctrl` is satisfied by ControlLeft or ControlRight, and likewise shift, alt and meta.
- **Most specific wins.** Among matches, only those with the most modifier flags fire; a tie fires all of them.
- **Paused.** Paused returns nothing but still tracks held keys.

- [ ] **Step 1: Write the failing tests**

```rust
// scan-code table (set 1, extended flag as LL hooks report it)
assert_eq!(code_for_scan(0x1D, false), Some("ControlLeft"));
assert_eq!(code_for_scan(0x1D, true),  Some("ControlRight"));
assert_eq!(code_for_scan(0x51, true),  Some("PageDown"));
assert_eq!(code_for_scan(0x51, false), Some("Numpad3"));
assert_eq!(code_for_scan(0x32, false), Some("KeyM"));
assert_eq!(code_for_scan(0x5B, true),  Some("MetaLeft"));
assert_eq!(code_for_scan(0x36, false), Some("ShiftRight"));
assert_eq!(code_for_scan(0x1C, true),  Some("NumpadEnter"));
assert_eq!(code_for_scan(0x64, false), Some("F13"));
// every code the table produces round-trips through is_known; "Banana" and "Mouse6" don't
```

Matcher tests (helper `m(binds)`, `t0 = Instant::now()`, `t(ms)` = `t0 + ms`):
- `page_down_alone_fires_mute_once_while_held`: downs at 0, 400, 800 ms → `[ToggleMute]`, `[]`, `[]`; `up`, then down at 900 → `[ToggleMute]`.
- `a_missed_key_up_doesnt_eat_the_next_press`: down at 0, no up, down at 5000 ms → `[ToggleMute]`.
- `shift_page_down_still_mutes` (superset rule).
- `right_ctrl_alone_fires_and_left_ctrl_doesnt`: bind `ControlRight`; down `ControlLeft` → `[]`; down `ControlRight` → `[ToggleDeafen]`.
- `right_ctrl_then_c_deafens_once`: down `ControlRight` → `[ToggleDeafen]`; down `KeyC` → `[]`.
- `a_lone_modifier_bind_also_counts_as_ctrl_for_combos`: deafen = `ControlRight`, mute = Ctrl+Shift+`KeyM`. Downs `ControlRight`, `ShiftLeft`, `KeyM` → `[ToggleDeafen]`, `[]`, `[ToggleMute]`.
- `the_most_specific_bind_wins`: mute = `KeyM`, deafen = Ctrl+`KeyM`. Ctrl held + `KeyM` → `[ToggleDeafen]`; `KeyM` alone → `[ToggleMute]`.
- `equal_matches_both_fire`: mute = Ctrl+`KeyM`, deafen = Shift+`KeyM`. Ctrl+Shift+`KeyM` → both.
- `mouse_buttons_fire`: mute = `Mouse3`; down `Mouse3` → `[ToggleMute]`. deafen = Ctrl+`Mouse4` works with Ctrl held.
- `paused_fires_nothing`.
- `a_key_held_through_unpause_doesnt_fire`: paused, down `PageDown` at 0, unpause, down at 300 (repeat) → `[]`.
- `new_binds_replace_old`: `set_binds` with mute unbound → `PageDown` fires nothing.

- [ ] **Step 2: Run, expect a compile failure.** `cargo test -p pulse-client bindmatch`

- [ ] **Step 3: Implement.**
  - The table is a `const` slice of `(u16 scan, bool extended, &str code)`. It covers letters, digits, F1–F24, punctuation, the numpad (NumLock is `0x45` extended; Pause is `0x45` non-extended), navigation, arrows, all eight modifiers, ContextMenu, PrintScreen, ScrollLock, CapsLock and the media/volume keys.
  - The down-set is a small `Vec<(&'static str or String, Instant)>`; at most a handful of keys are held, so no HashMap.

- [ ] **Step 4: Run, expect PASS.** `cargo test -p pulse-client bindmatch`

- [ ] **Step 5: Commit** `feat(keybinds): pure bind matcher and scan-code table`

### Task 2: Validation and labels for any key, lone modifiers and mouse buttons

**Files:**
- Modify: `client/src-tauri/src/keybinds.rs` (`validate`, `label`, `load`, `set_keybinds`; delete `registrable`, `types_text`, `everyday`, the Win-key rule)
- Test: same file's `tests` module (replace the obsolete tests listed below)

**Interfaces:**
- Consumes: `bindmatch::is_known`.
- Produces:
  - `impl Bind { pub fn normalized(self) -> Bind }` clears the flag of the bind's own modifier (ControlRight with `ctrl: true` becomes ControlRight with `ctrl: false`). `load` and `set_keybinds` normalize before `validate`.
  - `validate(&Keybinds) -> Result<(), String>`. Messages:
    - `"{action}: add a key"` for an empty code
    - `"{action}: Esc is used to cancel; pick another key"`
    - `"{action}: left and right click can't be binds"`
    - `"{action}: {key} isn't a key Pulse knows"`
    - the existing duplicate message
  - `label(&Bind)`:
    - `Mouse3` → `"Mouse 3"`;
    - `ControlRight` → `"Right Ctrl"` (likewise `Left/Right Shift|Alt|Win`);
    - otherwise as today: `"Ctrl+Shift+M"`.

- [ ] **Step 1: Replace the tests.**
  - Delete these, which encode the old stealing rules:
    - `keys_that_type_or_navigate_need_a_modifier`
    - `ctrl_letter_alone_is_refused_but_with_shift_is_fine`
    - `ctrl_alt_with_a_typing_key_is_refused`
    - `the_windows_key_is_refused`
    - `keys_windows_hotkeys_cant_use_are_refused`
  - Keep the other tests.
  - Add:
    - `any_key_may_stand_alone`: `PageDown`, `KeyM`, `Space`, `Numpad3`, `F13`, `ControlRight`, `MetaLeft`, `Mouse3`, `Mouse5`, and Win+`KeyM` are all `Ok`.
    - `escape_and_left_right_click_are_refused`: `Escape`, `Mouse1` and `Mouse2` are each `Err`, and the message contains "Esc" or "click".
    - `unknown_codes_are_refused`: `"Banana"` and `"Mouse6"` are `Err`.
    - `a_lone_modifier_drops_its_own_flag`: `Bind{code:"ControlRight", ctrl:true}.normalized().ctrl == false`.
    - `old_binds_still_load`: write a v1 file with Ctrl+Shift+`KeyM`, then `load` returns it unchanged.
    - `labels`: `"Mouse 3"`, `"Right Ctrl"`, `"Left Shift"`, `"Ctrl+Shift+M"`.
- [ ] **Step 2:** `cargo test -p pulse-client keybinds`, expect FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** `cargo test -p pulse-client keybinds`, expect PASS.
- [ ] **Step 5: Commit** `feat(keybinds): any key, lone modifiers and mouse buttons are valid binds`

### Task 3: Windows low-level hooks replace RegisterHotKey

**Files:**
- Modify: `client/src-tauri/src/hotkeys.rs` (rewrite; delete `Registrar`, `Hotkeys`, the plugin `platform` module and their tests)
- Modify: `client/src-tauri/Cargo.toml`:
  - remove `tauri-plugin-global-shortcut`;
  - add the `windows` features `Win32_UI_WindowsAndMessaging`, `Win32_UI_Input_KeyboardAndMouse` and `Win32_System_LibraryLoader`.
- Modify: `client/src-tauri/src/lib.rs`, only if a call site changes (it shouldn't).

**Interfaces:**
- Consumes: `bindmatch::{Matcher, code_for_scan}`, `commands::hotkey(&AppHandle, ipc::Command)`, `Action::command()`.
- Produces: the same public API as today, so `lib.rs` and `keybinds.rs` don't change:
  - `pub fn install(app: &tauri::App, binds: &Keybinds) -> tauri::Result<Vec<String>>`
  - `pub fn apply(app: &AppHandle, binds: &Keybinds) -> Vec<String>`
  - `pub fn pause(app: &AppHandle, paused: bool) -> Vec<String>`
  - The returned `Vec<String>` is now empty unless the hooks couldn't be installed. In that case it holds `"Keybinds couldn't start ({error}); restart Pulse to try again"`.

**Design:**
- **State.** A `static MATCHER: Mutex<Matcher>`. `apply` and `pause` lock it briefly; the hook proc locks it per event, and nothing else holds it.
- **Hook thread.** One thread named `pulse-keys`:
  - installs both hooks with `SetWindowsHookExW(.., GetModuleHandleW(None), 0)`;
  - then runs `GetMessageW` until the process exits;
  - reports install success or failure back to `install` through a oneshot `std::sync::mpsc` channel.
- **Keyboard proc.**
  - For `nCode == HC_ACTION`, map `scanCode` and `LLKHF_EXTENDED` to a code.
  - When `scanCode == 0` (some injected keys), use `MapVirtualKeyW(vkCode, MAPVK_VK_TO_VSC_EX)` instead.
  - `WM_KEYDOWN`/`WM_SYSKEYDOWN` → `down`; `WM_KEYUP`/`WM_SYSKEYUP` → `up`.
- **Mouse proc.**
  - `WM_MBUTTONDOWN`/`UP` → `Mouse3`.
  - `WM_XBUTTONDOWN`/`UP` with HIWORD(mouseData) `XBUTTON1` → `Mouse4`, `XBUTTON2` → `Mouse5`.
  - Every other mouse message returns immediately.
- **Firing.** Fired actions go over an `mpsc::Sender<Action>` to a forwarder thread. That thread calls `commands::hotkey(&app, a.command())`.
- **Pass-through.** Both procs **always** `return CallNextHookEx(None, ..)` and never filter injected events.
- **Logging.** Log `info!("keybinds: hooks installed")` once. No per-event logging.
- **Non-Windows.** The stub `platform` (returns empty) stays as it is.

- [ ] **Step 1:** Rewrite `hotkeys.rs` as above. The logic is covered by Task 1's tests; this file is a thin shell.
- [ ] **Step 2:** Run `cargo build -p pulse-client && cargo test -p pulse-client`. Expected: PASS on Linux.
- [ ] **Step 3:** Run `XWIN_ACCEPT_LICENSE=1 cargo xwin clippy -p pulse-client --target x86_64-pc-windows-msvc -- -D warnings`. Expected: no warnings.
- [ ] **Step 4:** Run `grep -rn global.shortcut client/`. Expected: no hits, including `capabilities/` and `package.json`.
- [ ] **Step 5: Commit** `feat(keybinds): pass-through low-level hooks on Windows (no more stolen keys)`

### Task 4: Recorder (lone modifiers, mouse buttons), Settings copy, Linux doc

**Files:**
- Modify: `client/ui/src/lib/keybinds.ts`, `client/ui/src/components/KeybindSettings.svelte`, `docs/linux-hotkeys.md`
- Test: `client/ui/src/lib/keybinds.test.ts`

**Interfaces:**
- Consumes: the Rust codes and labels from Task 2.
- Produces:
  - `export class Recorder` with:
    - `down(e: KeyLike): Recorded`
    - `up(e: KeyLike): Recorded`
    - `mouse(e: MouseLike): Recorded`
  - `type MouseLike = { button: number; ctrlKey; shiftKey; altKey; metaKey }`.
  - `Recorded` gains `{ kind: 'none' }`.
  - `keyLabel` and `bindLabel` learn the mouse and lone-modifier labels.

- [ ] **Step 1: Tests.** Keep the existing ones, using `new Recorder().down(…)` where `fromKeyEvent` was used, and add:
  - `a lone modifier is recorded on release`: `down(ControlRight, ctrlKey:true)` → `held`; `up(ControlRight)` → `bind {code:'ControlRight', ctrl:false, …}`.
  - `two modifiers released with no key record nothing`: down `ControlLeft`, down `ShiftLeft`, then up each → `held`/`none`, never `bind`.
  - `a modifier then a key records the combo, not the modifier`: down Ctrl, down `KeyC` → `bind KeyC ctrl`; a later `up(ControlLeft)` → `none`.
  - `mouse buttons`:
    - button 1 → `Mouse3`; 3 → `Mouse4`; 4 → `Mouse5`, carrying held modifiers;
    - buttons 0 and 2 → `none`.
  - `labels`:
    - `Mouse3` → `'Mouse 3'`; `ControlRight` → `'Right Ctrl'`; `MetaLeft` → `'Left Win'`; `AltRight` → `'Right Alt'`;
    - `bindLabel` with Ctrl + `Mouse4` → `'Ctrl + Mouse 4'`.
- [ ] **Step 2:** Run `pnpm -C client/ui test keybinds`. Expected: FAIL.
- [ ] **Step 3: Implement.**
  - **Recorder state.** It keeps the set of keys pressed in this recording and whether any key besides that one modifier was involved. Escape → `cancel`, as today.
  - **Svelte, new listeners.** Add `onkeyupcapture` and `onmousedowncapture` on the window.
  - **Svelte, mouse handling.** While recording, `preventDefault`/`stopPropagation` buttons 1/3/4, and also on `auxclick`, so wheel-click autoscroll and back/forward don't fire. Left and right clicks behave as today.
  - **Svelte, hint copy.** Exactly: "Works everywhere and the key keeps working in other apps. Doesn't work while an app running as administrator is focused."
  - **Svelte, recording prompt.** Becomes "Press a key, a combination or a mouse button. Esc cancels."
- [ ] **Step 4:** Run `pnpm -C client/ui test` and `pnpm -C client/ui check`. Expected: PASS.
- [ ] **Step 5: Update `docs/linux-hotkeys.md`.**
  - Show `bindn` (non-consuming, so the key still reaches apps), e.g. `bindn = , Next, exec, pulse-app --toggle-mute`.
  - Show the mouse buttons as `mouse:274` (wheel click) and `mouse:275`/`mouse:276` (side).
  - Delete the stale "Windows uses the same shortcuts (Ctrl+Shift+M / Ctrl+Shift+D)" line.
  - Update the Linux hint in the Svelte file to use `bindn` too.
- [ ] **Step 6: Commit** `feat(keybinds): record lone modifiers and mouse buttons; Linux bindn docs`

### Task 5: Wrap-up, checks and the Windows kit

- [ ] **Step 1:** Run `just check`. Expected: all green.
- [ ] **Step 2:** Run `XWIN_ACCEPT_LICENSE=1 cargo xwin clippy -p pulse-client --target x86_64-pc-windows-msvc -- -D warnings`. Expected: clean.
- [ ] **Step 3:** Push `keybinds-v2` (pre-authorised) and run `gh workflow run release-windows.yml --ref keybinds-v2`. Expected: the kit builds.
- [ ] **Step 4:** Give the owner the Windows checks from the spec:
  1. Page Down mutes while the browser still scrolls.
  2. Right Ctrl deafens and Ctrl+C still copies.
  3. Mouse 3 still opens a link in a new tab.
  4. Holding a bind toggles once.
  5. A non-admin game in focus gets every key.
  6. A GG/G Hub-remapped side button fires.
- [ ] **Step 5:** Merge only with the owner's OK, after `optimisation-run` is merged.
