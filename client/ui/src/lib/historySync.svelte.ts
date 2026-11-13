// Keeping loaded messages right after a reconnect (#263). A Ready refetches a chat's latest page but not the older
// pages (or a jump's window) still loaded, and reactions, edits and deletes may have happened to them meanwhile: each
// chat with messages loaded is marked, and when it is next shown its loaded range is fetched again (the window around
// its oldest message, then forward pages up to its newest: one request per ~50 loaded) and set right in place, adding
// nothing, so the list doesn't move or flash. No server "changed since" call: the server keeps no change times for
// reactions or hard deletes, and the loaded range is small (a chat left is trimmed to its last 100).
import type { Event } from './protocol/Event'
import type { Message } from './protocol/Message'
import { applyRefreshed, PAGE } from './state'
import { app, onAppEvent, onAppReady } from './store.svelte'
import { api } from './tauri'

/** Chats whose loaded messages may be from before a reconnect. */
export const historySync = $state({ stale: {} as Record<string, true> })

/** channelId -> messages changed live while its range reloads (the fetched copies are older than those). */
const loading = new Map<string, Set<string>>()

/** After a Ready: every chat with messages loaded may be out of date. */
export function markHistoryStale(): void {
  const stale: Record<string, true> = {}
  for (const [c, list] of Object.entries(app.state.messages)) if (list.length) stale[c] = true
  historySync.stale = stale
}

/** A live event: a change to a message while its chat reloads wins over the fetched copy. */
export function noteMessageEvent(e: Event): void {
  const note = (c: string, id: string) => loading.get(c)?.add(id)
  switch (e.t) {
    case 'MessageUpdated':
      return void note(e.d.message.channel_id, e.d.message.id)
    case 'MessageDeleted':
      return void note(e.d.channel_id, e.d.message_id)
    case 'ReactionAdded':
    case 'ReactionRemoved':
    case 'QuoteAwardChanged':
    case 'MessageUnpinned':
    case 'PollChanged':
      return void note(e.d.channel_id, e.d.message_id)
    case 'MessagePinned':
      return void note(e.d.channel_id, e.d.pinned.message.id)
  }
}

onAppReady(() => markHistoryStale())
onAppEvent(noteMessageEvent)

/** An `around` page's newer share (the server's split of a full page). */
const AROUND_NEWER = PAGE - 1 - PAGE / 2

const sortedPage = (page: Message[]) => [...page].sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0))

/** Fetch `channel`'s loaded range again and set its messages by it. `apply` makes each change (the view passes one
 *  that keeps the message on screen still). One at a time per chat; a failure (offline) leaves the rest as it was,
 *  and the next Ready marks the chat again. */
export async function refreshHistory(channel: string, apply: (change: () => void) => void | Promise<void> = (f) => f()): Promise<void> {
  if (loading.has(channel)) return
  const touched = new Set<string>()
  loading.set(channel, touched)
  if (historySync.stale[channel]) {
    const { [channel]: _done, ...rest } = historySync.stale
    historySync.stale = rest
  }
  // The range as it is now: messages added after this (live, or the latest page) are fresh already.
  const ids = (app.state.messages[channel] ?? []).map((m) => m.id)
  const ours = new Set(ids)
  const newest = ids[ids.length - 1]
  const put = (page: Message[], inSpan: (id: string) => boolean) =>
    apply(() => (app.state = applyRefreshed(app.state, channel, page, (id) => ours.has(id) && inSpan(id), touched)))
  try {
    let i = 0 // the oldest of ours not yet covered
    let cursor: string | null = null // the newest covered, when the next of ours follows it directly
    while (i < ids.length) {
      // The latest page took the list's place (#329): nothing of ours is left to set right.
      if (!(app.state.messages[channel] ?? []).some((m) => ours.has(m.id))) break
      let page: Message[]
      let from: string
      let end: boolean
      if (cursor === null) {
        from = ids[i]
        const around = await api.listMessagesAround(channel, from)
        if (!around) {
          // gone for good (or moved into a thread): only it is dropped, the next starts a window of its own
          await put([], (id) => id === from)
          i++
          continue
        }
        page = sortedPage(around)
        end = page.filter((m) => m.id > from).length < AROUND_NEWER
      } else {
        from = cursor
        page = sortedPage(await api.listMessagesAfter(channel, cursor))
        end = page.length < PAGE
      }
      const last = page[page.length - 1]?.id ?? from
      // Reaching the latest covers the rest of ours (none newer than this page is still there); never beyond the range
      // taken above, so a live message newer than the page isn't taken for a removed one.
      const to = end && newest > last ? newest : last
      const lo = from
      const inclusive = cursor === null
      await put(page, (id) => (inclusive ? id >= lo : id > lo) && id <= to)
      if (end) break
      while (i < ids.length && ids[i] <= to) i++
      // Ours are contiguous: a page ending on one of ours goes on forward; one ending on a message we don't hold ran
      // into a gap, so the next of ours starts a window of its own.
      cursor = ours.has(last) ? last : null
    }
  } catch {
    // offline: what's left stays as it was
  } finally {
    loading.delete(channel)
  }
  // a Ready came while it loaded: the pages may be from before it
  if (historySync.stale[channel]) await refreshHistory(channel, apply)
}
