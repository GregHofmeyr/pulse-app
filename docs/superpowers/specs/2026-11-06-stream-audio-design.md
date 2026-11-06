# Stream sound you can count on

Issue #215. Decided with the owner on 2026-11-06. The owner's own words and decisions are marked **(owner)**; everything else is the builder's call unless it says otherwise.

## Why

- A friend shared a window and viewers heard nothing. The log said `share: no single process owns the shared window; no stream audio` (`client/src-tauri/src/stream/audio.rs:591`); the sharer was never told.
- Today a window share finds its app by looking up the window's **exact title** at the moment sharing starts (`procs::window_pid`). Titles change (games, browser tabs) and two apps can share a title, so the lookup fails, and the stream goes out silent rather than guessing.
- Screen shares are fine: they capture everything except Pulse, app by app (the capture plan is a list of per-process captures, rescanned every 2 s).

## What it does

### A sound choice when sharing (owner)

- The share options get a three-way choice, styled like the existing quality and FPS pickers: **This app · Everything except Pulse · No sound**. A screen share shows only the last two.
- **Default:** This app, when the app is identified (below); otherwise Everything except Pulse.
- **This app** is greyed out **only when Pulse can't identify which app owns the window**, with a tooltip like "Can't tell which app owns this window" **(owner)**. Being silent right now is never a reason to grey it out: a paused game or video, or a tab not yet playing, is silent until it starts **(owner)**.
- Beside This app, a **live hint** while choosing: a small level meter or "Not playing sound right now", from Windows' list of apps with an active audio session (`procs::audio_session_pids`) **(owner)**.

### Finding the app reliably, in tiers (owner)

1. **The exact window picked:** use the picked window's own handle, not its title, to find its owning process, then capture that process and its children (the existing process-loopback include-tree mode). Find how the picker learns the window (the webview's capture source) and carry the handle through; if the platform only gives a title, match on title plus more (e.g. the window's class or the process image) and still prefer the exact window when several match.
2. **Related processes:** if the app's tree has no audio session while capturing, also capture processes in the same app family: the same executable path, its parent or launcher, or helpers it started (browsers and launchers often play sound from a separate process).
3. **Last resort (only when This app is chosen but nothing can be identified at all, i.e. the option was greyed out):** Everything except Pulse, also skipping always-on music apps so the stream is mostly the shared app **(owner)**. The skip list is a small, named list: Spotify plus other common music players, and Discord (my call: people often run it beside Pulse). The list lives in one place with a test.

### Never silent without saying so (owner)

- The stream controls show which source is in use (e.g. "Sound: Firefox", "Sound: everything except Pulse", "No sound").
- During a stream on This app, if the app stays silent for a while (builder picks, ~20 s) while sharing, a gentle note: "No sound from this app yet", with one click to switch to Everything except Pulse. Informational, never forced **(owner: no auto-switch)**.
- Window shares keep rescanning (today only screen shares rescan, `audio.rs:378`) so an app that starts sound later, or spawns a new audio process, is picked up.

## Constraints

- Windows is the platform that matters (friends are on Windows). Linux keeps today's behaviour unless it's free to improve.
- No change to what viewers do; the stream's audio track is the same.
- Privacy stays the point of This app: never capture other apps' sound unless the sharer chose Everything except Pulse, or the last-resort tier applies (and then the controls say so).
- Look and feel matches the existing share options and stream controls exactly.

## Testing

- Unit tests for the tier logic (inputs: windows, processes, sessions; outputs: the capture plan), the skip list, the default choice, and when This app is greyed out.
- The picker's choice and hint, the in-stream source label and the "no sound yet" note (UI tests).
- Windows checks via `cargo xwin clippy`; the real-capture behaviour is a manual owner check on Windows: share a browser window playing a video, a game, and a window of an app with no sound; switch tabs mid-stream; confirm what viewers hear and what the sharer sees.

## Out of scope

- Per-app volume inside a stream, or picking several apps by hand.
- Linux and macOS capture changes.
