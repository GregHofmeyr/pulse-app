// Trim maths for the soundboard's add dialog (tested in soundTrim.test.ts). A sound is a span of the
// picked file, never longer than 6 s. A handle moves only its own edge and stops at 6 s (and at the clip's
// ends); dragging inside the selection moves the whole span.

export type Span = { start: number; end: number }

export const MAX_SPAN_MS = 6000
export const STEP_MS = 100

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v))

/** The first 6 s (or the whole clip when it's shorter). */
export const initialSpan = (durationMs: number): Span => ({ start: 0, end: Math.min(MAX_SPAN_MS, durationMs) })

/** Move the start handle only: no earlier than 6 s before the end, at least one step before it. */
export function moveStart(span: Span, ms: number, durationMs: number): Span {
  const lo = Math.max(0, span.end - MAX_SPAN_MS)
  const hi = Math.max(lo, span.end - Math.min(STEP_MS, durationMs))
  return { start: clamp(Math.round(ms), lo, hi), end: span.end }
}

/** Move the end handle only: no later than 6 s after the start (or the clip's end), one step after it. */
export function moveEnd(span: Span, ms: number, durationMs: number): Span {
  const hi = Math.min(durationMs, span.start + MAX_SPAN_MS)
  const lo = Math.min(hi, span.start + STEP_MS)
  return { start: span.start, end: clamp(Math.round(ms), lo, hi) }
}

/** Slide the whole span by `deltaMs`, kept inside the clip and the same length. */
export function moveSpan(span: Span, deltaMs: number, durationMs: number): Span {
  const len = span.end - span.start
  const start = clamp(Math.round(span.start + deltaMs), 0, Math.max(0, durationMs - len))
  return { start, end: start + len }
}

/** Where play/pause starts: a paused playhead inside the span, else the start. */
export const playFrom = (span: Span, at: number | null): number =>
  at !== null && at >= span.start && at < span.end ? at : span.start

/** A fraction (0..1) along the waveform as a millisecond in the clip. */
export const msAt = (fraction: number, durationMs: number): number => Math.round(clamp(fraction, 0, 1) * durationMs)

/** "3.2 s" */
export const spanLabel = (s: Span): string => `${((s.end - s.start) / 1000).toFixed(1)} s`
