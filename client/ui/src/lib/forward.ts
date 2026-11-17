// Forwarding a message (#302), the pure parts: the chats you can forward to (servers' text and voice chats you're a
// member of, your DMs and groups), how the picker orders them (recent chats first with nothing typed, then the
// switcher's ranking), and how a forward names where it came from (a server channel by name, never a DM or group).
import { placesFrom, rankPlaces, recentPlaces, type Place } from './quickSwitch'
import { memberServers } from './reactions'
import type { AppState } from './state'
import type { Forwarded } from './protocol/Forwarded'

const TARGET_KINDS = new Set<Place['kind']>(['text', 'voice', 'dm', 'group'])
const KIND_ORDER: Record<string, number> = { dm: 0, group: 1, text: 2, voice: 3 }

/** Where you can post a forward: a server's text or voice chat (a member's: the server refuses anyone else), a DM
 *  or a group (not a closed one). Quotes and clips channels take only their own kind. */
export function forwardTargets(s: AppState): Place[] {
  const member = memberServers(s)
  return placesFrom(s).filter((p) => TARGET_KINDS.has(p.kind) && (!p.serverId || member.has(p.serverId)))
}

/** The picker's list: with nothing typed, your recent chats (the switcher's recents) then the rest, conversations
 *  first, by name; with a query, the switcher's ranking (recents lifted). */
export function forwardList(targets: readonly Place[], query: string, recents: readonly string[]): Place[] {
  if (query.trim()) return rankPlaces(targets, query, recents, 100)
  const recent = recentPlaces(targets, recents, null)
  const seen = new Set(recent.map((p) => p.key))
  const rest = targets
    .filter((p) => !seen.has(p.key))
    .sort((a, b) => KIND_ORDER[a.kind] - KIND_ORDER[b.kind] || (a.context ?? '').localeCompare(b.context ?? '') || a.name.localeCompare(b.name))
  return [...recent, ...rest]
}

/** How a target reads in the picker: `#general` with its server beside it, a group's or DM's name. */
export const targetLabel = (p: Place) => (p.kind === 'text' ? `#${p.name}` : p.name)

/** Where a forward came from, as its header says it: `#general in Main` (a voice chat by its name), or just "a DM" /
 *  "a group": a conversation is never named. */
export function forwardedFrom(s: AppState, f: Forwarded): string {
  if (f.channel_kind === 'dm') return 'a DM'
  if (f.channel_kind === 'group') return 'a group'
  const name = f.channel_name ?? 'a channel'
  const shown = f.channel_kind === 'voice' ? name : `#${name}`
  const server = f.server_id ? s.servers.find((x) => x.id === f.server_id)?.name : null
  return server ? `${shown} in ${server}` : shown
}

/** Whether you can open the original: its chat is one you can see here. */
export const canOpenOriginal = (s: AppState, f: Forwarded) => !f.unavailable && !!s.channels[f.channel_id]
