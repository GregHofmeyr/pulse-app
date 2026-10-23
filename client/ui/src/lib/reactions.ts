import type { ReactionSummary } from './protocol/ReactionSummary'
import type { AppState } from './state'
import { api, errorText } from './tauri'

const MAX_NAMES = 10

/** "A, B and C"; past ten names "A, B, … J and 3 others". `total` counts everyone, named or not. */
export function tooltipText(names: string[], total: number): string {
  const others = total - Math.min(names.length, MAX_NAMES)
  if (names.length > MAX_NAMES || others > 0) {
    const shown = names.slice(0, MAX_NAMES).join(', ')
    return others > 0 ? `${shown} and ${others} other${others === 1 ? '' : 's'}` : shown
  }
  if (names.length <= 1) return names[0] ?? ''
  return `${names.slice(0, -1).join(', ')} and ${names[names.length - 1]}`
}

/** The local flip applied before the server confirms; the next event's absolute count replaces it. */
export function optimistic(list: ReactionSummary[], key: string, add: boolean, _meId: string): ReactionSummary[] {
  const has = list.find((r) => r.emoji === key)
  if (add) {
    if (has?.me) return list
    if (!has) return [...list, { emoji: key, count: 1, me: true }]
    return list.map((r) => (r.emoji === key ? { ...r, count: r.count + 1, me: true } : r))
  }
  if (!has?.me) return list
  return list.map((r) => (r.emoji === key ? { ...r, count: r.count - 1, me: false } : r)).filter((r) => r.count > 0)
}

function flip(s: AppState, channel: string, id: string, key: string, add: boolean, meId: string): AppState {
  const list = s.messages[channel]
  if (!list) return s
  return {
    ...s,
    messages: {
      ...s.messages,
      [channel]: list.map((m) => (m.id === id ? { ...m, reactions: optimistic(m.reactions, key, add, meId) } : m)),
    },
  }
}

const entry = (s: AppState, channel: string, id: string, key: string) =>
  s.messages[channel]?.find((m) => m.id === id)?.reactions.find((r) => r.emoji === key)

function setEntry(s: AppState, channel: string, id: string, key: string, e: ReactionSummary | null): AppState {
  const list = s.messages[channel]
  if (!list) return s
  const put = (rs: ReactionSummary[]) => {
    const rest = rs.filter((r) => r.emoji !== key)
    if (!e) return rest
    return rs.some((r) => r.emoji === key) ? rs.map((r) => (r.emoji === key ? e : r)) : [...rest, e]
  }
  return { ...s, messages: { ...s.messages, [channel]: list.map((m) => (m.id === id ? { ...m, reactions: put(m.reactions) } : m)) } }
}

/**
 * Flips locally, then asks the server; a failure is handed to `onError`. On failure the server's
 * reactor list is the truth: an event may have landed during the call (even an identical-looking one,
 * e.g. the add committed but its response was lost), so decrementing blindly could drift the count.
 * Only if that fetch fails too is the flip undone locally.
 */
export async function react(
  get: () => AppState, set: (s: AppState) => void,
  channel: string, id: string, key: string, add: boolean, meId: string, onError: (msg: string) => void,
): Promise<void> {
  const before = get()
  if ((entry(before, channel, id, key)?.me ?? false) === add) return
  set(flip(before, channel, id, key, add, meId))
  try {
    await (add ? api.addReaction(id, key) : api.removeReaction(id, key))
  } catch (e) {
    try {
      const ids = await api.listReactors(id, key)
      set(setEntry(get(), channel, id, key, ids.length ? { emoji: key, count: ids.length, me: ids.includes(meId) } : null))
    } catch {
      set(flip(get(), channel, id, key, !add, meId))
    }
    onError(errorText(e))
  }
}

/** Fixed position for a popover next to `r`: below if it fits, else above; inside the viewport. */
export function popoverStyle(
  r: { left: number; top: number; bottom: number; right: number },
  size: { width: number; height: number },
  vp: { width: number; height: number },
): string {
  const gap = 6
  const top = r.bottom + gap + size.height <= vp.height ? r.bottom + gap : Math.max(gap, r.top - gap - size.height)
  const left = Math.min(Math.max(r.left, gap), Math.max(gap, vp.width - size.width - gap))
  return `position: fixed; top: ${Math.round(top)}px; left: ${Math.round(left)}px; z-index: 50;`
}

/** Servers you are a member of: which custom emoji the picker may offer. */
export function memberServers(s: AppState): Set<string> {
  const me = s.me?.id
  return new Set(s.servers.filter((x) => (s.members[x.id] ?? []).some((m) => m.user.id === me)).map((x) => x.id))
}
