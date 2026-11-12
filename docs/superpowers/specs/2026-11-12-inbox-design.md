# Inbox

Issue #306. Decided with the owner on 2026-11-12. **(owner)** marks the owner's decisions; the rest is the builder's call.

## What it does

- **One place for things meant for you**, across every server, DM and group: @mentions (you, @everyone/@here if the app has them, and role mentions when roles exist), **replies to your messages (owner)**, and any other per-you alerts the app already produces (e.g. thread replies you're part of). Plain unread messages in channels are not inbox items.
- **Tabs or a filter (owner):** **All** (mentions + replies) and **Mentions** (only @mentions). Builder picks tabs vs a toggle to match existing patterns; remember the choice per device.
- **Opening it:** an inbox button in the top bar / corner (next to other global buttons; builder matches existing placement), with an unread count badge; also reachable from the quick switcher (`>inbox` action and Ctrl+K "Inbox").
- **Each item:** author avatar/name, where (server › channel, DM or group), time, a one-line preview with the mention highlighted, and for replies the snippet it replied to. Clicking jumps to the message (reusing the app's jump-and-highlight) and marks the item read.
- **Read state:** an item is read when you jump to it, when you read that message in its channel (existing read points), or via "Mark all read" in the inbox. Unread items are visually distinct; the badge counts unread items.
- **History:** the inbox keeps items for 30 days (builder's call on paging; newest first, load more on scroll).
- **Respects notification settings:** muted channels/servers don't add items unless the item is an explicit @mention of you (match how the app's mention badges already treat mutes).
- **Live:** new items arrive in real time; multiple devices stay in sync (read on one, read on all).
- **Older clients (0.2.x)** are unaffected.

## How it gets made

- Server: derive inbox items from existing data where possible (messages with mentions of the user, replies whose parent is the user's) with an index-friendly query, plus a per-user inbox read marker or per-item read rows (builder's call; explain). Routes to list (paged), mark one / all read; events for new items and read changes to the user's devices. Next free migration number; foreign keys OFF at migration time.
- Client: the button + badge, the popover/panel (reuse existing popover/list styles), tabs/filter, jump, live updates, quick switcher entries.
- Look and feel matches existing popovers and lists exactly; existing tokens; icon sizes 14/16/18; reduced motion respected.

## Out of scope

- Custom unread filters by person (#307), reminders (#300).
