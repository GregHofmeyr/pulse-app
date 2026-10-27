import type { Event } from './protocol/Event'
import type { ReactionSummary } from './protocol/ReactionSummary'
import type { AppState } from './state'
import { api, errorText } from './tauri'

const MAX_NAMES = 10
/** The server's `list_reactors` page size. */
const REACTOR_CAP = 100

/** Whether picking `key` adds it: a plain pick always adds; a Shift (`keep`) pick toggles, so one you already have is removed. */
export const pickAdds = (reactions: ReactionSummary[], key: string, keep: boolean): boolean =>
  !keep || !reactions.some((r) => r.emoji === key && r.me)

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

/** The local flip applied before the server confirms; the next event's absolute count replaces it.
 * `onePerPerson` (a quote): adding also takes back my other reactions, as the server does. */
export function optimistic(list: ReactionSummary[], key: string, add: boolean, _meId: string, onePerPerson = false): ReactionSummary[] {
  const has = list.find((r) => r.emoji === key)
  if (add) {
    if (has?.me) return onePerPerson ? withoutMineExcept(list, key) : list
    if (onePerPerson) list = withoutMineExcept(list, key)
    if (!has) return [...list, { emoji: key, count: 1, me: true }]
    return list.map((r) => (r.emoji === key ? { ...r, count: r.count + 1, me: true } : r))
  }
  if (!has?.me) return list
  return list.map((r) => (r.emoji === key ? { ...r, count: r.count - 1, me: false } : r)).filter((r) => r.count > 0)
}

function withoutMineExcept(list: ReactionSummary[], key: string): ReactionSummary[] {
  if (!list.some((r) => r.me && r.emoji !== key)) return list
  return list.map((r) => (r.me && r.emoji !== key ? { ...r, count: r.count - 1, me: false } : r)).filter((r) => r.count > 0)
}

function flip(s: AppState, channel: string, id: string, key: string, add: boolean, meId: string, onePerPerson = false): AppState {
  const list = s.messages[channel]
  if (!list) return s
  return {
    ...s,
    messages: {
      ...s.messages,
      [channel]: list.map((m) => (m.id === id ? { ...m, reactions: optimistic(m.reactions, key, add, meId, onePerPerson) } : m)),
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

type Toggle = { add: boolean; landed: boolean }
// Per (message, emoji): the last queued toggle, and the one talking to the server now.
const queued = new Map<string, Promise<void>>()
const inFlight = new Map<string, Toggle>()
const toggleKey = (id: string, key: string) => `${id}\u0000${key}`

/** Feed every gateway event through here (before the reducer): it notes when the server has
 * already applied the toggle in flight, so a failed response doesn't undo a committed change. */
export function noteReactionEvent(e: Event, meId: string | undefined): void {
  if ((e.t !== 'ReactionAdded' && e.t !== 'ReactionRemoved') || e.d.user_id !== meId) return
  const t = inFlight.get(toggleKey(e.d.message_id, e.d.emoji))
  if (t && t.add === (e.t === 'ReactionAdded')) t.landed = true
}

/**
 * Flips locally, then asks the server; a failure is handed to `onError`. Toggles of one emoji on
 * one message run one at a time, in click order: a double click's add and remove can't reach the
 * server swapped. Each re-checks the state when its turn comes, so one that's become a no-op sends nothing.
 */
export function react(
  get: () => AppState, set: (s: AppState) => void,
  channel: string, id: string, key: string, add: boolean, meId: string, onError: (msg: string) => void, onePerPerson = false,
): Promise<void> {
  const k = toggleKey(id, key)
  // Each call settles: the core's HTTP client times requests out (15 s), and Tauri's invoke itself has
  // no timeout, so a core command that could hang would stall this emoji's later toggles.
  const run = (queued.get(k) ?? Promise.resolve()).catch(() => {}).then(() => toggle(get, set, channel, id, key, add, meId, onError, k, onePerPerson))
  queued.set(k, run)
  return run.finally(() => {
    if (queued.get(k) === run) queued.delete(k)
  })
}

/**
 * On failure the server's reactor list is the truth: an event may have landed during the call (even
 * an identical-looking one, e.g. the add committed but its response was lost), so decrementing
 * blindly could drift the count. If that list can't settle it (capped, or the fetch fails too), the
 * flip is undone locally, unless my own event for this toggle already arrived.
 */
async function toggle(
  get: () => AppState, set: (s: AppState) => void,
  channel: string, id: string, key: string, add: boolean, meId: string, onError: (msg: string) => void, k: string, onePerPerson: boolean,
): Promise<void> {
  const before = get()
  if ((entry(before, channel, id, key)?.me ?? false) === add) return
  set(flip(before, channel, id, key, add, meId, onePerPerson))
  const t: Toggle = { add, landed: false }
  inFlight.set(k, t)
  try {
    await (add ? api.addReaction(id, key) : api.removeReaction(id, key))
  } catch (e) {
    const at = entry(get(), channel, id, key)
    const undo = () => {
      if (!t.landed) set(flip(get(), channel, id, key, !add, meId))
    }
    try {
      const ids = await api.listReactors(id, key)
      const now = entry(get(), channel, id, key)
      // The server caps the list (oldest first): at the cap it can't give a count or my membership.
      if (ids.length >= REACTOR_CAP) undo()
      // An event landed during the fetch: its absolute count is newer than this snapshot.
      else if (now?.count === at?.count && now?.me === at?.me) {
        set(setEntry(get(), channel, id, key, ids.length ? { emoji: key, count: ids.length, me: ids.includes(meId) } : null))
      }
    } catch {
      undo()
    }
    onError(errorText(e))
  } finally {
    inFlight.delete(k)
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
