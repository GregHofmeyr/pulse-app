# Quotes, quote awards and Quote of the Year: design

Date: 2026-10-22. Status: for the owner's review (builder check done, answers folded in).

Depends on: **reactions and emoji** (its own push, designed and built first). **The implementation plan for quotes is
written after reactions has merged**, against its real shape (reaction storage, events, counts on `Message`).
Followed by: **badges** (its own push).

## 1. Intent

The group already keeps a Discord text channel of ridiculous things its members said. It is plain free text, for
example:

```
"this is a quote" - greg 2026
```

Some quotes come in layers: someone says something, someone else builds on it, and both lines go in one message with
their own speakers.

Pulse turns this into a first-class feature:

- a **quotes channel**: posting a quote is as fast as typing that line, but the speaker is a real member, so quotes
  are tracked per person;
- **reactions** as the quote's score;
- **quote awards**: small, everyday pats on the back that add up per person (later: a levelling badge);
- **Quote of the Year**: a yearly event with live voting and a winner.

The group is very competitive. Anything that can be shown off, counted or won is the point. It should feel playful
and a little game-like.

### Goals

- Adding a quote takes about as long as typing the free-text line did.
- Every quote line has a speaker that links to a member, so counts and awards follow people.
- Quote of the Year is an event: visible to everyone in the server, with live vote counts and a clear winner.

### Non-goals (this push)

- **Badges.** Quote awards and Quote of the Year wins are recorded now; the badges push turns them into badges,
  backdated (§7).
- **Importing the Discord history.** Later, with a script like the owner's Discord-to-Root work. This design keeps the
  door open (§3.4).
- A yearly awards show (most time in voice, most streamed, …). Idea parked by the owner; it needs extra tracking.
- Quote threads, quote search beyond the tabs in §4.3, exporting.

### Anti-goals

- **Slower than free text.** If posting a quote feels like filling in a form, people will stop.
- **Bloat.** The owner will cut anything that drifts into feature bloat. Prefer reusing messages, reactions and the
  gateway over parallel systems.
- **Stale numbers.** Vote counts and award counts must update live for everyone, like messages do.

## 2. Decisions made with the owner

| Topic | Decision |
|---|---|
| Layout | **Option B, chat-style:** the poster's avatar and name like a normal message, the quote in a block underneath, each line with its speaker on the right. Looks and feels like a text channel. |
| Composer | A bar at the bottom: quote text, "who said it?" picker, "+ line" to add a layer. |
| Year | Filled in automatically. |
| Poster | Shown (the message header), so people can jokingly complain at whoever quoted them. |
| Reactions | **Open:** any emoji, built-in or custom. A row under each quote with faded suggested reactions and a "+". |
| Score | **All reactions on a quote together**, not one specific emoji. |
| Quote awards | Per **line**. "Give quote award" in the line's menu. Goes to that line's **speaker**. Anyone, any time, no vote. One per person per line, can be taken back. You can't give one to your own line. Each line shows a 🏆 count; hover shows who gave them. |
| Award progression | Awards add up per person into a **levelling badge** (first at about 10–20, then every about 10). Numbers are tuned in the badges push. |
| Quote of the Year | An **event**. Top 5 quotes of the year by reactions, plus quotes anyone adds by hand. One vote each, changeable, **self votes allowed**. **Live vote counts.** |
| QotY length | Chosen at start: a few hours, a day, or open until ended. |
| Ending | **Anyone can end it early**, behind an "are you sure?" step (the starter may be away). Otherwise it ends when time runs out. |
| QotY look | **Green, retro pixel spill:** solid green breaking into chunky pixels that spill into the black. Filled green buttons. Same on the banner, the voting window, the voice strip and the winner card. Vote animations, chosen at build time. |
| History import | Wanted, **later**. Not in this push. |

The approved mockups are in `.superpowers/brainstorm/233656-1792662333/content/` (git-ignored, on the owner's
machine): `quotes-layout.html` (option B) and `qoty-event-v5.html` (the event). The palette is the app's
(`client/ui/src/app.css`) plus green `#39d98a`, a deep green `#14713f` for button edges and pixel shades, and
`#0b1f14` for text on green.

## 3. Quotes

### 3.1 The quotes channel

- A new channel kind, **`quotes`**, for server channels only. Created like any other channel (anyone can, flat
  permissions as today).
- Unread dots, mentions-free notifications, read points and the message list behave as in a text channel.
- **My call:** the channel holds quotes only. The composer always makes a quote. Banter about a quote goes in a normal
  channel, or in reactions. (If the group misses chatting there, a later change can allow replies to a quote.)
- **My call:** a server can have any number of quotes channels. Quote of the Year covers all of them in that server.

### 3.2 A quote

A quote is a **message** in a quotes channel (so reactions, editing, deleting, unread and the gateway all reuse the
message machinery). Its lines are stored alongside it:

- one or more **lines**, in order; each has text and a **speaker**;
- the **poster** is the message author;
- the **year** is the year it was said: the posting date, unless a "said on" date is set (import, §3.4).

**Speakers:** a line's speaker is a server member, or, for people not on Pulse yet, a plain name (§3.4).

**Lines have stable ids** (not positions), so editing never moves awards between lines.

**Editing (agreed with the owner, the group is competitive, so these close exploits):**

- The poster can edit their quote and delete it, like messages today.
- **Changing a line's text** keeps its awards.
- **Changing a line's speaker** removes that line's awards. The edit shows a warning first ("This line has 3 awards.
  Changing who said it removes them.").
- Lines can be added and removed; a removed line takes its awards with it.
- **While a quote is an option in a live Quote of the Year,** its speakers can't be changed (text fixes are fine).
- **Wins are stored with the speakers as they were when the event ended**, so a later edit never moves a win.

**Deleting** a quote removes its awards and reactions, and takes it out of any live Quote of the Year (votes for it
are dropped and those people can vote again). A quote that already won can still be deleted; the stored win stays.

Limits: **my call:** up to 10 lines per quote; each line up to the normal message length limit.

### 3.3 The composer

- Type the quote, pick the speaker from the server's members (type-ahead, like @mentions), press Enter.
- "+ line" adds another line with its own speaker before posting.
- Quotation marks are added by the display. If someone types them, they're stripped.
- No speaker picked: the composer won't post, and it says "Who said it?".

### 3.4 Speakers not on Pulse yet (and the later import)

- **My call:** the speaker picker also accepts a plain name ("someone not here yet"). The line shows that name.
- Anyone can later **link** a plain-name speaker to a member, which updates every line with that name in the server.
  From then on, those lines count for that member (awards included).
- **My call:** names match ignoring case and surrounding/repeated spaces. Each link is stored as a record (name →
  member, who linked it, when) and the original name stays on the line, so a wrong link can be **undone** or
  re-pointed by anyone. Undoing returns the lines to the plain name.
- A quote can carry a **"said on" date** for backdating. The composer doesn't offer it; it exists for the import.

### 3.5 Display (option B)

- Message header: poster avatar, name, date.
- Quote block: each line as `“text”` with the speaker name on the right; a line's 🏆 count sits beside its speaker
  (hidden at 0).
- Under the block: the reaction row (existing reactions, faded suggestions, "+").
- Line menu (hover or right-click a line): "Give quote award" / "Take back award", "Copy".
- Message menu: edit, delete (poster only), as today.

## 4. Scores and awards

### 4.1 Reaction score

A quote's score is its total number of reactions: every (person, emoji) pair counts once. **My call, flagged:** one
person adding five different emoji counts five. This follows the owner's "count reactions as a whole"; if it gets
gamed, switch to counting distinct people.

### 4.2 Quote awards

- One row per (line, giver). Can't give to your own line (speaker is you). Can't give to a plain-name speaker's line?
  **My call: you can.** It's kept on the line and counts for the person once the speaker is linked.
- Giving or taking back an award updates everyone's view live.
- **Per-person total:** the number of awards on lines whose speaker is that person. Shown on the profile as a stat
  ("🏆 23 quote awards") until badges replace it.
- Undo is the same menu item. No limit on how many lines you can award.

### 4.3 Channel tabs

**My call:** at the top of a quotes channel: **New** (default, chat order), **Top <year>** (by reaction score, this
year), **By person** (pick a member, see their quotes). These are views of the same channel.

## 5. Quote of the Year

### 5.1 Starting

- Anyone starts it from the quotes channel's header ("Start Quote of the Year").
- Choices: the **year** (defaults to the current one; in January, the previous year) and the **length** (a few hours,
  a day, custom, or open until ended).
- **My call:** one event per server at a time, and one *finished* event per server per year. Starting another for a
  finished year is refused with a clear message.
- The options start as the **top 5 quotes of that year by reaction score**, taken when the event starts. Fewer quotes
  that year: fewer options. Ties at 5th place: all tied quotes go in.

### 5.2 While it's live

- A **banner** at the top of every channel in that server: "Quote of the Year <year>", "LIVE", time left (or "until
  ended"), "N of M have voted", a filled **Vote** button.
- The **voting window**: every option with its quote, speakers, reaction score, live vote count and bar; your vote is
  highlighted. Click to vote; click another to move your vote; click yours again to remove it. Self votes are allowed.
- **Add a quote**: anyone can add another quote as an option (search the server's quotes). An added option shows "added
  by <name>". Options can't be removed except by deleting the quote. **My call, from the owner's example:** an added
  quote doesn't have to be from that year, so a great quote from the last days of December, said after the previous
  vote, can still go in by hand. Only the automatic top 5 is limited to the year. A quote that already won an earlier
  year can't be added.
- **In voice:** while the event is live, people in any voice channel of that server see a slim strip under the tiles:
  "Quote of the Year · <leader> leads with N · <time> left", with an "Open voting" button.
- Vote counts, added options and the "N of M" figure update live for everyone.
- "M" is the number of the server's members (joined users).
- **Who can take part, my call:** the same people who can post in the quotes channel today can give awards, vote,
  start, end and reopen an event. Events go to the same audience as the server's channel events today
  (`gateway/audience.rs`); no new privacy rule.

### 5.3 Ending

- **Early:** anyone presses "End voting…", confirms ("End voting now? Everyone's votes are final."), and it ends.
- **By time:** the server ends it when the time runs out, also if the server was restarted in between (on boot, any
  overdue event is ended).
- **Reopening (agreed with the owner):** an event that was **ended early** can be reopened by anyone within **24 hours**
  of ending, with all its votes and options kept. It gets a new length, chosen like at the start. An event that ended
  by time can't be reopened. Reopening is the only way back into a year: starting a new event for a year that already
  has a finished event stays refused, whether or not it can still be reopened.
- **The winner:** the option with the most votes. A tie: **co-winners**. No votes at all: it ends with no winner and
  says so.
- On ending, a **winner card** is posted as a system message in **the quotes channel holding the winning quote**
  (co-winners in different channels: one card in each): the big green quote mark, the
  quote, the speaker(s), "Quote of the Year <year>", the vote count and how it ended. The winning quote keeps a
  permanent "Quote of the Year <year>" mark in the channel.
- The win is recorded per speaker for the badges push.

### 5.4 Look and feel

As in the approved mockup: dark surfaces, green `#39d98a`, a pixel-spill edge (solid green → chunky pixels in three
greens → black), filled green buttons with a deep-green bottom edge, the quote mark as the event's icon.

- **My call:** the pixel spill is generated once, deterministic per event id (so it doesn't jump on rerender), and
  drawn with crisp edges.
- Vote animations: something game-like when you vote, and on the winner card. Library chosen at build time; it must
  respect reduced-motion settings.

## 6. Data and events (for the builder)

Names are suggestions; the builder may change them.

- `channels.kind` gains `'quotes'` (a migration; SQLite needs a table rebuild for the CHECK).
- `quote_lines(message_id, position, text, speaker_id NULL, speaker_name NULL)`: exactly one of the two speaker fields
  set.
- `messages` gains a nullable `said_on` date, or the quote keeps it in its own table.
- `quote_awards(line_id, giver_id, created_at)`, PK `(line_id, giver_id)`.
- `qoty_events(id, server_id, year, started_by, started_at, ends_at NULL, ended_at NULL, ended_by NULL, end_reason)`.
- `qoty_options(event_id, message_id, added_by NULL)`; `qoty_votes(event_id, user_id, message_id)`, PK `(event_id,
  user_id)`.
- `qoty_winners(event_id, message_id, speakers snapshot)` (co-winners are rows; speakers as they were at the end).
- Line ids: `quote_lines` gets its own id; awards reference it.
- `speaker_links(server_id, name_key, user_id, linked_by, linked_at)` for §3.4.
- Gateway events (server audience): quote award changed; event started, updated (votes, options), ended.
- Every rule in §3–§5 is enforced by the server; the client only asks.

## 7. Hand-off to badges

The badges push reads what this push records:

- **Quote awards** per person (§4.2) → a levelling "quote award" badge.
- **Quote of the Year wins** (§5.3) → a special badge per win.
- Both are backdated from the stored timestamps, so nobody loses progress by winning before badges exist.

Owner's badge notes, kept for that push: levelling badges in general; 1st/2nd/3rd-earner designs instead of a plain
gold border; a row of icons with tooltips on the quick profile card and a trophy case on the full profile; the quote
badge as big stylised quotation marks; a Founder badge for the owner and an exclusive badge for the owner's partner.

## 8. Tests

- Server units: line and speaker validation; award rules (own line refused, one per giver, undo); score counting;
  event rules (one live per server, one finished per year, reopen within 24 h of an early end only, top-5 snapshot with ties, add option only from that year,
  vote change and removal, self vote allowed, early end by anyone, end by time including after a restart, co-winners,
  no-vote end); deleting a quote mid-event; edit rules (text keeps awards, speaker change drops them, speakers locked while an option, wins keep their speaker snapshot); linking and unlinking a plain-name speaker.
- Gateway: everyone in the event's audience sees awards, votes, the end and a reopen live.
- UI: composer (layers, picker, no-speaker guard, stripping typed quote marks), option B rendering, the banner and
  voice strip appearing and disappearing with the event, reduced motion.
- A manual run with the group: post a dozen quotes, run a 10-minute Quote of the Year in a call.

## 9. Left to the builder

- Exact component structure, animation library, and the pixel-spill generator.
- Pagination of the Top/By-person views.
- Wording of empty states and errors, following the app's tone.
