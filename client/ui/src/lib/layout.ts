import type { Category } from './protocol/Category'
import type { Channel } from './protocol/Channel'
import type { ChannelKind } from './protocol/ChannelKind'
import type { SaveLayoutRequest } from './protocol/SaveLayoutRequest'
import type { AppState } from './state'

// Side panels in a narrow window (#97): the window body (sidebar + chat + panels) below this many pixels counts as
// narrow, and only one side panel shows there.
export const NARROW_BODY = 1100

/** Whether the search results step aside: narrow, searching, and a replies panel (a quote's or a clip's) is open.
 *  They're hidden, not closed, so closing the replies panel brings them back as they were. */
export function searchMakesRoom(s: { narrow: boolean; searchOpen: boolean; threadOpen: boolean }): boolean {
  return s.narrow && s.searchOpen && s.threadOpen
}

// ---- The sidebar's channel list by categories (#54) ----

/** One heading and its channels. Live: a category (`key` `cat:<id>`, `kind` null). Legacy: a kind section
 *  (`key` `<serverId>:<kind>`, `category` null). */
export type Section = { key: string; category: Category | null; kind: ChannelKind | null; name: string; channels: Channel[] }
/** `live`: the server has categories. `top`: the uncategorised channels, shown above the first heading. */
export type SidebarLayout = { live: boolean; revision: number; top: Channel[]; sections: Section[] }
/** A list's order as ids: the uncategorised channels, then each category with its channels. */
export type Order = { top: string[]; cats: { id: string; channels: string[] }[] }

const byPosition = (a: { position: number; name: string | null }, b: { position: number; name: string | null }) =>
  a.position - b.position || (a.name ?? '').localeCompare(b.name ?? '')

/** The kind sections an older server's list shows, with their names, in order. */
const LEGACY: { kind: ChannelKind; name: string }[] = [
  { kind: 'text', name: 'Text Channels' },
  { kind: 'quotes', name: 'Quotes' },
  { kind: 'clips', name: 'Clips' },
  { kind: 'voice', name: 'Voice Channels' },
]

/** `serverId`'s channel list: uncategorised channels (and any whose category isn't known) on top, then each
 *  category; without a layout (an older server), the four kind sections, always all four. */
export function sidebarLayout(s: AppState, serverId: string): SidebarLayout {
  const mine = Object.values(s.channels).filter((c) => c.server_id === serverId).sort(byPosition)
  const held = s.layouts[serverId]
  if (!held) {
    const sections = LEGACY.map(({ kind, name }) => ({ key: `${serverId}:${kind}`, category: null, kind, name, channels: mine.filter((c) => c.kind === kind) }))
    return { live: false, revision: 0, top: [], sections }
  }
  const cats = [...held.categories].sort(byPosition)
  const known = new Set(cats.map((c) => c.id))
  return {
    live: true,
    revision: held.revision,
    top: mine.filter((c) => !c.category_id || !known.has(c.category_id)),
    sections: cats.map((category) => ({ key: `cat:${category.id}`, category, kind: null, name: category.name, channels: mine.filter((c) => c.category_id === category.id) })),
  }
}

/** The order `l` shows (legacy kind sections aren't categories, so they're not in it). */
export function orderOf(l: SidebarLayout): Order {
  return {
    top: l.top.map((c) => c.id),
    cats: l.sections.flatMap((x) => (x.category ? [{ id: x.category.id, channels: x.channels.map((c) => c.id) }] : [])),
  }
}

/** `l` re-sorted by `o` (a draft shown before the server has it). Channels and categories `o` doesn't name keep
 *  their place, after the ones it does; ids it names that `l` lacks are skipped. */
export function applyOrder(l: SidebarLayout, o: Order): SidebarLayout {
  const all = new Map([...l.top, ...l.sections.flatMap((x) => x.channels)].map((c) => [c.id, c]))
  const placed = new Set<string>()
  const take = (ids: string[]) =>
    ids.flatMap((id) => {
      const c = all.get(id)
      if (!c || placed.has(id)) return []
      placed.add(id)
      return [c]
    })
  const top = take(o.top)
  const byId = new Map(l.sections.flatMap((x) => (x.category ? [[x.category.id, x] as const] : [])))
  const named: { section: Section; channels: Channel[] }[] = []
  for (const oc of o.cats) {
    const section = byId.get(oc.id)
    if (!section || named.some((n) => n.section === section)) continue
    named.push({ section, channels: take(oc.channels) })
  }
  const rest = l.sections.filter((x) => !named.some((n) => n.section === x)).map((section) => ({ section, channels: [] as Channel[] }))
  const left = (cs: Channel[]) => cs.filter((c) => !placed.has(c.id))
  return {
    ...l,
    top: [...top, ...left(l.top)],
    sections: [...named, ...rest].map(({ section, channels }) => ({ ...section, channels: [...channels, ...left(section.channels)] })),
  }
}

/** The layout route's body for `o`, based on the layout `revision` held. */
export function toRequest(o: Order, revision: number): SaveLayoutRequest {
  return { revision, uncategorised: [...o.top], categories: o.cats.map((c) => ({ id: c.id, channels: [...c.channels] })) }
}

function insertAt<T>(list: T[], index: number, x: T): T[] {
  const i = Math.max(0, Math.min(index, list.length))
  return [...list.slice(0, i), x, ...list.slice(i)]
}

/** `channelId` moved to `index` of `to.category`'s list (null: the uncategorised area), counted without it; past
 *  the end is last. An unknown channel or category changes nothing. */
export function moveChannel(o: Order, channelId: string, to: { category: string | null; index: number }): Order {
  const known = o.top.includes(channelId) || o.cats.some((c) => c.channels.includes(channelId))
  if (!known || (to.category !== null && !o.cats.some((c) => c.id === to.category))) return o
  const without = (ids: string[]) => ids.filter((id) => id !== channelId)
  const top = without(o.top)
  return {
    top: to.category === null ? insertAt(top, to.index, channelId) : top,
    cats: o.cats.map((c) => {
      const channels = without(c.channels)
      return { id: c.id, channels: c.id === to.category ? insertAt(channels, to.index, channelId) : channels }
    }),
  }
}

/** `categoryId`, with its channels, moved to `index` among the categories. */
export function moveCategory(o: Order, categoryId: string, index: number): Order {
  const cat = o.cats.find((c) => c.id === categoryId)
  if (!cat) return o
  return { top: o.top, cats: insertAt(o.cats.filter((c) => c !== cat), index, cat) }
}

/** `channelId` one place up (-1) or down (1). At a list's edge it goes to the end of the previous list or the start
 *  of the next (top ↔ first category ↔ …); null at the very top or bottom, or for an unknown channel. */
export function stepChannel(o: Order, channelId: string, dir: -1 | 1): Order | null {
  const lists = [{ category: null as string | null, ids: o.top }, ...o.cats.map((c) => ({ category: c.id as string | null, ids: c.channels }))]
  const li = lists.findIndex((l) => l.ids.includes(channelId))
  if (li < 0) return null
  const within = lists[li].ids.indexOf(channelId) + dir
  if (within >= 0 && within < lists[li].ids.length) return moveChannel(o, channelId, { category: lists[li].category, index: within })
  const next = lists[li + dir]
  if (!next) return null
  return moveChannel(o, channelId, { category: next.category, index: dir < 0 ? next.ids.length : 0 })
}

/** `categoryId` one place up (-1) or down (1) among the categories; null at the ends or for an unknown one. */
export function stepCategory(o: Order, categoryId: string, dir: -1 | 1): Order | null {
  const i = o.cats.findIndex((c) => c.id === categoryId)
  if (i < 0 || !o.cats[i + dir]) return null
  return moveCategory(o, categoryId, i + dir)
}
