// Bitrate table from the spec (kbps).
const KBPS: Record<number, Record<number, number>> = {
  720: { 15: 1500, 30: 3500, 60: 5500 },
  1080: { 15: 2500, 30: 5000, 60: 8000 },
  1440: { 15: 4000, 30: 7500, 60: 12000 },
}

export type Res = 720 | 1080 | 1440
export type Fps = 15 | 30 | 60

const RESOLUTIONS: Res[] = [720, 1080, 1440]

/** The always-on low simulcast layer. */
export const LOW_LAYER = { width: 640, height: 360, maxBitrate: 500_000, maxFramerate: 30 } as const

/** The middle layer (#582), for a preview tile too big for the low one: 1.5 Mbps (1 Mbps for a 15 fps share). */
export const MID_LAYER = { width: 1280, height: 720, maxBitrate: 1_500_000, maxFramerate: 30 } as const

export type SimulcastLayer = { width: number; height: number; maxBitrate: number; maxFramerate: number }

/**
 * The layers published under the top one (the chosen resolution), smallest first. A 1080p or 1440p share also sends
 * 720p, so a preview bigger than 640 x 360 on screen gets a sharp picture without pulling the full stream; a 720p
 * share's top layer already is that. Dynacast pauses a layer no one is asking for, so the middle one costs upload
 * only while someone's tile is showing it.
 */
export function simulcastLayersFor(res: Res, fps: Fps): SimulcastLayer[] {
  if (res < 1080) return [LOW_LAYER]
  return [LOW_LAYER, { ...MID_LAYER, maxBitrate: fps === 15 ? 1_000_000 : MID_LAYER.maxBitrate }]
}

export function encodingFor(res: Res, fps: Fps) {
  const text = fps === 15
  return {
    maxBitrate: KBPS[res][fps] * 1000,
    maxFramerate: fps,
    contentHint: (text ? 'detail' : 'motion') as 'motion' | 'detail',
    degradation: (text ? 'maintain-resolution' : 'maintain-framerate') as 'maintain-framerate' | 'maintain-resolution',
  }
}

/** Resolutions offered for a source: never above it. A source under 720 still offers 720 (capture stays native). */
export function resolutionsFor(sourceHeight: number): Res[] {
  const ok = RESOLUTIONS.filter((r) => r <= sourceHeight)
  return ok.length ? ok : [720]
}
