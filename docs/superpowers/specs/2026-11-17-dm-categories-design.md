# DM categories and pinned conversations

Issue #305 (owner's feature list, 2026-11-11). Built 2026-11-17; the builder's calls are below.

## What it does

- **Pin to top:** right-click a DM or group, **Pin to top** (or **Unpin**). Pinned conversations sit under a **Pinned** heading at the top of the list (your Notes stay above everything and can't be pinned or moved).
- **Your own categories:** right-click a conversation, **Move to category** → one of yours (ticked if it's there), **New category…** (the menu turns into a name field: **Create and move**), or **Remove from category**. Each category is a heading in your list, like the channel list's: click it to fold it shut or open (folded, it shows the count of what's new in it and only keeps the conversation you're in); right-click it (or Shift+F10 / the menu key) for **Rename category…** (in the heading), **Move up** / **Move down** (#419, only where it can go) and **Delete category** (no dialog: its conversations go back to the main list, pinned ones stay pinned).
- **Order:** Pinned, then your categories in your order (made last; moved with Move up / Move down, #419), then the rest under **Conversations** (that heading only shows when there's something above it). Inside each, newest first, as the list always was. A pinned conversation in a category shows under Pinned.
- **Names:** 1-32 characters on one line; two of your categories can't share a name (ignoring case), and none can be named like the Move flyout's own entries, **New category…** or **Remove from category** (ignoring case, `...` for `…`; #419). Up to 30 categories.
- **Yours, everywhere:** kept by the server per person, sent only to you (Ready's `dm_layout`, `DmLayoutChanged`), so every device shows the same list and folds. Nobody else ever sees your pins or categories.

## How it's made

- Migration 0055: `dm_categories` (id, owner, name, position, collapsed) and `dm_placements` (owner, conversation, pinned, category). A placement with neither pin nor category is deleted. A conversation you left drops out of your layout (filtered on read; its row goes with the chat when that's purged).
- Routes: `POST /me/dm-categories`, `PATCH`/`DELETE /me/dm-categories/{id}`, `PUT /me/dm-placements/{channel_id}`. Each answers with your whole layout and sends it to your sessions; the client replaces its copy (folding shows at once and is put back on a refusal).

## Not done

- Dragging conversations between categories, and dragging the categories themselves (the conversation list has no drag code; the channel list's is built on its own DOM). Categories move with Move up / Move down instead (#419).
