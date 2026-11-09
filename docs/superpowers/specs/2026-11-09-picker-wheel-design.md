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

## Follow-ups after 0.2.2 (#270)

Asked for by the owner after 0.2.2; the details are the builder's calls.

- **Just me spins are private (owner).** Spinning a Just me wheel never reaches the server: the popover draws on this device (the server's draw, ported) and spins its own wheel, shows the winner under the name, and stays open. Spin again and Remove winner & spin run there too; a removed winner is off this run of spins only (the saved wheel keeps it), and picking, switching or editing starts afresh. Nothing is posted, so it's on no other device and in no history. A quick wheel (typed, or `/wheel a, b`) stays public, also in DMs; Save as wheel into Just me makes it a Just me wheel, so its spins are private from then on.
- **Spin quietly (owner).** A checkbox under Spin (not shown for Just me wheels), remembered per device. A quiet spin goes to `POST /channels/{id}/wheel-spins/quiet`: drawn and checked as a spin (same rate limit, same chats), but nothing is stored (no line, no `wheel_spins` row); the server sends `WheelSpunQuietly` to whoever is in the chat now. Clients show it as a card at the bottom of the chat that spins live, shows the result for 6 s after landing (as long as it's pointed at or focused), then fades. Someone opening the chat meanwhile sees it landed; later, nothing. Spin again and Remove winner & spin work on the card while it's shown and are quiet too (a saved wheel's spin keeps its name: the request may name the wheel *and* a subset of its options). Public lines' Spin again stays public. `/wheel a, b, c` follows the checkbox as last left (one remembered choice, so the slash command and the popover agree; the card says "Quiet spin · not saved"). Clients from 0.2.2 and before skip the event (they don't see quiet spins). No migration.
- **+ New wheel** is a + icon button (14 px, the extra-small icon button, as the channel list's section headers) at the end of the Saved wheels header, tooltip "New wheel".
- **Breathing room:** the popover's side is three bands (the Server / Just me switch, Saved wheels, the wheel's options) split by the line token, like the poll composer's bands and Settings' sections; the stage's items sit a little further apart. Same 600 × 480 size, never taller than the window.
- **Your spin comes into view (owner):** a spin you start (popover, `/wheel`, Spin again, Remove winner & spin; public or quiet) takes the chat to the latest as sending a message does, back to the present from a jumped-to window. Other people's spins only follow if they're at the bottom already. Private spins stay in the popover.

## Result view, chime, the card stays until seen, case-sensitive options (#271)

Asked for by the owner after #270 (from a tester); the details are the builder's calls.

- **Case-sensitive options (owner).** `UPPER` and `upper` are two options; only exact duplicates merge (the editor, pasting, `/wheel a, b`, which now merges before counting, so `/wheel a, a` opens the popover). The server never merged options and matches a quiet spin's subset exactly, so it is unchanged (a test guards it); 0.2.2 clients just keep merging on their side.
- **Result view (owner).** When a spin lands the wheel stays, under a dark, centre-weighted radial fade, with the winner written large in white across the hub (it scales with the wheel and shrinks for long names, wrapping to three lines at most), with a soft shadow; the losing slices stay dimmed, so the winner's slice reads brighter. The fade is the video chrome's dark glass (`--video-chrome`, `--video-chrome-strong`, `--on-video`): it reads over the neon in every theme. It fades in with the app's `fade-in` (none under reduced motion, live or unfocused). Used by the public spin card, the quiet card and the popover's private spin (whose winner moves from under the name into the view; Remove winner & spin stays there). The hub gives way to it.
- **Landing chime (owner).** `wheel_result` in every sound set, from `scripts/gen_sounds.py`: Classic is three bells up a major chord (C6 E6 G6) ringing into each other, the Two-step sets strike their own timbre up the chord, Minimal plays its note a fifth then an octave up, Goofy gets a synthesised square-wave "ta-da" (not from the Kenney packs). 3 dB under join and leave. A volume mixer notch, Notifications › "Picker wheel result" (on at 100 %, also for mixers saved before it; 0 % is off). It plays when a public spin lands in a chat you have open (Pulse focused or not; a hidden voice chat panel doesn't count), and for your own private spin. Not for a spin opened later, not for chats you're not in, never for others' private spins. Quiet spins don't chime: they are left as #270 made them until the owner's redesign.
- **The card stays until seen (owner).** A public spin's card (wheel and result view) stays in the chat until you've seen it landed: Pulse focused, that chat on screen, the card in view. Then 20 s more, then it folds into the 🎡 line. A ✕ (Dismiss) folds it at once; it folds anyway an hour after the spin. Per device: seen and dismissed are kept in localStorage by message id (`pulse.wheelCards`, spins older than the hour pruned), so a reload doesn't bring a card back. Opening the chat within the hour shows the card with the result view, never a replay. Spin again, Remove winner & spin and Save as wheel are on the card and the line. The 🎡 line itself is unchanged (a new look is coming).
- **Quiet spins:** only the result view; their timing and everything else stays as #270 left it (the owner is redesigning them).

## Out of scope

- Weighted options, images on slices, custom colour palettes per wheel.
- Spinning from Settings.
- Notifications for spins.
