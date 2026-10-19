# Keybinds v2: pass-through binds, single keys and mouse buttons (design)

Status: draft, 2026-10-19. Builds on the keybinds already on main (`keybinds.rs`, `hotkeys.rs`,
`KeybindSettings.svelte`). Branch `keybinds-v2`, cut from `optimisation-run` (not merged yet).

## Why

Today's Windows binds go through `RegisterHotKey` (tauri-plugin-global-shortcut). Windows then
**takes the combination away from every other app**, so `validate` forces Ctrl/Alt onto every key
people type or use (Page Down, letters, arrows…) and rejects lone modifiers. That rules out how the
group actually binds things in Discord:

- the owner: **Page Down = toggle mute, Right Ctrl = toggle deafen**, and both keys keep doing their
  normal job in the focused app (Page Down still scrolls the browser, Right Ctrl still works as Ctrl);
- others: **mouse buttons**, mainly clicking the scroll wheel (Mouse 3), also the side buttons
  (Mouse 4/5).

Success = those exact binds work in Pulse on Windows, like they do in Discord, and nothing is lost
in the focused app.

## Goals

1. **Pass-through:** a bind fires its Pulse action *and* the key/button still reaches whatever app
   has focus, Pulse's own window included. Nothing is swallowed.
2. **Any key can be bound on its own** (Page Down, F-keys, a plain letter, a lone modifier such as
   Right Ctrl). Binding a silly key is the user's call, not ours.
3. **Mouse buttons:** Mouse 3 (wheel click), Mouse 4, Mouse 5 can be bound.
4. Combos still work (Ctrl+Shift+M, Ctrl+Mouse 4).

## Non-goals

- Push-to-talk (deferred: nobody in the group uses it).
- Actions beyond toggle mute / toggle deafen (later, when a use case shows up).
- Linux behaviour changes: Linux keeps compositor binds + the `pulse-app --toggle-*` CLI (only the
  doc is refreshed, see below).
- Running Pulse elevated / as admin.

## Anti-goals (would be a failure even if it "works")

- Any key or click being swallowed or delayed in other apps, games especially.
- A bind toggling repeatedly while a key is held (auto-repeat).
- Binds firing while the user is recording a new bind in Settings.

## Decided rules (the owner, 2026-10-19)

- **Never bindable:** Escape (used by the recorder), left click (Mouse 1), right click (Mouse 2).
  Everything else is allowed, alone or with modifiers.
- **Fires on press.** Holding a key toggles once; auto-repeat is ignored until the key is released.
- **A combo that contains the bind still fires it:** Right Ctrl+C deafens when Right Ctrl is the
  deafen bind; Shift+Page Down mutes when Page Down is the mute bind. This matches Discord and is an
  accepted trade-off ("we pick keys out of the way").
- **Left and right modifiers are different keys when bound alone:** binding Right Ctrl leaves Left
  Ctrl untouched.
- **Always on**, as today (not only in voice); paused while a bind is being recorded. A bind outside
  voice flips the stored mute/deafen state, same as clicking the button.
- **Only mute and deafen** are bindable.

## How (Claude's calls unless marked)

### Windows: low-level hooks instead of RegisterHotKey

- Replace tauri-plugin-global-shortcut with `WH_KEYBOARD_LL` + `WH_MOUSE_LL` hooks via the
  `windows` crate already in the tree (add the `Win32_UI_WindowsAndMessaging` /
  `Win32_UI_Input_KeyboardAndMouse` features). Drop the plugin dependency.
- Hooks live on **one dedicated thread with its own message loop** (hooks are delivered to the
  installing thread's loop). The hook procedure only: updates the held-keys set, decides whether a
  bind fired, and if so sends the `Action` over a channel; it **always** returns
  `CallNextHookEx` (pass-through) and never blocks, allocates heavily, or takes a contended lock.
  Windows silently unhooks a callback that exceeds `LowLevelHooksTimeout`, so all real work
  (`commands::hotkey`) happens off-thread, as today.
- Keys are matched by **scan code + extended flag → W3C `code`** (physical key, so a layout switch
  doesn't move binds, same as today's stored format). Mouse: `WM_MBUTTONDOWN` → `Mouse3`,
  `WM_XBUTTONDOWN` XBUTTON1/2 → `Mouse4`/`Mouse5`.
- **Injected input counts** (*Claude's call*): mouse/keyboard software (SteelSeries GG, Logitech G
  Hub, Razer Synapse, AutoHotkey remaps) often delivers side buttons and remapped keys as injected
  events. Ignoring them would silently kill exactly the binds the group uses. Pulse injects nothing,
  so there's no loop to guard against.
- **Matching:** on a key/button *down* that isn't an auto-repeat (track "already down" per key), a bind
  matches when its key is the one going down and **at least** its modifiers are held (the
  superset rule above; either-side Ctrl satisfies a combo's `ctrl`). If several binds match, only
  the most specific fires (most modifiers), so `M` and `Ctrl+M` can coexist. *Claude's call:* if
  two actions tie on specificity (mute = Ctrl+M, deafen = Shift+M, user presses Ctrl+Shift+M),
  **both fire**. Rare, and predictable.
- Lone modifier bind (`code = "ControlRight"`, no flags): fires when that exact side goes down.
- Held-state hygiene: low-level hooks see every key-up system-wide, except while an elevated window
  or the secure desktop (lock screen, UAC) has input. So a key can look "stuck down" afterwards and
  its next press would be mistaken for a repeat. Rule: a down for a key already in the "down" set is
  a repeat **only if** it arrives within 1 s of that key's previous down/repeat event (Windows
  auto-repeat runs every ≤ 0.5 s); otherwise it's a fresh press. Requirement: "held = once, never
  stuck". Builder may pick a cleaner rule that meets it.
- Pause/resume while recording stays (`pause_keybinds`), implemented as "hook installed but
  ignores binds" so recording doesn't need a reinstall. Paused = **only firing is suppressed**:
  held-key tracking continues, so the key still held when recording ends doesn't fire the new bind
  on its next auto-repeat.
- **Code layout** (*Claude's call*): the scan-code → W3C table and the `Matcher` live in an ungated
  module, tested on Linux CI; `#[cfg(windows)]` holds only the hook thread around them. The
  `Registrar` trait, `register_all` and the "taken by another app" tests go away with
  RegisterHotKey.
- **Known limit, documented, not fixed:** windows of elevated (admin) processes don't deliver input
  to a non-elevated hook (UIPI), so binds don't fire while an admin game has focus. Discord has the
  same limit. Mention it in Settings' hint text and the docs.
- The "taken by another app" problem path disappears (hooks can't conflict); keep the `problems`
  plumbing only for "hook couldn't be installed" (reported once in Settings).

### Validation (keybinds.rs)

`validate` shrinks to: code non-empty; code is a key we can map (keyboard W3C code we translate, or
`Mouse3`/`Mouse4`/`Mouse5`); not `Escape`, `Mouse1`, `Mouse2`; no two actions share an identical
bind. The typing/everyday/AltGr/Win rules go (they existed only because binds stole keys). `Meta`
modifiers are allowed now too (pass-through means Windows keeps its own Win shortcuts working).
Existing `keybinds.json` files stay valid (format is a superset).

### Recording (KeybindSettings.svelte / keybinds.ts)

- Keyboard: unchanged for normal keys. **Lone modifier:** pressing then releasing a modifier with no
  other key in between records that modifier alone (side-specific code, e.g. `ControlRight`, with
  its own flag cleared). While a modifier is held the button shows "Ctrl + …" as today. Pressing
  and releasing **several** modifiers with no other key records nothing (*Claude's call*), and the
  recorder keeps waiting.
- Mouse: while recording, `mousedown` with `button` 1/3/4 records `Mouse3`/`Mouse4`/`Mouse5` (plus
  held modifiers); `preventDefault` so wheel-click autoscroll / back-forward navigation don't fire in
  the webview. Left/right clicks keep their normal meaning (clicking elsewhere / the button again
  stops recording, as today).
- Esc while recording **cancels** (bind unchanged), as today; the × button removes a bind (the owner, 2026-10-19).
- Labels: `Mouse 3` / `Mouse 4` / `Mouse 5`, `Right Ctrl`, `Left Shift` etc. for lone modifiers;
  Rust `label()` mirrors them for messages.
- Hint text: "Works everywhere and the key keeps working in other apps. Doesn't work while an app
  running as administrator is focused."

### Linux

No code change. Refresh `docs/linux-hotkeys.md`: Hyprland's `bindn` (non-consuming, i.e.
pass-through) + mouse codes (`mouse:274` = wheel click, `mouse:275/276` = side buttons), and drop
the stale "Windows uses Ctrl+Shift+M by default" line (there are no defaults).

## Testing

- Rust unit tests for the pure parts: scan-code → code mapping, the matcher (superset rule, most
  specific wins, auto-repeat once, lone left vs right modifier, injected events ignored, paused),
  and `validate`'s new rules. The hook thread is a thin shell around a pure `Matcher`.
- vitest for recording: lone-modifier on release, mouse buttons, labels.
- `cargo xwin clippy` for the Windows code (Linux CI doesn't compile it).
- Windows manual check by the owner: Page Down mute while a browser scrolls; Right Ctrl deafen while
  Ctrl+C still copies; Mouse 3 in a browser still opens links in a new tab; holding a bind toggles
  once; a game in focus (non-admin) still gets every key.
