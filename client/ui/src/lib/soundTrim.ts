// Trim maths for the soundboard's add dialog (tested in soundTrim.test.ts). A sound is a span of the
// picked file, never longer than 6 s. Moving one handle so the span would be longer drags the other
// along (rather than refusing the move), so the handle under your finger always goes where you put it.

export type Span = { start: number; end: number }

export const MAX_SPAN_MS = 6000
export const STEP_MS = 100

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v))

/** The first 6 s (or the whole clip when it's shorter). */
export const initialSpan = (durationMs: number): Span => ({ start: 0, end: Math.min(MAX_SPAN_MS, durationMs) })

/** Move the start handle; the end follows when the span would pass 6 s. At least one step apart. */
export function moveStart(span: Span, ms: number, durationMs: number): Span {
  const start = clamp(Math.round(ms), 0, Math.max(0, span.end - Math.min(STEP_MS, durationMs)))
  return { start, end: Math.min(span.end, start + MAX_SPAN_MS) }
}

/** Move the end handle; the start follows when the span would pass 6 s. At least one step apart. */
export function moveEnd(span: Span, ms: number, durationMs: number): Span {
  const minEnd = Math.min(durationMs, span.start + STEP_MS)
  const end = clamp(Math.round(ms), minEnd, durationMs)
  return { start: Math.max(span.start, end - MAX_SPAN_MS), end }
}

/** A fraction (0..1) along the waveform as a millisecond in the clip. */
export const msAt = (fraction: number, durationMs: number): number => Math.round(clamp(fraction, 0, 1) * durationMs)

/** "3.2 s" */
export const spanLabel = (s: Span): string => `${((s.end - s.start) / 1000).toFixed(1)} s`
