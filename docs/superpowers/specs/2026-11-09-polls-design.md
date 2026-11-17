# Polls

Issue #157. Decided with the owner on 2026-11-09. The owner's own words and decisions are marked **(owner)**; everything else is the builder's call unless it says otherwise.

## Why

Friends decide things in chat (when to play, what to play, where to eat). A poll keeps the question, the options and the votes in one place, live. Promised in the 0.2.1 patch notes' Coming soon.

## What it does

- **Where:** server text channels, voice channels' chats (#226), DMs and groups. Not quotes or clips channels.
- **Starting one (owner):** a **poll button in the composer** (beside the wheel button, #158) or **`/poll`** opens the **poll composer** above the message box:
  - a **question** field across the top (a title bar) **(owner)**;
  - **quick toggles** under it: **Multiple choice** (off = single choice) and **Anonymous** (off = votes show who voted) **(owner)**, plus **Ends in** with **1 hour, 1 day, 1 week, No end** (default 1 day);
  - **option rows**, starting with **two empty text rows** and a **"+ Add option"** button underneath **(owner)**; each row can be removed once there are more than two;
  - **Post poll**, disabled until there's a question and at least two non-empty options. The same option twice is allowed (votes go by position, not text); tied options with the same text are named once on the ended line ("Pizza tied").
  - `/poll Question? a, b, c` prefills the composer (question up to the `?`, options split on commas) rather than posting straight away, so the toggles can still be set.
- **The poll in chat:** a poll card (a message in the chat like any other, so it scrolls, replies, reacts and can be deleted by its author like a message) showing the question, the options as rows with a vote bar, count and percentage, the toggles' state ("Multiple choice · Anonymous"), the time left or "Closed", and the total votes.
  - **Voting:** click a row to vote; single choice moves your vote, multiple choice toggles each. You can change or remove your vote until it closes. Your choices are marked.
  - **Results (agreed with the owner):** you see the results after you vote; the poll's creator sees them live from the start; everyone sees them once the poll closes. Before voting, rows show no counts.
  - **Who voted (owner):** by default the card shows **who voted for what** (small avatars on each row, the full list on hover or click). **Anonymous** polls never reveal voters to anyone, including the creator, and the server never sends voter identities for them (counts only). Anonymous is chosen at creation and can't be changed after.
  - **Closing:** at "Ends in", or early by the creator ("End poll" in the poll's menu). A closed poll shows the final results to everyone and a small "Final results" label; the winning option(s) are emphasised. When it closes, a system line "📊 alex's poll ended: Valheim won (4 votes) · See poll" appears under it (ties name all tied options); no notification.
- **Live:** votes, closes and deletes update for everyone without a reload.
- **Limits:** question up to 200 characters, 2–10 options of up to 80 characters each, and the usual message rate limits for posting; votes rate-limited per person enough to stop scripted flipping.

## Editing (#265, added 2026-11-17)

- **Who and when:** the poll's creator, while it is open: **Edit poll** in the poll's menu, or the pencil on its action bar. It opens the poll composer filled from the poll, with **Save changes**. The question and options change; **Multiple choice, Anonymous and the end time stay as posted** (changing them under people who already voted would change what their vote means).
- **Votes follow their option.** Votes are by position, so the edit says, for each row, which old option it is (`from`) or that it's new. Adding options, and moving them, keeps every vote with its option, with no confirm.
- **Rewording or removing an option that has votes clears just that option's votes** (they were for what it said; a vote for "Pizza" shouldn't silently become one for "Sushi"). Save asks first ("This clears 2 votes (Pizza).", the button turns into **Clear votes and save**). The server enforces it: such an edit without `clear_votes` is refused (409), so a vote that lands while the creator is editing is never cleared unasked. Rewording or removing an option nobody voted for needs no confirm. Whoever lost a vote can vote again.
- **Marked:** the card's status line ends in "(edited)" (`Poll.edited_at`); an edit that changes nothing isn't marked. The message's text for older clients (and search) follows the edit. Everyone's card updates live (`PollChanged`).

## How it gets made

- **Server:** a poll is a message with a poll payload (question, options, multi, anonymous, ends_at, closed_at) plus a votes table (poll message, option index, voter, voted_at; unique per voter/option, and per voter for single choice). Next free migration number (0030 at the time of writing; rebase first and say what it does). Routes to create (as a message, same membership and limits as sending), vote/unvote, and close early (creator only). Results are computed server-side and filtered by the visibility rules above: before you vote you get no counts unless you're the creator or it's closed; anonymous polls never include voter ids. A periodic sweep or on-read check closes due polls and posts the "ended" system line once. Events for vote changes and closes. Deleting the poll message deletes its votes. Tests for permissions, visibility (voted / not voted / creator / closed / anonymous), single vs multi, the close line posted exactly once, limits.
- **Client:** the composer button, `/poll` parsing (reuse the composer's slash handling, as the wheel did), the poll composer, the poll card, the system line, live updates. Reuse existing composer popover, toggle (Settings switch style), button, avatar stack and tooltip components; match the wheel popover's placement so the two composers feel like siblings.
- Look and feel matches existing cards, popovers and toggles exactly; existing tokens only; icon sizes 14 inline / 16 buttons / 18 nav (or match the neighbouring composer buttons, as the wheel did); reduced motion respected for the vote bars' growth.

## Testing

- Server: create/vote/change/unvote/close; visibility rules; anonymous never leaks ids; single vs multi; expiry posts the line once; delete cascade; non-members refused.
- Client: composer starts with two rows, + Add option, remove, Post disabled until valid; `/poll Q? a, b` prefills; toggles; card states (before vote, after vote, creator, closed, anonymous); End poll; live updates.
- **Manual (owner, Linux, two accounts):** a named single-choice poll and an anonymous multi-choice poll; vote from both; see the creator view vs voter view; end one early and see the ended line; leave a 1-hour poll to close on its own.

## Out of scope

- Editing a poll's question or options after posting (done later: see Editing, #265).
- Polls in quotes or clips channels.
- Ranked-choice or weighted voting; images on options.
