// The builds browse view's right-click menu and multi-select (#617): which menu items a build offers, the selection's
// toggles and ranges, and the selection bar's actions. Pure; BuildsChannel holds the state.
import { buildRights, isArchived } from './builds'
import type { AppState } from './state'
import type { Message } from './protocol/Message'
import type { IconName } from '../components/Icon.svelte'

type Rights = ReturnType<typeof buildRights>

/** A build's right-click menu items, in order. */
export type BuildMenuItem = 'open' | 'edit' | 'archive' | 'restore' | 'delete' | 'select'

/** Edit for its author; Archive (or Restore) and Delete for its author or the server's owner, as the build page's
 *  Actions menu (`buildRights`); Open and Select for anyone. */
export function buildMenuItems(r: Rights, archived: boolean): BuildMenuItem[] {
  return [
    'open',
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

/** One of the selection bar's actions. To add one (Favourite): an entry here and its call in BuildsChannel's `RUN`. */
export type BulkAction = {
  key: 'archive' | 'restore' | 'delete'
  label: string
  /** The past tense, for what's said afterwards ("Archived 3 builds."). */
  done: string
  /** Whether it does anything to this build (Archive: a live one; Restore: an archived one). */
  applies: (m: Message) => boolean
  /** Whether you may do it to a build it applies to. */
  allowed: (r: Rights) => boolean
  /** Asks first, with the count. */
  confirm?: boolean
}

export const BULK_ACTIONS: readonly BulkAction[] = [
  { key: 'archive', label: 'Archive', done: 'Archived', applies: (m) => !isArchived(m), allowed: (r) => r.archive },
  { key: 'restore', label: 'Restore', done: 'Restored', applies: (m) => isArchived(m), allowed: (r) => r.archive },
  { key: 'delete', label: 'Delete', done: 'Deleted', applies: () => true, allowed: (r) => r.delete, confirm: true },
]

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
export function bulkSummary(done: string, n: number, skipped: number): string {
  return `${done} ${builds(n)}.${skipped ? ` ${skipped} skipped: they aren't yours to change.` : ''}`
}

/** An action as the selection bar draws it. */
export type SelectBarAction = { key: string; label: string; icon: IconName; disabled: boolean; danger?: boolean; run: () => void }
