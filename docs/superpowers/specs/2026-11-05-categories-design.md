# Channel categories and drag to reorder

Issue #54 (categories), plus the reorder part of #55. Decided with the owner on 2026-11-05. The owner's own words and
decisions are marked **(owner)**; everything else is the builder's call unless it says otherwise.

## Why

- A server's sidebar today has four fixed sections (Text channels, Quotes, Clips, Voice channels), ordered by kind and
  then by creation. Friends can't group "the Gaming stuff" together or put the channel they use most at the top.
- Discord's model is what everyone already knows: named categories that hold any kind of channel, and you drag things
  where you want them.

## What it does (owner)

- **A category holds channels of every kind, mixed**, Discord-style: a "Gaming" category can hold #lfg, a quotes
  channel and the Gaming voice channel **(owner)**. Categories replace the four fixed sections.
- **Uncategorised channels** sit at the top of the list, above the first category, with no heading.
- **Make, rename and delete categories.** "Create category" lives in the server's menus (the server tab's right-click
  menu and the sidebar header's menu, alongside Server settings). A category heading's right-click menu has Rename
  category, Delete category and Create channel. Rename happens inline in the heading, as channel rename does today.
- **Deleting a category never deletes its channels:** they move to the uncategorised area at the top, keeping their
  order. No confirm dialog needed, since nothing is lost (my call); the category is simply gone.
- **Collapse:** clicking a category's name collapses or expands it, as the section headings do today. Collapsed is your
  own choice, on your device, not everyone's (as `sidebarSections.svelte.ts` does today, keyed by category id). A
  collapsed category still shows exactly what a collapsed section shows today, applied to every row whatever its
  kind (my call): the channel you're viewing, any unread unmuted channel (`ChannelList.svelte` `keep()`), and the voice
  channel you're in, with its occupants under it. Voice channels other people are in stay hidden, as today.
- **Collapsed state carries over** (my call): on first run, a device's old `serverId:kind` collapsed sections map onto
  that server's default category for the same kind (found by the category the migration made for it, e.g. by
  remembering which kind each default category was created for), so nobody's collapsed Voice Channels springs open
  after the update. A category renamed since still maps, because the match isn't by name.
- **Drag to reorder:** drag a channel within its category, into another category, or into the uncategorised area; drag
  a category to move it with all its channels. The order is the server's, the same for everyone, and changes appear for
  everyone straight away.
- **Who may:** anyone in the server, like managing channels today (v1 trust model, `2026-10-01-pulse-design.md:88`).
  People who can see a server without being a member see its categories but can't change them or drag.
- **Existing servers on day one (owner):** nothing looks moved. The migration gives each server a category per
  section it already shows, named as today's headings and in today's order: "Text Channels", "Quotes", "Clips",
  "Voice Channels", each holding its channels in their current order. (The owner agreed to "Text Channels" and "Voice
  Channels"; carrying Quotes and Clips over the same way is my call, since those are sections today too.) A server
  with no channel of some kind gets no category for it.
- **New servers** start the same way: the channels a new server is created with go into the matching default
  categories.

## Creating channels

- Today each section heading has a + button for its own kind. Each category heading now gets a + that creates a channel
  inside that category; the inline field gains a small kind picker (Text, Voice, Quotes, Clips), defaulting to Text
  (builder lays it out; it must match the existing inline field's look). The category's right-click "Create channel"
  opens the same field.
- A channel created from the server menus or anywhere without a category lands at the bottom of the uncategorised area.

## Feel (the owner's top priority today is look-and-feel consistency)

- Headings look exactly like today's section headings (small caps style, `--text-3`, the collapse chevron), the
  category name shown in upper case by CSS while stored as typed.
- While dragging: the row lifts (a subtle shadow, its usual hover background), a thin accent line shows where it will
  drop, the rest of the list moves aside smoothly, and dropping settles it in place without a jump or flash. Dragging
  over a collapsed category for a moment opens it (my call). Escape cancels a drag.
- Drag only starts after a small movement (about 4px), so a click still just opens the channel, and right-click still
  opens its menu.
- Motion follows the existing motion tokens and `motionCss.test.ts` rules, and respects reduced motion.
- **Keyboard:** a focused channel or category moves with Alt+Up / Alt+Down (into the neighbouring category at the
  edges), and the right-click menus of channels and categories gain "Move up" / "Move down" (my call: the keyboard and
  screen-reader path for what the mouse does by dragging).

## How it gets made (builder's call where not stated)

- **Storage** (migration `0023_categories.sql`, the next free number): a `categories` table (id, server_id, name,
  position, created_at, plus the kind it was made for when the migration or a new server created it as a default, for
  the collapsed-state carry-over), a nullable `channels.category_id`, and the per-server layout revision. The migration creates the default categories for every
  existing server and fills `category_id` and `position` from today's order. Positions become one order per
  category (and one for the uncategorised area) instead of per kind. Rehearse on a prod copy as usual. Rollback =
  restore the pre-deploy backup.
- **Routes:** create, rename and delete a category; and one **layout** route that takes the server's whole order
  (categories in order, each with its channels in order, plus the uncategorised channels) and saves it in one
  transaction. Each server has a **layout revision** (stored, bumped by every layout, category or channel create /
  delete / restore change, sent in Ready and in every layout event). The layout route takes the revision the client
  last saw; if it's not current, or the layout doesn't name exactly the server's live channels and categories, it
  refuses with a conflict and the client snaps back to the server's current order (my call: last-writer-wins on a
  stale layout would silently undo a friend's change, and a set check alone can't see a friend's reorder).
  Rate-limit generously, like renames.
- **Events:** categories ride along in Ready with the server's channels; a `LayoutChanged` (or category
  created/updated/deleted plus channel position) event goes to everyone who can see that server, using the same
  audience as channel events. Protocol additions are `serde(default)`, so an old client keeps working, showing its
  four fixed sections (an old client sorts by kind and position; that's fine for the hand-over window).
- **Limits:** name 1–64 characters (`clean_name`, as channels); a cap on categories per server (say 50).
- **Deleted channels (#112):** a soft-deleted channel keeps its category. Restored into a category that's since been
  deleted, it lands at the bottom of the uncategorised area.
- **Client:** `channelsFor` returns the category layout instead of four lists; `ChannelList.svelte` renders
  uncategorised channels then each category. The optimistic drop shows the new order immediately and reverts on a
  refused save. Drag is hand-built with pointer events (no new dependency) unless the builder finds a small, solid
  library that fits Svelte 5; either way it must feel smooth under load (#103 showed popovers struggling as load rises,
  so keep the work per frame small: transforms, no layout thrash).
- DMs and groups are untouched.

## Testing

- **Server:** migration on a prod copy keeps every channel, puts each in the right default category in today's order,
  passes integrity and foreign-key checks; category create, rename, delete (channels move up, order kept); the layout
  route accepts a valid order, refuses a stale or incomplete one, refuses non-members; events reach only those who can
  see the server; restore into a deleted category; caps and rate limit.
- **Client:** renders uncategorised then categories; collapse per category persists and keeps the active and occupied
  channels showing; drag within, between and out of categories, and of categories (pointer events in tests); the
  optimistic order and the revert on conflict; Alt+arrow and Move up/down; create a channel inside a category with
  each kind; non-members can't drag.
- **Manual (owner, two PCs):** drag a channel into a new category on one PC and watch it move on the other; delete the
  category and see the channels come up top; collapse a category and check it stays collapsed after a restart; an old
  client still shows everything.

## Out of scope (deferred → issues)

- Permissions per category or channel (private categories): Pulse has no roles yet.
- Deleting a server (#184).
- Server icons (rest of #55).
- Reordering servers in the server rail, and DMs.
