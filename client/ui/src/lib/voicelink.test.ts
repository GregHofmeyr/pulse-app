import { describe, expect, it } from 'vitest'
import { detailsText, jitterLevel, lossLevel, pingLevel, sparklinePath, transportLabel } from './voicelink'
import type { LinkReport } from './voiceui'

const report: LinkReport = { rtt_ms: 42, jitter_ms: 3, loss_pct: 0.5, transport: 'udp', server: 'voice.example.test', version: '1.2.3', history: [] }

describe('voicelink', () => {
  it('ping_thresholds', () => {
    expect([99, 100, 199, 200].map(pingLevel)).toEqual(['good', 'warn', 'warn', 'bad'])
  })
  it('jitter_thresholds', () => {
    expect([19, 20, 49, 50].map(jitterLevel)).toEqual(['good', 'warn', 'warn', 'bad'])
  })
  it('loss_thresholds', () => {
    expect([1, 1.1, 5, 5.1].map(lossLevel)).toEqual(['good', 'warn', 'warn', 'bad'])
  })
  it('transport_labels', () => {
    expect([transportLabel('udp'), transportLabel('tcp'), transportLabel('turn')]).toEqual(['UDP', 'TCP', 'TURN relay'])
  })
  it('details_text_lists_every_value_and_the_version', () => {
    const t = detailsText(report, 'Lounge')
    expect(t).toContain('Pulse version: 1.2.3')
    expect(t).toContain('Channel: Lounge')
    expect(t).toContain('Voice server: voice.example.test')
    expect(t).toContain('Connection type: UDP')
    expect(t).toContain('Ping: 42 ms')
    expect(t).toContain('Jitter: 3 ms')
    expect(t).toContain('Packet loss: 0.5%')
    const blank = detailsText({ ...report, rtt_ms: null, jitter_ms: null, loss_pct: null, transport: null, server: null }, 'Lounge')
    expect(blank).toContain('Ping: —')
    expect(blank).toContain('Jitter: —')
    expect(blank).toContain('Packet loss: —')
    expect(blank).toContain('Voice server: —')
    expect(blank).toContain('Connection type: —')
  })
  it('sparkline_spans_sixty_seconds', () => {
    expect(sparklinePath([], 120, 30)).toBe('')
    const d = sparklinePath([[60_000, 10], [30_000, 50], [0, 30]], 120, 30)
    const xs = [...d.matchAll(/[ML]\s*([\d.]+)[ ,]/g)].map((m) => Number(m[1]))
    expect(xs).toEqual([0, 60, 120])
    const ys = [...d.matchAll(/[ML]\s*[\d.]+[ ,]([\d.]+)/g)].map((m) => Number(m[1]))
    expect(Math.min(...ys)).toBeGreaterThanOrEqual(0)
    expect(Math.max(...ys)).toBeLessThanOrEqual(30)
    expect(ys[1]).toBeLessThan(ys[0]) // higher ping draws higher up
  })
})
