// Steam (#379): your link and what this server can do with Steam, for Settings › Privacy › Game activity. Read at
// each Ready (sign-in, reconnect) and kept current by SteamLinkChanged (linked from the browser, unlinked, a setting,
// Steam refusing). Whether a linked account shares achievements also tells the core to send the game's Steam app id
// with "Show game art" off (marked so the server keeps it to itself).
import { onAppEvent, onAppReady } from './store.svelte'
import { api, errorText } from './tauri'
import { setSteamAchievements } from './gamePrefs.svelte'
import type { Event } from './protocol/Event'
import type { SteamStatus } from './protocol/SteamStatus'

export const steam = $state<{ status: SteamStatus | null; busy: boolean; error: string }>({ status: null, busy: false, error: '' })

export function setSteamStatus(st: SteamStatus): void {
  steam.status = st
  setSteamAchievements(!!st.link?.share_achievements && st.achievements_available)
}

export async function loadSteam(): Promise<void> {
  try {
    const st = await api.steamStatus()
    if (st) setSteamStatus(st)
  } catch {
    /* an older server, or offline: Settings shows nothing about Steam */
  }
}

/** Run `call` (a Steam action) with the busy flag and its error shown. */
async function act(call: () => Promise<SteamStatus | void>): Promise<void> {
  steam.busy = true
  steam.error = ''
  try {
    const st = await call()
    if (st) setSteamStatus(st)
  } catch (e) {
    steam.error = errorText(e)
  } finally {
    steam.busy = false
  }
}

/** Opens Steam's sign-in in the browser; the link lands as a SteamLinkChanged. */
export const linkSteam = () => act(() => api.linkSteam())
export const unlinkSteam = () => act(() => api.unlinkSteam())
export const setSteamShare = (on: boolean) => act(() => api.setSteamShare(on))
export const setSteamHours = (on: boolean) => act(() => api.setSteamHours(on))

export function handleSteamEvent(e: Event): void {
  if (e.t === 'SteamLinkChanged') setSteamStatus(e.d.status)
}

/** Follow the link from boot; returns the unsubscribe function. */
export function startSteamListening(): () => void {
  const offEvent = onAppEvent(handleSteamEvent)
  const offReady = onAppReady(() => void loadSteam())
  return () => {
    offEvent()
    offReady()
  }
}

/** What Settings says under the Steam row, or null when all is well. */
export function steamNote(st: SteamStatus | null): string | null {
  if (!st) return null
  if (!st.link) return st.linking_available ? null : "Linking Steam isn't set up on this server."
  if (!st.achievements_available) return 'Achievements need the server owner to add a Steam key.'
  if (st.link.private) return 'Make your Steam game details public to share achievements.'
  return null
}

export { completionLine, formatRate, hoursText } from './achievements'
