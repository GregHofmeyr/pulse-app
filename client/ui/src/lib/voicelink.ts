// Voice link display helpers: status levels, labels, sparkline and the copyable details block.
import type { LinkReport } from './voiceui'

export type Level = 'good' | 'warn' | 'bad'

const level = (v: number, warnAt: number, badAt: number): Level => (v >= badAt ? 'bad' : v >= warnAt ? 'warn' : 'good')

export const pingLevel = (ms: number): Level => level(ms, 100, 200)
export const jitterLevel = (ms: number): Level => level(ms, 20, 50)
/** Green up to and including 1 %, amber up to and including 5 %, red above. */
export const lossLevel = (pct: number): Level => (pct > 5 ? 'bad' : pct > 1 ? 'warn' : 'good')

export const transportLabel = (t: 'udp' | 'tcp' | 'turn'): string => (t === 'turn' ? 'TURN relay' : t.toUpperCase())

const SPAN_MS = 60_000
/** Ping swings smaller than this draw as a nearly flat line instead of filling the height. */
const MIN_SPAN_MS = 20

/** An SVG path for the ping history (`[ms ago, rtt]`, oldest first): the x axis is the last 60 s. */
export function sparklinePath(points: [number, number][], w: number, h: number): string {
  const shown = points.filter(([ago]) => ago <= SPAN_MS)
  if (!shown.length) return ''
  const rtts = shown.map(([, r]) => r)
  const lo = Math.min(...rtts)
  const hi = Math.max(...rtts)
  const mid = (lo + hi) / 2
  const span = Math.max(hi - lo, MIN_SPAN_MS)
  const num = (n: number) => String(Math.round(n * 100) / 100)
  return shown
    .map(([ago, rtt], i) => {
      const x = (Math.max(0, SPAN_MS - ago) / SPAN_MS) * w
      const y = h / 2 - ((rtt - mid) / span) * h
      return `${i ? 'L' : 'M'}${num(x)} ${num(y)}`
    })
    .join(' ')
}

/** One displayed value: rounded once, so the colour always matches the text (99.6 ms reads "100 ms", amber). */
export function reading(v: number | null, digits: number, levelOf: (n: number) => Level, unit: string): { text: string; level: Level | null } {
  if (v == null) return { text: '—', level: null }
  const f = 10 ** digits
  const r = Math.round(v * f) / f
  return { text: `${r}${unit}`, level: levelOf(r) }
}

/** Plain text for bug reports: `key: value` lines, `—` where a value is missing. */
export function detailsText(r: LinkReport, channelName: string): string {
  const v = (n: number | null, unit: string) => reading(n, 1, () => 'good', unit).text
  return [
    `Pulse version: ${r.version}`,
    `Channel: ${channelName}`,
    `Voice server: ${r.server ?? '—'}`,
    `Connection type: ${r.transport ? transportLabel(r.transport) : '—'}`,
    `Ping: ${v(r.rtt_ms, ' ms')}`,
    `Jitter: ${v(r.jitter_ms, ' ms')}`,
    `Packet loss: ${v(r.loss_pct, '%')}`,
  ].join('\n')
}
