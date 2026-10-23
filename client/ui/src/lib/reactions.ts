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

/** Flips locally, then asks the server; a failure undoes the flip and is handed to `onError`. */
export async function react(
  get: () => AppState, set: (s: AppState) => void,
  channel: string, id: string, key: string, add: boolean, meId: string, onError: (msg: string) => void,
): Promise<void> {
  const before = get()
  const had = before.messages[channel]?.find((m) => m.id === id)?.reactions.find((r) => r.emoji === key)?.me ?? false
  if (had === add) return
  set(flip(before, channel, id, key, add, meId))
  try {
    await (add ? api.addReaction(id, key) : api.removeReaction(id, key))
  } catch (e) {
    set(flip(get(), channel, id, key, !add, meId))
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
