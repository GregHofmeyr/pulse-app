import type { Fps, Res } from './encodings'

// Per-device preferences: guarded localStorage, same pattern as lib/voiceui.ts.
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

export type GoLive = { res: Res; fps: Fps; audio: boolean }
const GO_LIVE_DEFAULT: GoLive = { res: 720, fps: 60, audio: true }

export const loadGoLive = (): GoLive => load('pulse.golive', GO_LIVE_DEFAULT)
export const saveGoLive = (v: GoLive) => save('pulse.golive', v)

export const loadDiagOverlay = (): boolean => load<{ on: boolean }>('pulse.stream.diag', { on: false }).on
export const saveDiagOverlay = (on: boolean) => save('pulse.stream.diag', { on })

/** Per-streamer playback volume (percent), keyed by user id. */
export const loadStreamVolumes = () => load<Record<string, number>>('pulse.stream.volumes', {})
export const saveStreamVolumes = (v: Record<string, number>) => save('pulse.stream.volumes', v)
