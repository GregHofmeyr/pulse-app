// Pure decisions for watching: who gets the "is live" toast, why Watch is disabled, the toast stack,
// which simulcast layer a watch asks for, and the watch cap.

import type { StreamEndReason } from '../protocol/StreamEndReason'

export type Toast = { channelId: string; streamer: string; name: string }

export const TOAST_MS = 4_000
export const MAX_TOASTS = 3

/** The toast goes to people in that voice channel, never to the streamer. */
export function toastFor(s: { channelId: string; streamer: string }, voiceChannelId: string | null, meId: string): boolean {
  return voiceChannelId === s.channelId && s.streamer !== meId
}

/** Why Watch can't be used right now, or null when it can. Linux first: joining voice would not help there. */
export function watchDisabledReason(a: { windows: boolean; voiceChannelId: string | null; channelId: string }): string | null {
  if (!a.windows) return "Watching from Linux isn't supported yet"
  if (a.voiceChannelId !== a.channelId) return 'Join voice to watch'
  return null
}

const same = (a: Toast, b: { channelId: string; streamer: string }) => a.channelId === b.channelId && a.streamer === b.streamer

/** Newest last; the same stream replaces its older toast; at most `MAX_TOASTS` stay. */
export function pushToast(list: Toast[], t: Toast): Toast[] {
  return [...list.filter((x) => !same(x, t)), t].slice(-MAX_TOASTS)
}

export function dropToast(list: Toast[], channelId: string, streamer: string): Toast[] {
  return list.filter((x) => !same(x, { channelId, streamer }))
}

/** A stream ended: ours only plays when the server ended it in our channel; someone else's when we watch it or share its channel. */
export function liveEndSound(
  e: { channelId: string; userId: string; reason: StreamEndReason },
  s: { voiceChannelId: string | null; meId: string; watching: boolean },
): boolean {
  const inMyChannel = s.voiceChannelId === e.channelId
  if (e.userId === s.meId) return (e.reason === 'timed_out' || e.reason === 'crashed') && inMyChannel
  return s.watching || inMyChannel
}

/** Where a watched stream shows: a grid tile or the mini-preview (`preview`), or the main area (`full`). */
export type View = 'preview' | 'full'
/** The viewer's pick: Auto asks for the full layer in full view, Data saver never does. */
export type Quality = 'auto' | 'saver'

/** Previews always take the low layer; full view takes the high one unless the viewer saves data. */
export function layerFor(view: View, quality: Quality): 'low' | 'high' {
  return view === 'full' && quality === 'auto' ? 'high' : 'low'
}

/** Adds `next` to `list`; past `max`, the oldest entries go first. */
export function admit<T>(list: T[], next: T, max: number): { list: T[]; dropped: T[] } {
  const all = [...list, next]
  const cut = Math.max(0, all.length - max)
  return { list: all.slice(cut), dropped: all.slice(0, cut) }
}

/** One watched stream: the streamer in a voice channel. */
export type WatchKey = { channelId: string; streamer: string }

/** A Join & watch can start watching: the join landed and is connected in that stream's channel. */
export function pendingWatchReady(p: WatchKey | null, v: { channelId: string | null; connection: string }): boolean {
  return !!p && v.channelId === p.channelId && v.connection === 'connected'
}
