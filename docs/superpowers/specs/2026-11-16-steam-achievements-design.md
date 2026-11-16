# Steam account linking and live achievements

Issue #379. Builds on game activity (#380, `2026-11-16-game-activity-design.md`). **(owner)** marks the owner's
decisions from the issue; everything else is a decision taken while building, listed at the end.

## Goals

- **Link Steam** in Settings → Privacy → Game activity, with Steam OpenID 2.0 (no API key needed): one Steam account
  per person, a SteamID already linked to someone else is refused, unlink any time. The linked persona name shows.
- **Live unlocks (owner):** while a linked person plays a Steam game (and for 10 minutes after), the server polls
  `ISteamUserStats/GetPlayerAchievements` every 2 minutes, diffs it against what it stored, and posts new unlocks as
  a card: icon, name, description, global rarity.
- **A bigger 100 % moment (owner):** unlocking the last achievement of a game posts a "100 % complete" card.
- **Completionist badge (owner):** the first 100 % game, through the badges catalogue.
- **Account completion rate as a profile stat (owner):** Steam's AGCR: the average completion over games with at
  least one achievement unlocked.
- **Settings toggle, default on (owner):** "Share Steam achievements" under Game activity; off = no polling for that
  person and nothing posted.
- **Yearly awards material (owner):** every live unlock and every 100 % is recorded (`steam_unlocks`), with its
  global rarity, so an awards show can ask "rarest unlock of the year" later.

## Config

- `PULSE_STEAM_API_KEY` (optional, a secret: redacted from Debug, never logged; `deploy/env.example`, `SECURITY.md`).
  Without it linking still works; achievements are off and Settings says the server owner needs to add a Steam key.
- Linking needs `PULSE_PUBLIC_URL` (already the embed page's origin): the OpenID `return_to` is
  `<public url>/steam/openid/return?state=…`. Without it Settings says linking isn't set up on this server.
- `SteamConfig` (in `Config`): API base, OpenID endpoint (both overridable so tests point at a fake), poll interval
  (2 min), tail (10 min), backfill gap between calls (2 s), daily call cap (50 000, server-wide).

## Linking (OpenID 2.0)

1. Client: `POST /me/steam/link` → `{ url }`; the core opens it in the system browser. The server mints a one-time
   state token (32 random bytes, kept hashed in memory, 10 min TTL, bound to the user) and builds the
   `checkid_setup` URL with `return_to` and `realm` from the public URL.
2. Steam sends the browser to `GET /steam/openid/return?state=…&openid.…` (no auth: the state is the credential).
   The server takes the state (one use), then checks: `openid.mode = id_res`; `op_endpoint` is the configured one;
   `return_to` is exactly ours with this state; `claimed_id = identity` and matches
   `^https://steamcommunity\.com/openid/id/(\d{17})$`. Then it asks Steam itself: the same `openid.*` fields with
   `mode=check_authentication`, POSTed to the endpoint, must answer `is_valid:true`.
3. Stored in `steam_links` (one row per user, `steam_id` UNIQUE). Linked to someone else → refused; the same person
   relinking the same account is a no-op. The persona name is read with `GetPlayerSummaries` when there's a key.
4. The page says "Steam linked — you can close this tab" (or what went wrong), plain HTML with a strict CSP; the
   person's sessions get `Event::SteamLinkChanged` with the new status, so Settings updates without a refresh.
5. Unlink (`DELETE /me/steam`) deletes the link, its achievement state and its pending backfill (cascade): relinking
   starts clean with a fresh backfill. Records already posted and `steam_unlocks` stay (they're history).

## Achievements

- **Which game:** the poller asks the hub for the person's current Playing; the Steam app id comes from the client's
  update. "Show game art" off used to drop the app id; now the client still sends it when achievements are shared,
  with `hide_art: true` (additive `PlayingUpdate` field): the server keeps the id for the poller only, fetches no art
  and shows no id to anyone.
- **Polling:** every 2 min for each linked person with sharing on, a key configured and not backed off, whose current
  game has an app id or whose last one ended under 10 minutes ago. One `GetPlayerAchievements` call each (≈30/hour per
  active player). Schema (`GetSchemaForGame`) and rarity (`GetGlobalAchievementPercentagesForApp`) are fetched only
  when there's something to post, cached per app for 24 h in `steam_app_cache`.
- **Diff (pure):** new = achieved now, not in the stored set. On a game with no stored state (never backfilled, new to
  them) only unlocks at or after the play session's start count; the rest are the baseline. So old unlocks are never
  announced, and a first achievement in a brand-new game still is.
- **Where it posts:** each server can pick an **Achievements channel** (server settings, owner only, default none =
  no posts). A card goes to the achievements channel of every server the person is a member of. One message per poll
  per server lists the new unlocks (up to 5 shown, "and N more"); reaching 100 % posts a second, bigger card with the
  game's header art. Icons and art are fetched by the server through the guarded fetcher and stored as media (kept
  by `steam_card_media`), so clients never reach Valve.
- **Badge:** `badge_stats.steam_perfect_games` (games at 100 %, backfill included) drives Completionist (1 game,
  Epic).
- **Completion rate:** recomputed from `steam_achievement_state` after each change; shown on the profile card as
  "Steam completion 62 %" while the person shares achievements.

## Backfill

On link (and at startup for any unfinished one) a single server-wide worker, one person at a time, one call every
2 s: `GetOwnedGames` (`include_appinfo`, `include_played_free_games`), then `GetPlayerAchievements` for every game
with community-visible stats and some play time. Each answer becomes that game's stored state (silently), which
also gives the totals for the completion rate and counts 100 % games. A game without stats is skipped.

## Privacy failures and backoff

`GetPlayerAchievements` answering "not public" (or `GetOwnedGames` answering an empty response) marks the link
`private_since` and backs that person off for an hour; Settings shows "Make your Steam game details public to share
achievements". The next successful call clears it; a backfill that met it resumes. Other errors back off 10 min.

## Budget

Every Steam Web API call takes one from a server-wide daily allowance (`daily_cap`, 50 000 by default, reset at UTC
midnight). Exhausted, calls are refused (logged once a day) until the next day; nothing else breaks.

## Storage: migration 0043 (additive tables only)

`steam_links`, `steam_achievement_state`, `steam_app_cache`, `steam_unlocks`, `steam_card_media`,
`server_achievement_channels`. Rollback = leave them (older servers never read them).

## Protocol (additive; no PROTOCOL_VERSION bump)

- `PlayingUpdate.hide_art` (default false).
- `rest::SteamStatus`, `SteamLinked`, `SteamLinkStart`, `SteamSettingsRequest`, `AchievementsChannel`,
  `SetAchievementsChannelRequest`, `AchievementCard`, `AchievementEntry`.
- `Message.achievement: Option<Box<AchievementCard>>` (system lines, payload type `achievement`).
- `ProfileStats.steam_completion` / `steam_perfect_games` (omitted when not shared).
- `Event::SteamLinkChanged { user_id, status }`, to the person's own sessions only.
- Routes: `GET/DELETE /me/steam`, `POST /me/steam/link`, `PUT /me/steam/settings`, `GET /steam/openid/return`,
  `GET/PUT /servers/{id}/achievements-channel`.

## Decisions taken while building

- Posting place: a per-server Achievements channel, default off. Considered: the person's inbox (nobody else sees
  it, which loses the point) and every server's general channel (noisy, nobody asked).
- "Share Steam achievements" is stored on the server per account, not per device like the other switches: the server
  does the polling, and two devices disagreeing would be meaningless. It still sits under the master switch: master
  off sends no Playing, so nothing is polled.
- Games completed before linking count for Completionist (Steam already shows them) but never post a card.
- The persona name needs the API key (`GetPlayerSummaries`); without one Settings shows "Steam account linked".

## Non-goals

- Polling people who aren't playing (no "achievements unlocked offline").
- Non-Steam achievement sources (Xbox, RetroAchievements).
- The yearly awards show itself (the data is kept for it).
