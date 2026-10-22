// Quiet diagnostics: samples getStats() while sharing or watching, writes one `share:` log line per
// sample (so a bug report carries the numbers), and keeps the latest for the overlay.
import { streamCmd } from '../tauri'
import { loadDiagOverlay } from './prefs'
import { inboundSummary, isHardwareEncoder, outboundSummary, type InboundSummary, type OutboundSummary } from './stats'

export const diag = $state({
  out: null as OutboundSummary | null,
  in: null as InboundSummary | null,
  /** `#v` connect time and time to the first painted frame of the watched stream (overlay only). */
  connectMs: null as number | null,
  firstFrameMs: null as number | null,
})

const SAMPLE_MS = 5_000
const SAMPLE_OVERLAY_MS = 2_000
const ENCODER_FIRST_MS = 5_000
const ENCODER_EVERY_MS = 30_000

/**
 * Starts sampling; returns the stop function. On the streamer (`out`) it also checks the full
 * layer's encoder 5 s in and then every 30 s, calling `onEncoderWarning(true)` when it is a
 * software encoder (and `false` when that changes back).
 */
export function startDiagnostics(
  kind: 'out' | 'in',
  getReport: () => Promise<RTCStatsReport | undefined>,
  targetFps: number,
  onEncoderWarning?: (warn: boolean) => void,
): () => void {
  let stopped = false
  let prev: RTCStatsReport | null = null
  let prevAt = 0
  const timers: ReturnType<typeof setTimeout>[] = []
  const next = () => (loadDiagOverlay() ? SAMPLE_OVERLAY_MS : SAMPLE_MS)

  const sample = async () => {
    const rep = await getReport().catch(() => undefined)
    if (stopped) return
    if (rep) {
      const now = performance.now()
      const dt = prev ? now - prevAt : 0
      if (kind === 'out') {
        diag.out = outboundSummary(prev, rep, dt, targetFps)
        streamCmd.log({ ev: 'out', ...diag.out })
      } else {
        diag.in = inboundSummary(prev, rep, dt)
        streamCmd.log({ ev: 'in', ...diag.in })
      }
      prev = rep
      prevAt = now
    }
    timers[0] = setTimeout(sample, next())
  }
  timers[0] = setTimeout(sample, next())

  if (kind === 'out') {
    let hardware: boolean | null = null
    const checkEncoder = async () => {
      const rep = await getReport().catch(() => undefined)
      if (stopped || !rep) return
      const full = outboundSummary(null, rep, 0, targetFps).layers.reduce<OutboundSummary['layers'][number] | null>(
        (a, b) => (!a || b.w > a.w ? b : a),
        null,
      )
      if (!full?.encoder) return
      const hw = isHardwareEncoder(full.encoder, full.powerEfficient ?? undefined)
      if (hw === hardware) return
      hardware = hw
      onEncoderWarning?.(!hw)
      streamCmd.log({ ev: 'encoder', impl: full.encoder, powerEfficient: full.powerEfficient })
    }
    timers[1] = setTimeout(() => {
      void checkEncoder()
      timers[2] = setInterval(() => void checkEncoder(), ENCODER_EVERY_MS)
    }, ENCODER_FIRST_MS)
  }

  return () => {
    stopped = true
    clearTimeout(timers[0])
    clearTimeout(timers[1])
    clearInterval(timers[2])
    diag[kind] = null
  }
}
