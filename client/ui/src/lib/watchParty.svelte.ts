// YouTube watch parties in voice (#384): the per-device "Watch parties" setting, the calls that change a party, and the
// bookkeeping that keeps the server's clock told and declines a party for you while the setting is off.
import type { WatchParty } from './protocol/WatchParty'
import type { WatchPartyAction } from './protocol/WatchPartyAction'
import { noteServerTime } from './serverClock'
import { app, onAppEvent } from './store.svelte'
import { api } from './tauri'
import { linkIn, mediaFromLink, validMedia, watching, type PartyMedia } from './watchParty'

const SETTING_KEY = 'pulse.watchParties'

function savedOn(): boolean {
  try {
    return localStorage.getItem(SETTING_KEY) !== 'off'
  } catch {
    return true
  }
}

/** "Watch parties" (this device only; on by default). Off: no "Start watch party" or "Join watch party", and a party
 *  handed to you is left at once (it passes on). The channel list still shows that one is on. */
export const watchPrefs = $state({ on: savedOn() })

export function setWatchParties(v: boolean): void {
  watchPrefs.on = v
  try {
    localStorage.setItem(SETTING_KEY, v ? 'on' : 'off')
  } catch {
    /* ignore */
  }
  // turned off while watching: out of every party you're in
  if (!v) for (const p of Object.values(app.state.watchParties)) if (watching(p, app.state.me?.id)) void leaveWatchParty(p.channel_id)
}

/** The party on in `channelId`, if any. */
export const partyIn = (channelId: string | null | undefined): WatchParty | null => (channelId ? (app.state.watchParties[channelId] ?? null) : null)

/** Start a party of `video` in `channelId` (or change its video, when you may). */
export function startWatchParty(channelId: string, video: PartyMedia, title: string | null): Promise<WatchParty> {
  if (!validMedia(video.source, video.id)) return Promise.reject(new Error('That isn’t a video a watch party can play.'))
  return api.startWatchParty(channelId, video.source, video.id, title)
}

export const watchPartyAction = (channelId: string, action: WatchPartyAction): Promise<void> => api.watchPartyAction(channelId, action)

/** Add the video a pasted or dropped link points at to `channelId`'s party queue (#464). The server fills in its title
 *  and thumbnail from a card it has (or fetches one). */
export function enqueueLink(channelId: string, text: string): Promise<void> {
  const m = mediaFromLink(linkIn(text))
  if (!m) return Promise.reject(new Error('That isn’t a YouTube or Twitch link.'))
  return watchPartyAction(channelId, { action: 'enqueue', source: m.source, video_id: m.id, title: null })
}

export const joinWatchParty = (channelId: string) => watchPartyAction(channelId, { action: 'join' })

export function leaveWatchParty(channelId: string): Promise<void> {
  return watchPartyAction(channelId, { action: 'leave' }).catch(() => {
    /* already gone (the party ended, or you left the call): nothing to undo */
  })
}

export const endWatchParty = (channelId: string): Promise<void> => api.endWatchParty(channelId)

/** Declines sent, so one party's burst of changes sends one. */
const declining = new Set<string>()

onAppEvent((e) => {
  if (e.t === 'WatchPartyEnded') declining.delete(e.d.channel_id)
  if (e.t !== 'WatchPartyChanged') return
  // each change carries the server's time as it went out: the player's sync goes by the server's clock
  noteServerTime(new Date(e.d.server_now_ms).toISOString())
  const p = e.d.party
  // watch parties off and it was handed to you: decline (it passes to the next viewer, or ends)
  if (!watchPrefs.on && watching(p, app.state.me?.id)) {
    if (declining.has(p.channel_id)) return
    declining.add(p.channel_id)
    void leaveWatchParty(p.channel_id).finally(() => declining.delete(p.channel_id))
  }
})
