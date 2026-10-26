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
