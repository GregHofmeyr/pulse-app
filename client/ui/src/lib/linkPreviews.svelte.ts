// Link previews: the per-device "Show link previews" setting, the one player playing app-wide,
// and the origin our embed page is served from (only when signed in to pulsechat.co.za).
import { api } from './tauri'

const SHOW_KEY = 'pulse.linkPreviews'

function savedShow(): boolean {
  try {
    return localStorage.getItem(SHOW_KEY) !== 'off'
  } catch {
    return true
  }
}

/** "Show link previews" (this device only; on by default). */
export const previews = $state({ show: savedShow() })

export function setShowPreviews(v: boolean): void {
  previews.show = v
  try {
    localStorage.setItem(SHOW_KEY, v ? 'on' : 'off')
  } catch {
    /* ignore */
  }
}

/** The one card whose player is open (`playerKey`); starting another replaces it. */
export const player = $state({ active: null as string | null })

export const playerKey = (messageId: string, url: string): string => `${messageId}\n${url}`

export function play(key: string): void {
  player.active = key
}

export function stop(key: string): void {
  if (player.active === key) player.active = null
}

/** Where players load from; null = no inline players (the play button opens the link). */
export const embed = $state({ origin: null as string | null })

let loading: Promise<void> | null = null

/** Asks the core once per sign-in; a failure is retried by the next card that mounts. */
export function loadEmbedOrigin(): void {
  if (loading) return
  const p: Promise<void> = api.embedOrigin().then(
    (o) => {
      if (loading === p) embed.origin = o ?? null
    },
    () => {
      if (loading === p) loading = null
    },
  )
  loading = p
}

/** Signed out: the next account may be on another server. Also stops any player. */
export function forgetEmbedOrigin(): void {
  loading = null
  embed.origin = null
  player.active = null
}
