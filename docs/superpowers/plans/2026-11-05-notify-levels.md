# Notification levels per server and channel (#195)

Discord's model: each server has a level per person (**All messages**, the default; **Only @mentions**; **Nothing**),
and each server channel may override it or **Follow server** (the default). The level decides the message sound and
the desktop notification for server messages; mute, DND, unread badges and DMs/groups are unchanged.

## Server and protocol

1. Migration `0023_notify_levels.sql`: `ALTER TABLE notification_prefs ADD COLUMN level TEXT CHECK (level IN
   ('all', 'mentions', 'nothing'))`. NULL = the default (server: All; channel: Follow server). Existing rows keep
   `level` NULL, so nothing changes for anyone until they pick a level (mute rows are untouched).
2. Protocol: `NotifyLevel { All, Mentions, Nothing }` (snake_case), `NotifyLevelPref { target_kind: MuteTarget,
   target_id, level }`, `SetNotifyLevelRequest { target_kind, target_id, level: Option<NotifyLevel> }` (None = back to
   the default). `Ready.notify_levels` (`serde(default)`), owner-only event `NotifyLevelsChanged { user_id, levels }`.
   Export bindings in `protocol/src/lib.rs` `export_all`.
3. Route `PUT /notify-levels` beside `/mutes` (`dms/routes.rs`): `mutate_user` rate limit, the same target check as
   mutes, plus server channels only (a DM or group has no level: 400). A server's All is stored as NULL. Upsert keeps
   `muted`/`muted_until`; a row with nothing left (unmuted, NULL level) is deleted. `clear_mute` now keeps a row that
   still holds a level (unmute = `muted = 0`, then drop the empty row). Publish `NotifyLevelsChanged` with
   `levels_of`.
4. Ready (`gateway/socket.rs`) sends `levels_of`; a restored channel republishes its holders' levels next to their
   mutes (`servers/channels.rs`). Purge already deletes every pref of the channel.

## Client

5. Tauri core: `api.set_notify_level`, command `set_notify_level`; `tauri.ts` `api.setNotifyLevel`.
6. `state.ts`: `notifyLevels` from Ready and `NotifyLevelsChanged`; a deleted channel drops its own, like mutes.
7. `conversations.ts`: `notifyLevel(s, ch)` (channel override, else server, else All) and `serverLevel`;
   `AlertInput.level`; `shouldAlert`: in a server channel `nothing` never alerts, `mentions` only on a mention, `all`
   on every message; mute still lets only mentions through; DND first. `store.svelte.ts` `maybeAlert` passes it (the
   sound and the desktop notification both go through `shouldAlert`; there are no message toasts), and its sound log
   names the level.
8. Menus: `MenuFlyout.svelte`, MuteFlyout's flyout made generic (items with an optional tick, as `StatusPicker`
   ticks: `menuitemradio` + `check` icon). `MuteFlyout` stays as a thin wrapper so its callers don't change.
   `ServerMenu`: "Notifications" flyout (All messages / Only @mentions / Nothing) under Mute. `ChannelMenu` (text,
   quotes, clips): "Notifications" flyout (Follow server / All messages / Only @mentions / Nothing). Two flyouts per
   menu: hovering one's trigger closes the other; the arrow keys and Escape close both. The server header menu has no
   mute, so nothing is added there.

## Tests (TDD)

- Server (`server/tests/dms.rs`): set/clear/read back via Ready; private per person; unknown target 404; DM 400;
  server All stored as default; unmuting keeps a level and clearing a level keeps a mute; owner-only event.
- Client: `conversations.test.ts` level table (all/mentions/nothing x mention x muted x dnd); `state.test.ts` Ready
  and event; `ServerMenu.test.ts`/`ChannelMenu.test.ts` flyout ticks and the invoke; `dnd.test.ts` allow list
  unchanged.
- Migration rehearsal on a scratch DB built through 0022 with mute rows.
