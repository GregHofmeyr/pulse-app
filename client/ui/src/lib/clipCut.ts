// The clip editor's cut model: pure and immutable, times in integer milliseconds.
// The video is a row of pieces; each is kept or dropped. Cuts split a piece, edges drag, undo steps back.

export type Piece = { start: number; end: number; dropped: boolean }
export type Cut = {
  duration: number
  pieces: Piece[]
  selected: number | null
  /** Piece lists to step back to, newest last. */
  past: Piece[][]
  /** The edge being dragged: further drags of it join the undo step already taken. */
  lastDrag: number | null
}

/** No piece is ever shorter than this. */
export const MIN_PIECE_MS = 100
/** The most a clip may keep (1:30). */
export const MAX_KEPT_MS = 90_000
/** The least a clip may keep. */
const MIN_KEPT_MS = 1000

export function newCut(duration: number): Cut {
  const end = Number.isFinite(duration) ? Math.max(0, Math.round(duration)) : 0
  return { duration: end, pieces: [{ start: 0, end, dropped: false }], selected: null, past: [], lastDrag: null }
}

/** The same cut with `pieces` as a new undo step. */
function step(c: Cut, pieces: Piece[], selected: number | null): Cut {
  return { ...c, pieces, selected, past: [...c.past, c.pieces], lastDrag: null }
}

/** Splits the piece under `t`; nothing happens if either side would be under MIN_PIECE_MS. */
export function cutAt(c: Cut, t: number): Cut {
  t = Math.round(t)
  const i = c.pieces.findIndex((p) => t > p.start && t < p.end)
  if (i < 0) return c
  const p = c.pieces[i]
  if (t - p.start < MIN_PIECE_MS || p.end - t < MIN_PIECE_MS) return c
  const pieces = [...c.pieces.slice(0, i), { ...p, end: t }, { ...p, start: t }, ...c.pieces.slice(i + 1)]
  return step(c, pieces, i + 1)
}

/** Selects a piece; selecting a dropped one brings it back (one undo step). */
export function select(c: Cut, i: number): Cut {
  const p = c.pieces[i]
  if (!p) return c
  if (!p.dropped) return { ...c, selected: i, lastDrag: null }
  return step(c, c.pieces.map((q, j) => (j === i ? { ...q, dropped: false } : q)), i)
}

export function dropSelected(c: Cut): Cut {
  const i = c.selected
  if (i === null || !c.pieces[i] || c.pieces[i].dropped) return c
  return step(c, c.pieces.map((q, j) => (j === i ? { ...q, dropped: true } : q)), i)
}

/** Moves the edge between pieces `edge - 1` and `edge`, keeping both at least MIN_PIECE_MS. Drags of the same edge in a row are one undo step (until `endDrag`). */
export function dragEdge(c: Cut, edge: number, t: number): Cut {
  if (!Number.isFinite(t) || !Number.isInteger(edge) || edge < 1 || edge >= c.pieces.length) return c
  const a = c.pieces[edge - 1]
  const b = c.pieces[edge]
  const at = Math.min(Math.max(Math.round(t), a.start + MIN_PIECE_MS), b.end - MIN_PIECE_MS)
  const pieces = c.pieces.map((p, j) => (j === edge - 1 ? { ...p, end: at } : j === edge ? { ...p, start: at } : p))
  if (c.lastDrag === edge) return { ...c, pieces }
  if (at === a.end) return c
  return { ...step(c, pieces, c.selected), lastDrag: edge }
}

/** The pointer was released: the next drag of any edge is a new undo step. */
export function endDrag(c: Cut): Cut {
  return c.lastDrag === null ? c : { ...c, lastDrag: null }
}

export function undo(c: Cut): Cut {
  if (c.past.length === 0) return c
  const pieces = c.past[c.past.length - 1]
  // Indexes shift when a cut is undone, so nothing stays selected (no silent drop of the wrong piece).
  return { ...c, pieces, selected: null, past: c.past.slice(0, -1), lastDrag: null }
}

/** The kept spans, adjacent kept pieces merged. */
export function kept(c: Cut): { start: number; end: number }[] {
  const out: { start: number; end: number }[] = []
  for (const p of c.pieces) {
    if (p.dropped) continue
    const last = out[out.length - 1]
    if (last && last.end === p.start) last.end = p.end
    else out.push({ start: p.start, end: p.end })
  }
  return out
}

export function keptMs(c: Cut): number {
  return c.pieces.reduce((n, p) => (p.dropped ? n : n + p.end - p.start), 0)
}

/** How much kept time lies before source time `t` ("0:12 / 0:41 kept"). */
export function keptTimeAt(c: Cut, t: number): number {
  return c.pieces.reduce((n, p) => (p.dropped ? n : n + Math.max(0, Math.min(t, p.end) - p.start)), 0)
}

/** Where playback may be at `t`: `t` if kept, else the next kept start, else null. */
export function nextPlayable(c: Cut, t: number): number | null {
  for (const p of c.pieces) {
    if (p.dropped || p.end <= t) continue
    return Math.max(t, p.start)
  }
  return null
}

/** How many kept pieces there are (adjacent ones count as one). */
export function pieceCount(c: Cut): number {
  return kept(c).length
}

/** Why it cannot be sent, if it cannot. */
export function sendBlock(c: Cut): null | 'over' | 'empty' {
  const ms = keptMs(c)
  if (ms < MIN_KEPT_MS) return 'empty'
  return ms > MAX_KEPT_MS ? 'over' : null
}
