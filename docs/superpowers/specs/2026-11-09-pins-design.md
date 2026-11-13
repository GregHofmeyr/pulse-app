# Pinned messages

Issue #178. Decided with the owner on 2026-11-09. The owner's own words and decisions are marked **(owner)**; everything else is the builder's call unless it says otherwise.

## Why

Friends want to keep a message in view for a while (a plan for tonight, a link, a joke) without it scrolling away. Promised in the 0.2.1 patch notes' Coming soon.

## What it does

- **Where:** text channels, DMs, groups, and voice channels' chats (#226) **(owner)**. Not quotes or clips channels; revisit only if someone asks **(owner)**. Since revisited: quotes and clips channels take pins too (#257, below).
- **Who:** anyone who can read and write the chat can pin and unpin **(owner)**. Pulse has no roles yet; the system line shows who pinned.
- **Pinning:** the message's menus (hover bar ⋯ / right-click, wherever its other actions live; match the existing surfaces) get **Pin message ›**, a hover flyout using the shared `MenuFlyout` (the same one mute uses, #236) with **1 hour, 1 day, 1 week, Forever** **(owner)**. A pinned message's menu shows **Unpin message** instead **(owner)**.
- **Expiry:** a timed pin disappears quietly when its time runs out; no line, no notification **(owner)**.
- **Marker:** a small pin icon (size 14) by the message's timestamp; its tooltip reads e.g. "Pinned by alex · 2 days left" or "Pinned by alex" for Forever **(owner)**.
- **System line:** when someone pins, a system line appears in the chat under the message: "alex pinned a message for 1 day · See pins" ("…pinned a message" for Forever). "See pins" opens the pins popover; clicking the line's message reference jumps to it. Unpins and expiries add no line **(owner)**. Pinning a message again within 10 minutes of your own line for it brings that line up to date (its length) instead of posting another, so pin, unpin, pin again doesn't fill the chat; someone else's pin, or one after longer, is news and gets its own line (#258).
- **Clock:** a timed pin runs out by the server's clock. Ready carries `server_now`, and clients time pins by it (and live messages' stamps), so a client clock running ahead doesn't hide a pin early or offer Pin on one the server holds (#258).
- **Pins popover:** a pin icon (size 18, matching the other header icons) in the chat header opens a popover listing the chat's pins, newest pin first: author, a short preview, who pinned it, time left for timed pins, **Jump** (scrolls to the message and highlights it like other jumps), and Unpin. Empty state: a short friendly line. A popover, not a side panel **(owner)**.
- **Cap:** 50 pins per chat **(owner)**. Pinning a 51st is refused with "Unpin something first" (a toast or the flyout's inline message; match how other refusals read).
- **Live:** pins, unpins and expiries show up for everyone in the chat without a reload (the existing realtime events).
- **Deletes and edits:** deleting a pinned message removes its pin. Editing keeps it.

## How it gets made

- **Server:** a pins table (chat, message, pinned by, pinned at, expires at or null), next free migration number (0028 at the time of writing). Rehearse it on a prod copy with foreign keys OFF, as `db.rs` runs migrations that way. Routes to pin, unpin and list, with the same membership checks as sending a message; a realtime event on pin/unpin/expiry; expiry by a periodic sweep or computed on read plus an event (builder's call, explain it). The system line is a normal system message so history, search and unreads treat it like the others. Rate limits as for other writes. Tests for permissions (non-members refused, DMs only for participants), the cap, expiry, delete cascade, and events.
- **Client:** the menu flyout, the marker, the system line rendering, the header icon and popover, the jump. Reuse the existing jump/highlight, popover, menu and tooltip components; don't build new ones.
- Look and feel matches existing menus, headers and popovers exactly; existing tokens only; reduced motion respected; guard tests (motionCss, colorTokens, icon sizes) pass.

## Testing

- Server: pin, unpin, list; refusals; the cap; a timed pin expires and the event fires; deleting the message drops the pin.
- Client: the flyout and its four options; Unpin replaces Pin; marker and tooltip text; the system line and its See pins; the popover lists, jumps and unpins; live updates from another device.
- **Manual (owner, Linux then Windows):** pin for 1 hour and Forever in a channel and a DM; see the line, the marker and the popover; jump; unpin; a second account sees it live.

## Quotes and clips channels (#257)

Added later, as the "only if someone asks" above allowed. Builder's calls, kept as close to chats as those channels allow:

- **What pins:** a quote or a clip (the channel's posts). Thread replies still can't be pinned, and system lines (a Quote of the Year card is one) still can't.
- **Who, how long, how many:** unchanged: anyone who can post there (a member of the server), the same four lengths, the same cap of 50 per channel, the same expiry and delete rules. No migration: quotes and clips are rows of `messages` already.
- **Menus:** a quote's right-click menu (any line's: the quote's only menu since #244, where Copy quote already acts on the whole quote) gets **Pin quote ›** / **Unpin quote**. A clip's ⋯ menu (list and wall) gets **Pin clip ›** / **Unpin clip**, after Download. A quotes or clips channel's plain messages (thread replies) get no Pin item.
- **Marker:** the same pin icon by a quote's time, and by a clip's time in the list and on the wall.
- **System line:** in a quotes channel, as in chats, worded "alex pinned a quote for 1 day" (in its content too, for older clients). In a clips channel, **no line**: the channel shows only its clips (to this client and older ones), so a line would sit in its history unseen.
- **Pins popover:** in the quotes channel's header (with the leaderboard) and the clips channel's header (by the view toggle), named "Pinned quotes" / "Pinned clips". A quote reads as who said it and its words; Jump scrolls to it and flashes it. A clip reads as its thumbnail and caption (else its game, else "Clip · 0:30") by its poster; **Open** opens its panel, as a search result does (a clips channel has no history to jump in).
- **Older clients** (0.2.x): see the quote line as a normal system line, ignore pins they don't show, and never ask for a quotes or clips channel's pins.

## Out of scope

- Pin notifications, or a line for unpins and expiries.
- Per-role pin permissions (waits on roles, #79).
