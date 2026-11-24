# Builds channel: design

Date: 2026-11-24. Owner decisions came from a design conversation plus two canvases of mockups (the Builds canvas and
its sticky notes). Where this doc says "the canvas" it means artboard **C · Gallery + list** (the chosen browse
view), **Quick add**, **Build page**, and **Quick access**.

## 1. Intent

A friend group keeps character builds and loadouts for many games: The First Descendant, Minecraft RPG modpacks,
Wynncraft, Deadlock, Borderlands, Warframe, Far Far West, gacha games and others. Today they keep them in thread
channels elsewhere, and three things go wrong:

1. **It gets messy.** Builds are hard to find and read, and get lost.
2. **People are lazy (by their own account).** They drop a YouTube link "to sort later", and later never comes.
3. **Games patch.** Old builds go stale. People post new ones beside them, and telling old from new means
   remembering dates.

A builds channel is a new channel kind (like quotes and clips) that stores each build as one tidy item, makes saving
one nearly effortless, and keeps old ones out of the way without deleting them.

### Goals
- Saving a build is as fast as pasting a link. Only a title is required, and a pasted link fills the title in.
- Browsing by game is the main way in. Search finds a build by title, notes, game or the video's title.
- Old builds are archived, never lost: hidden by default, one toggle shows them, and they can be restored.
- A build can stay on screen while you play: pinned beside any channel or voice view, or popped out.
- Friends can comment on a build. The author sees new comments and updates the build.

### Non-goals (this design)
- Others editing someone else's build. Only the author edits (owner decision). Comments carry suggestions.
- Structured build data (item databases, stat calculators, per-game schemas). Notes are free text.
- Version history of a build. An edit replaces the build. "Updated 16:52" is shown.
- A general forum/threads channel. The comment model is built so a future threads feature can reuse it (§6.3),
  but no generic threads UI ships here.
- Importing builds from other apps.

### Anti-goals (would make this a failure even if it works)
- A heavy form that people skip. Any required field beyond the title, or a save that needs more than paste + Enter.
- Archived builds mixed in with current ones by default, or archiving that deletes.
- A builds channel that feels like a chat timeline (scrolling to find things).

## 2. Decisions made with the owner

| Topic | Decision |
|---|---|
| Approach | A dedicated `builds` channel kind with its own storage, like quotes and clips (option 1 of 3; see §9) |
| Fields | Title (required). Game, build link, screenshots, notes (all optional) |
| Game | The server's existing games registry (the one clips use), picked from a dropdown, or a new name typed in |
| Browse view | Gallery of cards, with a Gallery/List toggle. List rows are taller and less cluttered than a chat list |
| Game filtering | A list of games with counts in a **right-hand, collapsible panel**. No filter pills |
| Right panel | The home for anything builds-related: games filter, Show archived, bulk archive, and later sort or tags |
| Game shown on a build | A small caps caption above the title (`THE FIRST DESCENDANT`), never a pill |
| Archive | Hidden by default. One "Show archived" checkbox shows them greyed in place with an ARCHIVED badge |
| Bulk archive | For one game or **All games**, everything saved before a date (pick a date, 30 days ago, 90 days ago) |
| Editing | Author only. Everyone else comments. The author sees new comments |
| Build page actions | One **Actions** menu: Pin to side, Pop out, Edit (author), Archive (author). Copy link waits for item links (§8) |
| Video | Clicking the build's video plays it **inside Pulse** in the media area |
| Quick add | Paste a link (or drop an image, or type a title) and Save. It's a "quick drop" until someone adds details |
| Paste images | Ctrl+V anywhere on the add/edit form adds an image to Screenshots, even while typing in Notes |
| Quick access | Pin a build to a side panel beside any view; pop it out into its own window; quick peek; Ctrl+K search |
| Sender, dates | Shown as for every message (who saved it, when, when last updated) |

## 3. The channel

### 3.1 Creating one
Server settings and the "+ channel" flow gain **Builds** beside Text, Voice, Quotes and Clips. Same permissions as
creating any channel.

### 3.2 Browse view (canvas artboard C)
- Header: the selected game's name (or "All games"), the count, a search box, the **Gallery / List** switch, **+ Add
  build**, and the panel toggle button (rightmost).
- **Gallery**: a grid of cards. Each card: first screenshot (else the video thumbnail, else a neutral tile), a "▶
  video" tag if it has a link, the game caption, the title, two lines of notes, and "who · when · N comments".
- **List**: one row per build. Thumbnail, game caption, title, "who · when · N comments", and a notes snippet.
- Sort: newest first, by when the build was saved. Updating a build does not reorder it.
- Quick drops (no details added yet) show a small "QUICK DROP" badge so they're easy to tidy.
- Archived builds (when shown) are greyed with an ARCHIVED badge.
- The Gallery/List choice and panel open/closed are remembered per channel on this device.

### 3.3 The right panel
Collapsible; open by default the first time. Sections, top to bottom:
1. **Games**: "All games" then every game that has at least one build in this channel, with counts (counts respect
   Show archived). Clicking one filters the view.
2. **Show archived** checkbox.
3. **Archive old builds**: game dropdown (defaults to the selected game; "All games" is an option), a "saved before"
   picker (date, 30 days ago, 90 days ago), and **Archive matching builds…**, which shows how many builds and which
   ones before confirming. Author-only rules apply (§5.1): the count and the action cover only builds the person is
   allowed to archive.

### 3.4 Search
- The browse view's search box searches this channel's builds: title, notes, game name and the link's fetched title.
- Ctrl+K (QuickSwitcher) gains builds: typing "raven" lists matching builds across the person's servers. A prefix
  `builds:` limits results to builds, and `builds: <game>` (a game name or a unique prefix, e.g. `builds: tfd` when it
  matches one game) filters by game. Archived builds appear below current ones, marked.
- In Ctrl+K: Enter opens the build page; Space opens a quick peek (§4.4).

## 4. A build

### 4.1 Build page (canvas "Build page")
- Header: caption `BUILDS · <GAME>` above the title, and an **Actions** menu.
- Meta line: "<author> · saved <time> · updated <time>".
- Media: the video (if any) and up to 4 screenshot tiles; "+N" when there are more. Clicking a screenshot opens the
  existing image viewer. Clicking the video plays it in place (§4.2).
- Notes: free text with the existing message markdown (bullets, bold, links).
- **Comments**: a list of comments with a composer at the bottom. Comments are ordinary messages (reactions, edits,
  deletes, mentions and links work as in chat). The author's comments are tagged "author".
- Opening a build from the browse view replaces the browse view (Back returns to it, keeping filter and scroll).

### 4.2 Video player
- YouTube links play in the media area using the same embed path LinkEmbed and watch party already use. Other links
  (a build-planner page, a non-YouTube video) show as a link card that opens in the browser.
- The player can be enlarged to fill the build page's media area; it stops when you leave the page.

### 4.3 Adding and editing (canvas "Quick add")
**Quick path:** paste into the builds channel (focus anywhere in the channel's browse view, or the "+ Add build"
field) and press Enter.
- A link: Pulse fetches the page's title and thumbnail with the existing unfurl code, uses the title as the build
  title, and guesses the game when the title contains a registered game's name (shown as "guessed"; one click
  clears it).
- An image: saved as a build titled "Untitled build" with that screenshot.
- Plain text: becomes the title.
- Saves at once as a **quick drop** (no notes, no explicit game). "Add details" opens the full form instead.

**Full form** (and Edit): Title (required), Game (combobox: pick from the registry, or "+ Use '<typed>' as a new
game", which adds it to the registry as clips do), Build link, Screenshots (drop, paste, or pick files; reorder;
remove), Notes.
- Ctrl+V anywhere on the form adds an image to Screenshots, including while focus is in Notes. Pasted text in a text
  field stays as text.
- A build stops being a quick drop once it has a game or notes (or the author marks it done; see §8).

### 4.4 Quick access (canvas "Quick access")
- **Pin to side**: the build appears in a right-hand side panel that stays while you move between channels, DMs and
  voice. One pinned build at a time (pinning another replaces it). Unpin with ×. The pin survives a restart.
- **Pop out**: the build opens in a small separate window (always on top by default, toggleable). It shows title,
  media and notes, scrollable; comments are not in the pop-out. *Builder's note (my call):* the existing
  `embed_window` can't do this (it only loads a provider's page and has no capabilities, by design), so the pop-out
  is a second Pulse window with its own label that loads the app in a read-only "build view", with the smallest
  capability set that works. That needs its own security review, so it ships as its own PR after pin-to-side.
- **Quick peek**: hovering a card/row for ~600 ms, or Space in Ctrl+K, shows a floating card with the title and the
  first lines of notes, plus Open and Pin to side.

## 5. Rules

### 5.1 Who can do what
- Anyone who can see the channel can view builds, comment, pin, pop out and search.
- Anyone who can send messages in the channel can add a build.
- Only the author can edit a build, change its game, or archive/restore it. The **server owner** can also archive,
  restore and delete any build (my call: Pulse has no admin role today, and owner-only actions already use the owner
  check).
- Deleting a build: the author or the server owner. It asks to confirm, and removes the build, its screenshots and
  its comments.
- Comments follow chat rules exactly: only a comment's author edits or deletes it (no one moderates others'
  comments in phase 1, as in chat today).

### 5.2 Archive
- Archiving sets `archived_at` and who archived it. Restoring clears both.
- Archived builds keep their comments and stay searchable and openable. Commenting on an archived build is allowed.
- Bulk archive (§3.3) runs on the server in one request and reports how many it archived. The person sees the list
  before confirming.

### 5.3 Notifications (my call: reuse how quote and clip threads already work)
- Comments are thread replies on the build, so they feed the inbox's existing **Threads** list exactly as quote and
  clip threads do: the build's author, and anyone who has commented, sees new comments there. The author always
  sees them, which is what the owner asked for.
- No extra "comment = mention" path. An @mention inside a comment notifies as anywhere else.
- The builds channel itself shows unread when a new build is added, like any channel; comments don't mark the
  channel unread for others (same as thread replies today).

## 6. Storage and API (for the builder)

### 6.1 Tables (new migration)
- `channels.kind` gains `builds` (the CHECK rebuild exactly as 0008/0009 did).
- **A build is a message**, exactly as a quote or clip is (decided after the builder check): a `messages` row in the
  builds channel (author, created time, unread, search triggers and attachments all come with it), plus a `builds`
  row keyed by `message_id`.
- `builds`: `message_id`, `title`, `game_id` (nullable, references the existing per-server `games` registry),
  `link_url` (nullable), `link_title`, `link_thumb` (fetched), `notes` (nullable), `quick_drop` (bool),
  `updated_at`, `archived_at` (nullable), `archived_by` (nullable).
- Screenshots are the message's attachments (the existing pipeline, sweeper and limits), with an order.
- Comments are thread replies whose root is the build's message (`messages.thread_root_id`, `messages/threads.rs`),
  so send, edit, delete, reactions, mentions and search all come from the existing pipeline. Comment counts come
  from the thread.
- Indexes for: channel + archived + created (browse), channel + game, and search.

### 6.2 API and gateway events
- REST: list builds for a channel (filters: game, include archived, search text, cursor), get one, create, update,
  archive/restore (one), bulk archive (game or all, before a time), delete.
- Gateway: build created / updated / archived / deleted events to channel members, and the usual message events
  for comments.
- The server enforces the rules in §5.1 and the input limits: title ≤ 120 characters, notes ≤ the message length
  limit, at most 10 screenshots, link ≤ 2048 characters.

### 6.3 Reuse
- Game registry: the clips one, unchanged (games are never deleted).
- Link title and thumbnail: the existing embeds/unfurl code.
- YouTube playback: the existing LinkEmbed/watch party embed path.
- Pop-out window: a new labelled app window (see §4.4); not embed_window.
- Ctrl+K: QuickSwitcher gains a builds source.
- Images: the attachments pipeline and image viewer.

## 7. Tests
- Server: create (title only, link only, image only), author-only edit and archive, owner archive and delete,
  bulk archive by game and by all games with a cutoff (and that it only touches allowed builds), search across
  title/notes/game/link title, permissions for a person without access, and migration.
- Client: quick add from a pasted link (title fill, game guess, quick drop badge), the full form including paste of
  an image while typing notes, Gallery/List toggle and remembering it, the right panel's filtering and Show archived
  counts, the bulk archive preview, the Actions menu (author vs others), the in-place video, pin to side surviving
  a channel change and restart, pop-out, quick peek, Ctrl+K `builds:` and `builds: <game>`.
- Comments: a comment shows in the author's and earlier commenters' inbox Threads list, as quote and clip threads
  do; the person commenting isn't notified of their own comment.

## 8. Left to the builder
Minor calls a sensible builder can make without changing what gets built:
- Exact card and row sizes, spacing and empty states (follow the canvas and the app's existing tokens).
- The quick peek delay and position.
- Whether an updated build shows a subtle "updated" marker in the browse view (it must not reorder the list).
- How a quick drop is "marked done" if it has neither a game nor notes (a "Mark as sorted" action is fine).
- Pagination size for the browse list.
- The pop-out window's default size.

Deferred (open an issue for each when this merges): **Copy link** (Pulse has no item links or deep-link scheme yet,
so it's left out of the Actions menu in phase 1), moderating other people's comments, an admin role, sorting
options in the right panel, tags beyond the game, version history, per-game structured fields.

## 9. Approaches considered
1. **A dedicated builds channel kind (chosen).** Own storage, comments under each build. Matches how quotes and clips
   work, and every part of the design maps onto it directly.
2. **A general forum channel with builds as a template.** More flexible and gives threads elsewhere, but slower and
   more abstract, and builds-specific parts (game list, video player, archive by game) fit awkwardly.
3. **Builds as chat messages with extra fields.** Fastest to start, but archiving, filtering and comments fight the
   chat timeline, which brings back "things get lost".
