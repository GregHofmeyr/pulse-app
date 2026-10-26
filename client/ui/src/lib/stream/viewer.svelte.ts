// Viewer UI state: the stream the main area is showing, and the "<name> is live" toasts.
import { dropToast, pushToast, TOAST_MS, type Toast } from './viewer'

/** What the main area should show. `stream.watching` follows once the video element has joined it. */
export const viewing = $state({ req: null as { channelId: string; streamer: string } | null })

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

export function watchStream(channelId: string, streamer: string) {
  removeToast(channelId, streamer)
  viewing.req = { channelId, streamer }
}
