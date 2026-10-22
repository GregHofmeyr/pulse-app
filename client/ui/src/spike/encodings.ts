// THROWAWAY (spike) — but this table is the spec's §3.2 and moves to the real build if S2/S4 agree.
const KBPS: Record<number, Record<number, number>> = {
  720: { 15: 1500, 30: 3500, 60: 5500 },
  1080: { 15: 2500, 30: 5000, 60: 8000 },
  1440: { 15: 4000, 30: 7500, 60: 12000 },
}

export type Res = 720 | 1080 | 1440
export type Fps = 15 | 30 | 60

export function encodingFor(res: Res, fps: Fps) {
  const text = fps === 15
  return {
    maxBitrate: KBPS[res][fps] * 1000,
    maxFramerate: fps,
    contentHint: (text ? 'detail' : 'motion') as 'motion' | 'detail',
    degradation: (text ? 'maintain-resolution' : 'maintain-framerate') as 'maintain-framerate' | 'maintain-resolution',
  }
}
