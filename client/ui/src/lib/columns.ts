// Resizable side columns (#160): the channel list (left) and the members list / search results (right) are dragged
// wider or narrower; the main area between them keeps at least MAIN_MIN. What's saved is what you chose; what shows
// is that, fitted to the window, so shrinking the window narrows the columns and growing it gives them back.

export type Side = 'left' | 'right'

/** Each column's limits, and the left one's default. The right column's default depends on what it shows. */
export const COLUMNS = {
  left: { def: 264, min: 200, max: 420 },
  right: { min: 180, max: 480 },
} as const

/** The right column's default width, by what it holds (`chat`: a voice channel's chat, #226). */
export const RIGHT_DEFAULT = { members: 240, group: 220, search: 340, chat: 360 } as const

/** The main area's least width. With the defaults it fits the smallest window (900px) with room to spare. */
export const MAIN_MIN = 320

/** The body's gap between panels (one per side column shown). */
export const GAP = 8

/** null: the default (never resized, or reset by a double-click). */
export type SavedWidths = { left: number | null; right: number | null }

export type FitInput = {
  saved: SavedWidths
  /** The body's content width; 0 (not laid out yet, or no ResizeObserver): only the min and max apply. */
  body: number
  /** Which side columns are showing. */
  left: boolean
  right: boolean
  rightDefault: number
}

export type Fitted = { left: number; right: number; maxLeft: number; maxRight: number }

const clamp = (v: number, lo: number, hi: number) => Math.max(lo, Math.min(hi, v))

export function fitColumns({ saved, body, left: showLeft, right: showRight, rightDefault }: FitInput): Fitted {
  let left = clamp(Math.round(saved.left ?? COLUMNS.left.def), COLUMNS.left.min, COLUMNS.left.max)
  let right = clamp(Math.round(saved.right ?? rightDefault), COLUMNS.right.min, COLUMNS.right.max)
  if (body <= 0) return { left, right, maxLeft: COLUMNS.left.max, maxRight: COLUMNS.right.max }

  const shownLeft = () => (showLeft ? left : 0)
  const shownRight = () => (showRight ? right : 0)
  const room = body - (showLeft ? GAP : 0) - (showRight ? GAP : 0) - MAIN_MIN
  // Too wide for the window: a saved column gives back what was widened (right first: it's the one people widen for
  // long names), then down to its minimum. A column never resized keeps its default, so the layout is as it was
  // before resizing existed; past that, the main area squeezes as it always has.
  const give = (side: Side, floor: number) => {
    const over = shownLeft() + shownRight() - room
    if (over <= 0) return
    if (side === 'right' && showRight && saved.right !== null) right = Math.max(floor, right - over)
    if (side === 'left' && showLeft && saved.left !== null) left = Math.max(floor, left - over)
  }
  give('right', Math.min(right, rightDefault))
  give('left', Math.min(left, COLUMNS.left.def))
  give('right', COLUMNS.right.min)
  give('left', COLUMNS.left.min)

  const maxLeft = clamp(room - shownRight(), COLUMNS.left.min, COLUMNS.left.max)
  const maxRight = clamp(room - shownLeft(), COLUMNS.right.min, COLUMNS.right.max)
  return { left, right, maxLeft, maxRight }
}

/** One arrow-key step, and a bigger one with Shift. */
export const KEY_STEP = 16
export const KEY_STEP_BIG = 64

/** The width a key on a column's handle moves it to (null: not a resizing key). The left column's handle grows it
 *  with ArrowRight, the right column's with ArrowLeft: the handle moves the way the arrow points. */
export function keyWidth(side: Side, width: number, key: string, shift: boolean, min: number, max: number): number | null {
  const step = shift ? KEY_STEP_BIG : KEY_STEP
  const grow = side === 'left' ? 'ArrowRight' : 'ArrowLeft'
  const shrink = side === 'left' ? 'ArrowLeft' : 'ArrowRight'
  switch (key) {
    case grow: return clamp(width + step, min, max)
    case shrink: return clamp(width - step, min, max)
    case 'Home': return min
    case 'End': return max
    default: return null
  }
}
