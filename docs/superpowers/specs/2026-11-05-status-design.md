# Status: Online, Do Not Disturb, Invisible, and appearing offline to chosen people

Issue #45 (invisible to specific people), widened by the owner on 2026-11-05 into one status batch. The owner's own words
and confirmed answers are marked **(owner)**; everything else is the builder's call unless it says otherwise.

## Why

- Today Pulse only knows "online" (at least one app connected) or "offline" with a last-seen time. There is no way to
  be on without being seen, and no way to stop being pinged.
- Friends want Discord's statuses: Invisible to hide that you're on, and Do Not Disturb to stop the noise without
  leaving. On top of that, the wishlist item (#45) is appearing offline to particular people only.

## The statuses (owner)

One status per account, set by you and applied everywhere:

| Status | What others see | What changes for you |
|---|---|---|
| **Online** | Green dot, "Online" | Nothing; today's behaviour |
| **Do Not Disturb** | Red DND dot, "Do Not Disturb" | Everything that grabs attention is silenced (below) |
| **Invisible** | Exactly what they'd see if you were offline | Nothing else; you use Pulse normally |

- The status is saved on the account. It follows you to every device and survives restarts and sign-outs **(owner)**.
- When none of your apps is connected you're offline, whatever your status says. Invisible therefore never looks
  different from really being offline.
- **No automatic Idle or Away** in this push. It gets a parked issue.

## Invisible (owner)

- **Presence lies, decisive actions don't.** Others see your dot, status text, the "Your people" board and member lists
  exactly as if you were offline. But anything you choose to do shows, as on Discord: sending, editing and reacting to
  messages, typing dots, joining a voice channel (you appear in it), speaking, going live, watching a stream as a named
  viewer, playing soundboard sounds, renaming people, and so on. Nothing you deliberately do is hidden or changed.
- **Last seen freezes** at the moment you became invisible to someone. It doesn't tick while you're on, and it doesn't
  jump when you finally close the app. Otherwise the timestamp gives you away. When you become visible again, the
  normal rules return: online if you're connected, else your real last disconnect.
- Ghost watching (#46) stays as it is: ghosts are anonymous to everyone already.

## Do Not Disturb (owner)

- **Silenced:** message sounds, desktop notifications, mention pings, the "someone went live" toast and its sound,
  the badge-unlocked sound (the toast may still show quietly), and ringing for incoming calls once ringing exists
  (#50, which must check DND).
- **Not silenced:** the sounds of a call you're already in (join and leave chirps, mute and deafen cues, the
  soundboard, voices, streams). You chose to be in that call.
- **Still updated:** unread badges, mention counts, server and call indicators. Only the alerts stop.
- **Timers**, like the existing mutes (`muteDurations.ts`): 1 hour, 8 hours, until tomorrow morning, or until I turn it
  off. When a timer runs out you return to Online. A timer only applies to DND; Invisible has no timer in this push.
- DND is enforced in the client: the client already decides alerts (`shouldAlert`, `notifyMessage`). Because the status
  lives on the server, every device knows it.

## Appearing offline to chosen people (owner)

- Right-click a person (member list, DM list, voice row, message author, profile card) and pick
  **"Appear offline to them"**. It's an ordinary item in each person right-click menu, never a button or text link on
  the profile card or under an avatar **(owner: every action between people lives in the right-click menu)**. To them you now look exactly as if you were Invisible, with the same rules: last seen
  frozen at the moment you hid, and decisive actions still showing. Everyone else sees your real status, DND included.
- Undo it with the same menu ("Stop appearing offline to them") or from **Settings → Privacy → Appear offline to**,
  a list of everyone you're hidden from, each with a Remove button.
- The hidden person is never told, and nothing in their payloads reveals the list. It's private to you, like private
  nicknames.
- You can hide from anyone you could otherwise be seen by (anyone sharing a server, DM or group with you).
- Being hidden from someone and being Invisible stack: if either applies, they see you offline.

## The status picker

- Click your own avatar at the bottom of the sidebar to open a small menu with the three statuses, each with its dot,
  and the DND timers as a submenu. The current status is ticked, and the avatar shows your status dot.
- Invisible shows on your own avatar as a hollow grey dot, so you always know you're hidden. Others never see that dot.
- Settings → Privacy holds the "Appear offline to" list.

## How it gets made (builder's call where not stated)

- **Privacy is enforced on the server.** A viewer must never receive your true presence when they shouldn't see it:
  not in Ready or REST payloads (`Person.online`, `last_seen_at`), not in `PresenceChanged`, not in anything else that
  hints at it. Each viewer gets their own view of each person. `audience_for` and `Hub::sync_presence` gain a
  per-viewer step: compute what each recipient should see and only send a change to those whose view changed.
- **Status reaches clients as data:** add a status to the presence payloads (`online` | `dnd` | `offline`, with an
  optional `last_seen_at`), as the viewer is allowed to see it. Your own sessions get your real status, your DND
  timer and your hidden list. New protocol fields are `serde(default)`.
- **Storage** (migration 0019): your status and DND end time on `users`, plus when you became invisible (for the frozen
  last seen); and a `presence_hidden(owner_id, target_id, hidden_at)` table, with `hidden_at` as the frozen last seen
  for that viewer. When both apply, the viewer sees the earlier of the two moments. Rollback = restore the pre-deploy
  backup. Note: `users.status` already exists from 0001 (STRICT, `CHECK (status IN ('online','dnd'))`) and nothing
  uses it. Either rebuild the table to widen the check, or add a new column and leave the old one unused. The builder
  picks.
- **"Until tomorrow morning"** is new (`MUTE_FOR` has no such option): the client computes 08:00 local the next day
  and sends the server an absolute end time.
- **The DND timer** is stored as an end time. The server treats an expired DND as Online, and clients flip back on time
  without a round trip. Builder picks how the server announces the change to others (a sweep or a check on read).
- **Routes:** set status (with an optional DND end time), and hide/unhide a person. Rate-limit both generously, like
  renames.
- **One client presence function** returns what to draw for a person (dot colour, text, last seen), and it's used
  everywhere a dot or "last seen" shows. A test pins the list of places.

## Testing

- **Server:**
  - every presence path for a viewer who's allowed versus not (Ready, REST, `PresenceChanged`, reconnects, a second
    device): no leak in any payload or event;
  - frozen last seen across connect and disconnect while invisible or hidden;
  - stacking of Invisible and hidden;
  - the DND timer expiring;
  - the hidden list only ever goes to its owner;
  - unhiding and going visible again announce correctly.
- **Client:**
  - DND silences each alert listed above and keeps call sounds and badges;
  - the timer flips back;
  - the picker and the Settings list work;
  - the presence function draws each case.
- **Manual (owner, Windows):**
  - Go Invisible: a friend sees you offline, with last seen frozen. Message them: they see the message. Join voice:
    they see you in the channel.
  - Go DND for an hour and have a friend mention you and DM you: no sound and no notification, but the badges count
    up. Check the timer flips you back.
  - Appear offline to one friend: they see you offline while a second friend sees you online.

## Out of scope (deferred → issues)

- Automatic Idle or Away after inactivity.
- Custom status text and emoji ("Gaming 🎮").
- Timers for Invisible.
- Per-server statuses.
