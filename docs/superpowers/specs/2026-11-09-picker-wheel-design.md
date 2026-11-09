# Picker wheel

Issue #158. Decided with the owner on 2026-11-09, using a playground artifact. The owner's own words and decisions are marked **(owner)**; everything else is the builder's call unless it says otherwise.

## Why

The group uses online wheel pickers to choose a game, food, or a character to play. A Pulse wheel keeps that in the chat and saves the lists. Promised in the 0.2.1 patch notes' Coming soon ("the rainbow picker wheel").

## What it does

- **Two entry points (owner):**
  - a **wheel button in the composer** (next to the existing composer buttons) or `/wheel` on its own opens the **wheel popover** above the composer: pick a saved wheel or type options, then Spin;
  - `/wheel a, b, c` (comma-separated) in the message box spins a **one-off wheel** straight away without opening the popover. Fewer than two options opens the popover instead.
- **One person spins, everyone watches (owner):** Spin posts a **wheel card** into the chat that spins live for everyone in the channel and lands on the same slice for all of them. The server picks the winner and the final angle; clients only animate to it. Someone who opens the channel late or mid-spin sees the settled result, not a replay.
- **Result:** when the spin lands, the card collapses into a compact **system line**: "🎡 alex spun Games tonight → Valheim" with a small swatch in the winning slice's colour. No notification is sent for spins **(owner)**; wheels are often just for the spinner (e.g. picking their own character).
- **On the settled card/line:** **Spin again** and **Remove winner & spin** (anyone in the channel can press them; they post a new spin of the same options, minus the winner for the second). Removed winners are per spin chain, not saved into the wheel.
- **Saved wheels, two kinds (owner):**
  - **Server wheels:** visible to and editable by everyone in the server.
  - **Just me wheels:** visible only to you, usable in any chat.
  - The popover has a Server / Just me switch, the saved list, "+ New wheel", and for the selected wheel: add options (pasting `a, b, c` adds several), remove one, Shuffle, **Clear all** (empties it so you can type new options) **(owner)**.
- **Save a quick wheel (owner):** after a one-off spin, the popover and the card offer **Save as wheel ›**: Server or Just me, then a name.
- **Managing wheels in Settings (owner):** a **Wheels** page in Server settings (server wheels) and one in your own settings (Just me wheels): create, rename, edit options, delete. No spinning there. Settings lists get "+ add" like the other Settings lists.
- **Where:** server text channels and voice channels' chats, DMs and groups (Just me wheels only there, since there's no server). Not quotes or clips channels.
- **Look (owner):** the slices run through the **neon spectrum** (fully saturated, evenly spaced hues, like the playground's "Neon spectrum": `hsl(h 100% 58%)` for slice i of n at h = 360·i/n), **labels along the slice**, **thin gaps** between slices, a pointer at the top and a Spin hub in the middle. Slice labels use dark text on the light hues (yellows, greens, cyans) and white text on the dark hues so every label reads. Everything that isn't a slice follows Pulse's own look (menus, popover, buttons, tokens). The playground is the reference: https://claude.ai/artifact/XiSm3HE68z1MtfKnhxXGxy (private to the owner; the builder works from this spec).
  - The slice colours are generated, not hand-picked literals: put the generator in one place (a small `wheelColours` helper) and make it pass or be explicitly allow-listed by the colour-literal guard test, with a comment saying why.
- **Motion:** a ~4–5 s eased spin with a quiet tick per slice (respecting the app's sound settings and output device; off when sounds are off). **Reduced motion:** a short (≤ 0.5 s) spin, no tick. Unfocused/streaming rules for animations apply as elsewhere.
- **Limits:** 2–100 options per wheel, 60 characters per option, a sensible cap on saved wheels (say 50 per server and 50 per person), and a rate limit on spins per person (say one every 3 s) so the chat can't be flooded. Refusals read like the app's other refusals.

## How it gets made

- **Server:** tables for saved wheels (scope server or personal, owner, name, options) and for spins (chat, spinner, wheel name or one-off, options snapshot, winner index, final angle, created at); the next free migration numbers (pins #178 is likely taking 0028; rebase before choosing, and say in the report). Migrations run with foreign keys OFF (`server/src/db.rs`); table rebuilds have a guard. Routes to list/create/update/delete wheels (server wheels need server membership; personal wheels only their owner) and to spin (membership like sending a message; the server draws the winner with a proper RNG and returns the angle). A realtime event carries a new spin to the channel. The settled spin is a normal system message so history, search and unreads treat it like other system lines. Tests for permissions, limits, the rate limit, deletes (deleting a server or channel removes its wheels/spins as the existing purge does), and that clients receive identical winner/angle.
- **Client:** the composer button and `/wheel` parsing (reuse the existing slash/composer handling), the popover, the wheel card (canvas or SVG; one shared `Wheel` component used by the card, the popover preview and Settings), the system line, the two Settings pages. Reuse existing popover, menu, list, Settings row and tooltip components.
- Look and feel matches the existing popovers, cards and Settings pages exactly; existing tokens (other than the generated slice colours); icon sizes 14 inline / 16 buttons / 18 nav; guard tests pass.

## Testing

- Server: CRUD and permissions for both scopes; spin returns a valid winner/angle within limits; rate limit; purge on delete; the event payload.
- Client: `/wheel a, b, c` spins; `/wheel` alone opens the popover; Server / Just me switch; add/paste/remove/shuffle/clear; Save as wheel; the card lands on the server's winner; collapse to the system line; Spin again and Remove winner & spin; reduced motion; Settings pages.
- **Manual (owner, Linux then Windows, two accounts):** both accounts watch the same spin land on the same slice; a late opener sees the result; save a quick wheel; manage wheels in Settings.

## Out of scope

- Weighted options, images on slices, custom colour palettes per wheel.
- Spinning from Settings.
- Notifications for spins.
