// Auto clip drop (#382): the core spots a recording NVIDIA or Game Bar just saved in a watched folder
// (`pulse://clip-drop`, src-tauri games/clipdrop.rs) and a toast asks "Share to #clips?". Share opens that clips channel
// with the usual clip editor on the file (ClipsChannel takes `ui.shareClip`); the channel is the last one you shared a
// clip in, else the only one there is, else you pick one in the toast. It can go to a DM instead (#394): that chat
// opens with the file queued in its composer. Turning the switch off (or Game activity's master) clears the toasts.
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { conversations } from './conversations'
import { gamePrefs } from './gamePrefs.svelte'
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

/** DMs and groups an offer can be sent in (pure): newest first, as the conversation list has them. */
export const DM_CHOICES = 20
export function dmTargets(s: AppState, nowIso: string): { id: string; name: string }[] {
  return conversations(s, nowIso)
    .slice(0, DM_CHOICES)
    .map((c) => ({ id: c.channel.id, name: c.name }))
}

/** Whether offers are wanted: Game activity on, and "Offer new recordings as clips" on. */
export const clipDropOn = (): boolean => gamePrefs.enabled && gamePrefs.clips

/** How long a Share's request waits for its chat to show (#394): one that never landed (the chat didn't open) is
 *  dropped, rather than opening the editor the next time that chat shows. */
export const PENDING_MS = 15_000
export const pendingFresh = (at: number, now = Date.now()): boolean => now - at <= PENDING_MS

export function addOffer(o: ClipOffer): void {
  if (!clipDropOn()) return
  if (clipDrop.offers.some((x) => x.path === o.path)) return
  clipDrop.offers = [...clipDrop.offers, o].slice(-MAX_OFFERS)
}

export function dismissOffer(path: string): void {
  clipDrop.offers = clipDrop.offers.filter((o) => o.path !== path)
}

/** Share: open `channelId` with the editor on the offered file. */
export function shareOffer(o: ClipOffer, channelId: string): void {
  dismissOffer(o.path)
  ui.shareClip = { channelId, path: o.path, at: Date.now() }
  ui.goTo = channelId
}

/** Send in a DM (#394): open that chat with the offered file queued in its composer, to send with a message or not. */
export function sendOfferToDm(o: ClipOffer, channelId: string): void {
  dismissOffer(o.path)
  ui.attachFiles = { channelId, paths: [o.path], at: Date.now() }
  ui.goTo = channelId
}

/** The switch went off: the toasts already showing go too. */
export function clearOffers(): void {
  clipDrop.offers = []
}

/** Follow the core's offers; the stop also clears them (signed out: they were for that session's screen). */
export async function startClipDrop(): Promise<UnlistenFn> {
  const off = await listen<ClipOffer>('pulse://clip-drop', (e) => addOffer(e.payload))
  return () => {
    off()
    clipDrop.offers = []
  }
}
