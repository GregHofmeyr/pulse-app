// The nickname actions (#48) a person's menu offers (#113): the profile card's rules, in one place.
import type { AppState } from './state'
import { isMember, serverNick } from './selectors'

export type NickItemId = 'nick' | 'nick-reset' | 'private-nick' | 'private-nick-clear'

/** A server nickname needs you and them in that server; anyone there may set anyone's, their own included. */
export function canServerNick(s: AppState, serverId: string | null, userId: string): serverId is string {
  return !!serverId && isMember(s, serverId) && (s.members[serverId] ?? []).some((m) => m.user.id === userId)
}

/** Change / reset their server nickname (reset only when one is set), then set / clear your private one
 *  for them (never for yourself; clear only when you gave one). */
export function nickItems(s: AppState, serverId: string | null, userId: string): NickItemId[] {
  const out: NickItemId[] = []
  if (canServerNick(s, serverId, userId)) {
    out.push('nick')
    if (serverNick(s, serverId, userId)) out.push('nick-reset')
  }
  if (userId !== s.me?.id) {
    out.push('private-nick')
    if (s.privateNicks[userId]) out.push('private-nick-clear')
  }
  return out
}

/** `hasPrivate`: you've already given them a private nickname. */
export function nickLabel(id: NickItemId, hasPrivate: boolean): string {
  switch (id) {
    case 'nick': return 'Change nickname'
    case 'nick-reset': return 'Reset nickname'
    case 'private-nick': return hasPrivate ? 'Change private nickname…' : 'Set private nickname…'
    case 'private-nick-clear': return 'Clear private nickname'
  }
}
