// Viewer UI state: the call view (which watch is in full view, where the shell should go next, a
// Join & watch waiting for its join), and the "<name> is live" toasts.
import { voice, voiceApi } from '../voice.svelte'
import { streamApi } from './stream.svelte'
import { dropToast, pushToast, TOAST_MS, type Toast, type WatchKey } from './viewer'

export type { WatchKey } from './viewer'

export const callView = $state({
  /** The watch in full view. Kept while you're elsewhere, so the mini-preview takes you back to it. */
  full: null as WatchKey | null,
  /** A channel the shell should show next (it clears this once it has). */
  goTo: null as string | null,
  /** A Join & watch whose join hasn't connected yet: the shell watches once it has. */
  joinThenWatch: null as WatchKey | null,
})

export const toasts = $state({ list: [] as Toast[] })

const timers = new Map<string, ReturnType<typeof setTimeout>>()
const key = (channelId: string, streamer: string) => `${channelId}/${streamer}`

export function showToast(t: Toast) {
  const k = key(t.channelId, t.streamer)
  clearTimeout(timers.get(k))
  toasts.list = pushToast(toasts.list, t)
  timers.set(k, setTimeout(() => removeToast(t.channelId, t.streamer), TOAST_MS))
}

export function removeToast(channelId: string, streamer: string) {
  const k = key(channelId, streamer)
  clearTimeout(timers.get(k))
  timers.delete(k)
  toasts.list = dropToast(toasts.list, channelId, streamer)
}

/** Puts `k` in full view (the high layer under Auto). */
export function openFull(k: WatchKey) {
  const prev = callView.full
  // one stream in full view: the one there before goes back to a preview
  if (prev && (prev.channelId !== k.channelId || prev.streamer !== k.streamer)) streamApi.setView(prev.channelId, prev.streamer, 'preview')
  callView.full = { channelId: k.channelId, streamer: k.streamer }
  streamApi.setView(k.channelId, k.streamer, 'full')
}

/** Back to the grid: the stream keeps playing as a preview. */
export function closeFull() {
  const f = callView.full
  callView.full = null
  if (f) streamApi.setView(f.channelId, f.streamer, 'preview')
}

/** Watch from a toast or the LIVE popover: the call view opens in full view on that stream. */
export function watchStream(channelId: string, streamer: string) {
  if (voice.channelId !== channelId) return joinAndWatch(channelId, streamer)
  removeToast(channelId, streamer)
  void streamApi.watch(channelId, streamer)
  openFull({ channelId, streamer })
  callView.goTo = channelId
}

/** A stream in a voice channel you're not in: join it (leaving yours), then watch once connected. */
export function joinAndWatch(channelId: string, streamer: string) {
  removeToast(channelId, streamer)
  callView.joinThenWatch = { channelId, streamer }
  callView.goTo = channelId
  // a second click while that join is in flight doesn't restart it
  if (voice.joining !== channelId) void voiceApi.join(channelId)
}
