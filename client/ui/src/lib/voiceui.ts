// Pure helpers for the voice UI (tested in voiceui.test.ts).

export type Flags = { muted: boolean; deafened: boolean }
export type SoundName = 'join' | 'leave' | 'mute' | 'unmute' | 'deafen' | 'undeafen'

export const clampVolume = (pct: number) => Math.round(Math.min(200, Math.max(0, pct)))

export function volumeLabel(pct: number): string {
  const v = clampVolume(pct)
  return v === 0 ? 'Muted' : `${v}%`
}

/** Which sound a mute/deafen transition plays (deafen wins over mute). */
export function soundFor(prev: Flags, next: Flags): SoundName | null {
  if (prev.deafened !== next.deafened) return next.deafened ? 'deafen' : 'undeafen'
  if (prev.muted !== next.muted) return next.muted ? 'mute' : 'unmute'
  return null
}

const cache = new Map<SoundName, HTMLAudioElement>()

export function playSound(name: SoundName) {
  try {
    let a = cache.get(name)
    if (!a) {
      a = new Audio(`/sounds/${name}.wav`)
      a.volume = 0.6
      cache.set(name, a)
    }
    a.currentTime = 0
    void a.play()
  } catch {
    // sounds are a nicety; never break the UI over them
  }
}

// Per-machine preferences (not shared state): safe in localStorage, guarded for private modes.
function load<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(key)
    return raw ? { ...fallback, ...JSON.parse(raw) } : fallback
  } catch {
    return fallback
  }
}
function save(key: string, value: unknown) {
  try {
    localStorage.setItem(key, JSON.stringify(value))
  } catch {
    /* ignore */
  }
}

export type AudioConfig = {
  input: string | null
  output: string | null
  input_gain_pct: number
  sensitivity: number
  echo_cancel: boolean
  noise_suppress: boolean
  auto_gain: boolean
}
export const defaultAudioConfig: AudioConfig = {
  input: null, output: null, input_gain_pct: 100, sensitivity: 0.01, echo_cancel: true, noise_suppress: true, auto_gain: false,
}
export const loadAudioConfig = () => load('pulse.audio', defaultAudioConfig)
export const saveAudioConfig = (c: AudioConfig) => save('pulse.audio', c)

export const loadVolumes = () => load<Record<string, number>>('pulse.volumes', {})
export const saveVolumes = (v: Record<string, number>) => save('pulse.volumes', v)
