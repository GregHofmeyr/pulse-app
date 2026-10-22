// Pure summaries of getStats() reports, as per-interval deltas (never cumulative totals).
export type FpsNote = 'ok' | 'still' | 'capture-limited' | 'encoder-limited' | 'network-limited'

export type OutboundLayer = {
  rid: string | null
  w: number
  h: number
  fps: number
  kbps: number
  encoder: string
  powerEfficient: boolean | null
  encodeMs: number | null
  qlr: string
}
export type OutboundSummary = { captureFps: number | null; layers: OutboundLayer[]; rttMs: number | null; note: FpsNote }

export type InboundSummary = {
  fps: number
  w: number
  h: number
  kbps: number
  decoder: string
  powerEfficient: boolean | null
  freezes: number
  freezeMs: number
  lost: number
  jitterMsPerFrame: number | null
  nacks: number
  plis: number
  stillLikely: boolean
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Stat = Record<string, any>

const stats = (r: RTCStatsReport | null): Stat[] => (r ? ([...r.values()] as Stat[]) : [])
const num = (v: unknown): number => (typeof v === 'number' && Number.isFinite(v) ? v : 0)
const delta = (cur: Stat, prev: Stat | undefined, k: string) => (prev ? Math.max(0, num(cur[k]) - num(prev[k])) : 0)
const byId = (r: RTCStatsReport | null) => new Map(stats(r).map((s) => [s.id as string, s]))

/** Why the sent frame rate is below target, in the order a person would check it. A still screen is not a problem. */
export function fpsNote(captureFps: number | null, sentFps: number, targetFps: number, qlr: string): FpsNote {
  if (sentFps >= 0.85 * targetFps) return 'ok'
  if (qlr === 'cpu') return 'encoder-limited'
  if (qlr === 'bandwidth') return 'network-limited'
  if (captureFps !== null && captureFps <= 5) return 'still'
  if (captureFps !== null && captureFps < 0.85 * targetFps) return 'capture-limited'
  return 'encoder-limited'
}

/** Hardware unless the browser says otherwise or the implementation name is a known software codec. */
export function isHardwareEncoder(impl: string, powerEfficient: boolean | undefined): boolean {
  return powerEfficient ?? !/OpenH264|libvpx|libaom|software/i.test(impl)
}

export function outboundSummary(prev: RTCStatsReport | null, cur: RTCStatsReport, dtMs: number, targetFps: number): OutboundSummary {
  const before = byId(prev)
  const all = stats(cur)
  const dt = dtMs / 1000
  const layers = all
    .filter((s) => s.type === 'outbound-rtp' && s.kind === 'video')
    .map((s) => {
      const p = before.get(s.id)
      const encFrames = delta(s, p, 'framesEncoded')
      return {
        rid: (s.rid as string | undefined) ?? null,
        w: num(s.frameWidth),
        h: num(s.frameHeight),
        fps: p ? (dt > 0 ? delta(s, p, 'framesSent') / dt : 0) : num(s.framesPerSecond),
        kbps: p && dt > 0 ? Math.round((delta(s, p, 'bytesSent') * 8) / dt / 1000) : 0,
        encoder: (s.encoderImplementation as string | undefined) ?? '',
        powerEfficient: (s.powerEfficientEncoder as boolean | undefined) ?? null,
        encodeMs: p && encFrames > 0 ? (delta(s, p, 'totalEncodeTime') * 1000) / encFrames : null,
        qlr: (s.qualityLimitationReason as string | undefined) ?? 'none',
      }
    })
  const src = all.find((s) => s.type === 'media-source' && s.kind === 'video')
  const captureFps = src && typeof src.framesPerSecond === 'number' ? src.framesPerSecond : null
  const rtt = all.find((s) => s.type === 'remote-inbound-rtp' && s.kind === 'video' && typeof s.roundTripTime === 'number')
  const full = layers.reduce<OutboundLayer | null>((a, b) => (!a || b.w > a.w ? b : a), null)
  return {
    captureFps,
    layers,
    rttMs: rtt ? Math.round(rtt.roundTripTime * 1000) : null,
    note: full ? fpsNote(captureFps, full.fps, targetFps, full.qlr) : 'ok',
  }
}

export function inboundSummary(prev: RTCStatsReport | null, cur: RTCStatsReport, dtMs: number): InboundSummary {
  const s = stats(cur).find((x) => x.type === 'inbound-rtp' && x.kind === 'video')
  const empty: InboundSummary = { fps: 0, w: 0, h: 0, kbps: 0, decoder: '', powerEfficient: null, freezes: 0, freezeMs: 0, lost: 0, jitterMsPerFrame: null, nacks: 0, plis: 0, stillLikely: false }
  if (!s) return empty
  const p = byId(prev).get(s.id)
  const dt = dtMs / 1000
  const emitted = delta(s, p, 'jitterBufferEmittedCount')
  const fps = p ? (dt > 0 ? delta(s, p, 'framesDecoded') / dt : 0) : num(s.framesPerSecond)
  const lost = delta(s, p, 'packetsLost')
  return {
    fps,
    w: num(s.frameWidth),
    h: num(s.frameHeight),
    kbps: p && dt > 0 ? Math.round((delta(s, p, 'bytesReceived') * 8) / dt / 1000) : 0,
    decoder: (s.decoderImplementation as string | undefined) ?? '',
    powerEfficient: (s.powerEfficientDecoder as boolean | undefined) ?? null,
    freezes: delta(s, p, 'freezeCount'),
    freezeMs: Math.round(delta(s, p, 'totalFreezesDuration') * 1000),
    lost,
    jitterMsPerFrame: emitted > 0 ? (delta(s, p, 'jitterBufferDelay') * 1000) / emitted : null,
    nacks: delta(s, p, 'nackCount'),
    plis: delta(s, p, 'pliCount'),
    stillLikely: fps <= 5 && lost === 0,
  }
}
