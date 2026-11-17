// A group chat's own system lines (#114): "alex added sam and kim", written with the names that apply today (display
// names, nicknames: `displayName`) from the ids the server sends, not the names it wrote at the time.
import type { AppState } from './state'
import type { Message } from './protocol/Message'
import { displayName, userOf } from './selectors'
import { joinNames } from './sports'

/** The line `m` reads as: from its `group_line` when it has one and everyone in it is someone we know, else its stored
 *  text (an older server's line, or someone who left before this client met them). */
export function groupLineText(s: AppState, m: Message): string {
  const g = m.group_line
  if (!g) return m.content
  const known = (id: string) => id === s.me?.id || !!userOf(s, id) || !!s.privateNicks[id]
  if (![g.by, ...g.users].every(known)) return m.content
  const name = (id: string) => displayName(s, null, id)
  const by = name(g.by)
  const users = joinNames(g.users.map(name))
  switch (g.kind) {
    case 'created':
      return `${by} created the group with ${users}`
    case 'members_added':
      return `${by} added ${users}`
    case 'member_left':
      return `${by} left`
    case 'member_removed':
      return `${by} removed ${users}`
    case 'renamed':
      return g.name ? `${by} renamed the group to ${g.name}` : m.content
    case 'name_removed':
      return `${by} removed the group name`
    case 'icon_changed':
      return `${by} changed the group icon`
    case 'icon_removed':
      return `${by} removed the group icon`
    default:
      return m.content
  }
}

/** A server membership line (#418), as `groupLineText` writes a group's: "alex left the server" / "sam was removed",
 *  with the name that applies in `serverId` today; its stored text when the client doesn't know them (someone who left
 *  before this client met them). */
export function memberLineText(s: AppState, m: Message, serverId: string | null): string {
  const l = m.member_line
  if (!l) return m.content
  const known = l.user_id === s.me?.id || !!userOf(s, l.user_id) || !!s.privateNicks[l.user_id]
  if (!known) return m.content
  const name = displayName(s, serverId, l.user_id)
  return l.removed_by ? `${name} was removed` : `${name} left the server`
}
