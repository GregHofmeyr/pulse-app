// Keeping loaded messages' pins right (#258). A reconnect fetches a chat's latest page again but not the older pages
// (or a jump's window) still loaded, and pins may have come and gone meanwhile: each such chat is marked, and when
// it is next shown one fetch of its pins (at most 50) sets every loaded message right. The same fetch follows a
// refused pin or unpin, whose refusal means the copy here was out of date. This module also keeps the server's clock
// (serverClock) told, which times pins.
import { takesPins } from './pins'
import type { Event } from './protocol/Event'
import { noteServerTime, resetServerClock } from './serverClock'
import { applyPinList } from './state'
import { app, onAppEvent, onAppReady } from './store.svelte'
import { api } from './tauri'

/** Chats whose loaded messages may show pins from before a reconnect. */
export const pinSync = $state({ stale: {} as Record<string, true> })

/** channelId -> messages whose pin changed live while its pins were loading (the list is older than those). */
const loading = new Map<string, Set<string>>()

/** After a Ready: every chat with messages loaded (and pins) may be out of date. */
export function markPinsStale(): void {
  const stale: Record<string, true> = {}
  for (const [c, list] of Object.entries(app.state.messages)) if (list.length && takesPins(app.state.channels[c]?.kind)) stale[c] = true
  pinSync.stale = stale
}

/** A live event: a pin or unpin while that chat's pins load wins over the list. */
export function notePinEvent(e: Event): void {
  if (e.t === 'MessagePinned') loading.get(e.d.channel_id)?.add(e.d.pinned.message.id)
  else if (e.t === 'MessageUnpinned') loading.get(e.d.channel_id)?.add(e.d.message_id)
}

// The server's clock times pins (serverClock): each Ready and each live message tells it.
onAppReady((r) => {
  resetServerClock(r.server_now)
  markPinsStale()
})
onAppEvent((e) => {
  if (e.t === 'MessageCreated') noteServerTime(e.d.message.created_at)
  notePinEvent(e)
})

/** Fetch `channel`'s pins and set its loaded messages by them. One at a time per chat; a failure (offline) waits
 *  for the next Ready to mark the chat again. */
export async function refreshPins(channel: string): Promise<void> {
  if (loading.has(channel)) return
  const touched = new Set<string>()
  loading.set(channel, touched)
  if (pinSync.stale[channel]) {
    const { [channel]: _done, ...rest } = pinSync.stale
    pinSync.stale = rest
  }
  try {
    const list = await api.listPins(channel)
    app.state = applyPinList(app.state, channel, list, touched)
  } catch {
    // nothing to show: the pins stay as they were
  } finally {
    loading.delete(channel)
  }
  // a Ready came while it loaded: the list may be from before it
  if (pinSync.stale[channel]) await refreshPins(channel)
}
