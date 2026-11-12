// The inbox (#306), as pure functions: where an item is, its preview with your mention picked out, and how a list of
// items follows the events that change it. The reactive store is inbox.svelte.ts; the popover InboxPopover.svelte.
import { mentionSpans } from './composerDom'
import { conversationName } from './conversations'
import type { AppState } from './state'
import type { Event } from './protocol/Event'
import type { InboxItem } from './protocol/InboxItem'

export type InboxTab = 'all' | 'mentions'

/** Items per page (the server's). */
export const INBOX_PAGE = 30

const TAB_KEY = 'pulse.inbox.tab'

/** The tab last picked on this device (All when none, or storage is out of reach). */
export function loadTab(): InboxTab {
  try {
    return localStorage.getItem(TAB_KEY) === 'mentions' ? 'mentions' : 'all'
  } catch {
    return 'all'
  }
}

export function saveTab(tab: InboxTab): void {
  try {
    localStorage.setItem(TAB_KEY, tab)
  } catch {
    /* remembered for this session only */
  }
}

/** Where an item is: `Server › #channel` (a voice channel's chat by its name), a group's name, or a DM; a reply adds
 *  `› replies`. A chat no longer known here reads `a deleted chat`. */
export function whereLabel(s: AppState, item: InboxItem): string {
  const m = item.message
  const ch = s.channels[m.channel_id]
  let place: string
  if (!ch) place = 'a deleted chat'
  else if (ch.server_id) {
    const server = s.servers.find((x) => x.id === ch.server_id)?.name ?? 'a server'
    place = `${server} › ${ch.kind === 'voice' ? (ch.name ?? '') : `#${ch.name ?? ''}`}`
  } else if (ch.kind === 'group') place = conversationName(s, ch.id)
  else place = 'Direct message'
  return m.thread_root_id ? `${place} › replies` : place
}

export type Part = { text: string; me: boolean }

/** `text` (already one line) in pieces, with each `@mention` of `meId` marked, written as `@` + `label` of its person
 *  (every known mention shows the name you see, never the username, #47). Neighbouring plain text joins up. */
export function highlightParts(text: string, lookup: (name: string) => string | null, label: (userId: string) => string, meId: string | null): Part[] {
  const out: Part[] = []
  const add = (t: string, me: boolean) => {
    if (!t) return
    const last = out[out.length - 1]
    if (last && last.me === me && !me) last.text += t
    else out.push({ text: t, me })
  }
  let at = 0
  for (const span of mentionSpans(text)) {
    const id = lookup(span.username)
    if (!id) continue
    add(text.slice(at, span.start), false)
    add(`@${label(id)}`, id === meId)
    at = span.end
  }
  add(text.slice(at), false)
  return out
}

/** One line: whitespace runs (newlines included) become one space. */
export const oneLine = (text: string) => text.replace(/\s+/g, ' ').trim()

/** `items` with those `pick` matches marked read (the same array when nothing changes). */
export function markRead(items: InboxItem[], pick: (i: InboxItem) => boolean): InboxItem[] {
  let changed = false
  const next = items.map((i) => {
    if (i.read || !pick(i)) return i
    changed = true
    return { ...i, read: true }
  })
  return changed ? next : items
}

/** A list of items (one tab's) after `e`, or the same array when it doesn't touch it. `tab` decides whether a new item
 *  belongs. A new item goes on top (a list is newest first; one already there isn't added again). */
export function applyToList(items: InboxItem[], e: Event, tab: InboxTab): InboxItem[] {
  switch (e.t) {
    case 'InboxItemAdded': {
      const item = e.d.item
      if (tab === 'mentions' && item.kind !== 'mention') return items
      if (items.some((i) => i.message.id === item.message.id)) return items
      return [item, ...items]
    }
    case 'InboxUpdated': {
      const { read, read_upto, removed } = e.d
      let next = items
      if (removed.length) next = next.filter((i) => !removed.includes(i.message.id))
      if (read.length) next = markRead(next, (i) => read.includes(i.message.id))
      if (read_upto) next = markRead(next, (i) => i.message.id <= read_upto)
      return next
    }
    case 'ReadStateUpdated': {
      const { channel_id, last_read_message_id: upto } = e.d
      if (!upto) return items
      return markRead(items, (i) => i.message.channel_id === channel_id && !i.message.thread_root_id && i.message.id <= upto)
    }
    case 'ThreadSeen': {
      const { root_id, last_seen_id } = e.d
      return markRead(items, (i) => i.message.thread_root_id === root_id && i.message.id <= last_seen_id)
    }
    default:
      return items
  }
}

/** Items newest first, with `more` (an older page) after them, dropping any already listed. */
export function appendPage(items: InboxItem[], more: InboxItem[]): InboxItem[] {
  const have = new Set(items.map((i) => i.message.id))
  return [...items, ...more.filter((i) => !have.has(i.message.id))]
}

/** The badge's text: the count, `99+` past 99. */
export const badgeText = (n: number) => (n > 99 ? '99+' : String(n))
