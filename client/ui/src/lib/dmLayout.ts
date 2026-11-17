// Your conversation list's layout (#305), the pure part: which conversations are pinned to the top, your own
// categories (in your order, each folded open or shut) and which conversations sit in each. Pinned wins: a pinned
// conversation in a category shows under Pinned. Within each part, newest first (as the list always was). Your
// notes (#308) stay on top of everything and are never placed.
import type { DmLayout } from './protocol/DmLayout'
import type { DmCategory } from './protocol/DmCategory'
import type { DmPlacement } from './protocol/DmPlacement'

export const DM_CATEGORY_CHARS = 32

export type DmSections<T> = {
  notes: T[]
  pinned: T[]
  categories: { category: DmCategory; items: T[] }[]
  rest: T[]
}

/** Split the list (already newest first) into its parts. */
export function dmSections<T extends { channel: { id: string }; notes: boolean }>(list: T[], layout: DmLayout): DmSections<T> {
  const place = new Map(layout.placements.map((p) => [p.channel_id, p]))
  const known = new Set(layout.categories.map((c) => c.id))
  const out: DmSections<T> = { notes: [], pinned: [], categories: layout.categories.map((category) => ({ category, items: [] })), rest: [] }
  const byCat = new Map(out.categories.map((c) => [c.category.id, c.items]))
  for (const c of list) {
    const p = place.get(c.channel.id)
    if (c.notes) out.notes.push(c)
    else if (p?.pinned) out.pinned.push(c)
    else if (p?.category_id && known.has(p.category_id)) byCat.get(p.category_id)!.push(c)
    else out.rest.push(c)
  }
  return out
}

/** A conversation's placement, or none. */
export const placementOf = (layout: DmLayout, channelId: string): DmPlacement | null =>
  layout.placements.find((p) => p.channel_id === channelId) ?? null

/** Names no category may have (ignoring case, and "..." for "…"): the Move to category flyout's own entries (#419).
 *  The server's `dm_layout_limits::RESERVED_NAMES`, which refuses them too. */
export const RESERVED_CATEGORY_NAMES = ['New category…', 'Remove from category']

/** Whether `name` is one of [RESERVED_CATEGORY_NAMES]. */
export function categoryNameReserved(name: string): boolean {
  const n = name.trim().replaceAll('...', '…').toLowerCase()
  return RESERVED_CATEGORY_NAMES.some((r) => r.toLowerCase() === n)
}

/** Name check for a category: 1-32 characters after trimming, and not a menu entry's name. */
export function categoryNameOk(name: string): boolean {
  const n = [...name.trim()].length
  return n > 0 && n <= DM_CATEGORY_CHARS && !categoryNameReserved(name)
}
