// Auto clip drop (#382): the core spots a recording NVIDIA or Game Bar just saved in a watched folder
// (`pulse://clip-drop`, src-tauri games/clipdrop.rs) and a toast asks "Share to #clips?". Share opens that clips channel
// with the usual clip editor on the file (ClipsChannel takes `ui.shareClip`); the channel is the last one you shared a
// clip in, else the only one there is, else you pick one in the toast.
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import type { Channel } from './protocol/Channel'
import type { AppState } from './state'
import { ui } from './store.svelte'
import type { ClipOffer } from './tauri'

const KEY = 'pulse.lastClipsChannel'
/** Offers shown at once; an older one goes when a fourth arrives. */
export const MAX_OFFERS = 3

function loadLast(): string | null {
  try {
    return localStorage.getItem(KEY)
  } catch {
    return null
  }
}

export const clipDrop = $state({
  offers: [] as ClipOffer[],
  /** The clips channel you last shared a clip in (per device). */
  last: loadLast(),
})

/** Where an offered clip goes: a channel, a pick among `choices`, or nowhere (no clips channel you're in). */
export type ClipTarget = { kind: 'channel'; channel: Channel } | { kind: 'ask'; choices: Channel[] } | { kind: 'none' }

/** The clips channels you can share to (pure): of servers you're in, by server then position. */
export function clipsChannels(s: AppState): Channel[] {
  const order = new Map(s.servers.map((x, i) => [x.id, i]))
  return Object.values(s.channels)
    .filter((c) => c.kind === 'clips' && c.server_id !== null && order.has(c.server_id))
    .sort((a, b) => order.get(a.server_id!)! - order.get(b.server_id!)! || a.position - b.position)
}

/** The target for an offer (pure): the last clips channel used if it's still there, else the only one, else ask. */
export function clipsTarget(s: AppState, last: string | null): ClipTarget {
  const all = clipsChannels(s)
  const remembered = last ? all.find((c) => c.id === last) : undefined
  if (remembered) return { kind: 'channel', channel: remembered }
  if (all.length === 1) return { kind: 'channel', channel: all[0] }
  if (all.length) return { kind: 'ask', choices: all }
  return { kind: 'none' }
}

/** A clip was opened for sharing in `channelId`: the next offer goes there. */
export function rememberClipsChannel(channelId: string): void {
  clipDrop.last = channelId
  try {
    localStorage.setItem(KEY, channelId)
  } catch {
    /* ignore */
  }
}

export function addOffer(o: ClipOffer): void {
  if (clipDrop.offers.some((x) => x.path === o.path)) return
  clipDrop.offers = [...clipDrop.offers, o].slice(-MAX_OFFERS)
}

export function dismissOffer(path: string): void {
  clipDrop.offers = clipDrop.offers.filter((o) => o.path !== path)
}

/** Share: open `channelId` with the editor on the offered file. */
export function shareOffer(o: ClipOffer, channelId: string): void {
  dismissOffer(o.path)
  ui.shareClip = { channelId, path: o.path }
  ui.goTo = channelId
}

/** Follow the core's offers; the stop also clears them (signed out: they were for that session's screen). */
export async function startClipDrop(): Promise<UnlistenFn> {
  const off = await listen<ClipOffer>('pulse://clip-drop', (e) => addOffer(e.payload))
  return () => {
    off()
    clipDrop.offers = []
  }
}
