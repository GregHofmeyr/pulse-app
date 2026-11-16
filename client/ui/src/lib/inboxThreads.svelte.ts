// The inbox's Threads tab (#326): quote and clip threads you started or replied in that have replies by others you
// haven't seen (the server works it out: lib's GET /inbox/threads). Fetched on every Ready, then again (a moment
// later, many changes making one fetch) when a reply lands in a thread you're watching, your seen point moves, or a
// mute or delete may have changed it. Each thread counts once toward the inbox button's badge until you look.
import type { Event } from './protocol/Event'
import type { InboxThread } from './protocol/InboxThread'
import { openMessage } from './openMessage'
import { app, onAppEvent, onAppReady, ui } from './store.svelte'
import { api, errorText } from './tauri'

export const threads = $state({
  list: [] as InboxThread[],
  /** Every thread you're part of (its root id). */
  watching: [] as string[],
  loaded: false,
  error: '',
})

/** How long changes gather before one fetch. */
export const REFETCH_MS = 800

let epoch = 0
let timer: ReturnType<typeof setTimeout> | null = null

export async function refreshThreads(): Promise<void> {
  if (timer) clearTimeout(timer)
  timer = null
  const mine = ++epoch
  try {
    const t = await api.inboxThreads()
    if (mine !== epoch) return
    threads.list = t.threads
    threads.watching = t.watching
    threads.loaded = true
    threads.error = ''
  } catch (e) {
    if (mine === epoch) threads.error = errorText(e)
  }
}

function soon() {
  if (timer) clearTimeout(timer)
  timer = setTimeout(() => void refreshThreads(), REFETCH_MS)
}

const drop = (rootId: string) => {
  threads.list = threads.list.filter((t) => t.root.message_id !== rootId)
}

/** Open a thread at its newest reply (its quote's replies, or its clip's panel) and count it seen. */
export function openThread(t: InboxThread, close: () => void): void {
  close()
  drop(t.root.message_id)
  api.markRepliesSeen(t.root.message_id, t.latest.id).catch(() => soon())
  if (!openMessage(t.latest)) ui.notice = 'That channel was deleted.'
}

/** Mark all read reads these too: each thread's replies, seen. */
export function seeAllThreads(): void {
  const list = threads.list
  threads.list = []
  for (const t of list) api.markRepliesSeen(t.root.message_id, t.latest.id).catch(() => soon())
}

export function noteThreadsEvent(e: Event): void {
  const me = app.state.me?.id
  switch (e.t) {
    case 'MessageCreated': {
      const m = e.d.message
      const root = m.thread_root_id
      if (!root || m.deleted) return
      if (m.author_id === me) {
        // replying makes you part of it, and counts as looking
        if (!threads.watching.includes(root)) threads.watching = [root, ...threads.watching]
        drop(root)
      } else if (threads.watching.includes(root)) soon()
      return
    }
    case 'ThreadSeen':
      if (e.d.user_id !== me) return
      for (const t of threads.list) {
        if (t.root.message_id !== e.d.root_id) continue
        if (e.d.last_seen_id >= t.latest.id) drop(t.root.message_id)
        else soon()
      }
      return
    case 'MessageDeleted':
      if (threads.watching.includes(e.d.message_id) || threads.list.some((t) => t.latest.id === e.d.message_id)) soon()
      return
    case 'MutesChanged':
    case 'ChannelDeleted':
    case 'ChannelRestored':
    case 'ServerDeleted':
    case 'ServerRestored':
    case 'ChannelRemoved':
      soon()
      return
  }
}

/** For tests: forget everything. */
export function resetThreads(): void {
  if (timer) clearTimeout(timer)
  timer = null
  epoch++
  threads.list = []
  threads.watching = []
  threads.loaded = false
  threads.error = ''
}

onAppReady(() => void refreshThreads())
onAppEvent(noteThreadsEvent)
