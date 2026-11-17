// Hide what I'm playing per person, and Invite to game (#436), the client side.
//
// Hiding is the server's to enforce (a hidden person's clients never get your game); here each change asks it, and
// its answer (your whole list, also sent to your other devices as ActivityHiddenChanged) replaces ours.
//
// An invite is a message with a card (`game_invite`); its GameInviteChanged event puts it in place (lib/state.ts).
// Where to join is never in the message: Join asks the server for it (only a member of the chat gets it) and opens
// Steam with the same checks as any Join.
import type { AppState } from './state'
import type { Message } from './protocol/Message'
import { applyEvent } from './state'
import { app, openDm, ui } from './store.svelte'
import { api, errorText } from './tauri'

/** Whether you hide what you're playing from `userId`. */
export const hidesGameFrom = (s: AppState, userId: string): boolean => s.activityHidden.includes(userId)

/** Whether you're in a game now, as your own "Playing" says (the game detection on this or another device). */
export const inGame = (s: AppState): boolean => !!(s.me && s.people[s.me.id]?.playing)

/** Hide what you're playing from `userId` (`on`), or show it again. */
export async function setActivityHidden(userId: string, on: boolean): Promise<void> {
  const list = await api.setActivityHidden(userId, on)
  app.state = { ...app.state, activityHidden: list }
}

/** Invite the DM or group `channelId` to the game you're playing; it shows at once (its MessageCreated is the same).
 *  Rejects with the server's reason ("You're not in a game right now."). */
export async function sendGameInvite(channelId: string): Promise<Message> {
  const m = await api.sendGameInvite(channelId)
  app.state = applyEvent(app.state, { t: 'MessageCreated', d: { message: m, nonce: null } }, Date.now())
  return m
}

/** "Invite to game…" in a person's menu: into your DM with them (made if needed), then there. Failures show as the
 *  app's notice. */
export async function invitePerson(userId: string): Promise<void> {
  try {
    const channelId = await openDm(userId)
    await sendGameInvite(channelId)
    ui.goTo = channelId
  } catch (e) {
    ui.notice = errorText(e)
  }
}

/** Join an invite's game: where, from the server, then Steam. */
export async function joinGameInvite(messageId: string): Promise<void> {
  const join = await api.gameInviteJoin(messageId)
  await api.joinSteamGame(join)
}
