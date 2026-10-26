import { describe, expect, it } from 'vitest'
import { inboundTrouble, layerLine, noteLabel } from './diagText'
import type { FpsNote, InboundSummary, OutboundLayer } from './stats'

const inb = (o: Partial<InboundSummary> = {}): InboundSummary => ({
  fps: 30, w: 1280, h: 720, kbps: 2000, decoder: 'x', powerEfficient: true,
  freezes: 0, freezeMs: 0, lost: 0, jitterMsPerFrame: null, nacks: 0, plis: 0, stillLikely: false, ...o,
})

describe('diagText', () => {
  it('maps every note to its wording', () => {
    const want: Record<FpsNote, string> = {
      ok: 'OK',
      still: 'Still picture (normal)',
      'capture-limited': 'Capture is below target',
      'encoder-limited': "Encoder can't keep up",
      'network-limited': 'Upload is limiting',
    }
    for (const [k, v] of Object.entries(want)) expect(noteLabel(k as FpsNote)).toBe(v)
  })
  it('a still picture never counts freezes as trouble', () => {
    expect(inboundTrouble(inb({ freezes: 3, freezeMs: 4000, stillLikely: true }))).toBe(false)
  })
  it('freezes on a moving picture are trouble', () => {
    expect(inboundTrouble(inb({ freezes: 1 }))).toBe(true)
    expect(inboundTrouble(inb())).toBe(false)
  })
  it('formats a layer line', () => {
    const l: OutboundLayer = { rid: 'f', w: 1280, h: 720, fps: 59.6, kbps: 3500.4, encoder: 'e', powerEfficient: true, encodeMs: 4.26, qlr: 'none' }
    expect(layerLine(l)).toBe('1280×720 60fps 3500kbps 4.3ms')
    expect(layerLine({ ...l, encodeMs: null })).toBe('1280×720 60fps 3500kbps')
  })
})
