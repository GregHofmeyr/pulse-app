// Which clip is playing (one at a time), and the volume every player shares: the volume mixer's Clips notch, so a
// player's own slider and Settings → Volume mixer move the same value (kept on this device in `pulse.mixer`).
import { percentToGain } from './mixer'
import { mixer, reloadMixer, setClipsMuted, setNotch } from './mixer.svelte'

const now = $state<{ id: string | null }>({ id: null })

/** The playing message's id (or null), and the volume (percent, 0..100) and mute shared by every player. */
export const playback = {
  get nowPlaying(): string | null {
    return now.id
  },
  get volume(): number {
    return mixer.media.clips
  },
  set volume(pct: number) {
    setVolume(pct)
  },
  get muted(): boolean {
    return mixer.media.clipsMuted
  },
  set muted(m: boolean) {
    setMuted(m)
  },
}

/** A `<video>`'s volume for a Clips percent: the mixer curve, at most the element's own full volume. */
export const elementVolume = (pct: number): number => Math.min(1, percentToGain(pct))

let owner: symbol | null = null
let pauseCurrent: (() => void) | null = null

/** Makes this player (one instance, whatever its message) the one playing clip: the previous holder is
 * paused through the callback it registered. Returns the token to `release` with. */
export function claim(id: string, pause: () => void, token: symbol = Symbol(id)): symbol {
  if (owner !== null && owner !== token) pauseCurrent?.()
  owner = token
  now.id = id
  pauseCurrent = pause
  return token
}

/** Gives up the claim, if this token still holds it. */
export function release(token: symbol): void {
  if (owner !== token) return
  owner = null
  now.id = null
  pauseCurrent = null
}

/** Re-reads the saved volume (startup and tests). */
export function loadVolume(): void {
  reloadMixer()
}

/** Percent, 0..100. */
export function setVolume(pct: number): void {
  setNotch('media', 'clips', pct)
}

export function setMuted(m: boolean): void {
  setClipsMuted(m)
}
