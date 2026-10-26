import { resolutionsFor, type Fps, type Res } from './encodings'

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

const FPS: Fps[] = [15, 30, 60]

/** The stored choice made safe for a source of `sourceHeight`: storage is untrusted, and the source may be smaller than last time. */
export function clampGoLive(stored: GoLive, sourceHeight: number): GoLive {
  const offered = resolutionsFor(sourceHeight)
  const largest = offered[offered.length - 1]
  const res = offered.includes(stored.res) ? stored.res : stored.res > largest ? largest : GO_LIVE_DEFAULT.res
  const fps = FPS.includes(stored.fps) ? stored.fps : GO_LIVE_DEFAULT.fps
  const audio = typeof stored.audio === 'boolean' ? stored.audio : GO_LIVE_DEFAULT.audio
  return { res, fps, audio }
}

export const loadHidePreview = (): boolean => load<{ on: boolean }>('pulse.stream.hidepreview', { on: false }).on
export const saveHidePreview = (on: boolean) => save('pulse.stream.hidepreview', { on })

/** Stream audio muted, carried to the next Go Live. */
export const loadAudioMuted = (): boolean => load<{ on: boolean }>('pulse.stream.audiomuted', { on: false }).on
export const saveAudioMuted = (on: boolean) => save('pulse.stream.audiomuted', { on })

export const loadDiagOverlay = (): boolean => load<{ on: boolean }>('pulse.stream.diag', { on: false }).on
export const saveDiagOverlay = (on: boolean) => save('pulse.stream.diag', { on })

/** Per-streamer playback volume (percent), keyed by user id. */
export const loadStreamVolumes = () => load<Record<string, number>>('pulse.stream.volumes', {})
export const saveStreamVolumes = (v: Record<string, number>) => save('pulse.stream.volumes', v)

/** "Show non-streaming participants" in the call grid, per signed-in user; on by default. */
export const loadShowEveryone = (userId: string): boolean => load<{ on: boolean }>(`pulse.callgrid.everyone.${userId}`, { on: true }).on
export const saveShowEveryone = (userId: string, on: boolean) => save(`pulse.callgrid.everyone.${userId}`, { on })
