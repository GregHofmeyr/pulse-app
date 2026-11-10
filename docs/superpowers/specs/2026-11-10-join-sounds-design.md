# Custom join sounds

Issue #121. Decided with the owner on 2026-11-10. The owner's own words and decisions are marked **(owner)**; everything else is the builder's call unless it says otherwise.

## Why

Discord lets people pick a soundboard sound that plays when they join a call; the group uses it and finds it "annoyingly funny" when someone joins mid-conversation and everyone stops talking until it finishes **(owner)**. Promised in the 0.2.2 patch notes' Coming soon.

## What it does

- **The sound comes from the server's soundboard (owner).** In a server's soundboard, right-click a sound → **Set as my join sound**. On your current join sound the menu shows **Clear my join sound** instead **(owner)**. One join sound per person per server; none set = the normal join cue. Nothing about join sounds lives in profile settings **(owner)**.
- **When it plays:** when you join a voice channel in that server, everyone in the call (you included) hears your sound instead of the normal join cue. DM and group calls have no server, so they keep the normal cue. **Leaving keeps the normal leave cue (owner).**
- **Length: the full sound, up to the soundboard's 6-second limit (owner).** It plays through the same path as soundboard plays (each listener plays it locally, through their output device and the mixer).
- **Volume and opting out (owner):**
  - a global **"Join sounds"** row in the volume mixer (0 % = you hear the normal join cue for everyone instead);
  - **"Mute join sound"** in the person right-click menu on every surface (members, voice rows, stage tiles, DMs), per person, per device: that person's custom sound is replaced by the normal join cue for you. The menu shows **"Unmute join sound"** while muted. (Person-to-person actions live in right-click menus on every surface.)
- **Edge cases:** if the sound is deleted from the soundboard, the join sound clears. Rejoining in quick succession plays the custom sound at most once per 30 s per person (otherwise the normal cue), so a flaky connection or channel hopping can't spam it. A server-muted or deafened joiner still gets their join sound (it's a cue, not their voice). Joining as a ghost viewer or someone who appears invisible plays nothing, as today.
- **Older clients** keep hearing the normal join cue; nothing breaks.

## How it gets made

- **Server:** store each member's join sound per server (next free migration number; rehearse with foreign keys OFF as `server/src/db.rs` runs them); routes to set/clear (server membership; the sound must belong to that server); clear on sound delete and on leaving the server; include the joiner's join sound in whatever the join event/voice state already carries so listeners know which sound to play, plus the 30 s once-per-person rule.
- **Client:** the soundboard right-click items, playing the join sound instead of the join cue when a member joins your call (reuse the soundboard's local playback and cache, so it starts fast; preload sounds of people in the server where cheap), the mixer row, the per-person mute in the shared person menu with its per-device store.
- Look and feel matches the existing soundboard menu, mixer rows and person menus exactly; existing tokens; guard tests pass.

## Testing

- Server: set/clear, wrong-server sound refused, cleared on delete/leave, the join event carries it, the 30 s rule.
- Client: menu items swap Set/Clear; join plays the custom sound for everyone in the call; mixer 0 % and per-person mute fall back to the normal cue; DMs keep the normal cue; leave cue unchanged.
- **Manual (owner, two accounts):** set a 6-second sound, join mid-conversation, everyone hears it fully; mute it for one person; clear it.

## Out of scope

- Leave sounds, personal uploads outside the soundboard, join sounds in DM calls.
