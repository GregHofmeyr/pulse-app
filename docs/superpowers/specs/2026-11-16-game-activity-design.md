# Game activity: "Playing <Game>"

Issue #380. Decided with the owner on 2026-11-16 (issue comments). **(owner)** marks the owner's decisions.

## Goals

- **"Playing <Game>" with elapsed time**, next to (never replacing) "Listening to" (#275).
- **Windows only (owner).** Linux and macOS detect nothing: the watcher is a no-op there.
- **Detection v1 (owner):**
  - Steam: `HKCU\Software\Valve\Steam\RunningAppID` (a DWORD; non-zero = a Steam game runs). Its name comes from
    that app's `appmanifest_<appid>.acf`, looked for in every Steam library listed in
    `<SteamPath>/steamapps/libraryfolders.vdf` (games live on several drives; `SteamPath` is read from
    `HKCU\Software\Valve\Steam`). No fixed `C:\` paths.
  - Manual: Settings → "Add a game": pick an `.exe` and give it a name. Pulse then matches running processes by
    full image path (case-insensitive, `/` and `\` alike), falling back to the file name only when Windows won't give
    the path.
  - Polled every 15 s (one registry read, and a process snapshot only when manual games exist); sent only on change.
  - No scraping Discord's detectable list.
- **Art, Steam games only (owner):** the server fetches `header.jpg`, `library_hero.jpg` and `logo.png` for the
  app id from Steam's public CDN through the guarded embed fetcher, re-encodes them with the link-thumbnail pipeline
  (`media::process::link_thumb`, stored as `link_thumb` media), charges the "Listening to" art budget
  (`listening_art_user`, one hit per game fetched), and caches them by app id. Clients never contact Valve.
- **Where art shows (owner):** (1) a "Playing" card on the profile card: the wide art (hero, else header) faded
  behind a strip with the logo (else the name) and elapsed time; (3) a voice tile backdrop: a non-streaming tile in a
  call shows that person's game art dimmed behind the avatar. **Not the profile banner (owner).**
- **Toggles (owner), Settings → Privacy → Game activity, all default ON**, per device like the media switches:
  - **Game activity** (master): off = no watcher, no registry read, nothing sent; turning it off clears what the
    server holds.
  - **Show game art**: off = the Steam app id is never sent, so the server has nothing to fetch art for.
  - **Show play time**: off = no elapsed time is sent, so nobody sees one.
  - Later switches (Join, achievements, clip drop) go in the same group: the core's settings struct and the page's
    prefs are one object keyed by switch, so adding one is one field each.
- **Privacy:** only what the switches allow leaves the PC: the game's name, the Steam app id (art on), seconds
  played (play time on). The exe path never leaves the PC; manual games live in this device's webview storage and
  the core's memory only.

## Protocol (additive; no PROTOCOL_VERSION bump, no migration)

"Listening to" is its own field and event, not a generic slot, so "Playing" sits beside it the same way:

- `ClientFrame::Playing { playing: Option<PlayingUpdate> }`, `PlayingUpdate { name, steam_app_id?, elapsed_secs? }`.
  Older servers ignore the frame. Debug keeps the game name out of logs.
- `rest::Playing { name, steam_app_id?, started_at?, art? }` with `GameArt { wide?, header?, logo? }` (each an
  `EmbedThumb`). `started_at` is server time (`now - elapsed_secs`), so client clocks don't matter.
- `Person.playing` (omitted when none) and `Event::PlayingChanged { user_id, playing }`, told per viewer by the
  same pass as presence (`Hub::sync_presence`): Invisible, "appear offline to" and offline show none, exactly like
  Listening. Never published to an audience (`audience_for` sends it nowhere). Older clients skip the event.

Why not one generic `activities: Vec<Activity>`: it would change the shape older clients read for Listening and buy
nothing for two activities; the hub's per-connection bookkeeping is shared code (a generic slot) instead.

## Flows

1. Core: the watcher (Windows, master on) reads `RunningAppID` every 15 s; a non-zero id is named from its
   manifest (library list re-read only when an id isn't found in the cached one); else running processes are matched
   against the manual games. A pure `Tracker` turns "what runs now" into a change (same game → nothing; a new game →
   a new start time) and the frame is built from the switches.
2. Server: `activity::playing_received` cleans the update (one line of text, length cap, app id range), holds it per
   connection (newest ticket wins, like Listening), fetches art in the background when an app id is new to that
   connection, then `sync_presence` tells each viewer.
3. UI: `PlayingChanged` updates `people[id].playing`; the profile card shows `PlayingPanel`; `UserTile` shows the
   art behind the avatar; Settings shows the three switches and the manual games list.

## Non-goals / deferrals

- SteamGridDB art for non-Steam games (owner: "later"): non-Steam games show name and elapsed only.
- Join, achievements, clip drop (their own switches later, same group).
- Linux/macOS detection (owner: nobody games on Linux).
- Discord detectable-list scraping (owner: no).
- "Playing" in the member list / DM header lines (only the two owner-picked art spots, plus the profile card strip).
- Steam non-game apps (tools, Wallpaper Engine) show as "Playing" like games; filtering them is later.
