# Soundboard

Issue #43. Owner's picks (design-check page and a Discord screenshot, 2026-11-03), summarised on the issue.
**Status: approved by the owner 2026-11-03.** Custom join sounds wait until the soundboard has proven itself.

## Why

On Discord the soundboard is one of the group's most used features. Pulse has nothing like it.

## Goals

- In a call, open the soundboard and play a short sound that everyone in the call hears.
- Sounds belong to a server; anyone in it can add one, and you can play sounds from every server you're in, in any
  call (like custom emoji).
- Find a sound fast: search, favourites, frequently used, then each server's sounds.
- Each listener controls how loud the soundboard is for them, and can mute one person's sounds.
- Preview a sound only for yourself before playing it to the call.

## Non-goals

- Join sounds (later, once this has proven itself).
- Sounds outside calls (in text chat).
- A spam guard: none, by the owner's choice ("it's chaos… part of the fun").
- A slot limit per server.

## How it looks (from Discord's, which the owner likes)

- A **soundboard button** in the call controls opens a popover.
- **Top:** a search field ("Find a sound") and, at the right, a speaker icon that opens the personal **Soundboard
  volume** slider.
- **Left rail:** Favourites ★, Frequently used 🕒, then one icon per server.
- **Main area:** sections (Favourites, Frequently used, each server) of tiles in a 3-column grid. Each tile shows the
  sound's emoji and name, and a small 🔊 to preview it only for you. Clicking the tile plays it to the call.
- A tile's menu (right-click): Favourite / Unfavourite, Edit (name, emoji, volume), Delete.
- **Add sound** at the end of a server's section opens the upload dialog.

## Rules

- **Upload:** anyone in the server. An audio file (mp3, ogg, wav, m4a) is trimmed in-app to at most **6 s** (a simple
  start/end handle on a waveform), given a name (1–32 characters) and an optional emoji. Quiet sounds are evened out
  (loudness-normalised) and there's a per-sound **boost** slider, so a soft clip can be made audible.
- **Delete:** anyone in the server (owner's pick), with a confirm.
- **No slot limit.** Past 24 sounds in a server, the server's section shows a gentle "Tidy up?" that lists the
  least-played sounds with a one-click delete. Plays are counted per sound for this.
- **Playing:** anyone in a call, any sound from any server they're in. No cooldown; sounds may overlap.
- **Hearing:** each listener has a Soundboard volume (0 = off), saved per device, and can mute one person's sounds from
  that person's menu. Ties into per-category volume (#44).
- **Favourites and frequently used** are per user, synced across their devices.

## How it gets made (builder's call where not stated)

- **Playback is local on each listener**, not mixed into the player's microphone: when someone plays a sound, the
  server relays a `SoundPlayed { channel, sound, by }` event to everyone in that call, and each client plays the file
  itself through its own mixer at its own soundboard volume. This makes per-listener volume and per-person mute free,
  and works while the player's mic is muted. Clients cache sound files (they're small).
- **Files:** encoded in the client before upload (the core already ships ffmpeg for clips): trim, loudness-normalise
  (EBU R128), apply the boost, encode Ogg Opus mono ~64 kbps (≈50 KB for 6 s). The server checks the container, the
  duration (≤ 6.2 s) and the size, then stores it as a new media kind on disk (`pulse-media`, like emoji).
- **Tables:** `sounds (id, server_id, name, emoji, media_hash, volume, added_by, created_at, deleted_at, plays)`,
  `sound_favourites (user_id, sound_id)`, usage for "frequently used" (like `emoji_usage`).
- **Rate limiting:** only abuse-level (the owner wants chaos, not a flood that hurts the server), e.g. 10 plays a
  second per person.
- **Linux:** playback works wherever voice works; upload uses the system ffmpeg as clips do.

## Testing

- Server: upload validation (type, duration, size), anyone deletes, plays relayed only to the call, play counts,
  favourites per user, rate limit.
- Client: the popover (search, rail, sections, favourites, frequently used), preview plays only locally, soundboard
  volume and per-person mute apply, overlapping sounds mix.
- Manual (owner): upload a quiet clip and boost it; play sounds in a two-person call; preview without the other person
  hearing; turn your soundboard volume down; mute one person's sounds; search; favourite.
