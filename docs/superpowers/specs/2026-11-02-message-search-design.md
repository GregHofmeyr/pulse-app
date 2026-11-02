# Message search

## Why

Old messages are hard to find again. The owner (2026-10-30 and 2026-11-02): he looks for a message "generally
[by] a keyword in it, or who said it. Those are the 2 big ones." He wants filters that combine ("filter for a
person and for contains media, for example, when we allow sending media") in "a nice little search box, use a
keyword or something like that". He confirmed this shape and said search covers "only the server you're currently
in".

## Goals

- **One search box.** Plain words find messages containing them. Small filter words narrow the results, and they
  combine (all must match).
- **Scope:**
  - In a server: every channel of that server you can read.
  - In a DM or group: that conversation only.
  - Never across servers.
- **Results:**
  - shown newest first in a side panel;
  - the matching words highlighted;
  - each result shows who wrote it, where, and when;
  - a click jumps to the message in its channel, highlighted, with the history around it loaded.
- **Fast enough to feel instant** on a friend-group server, i.e. tens of thousands of messages.

## Non-goals

- Searching across servers, or across all DMs at once.
- Searching inside clip videos, images or link previews. Captions and message text only.
- Saved searches, search history synced between devices, and regex.

## Anti-goals

- Search must never show a message the searcher can't read: another server, a DM they're not in, a deleted
  message, or a channel they've lost access to.
- No new service. It stays inside SQLite.
- Search must never slow down sending messages noticeably.

## The search box and filters

- **Where:** a search field in the channel header, on the right next to the existing header buttons. Ctrl+F (or
  Ctrl+K, builder's call; choose what doesn't clash) focuses it.
  - In a server the placeholder reads "Search Main".
  - In a DM it reads "Search this conversation".
- **Plain words:** all of them must appear (AND). Matching is word-based and forgiving: case-insensitive,
  accent-insensitive, and a word matches as a prefix ("clip" finds "clips"). A phrase in quotes must appear as is.
- **Filters** (typed in the same box; my defaults):
  - `from:sam`: written by sam. In a quotes channel it also matches quotes where sam is a speaker (`quote_lines.speaker_id`, or a speaker name linked to sam through `speaker_links` at search time), because there "who said it" means the speaker. Autocomplete from the server's members, or the conversation's people. **(settled after builder check)**
  - `in:#general`: one channel. Server only. Autocomplete from its channels.
  - `has:clip`, `has:quote`, `has:link`: messages carrying that. `has:media` is added when general media sending
    exists; `has:clip` covers it for now. (`has:emoji` dropped: not asked for, and standard-emoji detection would
    need a Rust-side flag. **(settled after builder check)**)
  - `mentions:sam`: messages that pinged sam, i.e. the `mentions` table, including reply pings. An `@sam` added
    later by an edit isn't counted, matching notifications. **(settled after builder check)**
  - `before:2026-10-01` / `after:…`: optional. The builder may leave these out if they make the box fussy.
- **The filter picker (owner, 2026-11-02: "like Discord").** When the box is focused and empty, or at the start of
  a new term, a dropdown lists the filters as clickable rows. Each row shows the syntax and a short line, e.g.
  "from: user / From a specific user". Clicking a row (or Enter/Tab, with arrow keys moving the highlight) puts
  that filter in the box. While its value is being typed, the dropdown suggests matches:
  - members, with avatar and name style, for `from:` and `mentions:`. Match on any part of the name, prefix
    matches first. This is "especially helpful for those names you can't remember".
  - channels for `in:`;
  - the fixed list for `has:`.
  Picking a suggestion completes the chip. Fully keyboard-driven, with combobox/listbox semantics.
- Filters show as chips once completed, removable with ✕ or backspace.
- A search with only filters and no words works ("everything from sam").
- Typing a filter word shows a short hint list of the filters (a small popover), so they're discoverable without
  documentation.

## Results panel

- **Layout:**
  - It opens on the right, where the member list or replies panel sits, and replaces it while open.
  - A header shows the result count, "50+" past a page.
  - Each result is a compact message card: avatar, name with the user's name style, channel name (server search),
    relative time, and the text with matches highlighted. A clip shows its thumbnail; a quote shows its first line.
  - Replies inside a quote's thread are included, labelled "reply in quote".
- **Paging:** newest first, 25 at a time, with "Load more" or infinite scroll.
- **Empty and failure states:** "No messages match" plus the active filters; and an error state with retry.
- **Click:** jumps through the existing `jumpTo`. A thread reply opens its quote's replies panel, as the
  notification click does. The panel stays open so you can try the next result. Esc closes it.
- **Live updates:** results are a snapshot at search time; there is no live updating. An edited or deleted message
  is simply missing or changed on the next search.

## How it gets made (builder's call where not stated)

- **Index:** an SQLite FTS5 table over message text, kept in sync by triggers on insert, update and delete.
  - It is an external-content table over `messages` (or an equivalent).
  - Use the `unicode61 remove_diacritics 2` tokenizer, with prefix indexes for 2 and 3 characters.
  - Migration 0011 creates it and fills it from existing messages. It must stay fast on the live database
    (thousands of messages; a one-off build).
  - Verify the bundled SQLite in sqlx has FTS5. If it doesn't, decide and record the alternative.
- **What goes in the index:** (FTS5 is confirmed: the bundled libsqlite3-sys builds with SQLITE_ENABLE_FTS5.)
  - The message text with custom-emoji tokens reduced to their names and mentions as "@name".
  - The clip caption (its message content), and quote lines, including image-line captions from
    `quote_lines.caption`.
  - Thread replies (quote and clip threads), labelled "reply in quote" / "reply in clip".
  - Not deleted messages: deleting removes the row from the index.
  - System lines are excluded.
- **API:** `GET /servers/{id}/search` and `GET /channels/{id}/search` (for a DM or group).
  - Parameters: `q` (the words), `from`, `in`, `has[]`, `mentions`, `before`, `after`, `cursor`.
  - The filter syntax is parsed in the client; the server gets structured parameters. Builder's call: parse in one
    place, share tests.
  - The server checks membership exactly like message listing does.
  - Results come back newest first, with highlight ranges (from FTS5 `highlight`/`offsets`, or computed), not HTML.
  - Rate-limited like other reads.
- **Permissions:** search follows read access exactly. Today any signed-in user may read any server's channels
  (`access::channel_for`), and there is no "leave server". So server search requires nothing more than listing
  does: it never shows anything the user couldn't already scroll to. If server read access tightens later,
  search must use the same check, so share it. **(settled after builder check)** Add a test for each anti-goal:
  - a deleted message;
  - a DM or group you're not in;
  - a group you've left;
  - a message from another server never appears in this server's results.
- **Client:**
  - a search store;
  - the box with its chips and autocomplete;
  - the results panel, built in the UI polish pass's shared styles (`.input`, menu surface, buttons, `pop-in`);
  - a highlight renderer that never injects HTML.
- **Performance:** a test or bench with about 50k synthetic messages. A typical query should take under 50 ms on
  the server.

## Testing

- **Server:**
  - the index stays correct through insert, edit, delete and quote replies;
  - each filter, and combined filters;
  - prefix and accent matching;
  - every permission anti-goal;
  - paging is stable;
  - the migration fills the index from existing data.
- **Client:**
  - filter parsing (words, quotes, filters, unknown filters treated as words);
  - chips;
  - the panel's states;
  - a click jumps;
  - highlighting is safe with HTML-like text.
- **Manual (owner):**
  - search a word in a busy server;
  - `from:` plus a word;
  - `has:clip`;
  - click a result far back in history;
  - search in a DM;
  - a deleted message never shows.
