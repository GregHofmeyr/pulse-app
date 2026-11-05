# Name layers and nickname renaming

Issues #47 (name layers) and #48 (nickname renaming, the Name Archive). Decided by the owner on 2026-11-03 on the
design-check page; each decision below is his pick, with his notes folded in.

## Why

- Usernames are lowercase login names. People want to look like themselves: capitals, spaces, emoji ("having all
  lower case names will annoy the more particular friends").
- Renaming each other was part of the v1 design (`2026-10-01-pulse-design.md` §7) and never built: there is no
  rename route, `nickname_history` (migration 0001) is never written, the Name Archive is empty and "times renamed"
  is always 0.

## The layers

| Layer | Who sets it | Where it applies |
|---|---|---|
| **Username** | Chosen at sign-up | Login only. Shown nowhere but the profile card (`@username`, small). |
| **Display name** | Only you, in Settings → Profile | Everywhere, by default. |
| **Server nickname** | Anyone in the server, for anyone (themselves included) | Only inside that server. |
| **Private nickname** | You, for anyone | Only on your own screen, everywhere. |

**Which name shows:**
- Inside a server: server nickname, else your private nickname for them, else their display name.
- Everywhere else (DMs, groups, the friends list, notifications from DMs): your private nickname for them, else their
  display name.
- The username never shows as a name once a display name exists. Everyone gets a display name (see Migration), so in
  practice the username only shows on the profile card.

## Rules

- **Display name** (n1, n2): not unique. 1–32 characters after trimming; any letters, spaces and emoji; no line breaks
  or control characters. Two people may share one; the profile card's `@username` tells them apart.
- **Usernames stay hidden** (n3): `@mentions` render and insert the display name / nickname that applies, never the
  username. The only place a username shows is the profile card.
- **Existing accounts** (n4): the display name starts as a copy of the username. Settings → Profile nudges once
  ("Pick how your name looks"), dismissible.
- **Server nicknames** (n5, n6): anyone in the server can set anyone's nickname, including their own, and anyone can
  reset one (reset = rename to nothing, recorded like any rename). Same length rules as display names.
- **Renames are quiet** (n7): no system line in any channel. Each rename is recorded in the Name Archive, and everyone
  connected sees the new name live (gateway event).
- **No cooldown, a generous rate limit** (n8): typos happen, so people may rename freely; the endpoint gets a rate limit
  that only stops real abuse (builder's call, e.g. 30 renames a minute per renamer, tested). A refused rename says so.
- **Name Archive** (n9): every nickname a person has had in a server, who gave it and when, newest first, with the
  count ("Renamed 14 times"). Everyone in the server can see it, in the **full profile view only** (not the small
  profile card). This is also where a future renaming badge reads its count.
- **Private nicknames** (n10): built in this push. Set from a DM's right-click menu in the DM list ("Change
  nickname", #137), cleared in that dialog. Stored per owner, never sent to anyone else, never in another user's payload.
- **Mentions and search** (n11): `@` suggestions and search's `from:` / `mentions:` match the username, display name,
  server nickname and your private nickname; what's inserted and shown is the name that applies.

## How it gets made (builder's call where not stated)

- **Server**
  - `users.display_name` (new column, migration 0013; filled from `username` for existing rows). Settable through the
    profile update route.
  - Rename route for server nicknames: write `server_members.nickname` and a `nickname_history` row in one
    transaction; broadcast a member-updated event to the server.
  - `private_nicknames(owner_id, target_id, nickname)` table; routes to set/clear; included only in the owner's
    Ready payload and their own events.
  - Name Archive route (paged), readable by server members. "Times renamed" = count of history rows.
  - Everything the server renders with a name (search display text, notification titles, system lines that name
    people, link-preview authors if any) uses the same precedence; the server never knows private nicknames, so
    server-rendered text uses display name or server nickname, and the client re-renders names where it can.
- **Protocol:** `User.display_name`, private nicknames in Ready, new events; all new fields `serde(default)` so old
  clients keep working.
- **Client:** one name function (`displayName` in `lib/selectors.ts` today) implements the precedence and is used
  everywhere a person's name renders: messages, member list, voice rows, mentions (render and autocomplete), search
  results and chips, notifications, profile card and full profile, quotes speakers (linked members), clips authors,
  typing indicator, reactions tooltips, stream tiles. Name styles (font/colour) apply to whichever name shows.
- **UI**
  - Settings → Profile: a "Display name" field with a preview in your name style.
  - ~~Profile card / member menu: "Change nickname" (server), "Reset nickname", "Set private nickname…".~~
    Superseded by #137: right-click menus only (owner: every action between people lives in the right-click menu,
    never a button on a card). In a server (member list, voice participant, message author, and a right-click on
    the profile card): "Change server nickname", then "Set private nickname" / "Change private nickname" (your
    private one, never for yourself). Outside servers (DM list, group call, a card opened outside a server):
    "Change nickname" (your private one). Reset / Clear only inside the nickname dialog; the profile card has no
    nickname buttons.
  - Full profile view: the Name Archive list.

## Migration

0013 adds `users.display_name` (copied from `username`) and `private_nicknames`. No data loss; rollback = restore the
pre-deploy backup.

## Testing

- Server: precedence inputs; rename writes history and broadcasts; anyone can rename/reset; rate limit; private
  nicknames never leak to other users (payloads and events); display name validation; archive paging and counts.
- Client: the name function's precedence in every context (server, DM, group, no membership); mentions insert and
  render the shown name; autocomplete matches all four names; archive renders.
- Manual (owner): set a display name with capitals and emoji; rename a friend in a server and see it live elsewhere in
  that server only; reset it; set a private nickname and confirm the friend can't see it; check the archive in the full
  profile; @mention someone by their old username and get their shown name.

## Out of scope (deferred → issues if not already)

- The renaming badges ("Identity Crisis") — the badges push (#49) reads the archive.
- Role colours / permissions (#79).
