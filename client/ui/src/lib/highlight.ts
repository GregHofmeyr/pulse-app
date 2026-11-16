// A search hit's text in plain and matched runs, for rendering as text nodes (never as HTML).
import { mentionSpans } from './composerDom'
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
export function lineWithHit(text: string, ranges: HighlightRange[]): { text: string; highlights: HighlightRange[]; index: number } {
  let start = 0
  const lines = text.split('\n').map((line) => {
    const l = { text: line, start }
    start += line.length + 1
    return l
  })
  const inLine = (l: { text: string; start: number }) =>
    ranges.filter((r) => r.end > l.start && r.start < l.start + l.text.length).map((r) => ({ start: Math.max(0, r.start - l.start), end: Math.min(l.text.length, r.end - l.start) }))
  const found = lines.findIndex((l) => inLine(l).length)
  const index = found < 0 ? 0 : found
  return { text: lines[index].text, highlights: inLine(lines[index]), index }
}

/** A search hit's text with each known `@username` written as `@` + `label` of its person (#114: the username never
 *  shows, as in messages and previews), its ranges moved to match. A range wholly outside a mention shifts with the
 *  text; one that touches part of a mention covers the whole name written there, so the highlight stays on the visible
 *  text. `lookup` and `label` are those of `showMentions` (mentionStyles.ts). */
export function relabelMentions(
  text: string,
  ranges: HighlightRange[],
  lookup: (name: string) => string | null,
  label: (userId: string) => string,
): { text: string; highlights: HighlightRange[] } {
  // each mention replaced, in the old text's offsets and the new one's
  const swaps: { start: number; end: number; newStart: number; newEnd: number }[] = []
  let out = ''
  let at = 0
  for (const m of mentionSpans(text)) {
    const id = lookup(m.username)
    if (!id) continue
    out += text.slice(at, m.start)
    const shown = `@${label(id)}`
    swaps.push({ start: m.start, end: m.end, newStart: out.length, newEnd: out.length + shown.length })
    out += shown
    at = m.end
  }
  if (!swaps.length) return { text, highlights: ranges }
  out += text.slice(at)
  const move = (p: number, side: 'start' | 'end'): number => {
    let delta = 0
    for (const s of swaps) {
      if (p > s.start && p < s.end) return side === 'start' ? s.newStart : s.newEnd
      if (s.end <= p) delta = s.newEnd - s.end
    }
    return p + delta
  }
  return { text: out, highlights: ranges.map((r) => ({ start: move(r.start, 'start'), end: move(r.end, 'end') })) }
}
