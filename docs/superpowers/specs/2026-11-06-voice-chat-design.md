# Text chat in voice channels

Issue #226. Decided with the owner on 2026-11-06. The owner's own words and decisions are marked **(owner)**; everything else is the builder's call unless it says otherwise.

## Why

Friends use Discord's voice-channel chat constantly: a side panel next to the call for links, jokes and "can you hear me?". Today the server refuses messages in voice channels (`server/src/messages/routes.rs:283`, "voice channels have no messages").

## What it does

- **Each voice channel has its own persistent chat**, like Discord: what was said last session is still there **(owner)**.
- **Anyone in the server can read and write it**, in the call or not, like any text channel **(owner)**.
- **Two ways to open it (owner):**
  - a **chat icon on the voice channel's row** in the channel list, which opens the chat from outside the call (it opens as the main view, like a text channel, or as the panel if you're on that channel's stage; builder decides what reads best);
  - a **chat icon at the top right of the voice stage**, which toggles a **right-side panel** next to the stage.
- The panel is the normal chat: messages, composer, replies, reactions, emoji, mentions, links and embeds. Reuse the text channel's components; don't build a second chat.
- The panel is resizable like the other columns (#160's handles) and remembers open/closed and width on this device.
- **Unreads (my call):** a voice channel's chat counts unreads and mentions like a text channel, shown as a small dot or count on the row's chat icon (not on the channel name, which shows who's in the call). Notification levels (#195) apply as for a text channel.
- Streams: when watching a stream on the stage, the panel still opens beside it. #63 (stream fullscreen with chat as a side strip) reuses this panel later.

## How it gets made

- **Server:** allow normal messages in voice channels (the same rules as text channels: membership, limits, rate limits, mentions, embeds, reactions, edits and deletes, search). Quotes, clips and polls stay in their own channel kinds. Check every place that assumes "voice channels have no messages" (history, reads and unreads, search, Ready's latest pointers, channel delete and purge) and add tests. No migration should be needed (messages are keyed by channel); if one is, take the next free number.
- **Client:** the row's chat icon, the stage header toggle, the panel (reusing the text channel view's message list and composer), unread state for voice channels, and the auto-focus behaviour from #202 when the panel opens by click.
- Look and feel matches the existing columns, headers and icons exactly; respects reduced motion for the panel's open and close.

## Testing

- Server: send, read, edit and delete in a voice channel; non-members refused; unreads and mentions; search includes them; deleting the voice channel removes its chat.
- Client: both entry points; the panel toggles and remembers; unread dot on the row; the composer focuses on open; a second device sees new messages live.
- **Manual (owner, two PCs):** chat in the voice channel from inside the call and from outside it; see the unread dot on the row; scroll back to an earlier session's messages.

## Out of scope

- Voice-only (ephemeral) chat that disappears when the call ends.
- Threads inside voice chat beyond what text channels already support.
