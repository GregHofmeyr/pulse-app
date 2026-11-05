// The nickname items a person's menu offers (#113, reshaped by #137). In a server: their server nickname, then your
// private nickname for them. Outside servers (the DM list, a group call) only your private one. Reset and Clear live
// in the nickname dialog, not in menus; the profile card has no nickname buttons (owner: every action between people
// lives in the right-click menu), its own right-click menu offers these items instead.
import type { AppState } from './state'
import { isMember } from './selectors'

export type NickItemId = 'nick' | 'private-nick'

/** A server nickname needs you and them in that server; anyone there may set anyone's, their own included. */
export function canServerNick(s: AppState, serverId: string | null, userId: string): serverId is string {
  return !!serverId && isMember(s, serverId) && (s.members[serverId] ?? []).some((m) => m.user.id === userId)
}

/** `serverId`: the server the menu opens in, or null outside servers. Their server nickname where it applies (yours
 *  too), then your private nickname for them (never for yourself). */
export function nickItems(s: AppState, serverId: string | null, userId: string): NickItemId[] {
  const out: NickItemId[] = canServerNick(s, serverId, userId) ? ['nick'] : []
  if (userId !== s.me?.id) out.push('private-nick')
  return out
}

/** Beside a server nickname the private one says so, and whether you have one; outside servers it's just "nickname". */
export function nickLabel(s: AppState, serverId: string | null, userId: string, id: NickItemId): string {
  if (id === 'nick') return 'Change server nickname'
  if (!serverId) return 'Change nickname'
  return s.privateNicks[userId] ? 'Change private nickname' : 'Set private nickname'
}
