# Quotes, quote awards and Quote of the Year: design

Date: 2026-10-22. Status: approved 2026-10-22; owner answers to the plan's questions added 2026-10-27 (§10).

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
- Quote search beyond the leaderboard in §4.3, exporting.
- Placeholders for badges (for example a "23 quote awards" profile stat). The data is recorded; badges show it.

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
| Score | Any emoji counts (not one specific emoji), but **one reaction per person per quote**. The score is the number of people who reacted. |
| Replies | A quote can have **replies** for banter: "Reply" opens a **side panel** with a chat about that quote (like Root's side panels, thread-like). The quote shows "View N replies". Replies never affect scores, votes or awards. |
| Leaderboard | No Top tab. A **leaderboard icon** in the channel header opens a small view: Quote of the Year winners, most-reacted quotes, and more. |
| Image quotes | A quote line can be an **image** (a screenshot of chat or in-game text) instead of text, still with a speaker. Uses the media storage the reactions and emoji push brings. |
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
- The channel's main list holds **quotes only**; the composer at the bottom always makes a quote. Chat about a quote
  happens in its **replies** (§3.6).
- A server will normally have one quotes channel, but there's no hard limit. Quote of the Year covers all of them in
  that server.

### 3.2 A quote

A quote is a **message** in a quotes channel (so reactions, editing, deleting, unread and the gateway all reuse the
message machinery). Its lines are stored alongside it:

- one or more **lines**, in order; each has a **speaker** and either **text** or an **image** (§3.7);
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

Limits: **my call:** up to 10 lines per quote; each text line up to the normal message length limit.

### 3.3 The composer

- Type the quote, pick the speaker from the server's members (type-ahead, like @mentions), press Enter.
- "+ line" adds another line with its own speaker before posting.
- Quotation marks are added by the display. If someone types them, they're stripped.
- No speaker picked: the composer won't post, and it says "Who said it?".
- Pasting or dropping an image into a line (or an image button on the line) makes it an image line (§3.7).

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
- Under the block: the reaction row (existing reactions, faded suggestions, "+"), then **"View N replies"** when there
  are any (with a "new" marker if there are replies you haven't seen).
- Line menu (hover or right-click a line): "Give quote award" / "Take back award", "Copy".
- Quote menu: **Reply** (opens the replies panel), edit and delete (poster only).

### 3.6 Replies (the banter panel)

- **Reply** on a quote opens a **side panel** on the right: the quote pinned at the top, then a normal chat (text,
  mentions, reactions, edit, delete, as in a text channel) about that quote only. Closing the panel returns to the
  channel. One panel at a time.
- Replies **never count** toward anything: not the score, votes, awards or badges.
- **My calls:**
  - Replies are messages in the same channel, tied to their quote, and kept **out of the main list**.
  - New replies don't make the channel show unread. "View N replies" gets a "new" marker for replies since you last
    opened that panel. An @mention in a reply notifies you as usual.
  - A reply is text only (no quote lines). Deleting the quote deletes its replies.

### 3.7 Image quotes

- A line can hold one image instead of text: a screenshot of something from another channel, in-game text, and so on.
  It still has a speaker, so it counts like any other line (awards, Quote of the Year).
- Images use the upload and storage built in the **reactions and emoji** push (custom emoji need it first). Same
  limits and formats as that push sets.
- Display: the image inline in the quote block at a sensible max size, click to view full size.
- **My call:** an image line can have an optional short caption.

## 4. Scores and awards

### 4.1 Reaction score

**Agreed with the owner:** in a quotes channel each person can have **one reaction per quote**, any emoji. Picking a
different emoji replaces yours; picking the same one removes it. The score is the number of people who reacted. (This
rule is for quotes only; replies and other channels keep normal reactions.)

### 4.2 Quote awards

- One row per (line, giver). Can't give to your own line (speaker is you). Can't give to a plain-name speaker's line?
  **My call: you can.** It's kept on the line and counts for the person once the speaker is linked.
- Giving or taking back an award updates everyone's view live.
- **Per-person total:** the number of awards on lines whose speaker is that person. Recorded for badges; not shown on
  the profile in this push (the owner prefers no placeholders).
- Undo is the same menu item. No limit on how many lines you can award.

### 4.3 Leaderboard

- A **leaderboard icon** in the quotes channel header opens a small view (a popover or the side panel).
- **My call on its sections:** Quote of the Year winners (by year); most-reacted quotes (this year, all time); most
  quoted people; most quote awards. Each entry jumps to the quote.
- The channel itself has no tabs; it's always the chat-order list.

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
- Replies: `messages` gains a nullable `thread_root_id` (the quote); the main list excludes rows that have it; a per-user "replies last seen" point per quote for the "new" marker.
- Image lines: `quote_lines` gains a nullable media reference (from the reactions and emoji push's storage) and a
  caption; exactly one of text or media is set.
- One reaction per person per quote: enforced by the server for quotes in a quotes channel.
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

- Server units: line and speaker validation (text or image, not both); one reaction per person per quote (replace,
  remove); replies kept out of the main list and out of every count; award rules (own line refused, one per giver, undo); score counting;
  event rules (one live per server, one finished per year, reopen within 24 h of an early end only, top-5 snapshot with ties, add option only from that year,
  vote change and removal, self vote allowed, early end by anyone, end by time including after a restart, co-winners,
  no-vote end); deleting a quote mid-event; edit rules (text keeps awards, speaker change drops them, speakers locked while an option, wins keep their speaker snapshot); linking and unlinking a plain-name speaker.
- Gateway: everyone in the event's audience sees awards, votes, the end and a reopen live.
- UI: composer (layers, picker, no-speaker guard, stripping typed quote marks, pasting an image), the replies panel,
  the leaderboard view, option B rendering, the banner and
  voice strip appearing and disappearing with the event, reduced motion.
- A manual run with the group: post a dozen quotes, run a 10-minute Quote of the Year in a call.

## 9. Left to the builder

- Exact component structure, animation library, and the pixel-spill generator.
- Pagination of the Top/By-person views.
- Wording of empty states and errors, following the app's tone.

## 10. Owner answers to the builder's questions (2026-10-27)

Decided after the plan was written; they override anything above that says otherwise. The plan's "Spec
reconciliation" carries the detail.

1. **Protocol bump:** yes. Clients older than this push get the "update required" screen.
2. **Votes are public.** Each Quote of the Year option shows who voted for it (a small stack of avatars, names on
   hover), in the voting window, on the winner card and in the leaderboard's winners. Quote awards show their givers
   the same way. ("I like the competition of asking someone why on earth they voted for X.")
3. **Leaderboard entries jump to the quote** in its channel: the channel loads history around that quote, scrolls to
   it and highlights it, and you can scroll on to the present. Built to be reused later for search, mentions and pins.
4. **A reply's @mention** counts in the channel's mention pill until you open that quote's replies.
5. **Image quotes stay animated**, like custom emoji: an animated image plays and freezes by the same rules (the
   "Animate emoji" setting, a focused window, frozen while you're live, never with reduced motion), with a still
   first frame. Sensible size caps apply.
6. **Lengths:** "a few hours" = 3 hours, "a day" = 24 hours, custom = 5 minutes to 14 days, or until ended.
7. **No votes:** the "no winner" line goes in every quotes channel of the server.
8. **Zero-reaction quotes** are never automatic options, so a quiet year can have fewer than 5; adding by hand still
   works.
9. **Reopening** removes the earlier winner card.
10. **Linking a plain-name speaker** to a member is allowed while a vote is live (it isn't an edit).
