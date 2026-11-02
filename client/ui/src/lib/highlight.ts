// A search hit's text in plain and matched runs, for rendering as text nodes (never as HTML).
import type { HighlightRange } from './protocol/HighlightRange'

export type Segment = { text: string; hit: boolean }

/** `text` cut at `ranges` (UTF-16 offsets, as JS strings count). Ranges may come in any order, overlap
 *  or run past the end; empty ones are dropped. */
export function segments(text: string, ranges: HighlightRange[]): Segment[] {
  const sorted = ranges
    .map((r) => ({ start: Math.max(0, Math.min(r.start, text.length)), end: Math.max(0, Math.min(r.end, text.length)) }))
    .filter((r) => r.end > r.start)
    .sort((a, b) => a.start - b.start)
  const out: Segment[] = []
  let at = 0
  for (const r of sorted) {
    const start = Math.max(r.start, at)
    if (r.end <= start) continue
    if (start > at) out.push({ text: text.slice(at, start), hit: false })
    out.push({ text: text.slice(start, r.end), hit: true })
    at = r.end
  }
  if (at < text.length) out.push({ text: text.slice(at), hit: false })
  // Two matched runs that touch read as one.
  return out.reduce<Segment[]>((acc, s) => {
    const last = acc[acc.length - 1]
    if (last && last.hit === s.hit) last.text += s.text
    else acc.push({ ...s })
    return acc
  }, [])
}

/** One line of a multi-line text (a quote's lines): the first with a match, else the first, with the
 *  ranges that fall in it moved to its own offsets. */
export function lineWithHit(text: string, ranges: HighlightRange[]): { text: string; highlights: HighlightRange[] } {
  let start = 0
  const lines = text.split('\n').map((line) => {
    const l = { text: line, start }
    start += line.length + 1
    return l
  })
  const inLine = (l: { text: string; start: number }) =>
    ranges.filter((r) => r.end > l.start && r.start < l.start + l.text.length).map((r) => ({ start: Math.max(0, r.start - l.start), end: Math.min(l.text.length, r.end - l.start) }))
  const pick = lines.find((l) => inLine(l).length) ?? lines[0]
  return { text: pick.text, highlights: inLine(pick) }
}
