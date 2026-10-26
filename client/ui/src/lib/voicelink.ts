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

/** An SVG path for the ping history (`[ms ago, rtt]`, oldest first): the x axis is the last 60 s. */
export function sparklinePath(points: [number, number][], w: number, h: number): string {
  if (!points.length) return ''
  const rtts = points.map(([, r]) => r)
  const lo = Math.min(...rtts)
  const hi = Math.max(...rtts)
  const num = (n: number) => String(Math.round(n * 100) / 100)
  return points
    .map(([ago, rtt], i) => {
      const x = (Math.min(SPAN_MS, Math.max(0, SPAN_MS - ago)) / SPAN_MS) * w
      const y = hi === lo ? h / 2 : h - ((rtt - lo) / (hi - lo)) * h
      return `${i ? 'L' : 'M'}${num(x)} ${num(y)}`
    })
    .join(' ')
}

/** Plain text for bug reports: `key: value` lines, `—` where a value is missing. */
export function detailsText(r: LinkReport, channelName: string): string {
  const v = (n: number | null, unit: string) => (n == null ? '—' : `${Math.round(n * 10) / 10}${unit}`)
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
