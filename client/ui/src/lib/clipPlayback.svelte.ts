// Which clip is playing (one at a time) and the volume every player shares, kept on this device.
const KEY = 'pulse.clipVolume'

type Saved = { volume: number; muted: boolean }

function read(): Saved {
  const fallback = { volume: 1, muted: false }
  try {
    const raw = localStorage.getItem(KEY)
    const v = raw ? (JSON.parse(raw) as Partial<Saved> | null) : null
    return {
      volume: typeof v?.volume === 'number' && Number.isFinite(v.volume) ? Math.min(1, Math.max(0, v.volume)) : fallback.volume,
      muted: v?.muted === true,
    }
  } catch {
    return fallback
  }
}

/** The playing message's id (or null), and the volume (0..1) and mute shared by every player. */
export const playback = $state<{ nowPlaying: string | null; volume: number; muted: boolean }>({ nowPlaying: null, ...read() })

let owner: symbol | null = null
let pauseCurrent: (() => void) | null = null

/** Makes this player (one instance, whatever its message) the one playing clip: the previous holder is
 * paused through the callback it registered. Returns the token to `release` with. */
export function claim(id: string, pause: () => void, token: symbol = Symbol(id)): symbol {
  if (owner !== null && owner !== token) pauseCurrent?.()
  owner = token
  playback.nowPlaying = id
  pauseCurrent = pause
  return token
}

/** Gives up the claim, if this token still holds it. */
export function release(token: symbol): void {
  if (owner !== token) return
  owner = null
  playback.nowPlaying = null
  pauseCurrent = null
}

/** Re-reads the saved volume (startup and tests). */
export function loadVolume(): void {
  Object.assign(playback, read())
}

function save(): void {
  try {
    localStorage.setItem(KEY, JSON.stringify({ volume: playback.volume, muted: playback.muted }))
  } catch {
    /* ignore */
  }
}

export function setVolume(v: number): void {
  playback.volume = Math.min(1, Math.max(0, v))
  save()
}

export function setMuted(m: boolean): void {
  playback.muted = m
  save()
}
