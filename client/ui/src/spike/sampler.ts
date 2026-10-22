/** Fixed-size ring: the soak sampler must not itself leak (S3 measures leaks). */
export class Ring<T> {
  private buf: T[] = []
  constructor(private cap: number) {}
  push(x: T) {
    this.buf.push(x)
    if (this.buf.length > this.cap) this.buf.shift()
  }
  items(): T[] {
    return [...this.buf]
  }
}

export type Sample = { t: number; framesEncoded: number; totalEncodeTime: number; framesSent: number; qualityLimitationReason: string }

/** Spec §6.2 strain signals between two outbound-rtp samples (the full-layer encoding). */
export function strain(prev: Sample, cur: Sample, targetFps: number) {
  const dt = (cur.t - prev.t) / 1000
  const frames = cur.framesSent - prev.framesSent
  const fps = dt > 0 ? frames / dt : 0
  const encFrames = cur.framesEncoded - prev.framesEncoded
  const encMs = encFrames > 0 ? ((cur.totalEncodeTime - prev.totalEncodeTime) * 1000) / encFrames : 0
  return {
    lowFps: fps < 0.85 * targetFps,
    encodeHeavy: encMs > 0.7 * (1000 / targetFps),
    cpuLimited: cur.qualityLimitationReason === 'cpu',
  }
}
