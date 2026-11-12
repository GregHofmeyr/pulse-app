# "Listening to…" and live Jam art on the stage

Issue #275 (research comment on #234 is the background). Decided with the owner on 2026-11-12. **(owner)** marks the owner's decisions.

## What it does

- **Opt-in "Listening to" activity:** Pulse reads what the local Spotify app is playing through the OS media session (MPRIS on Linux, the system media transport controls / GSMTC on Windows). No Spotify login or Web API. When something is playing, your activity shows "Listening to <title> · <artist>" with the Spotify icon and album art where available, in your profile card, the member list and DM header (wherever status already shows), **visible to everyone (owner)**. Pausing/stopping clears it after a short delay.
- **Two settings (owner):**
  - **Show my activity** (on/off): whether others can see it.
  - **Read media activity** (on/off): whether Pulse reads the media session at all. Off = zero work done (no listener, no polling) for people who want every bit of headroom **(owner)**. Show-activity off but reading on is only useful for the Jam art below; builder decides whether that combination exists or "Show" implies reading (explain).
  - Default: both **off** until the person turns them on (opt-in), with a one-line explanation in Settings.
- **"Listen on Spotify" from the activity** when a track id/URL is known (MPRIS often has it; GSMTC may not: fall back to a search link or no button).
- **Live Jam art on the stage:** when you've shared a Jam to the stage (#234) and your activity is being read, the stage card shows the song currently playing on your Spotify (title, artist, art) and updates when it changes ("Now playing" line under the Jam). Only the sharer's client sends updates, once per song change. Others see it update live.
- **Older clients** are unaffected.

## How it gets made

- Client core: a media-session watcher behind the "Read media activity" setting, event-driven (MPRIS PropertiesChanged over D-Bus; GSMTC events), no polling; filters to Spotify's player only. Measure CPU/memory with it on vs off and report.
- Server: store/relay the current activity per user (in memory is fine; it's ephemeral), included in presence/Ready, events to people who can see the user; rate-limit updates; for stage Jams, the sharer's now-playing updates the share's display fields.
- UI: activity line in the existing status spots, Settings → Activity (or under Privacy) with the two toggles, stage card "Now playing".
- Spotify rules from #196 still apply: single-colour Spotify icon, approved wording, no remote control.

## Out of scope

- Other players (YouTube Music, Apple Music), games activity, Spotify Web API.
