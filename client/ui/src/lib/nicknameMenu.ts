// The nickname item a person's menu offers (#113, reshaped by #137): the one nickname that applies where the menu
// opens. In a server that's their server nickname; outside servers (the DM list) your private one for them. Reset
// and Clear live in the nickname dialog, not in menus; the profile card has no nickname buttons.
import type { AppState } from './state'
import { isMember } from './selectors'

export type NickItemId = 'nick' | 'private-nick'

/** A server nickname needs you and them in that server; anyone there may set anyone's, their own included. */
export function canServerNick(s: AppState, serverId: string | null, userId: string): serverId is string {
  return !!serverId && isMember(s, serverId) && (s.members[serverId] ?? []).some((m) => m.user.id === userId)
}

/** `serverId`: the server the menu opens in, or null outside servers. In a server: change their server nickname
 *  (yours too). Outside: change your private nickname for them (never for yourself). */
export function nickItems(s: AppState, serverId: string | null, userId: string): NickItemId[] {
  if (serverId) return canServerNick(s, serverId, userId) ? ['nick'] : []
  return userId !== s.me?.id ? ['private-nick'] : []
}

export function nickLabel(id: NickItemId): string {
  return id === 'nick' ? 'Change server nickname' : 'Change nickname'
}
