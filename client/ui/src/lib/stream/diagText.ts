// Wording and formatting for the stream diagnostics overlay (pure; the component only lays it out).
import type { FpsNote, InboundSummary, OutboundLayer } from './stats'

const NOTES: Record<FpsNote, string> = {
  ok: 'OK',
  still: 'Still picture (normal)',
  'capture-limited': 'Capture is below target',
  'encoder-limited': "Encoder can't keep up",
  'network-limited': 'Upload is limiting',
}

export const noteLabel = (n: FpsNote): string => NOTES[n]

/** Freezes only count as trouble on a moving picture: a still screen freezes by design. */
export const inboundTrouble = (i: InboundSummary): boolean => !i.stillLikely && i.freezes > 0

export const layerLine = (l: OutboundLayer): string =>
  `${l.w}×${l.h} ${Math.round(l.fps)}fps ${Math.round(l.kbps)}kbps` + (l.encodeMs == null ? '' : ` ${l.encodeMs.toFixed(1)}ms`)
