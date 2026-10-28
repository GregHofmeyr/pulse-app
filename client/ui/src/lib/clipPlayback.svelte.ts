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

let pauseCurrent: (() => void) | null = null

/** Makes `id` the one playing clip: the previous one is paused through the callback it registered. */
export function claim(id: string, pause: () => void): void {
  if (playback.nowPlaying !== null && playback.nowPlaying !== id) pauseCurrent?.()
  playback.nowPlaying = id
  pauseCurrent = pause
}

/** Gives up the claim, if `id` still holds it. */
export function release(id: string): void {
  if (playback.nowPlaying !== id) return
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
