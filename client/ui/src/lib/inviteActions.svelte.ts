// Server invites (#80), what the app does with them: send a direct invite (it arrives as a card in your DM with them),
// and join with one (then go to the server).
import { noteInviteState } from './invitePreviews.svelte'
import { stateFromError, type CardState } from './serverInvites'
import { app, ui } from './store.svelte'
import { api, errorText } from './tauri'

/** A direct invite to `serverId` for `userId`. Rejects with the server's reason. */
export async function sendDirectInvite(serverId: string, userId: string): Promise<void> {
  await api.createServerInvite(serverId, { for_user: userId })
}

/** "Invite to server ›" in a person's menu: the invite, then a toast saying so (or why not). */
export async function invitePersonTo(serverId: string, userId: string): Promise<void> {
  try {
    await sendDirectInvite(serverId, userId)
    ui.notice = 'Invite sent'
  } catch (e) {
    ui.notice = errorText(e)
  }
}

/** Join with invite `code`, then go to its server. A refusal that says what the invite is now (expired, used, not
 *  found…) puts that on its card and resolves with that state; any other failure rejects with its words. */
export async function acceptInvite(code: string): Promise<CardState | null> {
  try {
    const server = await api.acceptInvite(code)
    noteInviteState(code, 'already_member')
    if (!app.state.servers.some((s) => s.id === server.id)) app.state = { ...app.state, servers: [...app.state.servers, server] }
    ui.goServer = server.id
    return null
  } catch (e) {
    const state = stateFromError(errorText(e))
    if (!state) throw e
    noteInviteState(code, state)
    return state
  }
}
