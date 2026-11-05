# Calls in DMs and groups, with ringing

Issue #50. Built on the v1 design (`2026-10-01-pulse-design.md` §6.1–6.2, `2026-10-08-dms-design.md` sub-project 2)
and the owner's answers on 2026-11-05. The owner's own words and decisions are marked **(owner)**; everything else is
the builder's call unless it says otherwise.

## Why

- DMs and groups have no voice today. The server already issues LiveKit tickets for private channels (members only,
  `voice/routes.rs`), but there is no way to start a call, and the people board's Call button is disabled ("Calls arrive
  soon").
- Friends call each other directly. A call should find you wherever you are in the app, the way Discord's does.

## Calls

- **Start a call:** a phone button in the DM or group header, and the people board's Call button (enabled). Starting a
  call joins you to the conversation's voice room and rings everyone else in it.
- **One voice connection per client** (v1 §6.1). Starting or joining a call leaves whatever voice channel or call you're
  in, as switching voice channels does today.
- **The call lives at the top of the conversation**, Discord-style: a band above the messages showing the people in the
  call (avatars with the speaking ring, mute/deafen flags), with Join / Leave and the usual voice controls. The bottom
  voice bar works for a call exactly as it does for a server voice channel (name shows the DM or group).
- **Anyone in the conversation can join** a call that's in progress, ringing or not, from the band or the conversation
  list (which shows a small "in call" marker on that conversation).
- The call ends when the last person leaves. Server channels stay silent: no ringing, no lines (v1 §6.2).
- "In voice" indicators elsewhere in the app keep showing server voice channels only (v1 §6.2): a DM call is visible
  only to the people in that DM or group.
- Everything a voice channel already has (mute, deafen, per-person volume, noise suppression, server-mute is not
  applicable here) works in a call. Streaming in calls stays out (#51); the soundboard follows its existing rules for
  where it's available (builder checks and keeps it consistent).

## Ringing (owner)

- **Who rings:** starting a call rings every other member of the DM or group. Joining a call that's already going rings
  nobody.
- **Outside that conversation:** a **popover** appears showing the call: who's calling, their avatar (the group's icon
  and name for a group, with the caller's name), and **Join** / **Decline** **(owner)**. It sits above the app's content
  without blocking it (builder picks the spot, e.g. top centre or above the voice bar) and stays until answered,
  declined, or the ring times out.
- **Inside that conversation:** no popover. The call band at the top of the chat shows the incoming call statically,
  ringing, with Join / Decline, and no modal **(owner)**.
- **Window minimised or unfocused:** also an OS notification ("sam is calling you"); clicking it brings Pulse up on that
  conversation. Respects the notification switches.
- **Ringtone:** plays while ringing, on loop, through the chosen output device. It gets its own notch in the volume
  mixer under Call sounds ("Ringing"), as the mixer spec promised. The builder picks a sound in the style of the
  existing set.
- **Timeout:** ringing stops after **30 seconds** (v1).
- **Decline** stops the ringing for you only. The caller is **not told**, the same principle as no read receipts.
- **Answered elsewhere:** joining, or declining, on one device stops the ringing on your other devices.
- **The call line, and missed calls** (Discord's pattern): starting a call writes one system line in the conversation,
  "sam started a call". When the call ends it reads "sam started a call · 12 min". The server records who joined, and
  each member's client renders the line as **"Missed call from sam"** for anyone who never joined. It counts as unread
  (not as a mention), so a missed call shows up in the conversation list. One line per call; nobody sees who else
  missed it.
- **No busy signal:** the caller can't tell whether you're in another call (v1). You still get rung while in another
  call; joining moves you over.
- **A muted DM or group** behaves like DND for that conversation: no popover, no ringtone, no notification. The band
  and the call line still show (v1 §7 already says muted conversations don't ring; the popover follows it, my call).
- **Do Not Disturb:** no popover, no ringtone, no OS notification. The missed-call line still appears and the
  conversation shows the call is in progress, so you can still join on purpose (status design, and the #50 note).
- **Invisible / appear offline:** ringing works as normal; starting a call or joining one is a decisive action and
  shows, per the status design.

## How it gets made (builder's call where not stated)

- **What a call is** (my call): a call starts when a conversation's voice room goes from empty to occupied, however
  someone got in, and ends when it has been empty for 30 seconds (so everyone dropping and reconnecting continues the
  same call). Only the start route rings. Calls are **stored** (migration, next free number): call id, conversation,
  starter, started/ended, its line, and who joined. After a server restart, `voice::restore` picks the open call back
  up, so the line still gets its duration and nobody who was in it sees "Missed call".
- **Unread** (my call): the call line counts as unread for everyone except the caller, like a normal message, from the
  moment the call starts. No special case when it ends; the "Missed call" wording is just how the line renders for
  members who never joined.
- **Server owns the ring.** Starting a call is a route (e.g. `POST /calls/{channel}`) that the server turns into a
  `CallRinging` event to the other members' sessions only (`audience_for` over the conversation's members), with a
  call id, caller and expiry. The server ends ringing (`CallRingEnded` with a reason: answered, declined, timed out,
  call ended) so every device of every member agrees. In-memory call state, like stream state; a restart drops
  ringing.
- **Who's in the call** comes from the existing LiveKit webhook path (voice state for private channels), sent only to
  that conversation's members, never to everyone.
- **The call line** is a system message the server writes at call start and updates at call end (duration and the set
  of members who joined, as message metadata only members of the conversation receive).
- **Rate limit** starting calls generously (stops ring spam, not normal use).
- **Client:** one call store; the popover, the band and the conversation-list marker read from it. DND gating uses
  `dndActive` (`lib/status.ts`); add the ringing files to `lib/dnd.test.ts`'s allow list.
- Protocol additions are `serde(default)`; any migration takes the next free number.

## Testing

- **Server:** ringing goes only to members (never the caller, never non-members); timeout, decline, answered-on-another-
  device and call-ended all end the ring everywhere; missed-call lines are written once and only for members who didn't
  join; call presence never leaks outside the conversation; rate limit.
- **Client:** popover outside the conversation, static band inside it; Join moves you into the call (leaving other
  voice); Decline and timeout stop the ringtone; DND suppresses popover, sound and notification but keeps the line;
  the mixer's Ringing notch scales the ringtone.
- **Real LiveKit:** a two-person DM call connects and carries audio (extend the existing `voice_it` tests).
- **Manual (owner, two PCs):** call sam from another page and from inside the DM; decline; let it time out; answer on
  one device with Pulse open on two; call while sam is in DND; a three-person group call where one person misses it.

## Out of scope (deferred → issues if not already)

- Streaming in DM and group calls (#51).
- A busy signal or "sam is in another call" (never, by design).
- Video (#68).
- Choosing a ringtone from several (could ride on a future sound-set picker).
