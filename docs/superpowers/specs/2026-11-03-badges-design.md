# Badges

Issue #49. Owner's picks from the design-check page (2026-11-03), summarised on the issue. The v1 design (§9) set the
basics; this replaces it where they differ. **Status: approved by the owner 2026-11-03** (Skill Issue's trigger to come; the partner badge named 2026-11-06, #126).

## Why

The group is competitive and loves this. The trophy case and badge row already exist in the profile UI but nothing
awards a badge (`user_badges` is never written).

## Goals

- Badges people earn by using Pulse together, announced to the earner, collected in a trophy case, a few shown off on
  their profile.
- Tiered badges that level up, rarity that makes some feel special, hidden ones to discover, and a first earner who is
  remembered.
- Competitive: most badges are reachable; some take real dedication; a few almost nobody gets.

## Non-goals

- Final badge art. Emoji placeholders ship first; the art (tier textures, first-earner sparkles) is its own design job.
- Badges from DMs or groups (v1 privacy rule: never earned from, and never revealing, private activity).
- Backdating: everything counts from launch (owner: fairness, and accounts may be reset for the real launch).
- Roles, levels or XP systems.

## The engine

- **The server awards every badge.** Rules run on the server after the events they depend on (a message posted, a voice
  session ended, a quote award given…), and on a slow sweep for time-based ones.
- **Counters, not recounts.** A `badge_stats(user_id, key, value)` table is bumped as events happen, so badges survive
  deleted channels and purges and stay cheap to check. Rules read counters and the event.
- **Global per user** (owner): one trophy case across all servers. Server activity counts toward it; DM/group activity
  never does.
- **Tiers:** a tiered badge has thresholds (bronze → silver → gold → diamond → …). `user_badges` gains `tier`; each tier
  reached is its own unlock. Each tier will get its own border and inner art; placeholders show the tier as a coloured
  ring and a numeral.
- **First earner:** the first person to reach a badge (each tier separately) keeps `first_earner` forever: a faint gold
  border plus a sparkle on the badge.
- **Rarity:** a class (Common, Uncommon, Rare, Epic, Legendary, Mythical) chosen by hand, shown on the badge, plus the
  live share "2 of 8 have this". **Static badges have one rarity; tiered badges have a rarity per tier** (owner,
  2026-11-03: renamed 10× is common, 50× epic, 100× legendary), so a higher tier is also a rarer badge. The ring, the
  sort order ("rarest first") and the showcase default all use the rarity of the tier reached. Hidden badges show as
  "???" with their rarity until earned.
- **Client-reported events.** One badge ("The Doos") needs a signal only the app sees: speaking into a muted mic. The
  client reports it as an event; the server counts it with a rate limit and caps. It's spoofable, which is acceptable
  among friends; no other badge relies on the client.
- **Testing without real history:** a server-side `testing`/admin CLI command can award or reset badges for a user, and
  rules have unit tests over synthetic events. Not exposed in the app.

## Unlock experience

- The earner gets a toast and an unlock sound (no channel message, owner's pick).
- A **[NEW]** shine on the badge and a small dot on their avatar's profile entry until they open their trophy case.
- Others see it when they open the profile. No announcements to others.

## Showing badges

- **Trophy case** (full profile view): every badge earned, with tier, rarity, date and "First!" where it applies;
  locked badges shown greyed with how to earn them (hidden ones as "???"); per-badge progress bars for tiers
  ("412 / 500 messages").
- **Showcase:** up to **5** badges shown on the profile card and full profile header, chosen and ordered by the
  profile's owner in the trophy case (owner's idea). Default: the 5 rarest earned. Shown smaller than the trophy case
  tiles (owner); exact size tuned on the real profile cards.

## Catalogue (draft: thresholds are proposals to tune)

Rarity in brackets (tiered badges: per tier, tuned on the badge page). ⓗ = hidden until earned. Tiers listed as thresholds.

**Voice**
- **Night Owl** [Rare] — in a voice channel with at least one other person at 03:00 South African time
  (Africa/Johannesburg, UTC+2, no DST).
- **Early Bird** [Uncommon] — first person into voice on 10 different days.
- **Ghost** [Rare] — joined and left voice within 5 s, 10 times.
- **The Regular** [tiers: Common → Mythical] — hours in voice with at least one other person: 10 / 50 / 100 / 250 / 500
  / 1000 h.
- **The Doos** [Epic] — talked into a muted mic (the "you're muted" detector fired) 10 times. Client-reported.

**Names**
- **Identity Crisis** [tiers: Common → Uncommon → Epic → Legendary] — renamed by others: 10 / 25 / 50 / 100 times. Reads the Name Archive (#48).

**Chat**
- **The Yapper** [tiers] — messages sent in servers: 100 / 500 / 1 000 / 5 000 / 10 000.
- **Reaction Farmer** [tiers: Rare → Epic → Legendary → Mythical] — reactions received from others: 50 / 100 / 500 / 1 000.
- **Emoji Smith** [Rare] — added 5 custom emoji that others used 10+ times each.

**Quotes**
- **Quotable** [tiers] — quote awards received: 10 / 20 / 30 / 50 / 100 (the levelling badge from the quotes design).
- **Quote of the Year** [Legendary, stacking] — won QOTY; the count shows on the badge (×2, ×3).
- **John Quoter** [Uncommon] — quoted others 25 times.
- **Personal Vendetta** [Rare] — quoted the same person 20 times.
- **Friendly Fire** ⓗ [Uncommon] — quoted yourself.

**Clips and streams**
- **Clipper** [tiers] — clips shared: 5 / 25 / 100.
- **Highlight Reel** ⓗ [Uncommon] — a clip with 5 reactions. "Bro is good at games."
- **"Chat, did you see that?"** [tiers] — hours streamed: 50 / 200 / 500 / 1000. Needs stream sessions recorded (new
  table: who, channel, started, ended).

**Meta and rare**
- **Early Adopter** [Epic] — joined within the first week after launch. The launch date isn't set yet: it's a
  server config value (unset = the badge can't be earned yet), set by the owner when the group moves over.
- **Badge Collector** [tiers] — earned 10 / 20 / 30 badges.
- **How Did We Get Here?** [Epic] — unlocked 3 hidden badges (lowered from 5 on 2026-11-03: only three are earnable).
- **Perfectionist** [Mythical] — reached the top tier of 5 tiered badges.
- **Combo** [tiers] — 100 messages + 100 h voice + 10 clips, then 500/500/25, then 1 000/1 000/100 (name TBD).
- **Logo egg** ⓗ [Epic] — the hidden logo-click egg from the v1 plan (new icon).
- **Skill Issue** ⓗ [Epic] — awarded for something spectacularly embarrassing. **Trigger TBD by the owner** (the group's
  own running jokes).

**Hand-given** (unique, awarded by the owner with the admin command; never earnable, never "???"):
- **Founder** [Mythical, one of a kind] — the owner.
- **alienussy** (`partner`) [Mythical, one of a kind] — "One of a kind, for a special someone." (#126); 💚 until its picture is uploaded.
- **Patient Zero** (`patient_zero`) [Mythical, one of a kind] — "One of a kind: the first soul brave enough to test Pulse."; 🧪 until its picture is uploaded.

The one-of-a-kind badges have pictures (#225), kept only on the server (never in this public repo): `just badge-art <key> <file>` stores one in the media store (`badge_art` table, a still fitted within 256 px) and the badge's definition carries its hash as `art`; clients draw it round inside the rarity ring, fitted whole, and fall back to the emoji.

**Parked:** 3 am quote ("get quoted for something said after 02:00").

## How it gets made (builder's call where not stated)

- Migration: `badge_stats`, `user_badges.tier` (+ showcase order), `stream_sessions`, a `badge_meta` table or code
  constants for launch date and time zone.
- Badge definitions in code (`key, name, description, icon, rarity, hidden, tiers, rule`); rules are pure functions over
  counters + event, unit-tested.
- Hooks where events already happen: message send, reaction add (to the author), voice join/leave (session end),
  quote post / award / QOTY end, clip post, stream start/stop, rename (#48), client "muted speaking" event.
- Protocol: `BadgeEarned` event to the earner; badges + showcase in profiles; a catalogue endpoint (definitions, rarity
  share) for the trophy case.
- UI: trophy case in the full profile, showcase on the card, unlock toast + sound, [NEW] state, showcase editor.

## Testing

- Each rule with synthetic events, including tier boundaries and first earner races.
- DM/group activity never counts; deleted channels don't remove earned badges or counters.
- Showcase order persists; hidden badges never leak names/descriptions before earned (payload test).
- Manual (owner): earn a badge with the test command; see toast, sound, [NEW]; arrange the showcase; check another
  person's profile.
