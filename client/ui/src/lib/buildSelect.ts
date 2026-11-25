// The builds browse view's right-click menu and multi-select (#617): which menu items a build offers, the selection's
// toggles and ranges, and the selection bar's actions. Pure; BuildsChannel holds the state.
import { buildRights, isArchived, isFavourite } from './builds'
import type { AppState } from './state'
import type { Message } from './protocol/Message'
import type { IconName } from '../components/Icon.svelte'

type Rights = ReturnType<typeof buildRights>

/** A build's right-click menu items, in order. */
export type BuildMenuItem = 'open' | 'favourite' | 'unfavourite' | 'edit' | 'archive' | 'restore' | 'delete' | 'select'

/** Edit for its author; Archive (or Restore) and Delete for its author or the server's owner, as the build page's
 *  Actions menu (`buildRights`); Open, Favourite (or Unfavourite: your own star) and Select for anyone. */
export function buildMenuItems(r: Rights, archived: boolean, favourite: boolean): BuildMenuItem[] {
  return [
    'open',
    favourite ? 'unfavourite' : 'favourite',
    ...(r.edit ? (['edit'] as const) : []),
    ...(r.archive ? [archived ? ('restore' as const) : ('archive' as const)] : []),
    ...(r.delete ? (['delete'] as const) : []),
    'select',
  ]
}

/** `id` in or out of the selection. */
export function toggleSelected(sel: string[], id: string): string[] {
  return sel.includes(id) ? sel.filter((x) => x !== id) : [...sel, id]
}

/** Shift+click: every build from `anchor` to `id` in `order` (as shown) added to the selection; without an anchor
 *  still shown, just `id`. */
export function rangeSelected(sel: string[], order: string[], anchor: string | null, id: string): string[] {
  const from = anchor === null ? -1 : order.indexOf(anchor)
  const to = order.indexOf(id)
  const span = from < 0 || to < 0 ? [id] : order.slice(Math.min(from, to), Math.max(from, to) + 1)
  const have = new Set(sel)
  return [...sel, ...span.filter((x) => !have.has(x))]
}

/** One of the selection bar's actions. To add one: an entry here and its call in BuildsChannel's `RUN`. */
export type BulkAction = {
  key: 'favourite' | 'unfavourite' | 'archive' | 'restore' | 'delete'
  label: string
  /** What's said afterwards, given "3 builds". */
  done: (what: string) => string
  /** Why the server skipped some (a build gone meanwhile, or one not yours). */
  skipWhy: string
  /** Whether it does anything to this build (Archive: a live one; Restore: an archived one). */
  applies: (m: Message) => boolean
  /** Whether you may do it to a build it applies to. */
  allowed: (r: Rights) => boolean
  /** Shown (disabled) with nothing selected; the others show once a selected build needs them. */
  whenNone: boolean
  /** Asks first, with the count. */
  confirm?: boolean
}

const NOT_YOURS = "they aren't yours to change"
export const BULK_ACTIONS: readonly BulkAction[] = [
  // a star is your own: any build may have it
  { key: 'favourite', label: 'Favourite', done: (w) => `Added ${w} to your favourites.`, skipWhy: 'gone meanwhile', applies: (m) => !isFavourite(m), allowed: () => true, whenNone: true },
  { key: 'unfavourite', label: 'Unfavourite', done: (w) => `Removed ${w} from your favourites.`, skipWhy: 'gone meanwhile', applies: isFavourite, allowed: () => true, whenNone: false },
  { key: 'archive', label: 'Archive', done: (w) => `Archived ${w}.`, skipWhy: NOT_YOURS, applies: (m) => !isArchived(m), allowed: (r) => r.archive, whenNone: true },
  { key: 'restore', label: 'Restore', done: (w) => `Restored ${w}.`, skipWhy: NOT_YOURS, applies: isArchived, allowed: (r) => r.archive, whenNone: false },
  { key: 'delete', label: 'Delete', done: (w) => `Deleted ${w}.`, skipWhy: NOT_YOURS, applies: () => true, allowed: (r) => r.delete, whenNone: true, confirm: true },
]

/** The actions the bar shows for a selection: each that applies to a selected build; with none selected, the base
 *  ones (Favourite, Archive, Delete). */
export function barActions(selected: Message[]): BulkAction[] {
  return BULK_ACTIONS.filter((a) => (selected.length ? selected.some(a.applies) : a.whenNone))
}
export const barKeys = (selected: Message[]) => barActions(selected).map((a) => a.key)

/** What an action does to the selection: the builds it applies to that you may change (sent), and those it applies to
 *  that you may not (skipped, and said so). Builds it doesn't apply to aren't counted either way. */
export function bulkPlan(s: AppState, serverId: string, selected: Message[], a: BulkAction): { ids: string[]; skipped: number } {
  const ids: string[] = []
  let skipped = 0
  for (const m of selected) {
    if (!a.applies(m)) continue
    if (a.allowed(buildRights(s, serverId, m))) ids.push(m.id)
    else skipped++
  }
  return { ids, skipped }
}

/** The server takes at most this many builds per batch. */
export const BATCH_MAX = 100

export function batchChunks(ids: string[], n = BATCH_MAX): string[][] {
  const out: string[][] = []
  for (let i = 0; i < ids.length; i += n) out.push(ids.slice(i, i + n))
  return out
}

const builds = (n: number) => (n === 1 ? '1 build' : `${n} builds`)

/** "Archived 3 builds." and, when some were skipped, why. */
export function bulkSummary(a: BulkAction, n: number, skipped: number): string {
  return `${a.done(builds(n))}${skipped ? ` ${skipped} skipped: ${a.skipWhy}.` : ''}`
}

/** An action as the selection bar draws it. */
export type SelectBarAction = { key: string; label: string; icon: IconName; disabled: boolean; danger?: boolean; run: () => void }
