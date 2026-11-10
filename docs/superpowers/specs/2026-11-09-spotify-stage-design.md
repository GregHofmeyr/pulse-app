# Share a Spotify Jam or song to the voice stage

Issue #234. Drafted 2026-11-09 from the research on #234 and the #196 decision; layout decided with the owner on 2026-11-10 using a playground (https://claude.ai/artifact/EoSWhacUyfQLxxMmdbwhMv, private). The owner's decisions are marked **(owner)**; **(pick)** marks the recommendations the owner is being asked to confirm.

## Why

Everyone in the group has Spotify Premium and uses Jam. Today a Jam link in chat scrolls away; on the voice stage, everyone in the call can see what's playing and join with one click.

## What it does

- **Sharing:** in a voice channel, someone in the call can share a Spotify link to the stage:
  - from a **Spotify card in that channel's chat** (#196 cards, #226 voice chat), a "Share to stage" button on the card;
  - from the **stage's header**, a Spotify button that takes a pasted link (Jam invite, track, album, playlist, episode or show; the same links #196 already unfurls).
- **One card per channel (pick):** the stage shows at most one shared card; a new share replaces it. (Per-person cards were the alternative; one card keeps the stage calm and fits "we're all in this Jam".)
- **The card:** the #196 Spotify card look (art uncropped, title, artist or host, the single-colour Spotify icon, approved button wording "Join the Jam" / "Listen on Spotify" / "Open Spotify"), plus "Shared by alex". A Jam card says **"may have ended"** in its subtitle after an hour, because Pulse can't tell whether a Jam is still live (no public Jam API).
- **Where it sits (owner), per viewer:**
  - **Expanded (default): a strip above the tiles** (art, title, subtitle with "Shared by alex", the Spotify icon, the Spotify button, Minimise, ✕). It never covers tiles or a stream.
  - **Minimised: a small floating pill in the stage's bottom-right corner** (round art, Spotify icon, title). **Hovering** the pill previews the corner card; **clicking** it opens the **corner card** (a floating mini player: art, title, subtitle, the Spotify button, "Shared by", and controls to go back to the strip, minimise again, or ✕).
  - **✕ hides it for you** until you click the **Spotify button in the stage header**, which turns green while something's shared and you've hidden it.
  - **Watching a stream big (owner):** while you have a stream expanded or fullscreen on the stage, the Spotify strip/card/pill is hidden for you to keep the room for the stream, and comes back as it was when you leave that view.
  - The state (expanded / minimised / corner / hidden) is remembered per device; a new share shows expanded again unless you'd hidden the previous one.
- **How long it stays (pick):** until the sharer clears it, **the sharer leaves the call**, or **3 hours** pass, whichever is first. Everyone in the call sees it live; people joining later see it too.
- **Stopping it for everyone (pick):** the ✕ only hides it for you; "Stop sharing" (in the strip's and corner card's controls) clears it for everyone and is available to anyone in the call, since a stale Jam shouldn't need its sharer. A small system line in the channel's chat records shares, not clears: "🎵 alex shared a Jam to the stage" (no line for clears or expiry).
- **Older clients** ignore the new event; the system line's text reads fine on them.

## How it gets made

- **Server:** per-channel stage share state (in memory is fine: it's ephemeral and dies with the call; or a tiny table if restarts should keep it, builder's call, explain), routes to share and clear with call membership checks, link validation through the existing Spotify embed resolver (no Spotify API keys, no remote control, per #196), events to the channel, cleared when the sharer leaves the call or after 3 h, and on channel delete.
- **Client:** the "Share to stage" button on Spotify cards in voice chats, the stage header button and paste box, the stage strip/pill, live updates, the ✕.
- Look and feel matches the stage, the #196 Spotify card and Spotify's design rules exactly; existing tokens; icon sizes 14/16/18; reduced motion respected.

## Testing

- Server: share/replace/clear; non-members refused; non-Spotify links refused; cleared when the sharer leaves and after 3 h; events.
- Client: both entry points, the strip and pill, replace, clear, "may have ended" after an hour, late joiners see it.
- **Manual (owner, two accounts):** share a real Jam from chat and from the header; the other account sees it and Join the Jam opens Spotify; the sharer leaves and the card goes.

## Out of scope

- Detecting whether a Jam is live, listen-along or controlling anyone's Spotify (Spotify policy, #196).
- "Listening to…" presence from the local Spotify app (research option 2; separate issue if wanted).
- Full-track in-app playback (research option 4; needs a Windows check first).
