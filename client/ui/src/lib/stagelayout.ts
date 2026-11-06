// Where the voice stage's tiles go (#222): pure, so it's tested without the UI (stagelayout.test.ts).
// Every tile (a person or a stream) is the same size at one aspect. The row count is the one that makes the
// biggest tiles in the stage, so 1 nearly fills it, 2 sit side by side or stack by the stage's shape, 3 make a
// pyramid, 4 a 2x2, 5 and 6 a 3 + 2 / 3 x 2; a short last row is centred, and so is the whole arrangement.

export type Rect = { x: number; y: number; w: number; h: number }

/** The tiles' shape: the stream and user tiles were 16:9 before (#222) and stay so. */
export const TILE_ASPECT = 16 / 9
/** A tile never grows wider than this, however big the window. */
export const TILE_MAX_W = 1440
/** Nor narrower: past this the stage scrolls instead. */
export const TILE_MIN_W = 192

export function stageLayout(a: {
  count: number
  width: number
  height: number
  aspect?: number
  gap?: number
  pad?: number
  maxW?: number
  minW?: number
}): { rects: Rect[]; height: number } {
  const { count: n, aspect = TILE_ASPECT, gap = 14, pad = 20, maxW = TILE_MAX_W, minW = TILE_MIN_W } = a
  const innerW = Math.max(0, a.width - 2 * pad)
  const innerH = Math.max(0, a.height - 2 * pad)
  if (n <= 0) return { rects: [], height: a.height }

  let w = -1
  let cols = n
  for (let r = 1; r <= n; r++) {
    const c = Math.ceil(n / r)
    const rr = Math.ceil(n / c)
    if (rr !== r) continue // the same arrangement as a smaller row count
    const fit = Math.min((innerW - (c - 1) * gap) / c, ((innerH - (rr - 1) * gap) / rr) * aspect)
    if (fit > w) {
      w = fit
      cols = c
    }
  }
  w = Math.min(w, maxW)
  if (w < minW) {
    w = minW
    cols = Math.max(1, Math.min(n, Math.floor((innerW + gap) / (minW + gap))))
  }
  const h = w / aspect
  const rowCount = Math.ceil(n / cols)
  const contentH = rowCount * h + (rowCount - 1) * gap
  const top = pad + Math.max(0, (innerH - contentH) / 2)

  const rects: Rect[] = []
  for (let row = 0; row < rowCount; row++) {
    const k = row < rowCount - 1 ? cols : n - cols * (rowCount - 1)
    const rowW = k * w + (k - 1) * gap
    const x0 = pad + Math.max(0, (innerW - rowW) / 2)
    const y = top + row * (h + gap)
    for (let i = 0; i < k; i++) rects.push({ x: x0 + i * (w + gap), y, w, h })
  }
  return { rects, height: Math.max(a.height, contentH + 2 * pad) }
}

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v))

/** A user tile's avatar, name and mute badge for a tile `h` pixels tall: they grow with it, within limits. */
export function tileScale(h: number): { avatar: number; font: number; badge: number; icon: number } {
  const avatar = clamp(Math.round(h * 0.5), 48, 160)
  const font = clamp(Math.round(12 + h / 60), 14, 20)
  const badge = clamp(Math.round(avatar / 3), 22, 40)
  return { avatar, font, badge, icon: badge >= 34 ? 18 : 14 }
}

/** FLIP: the transform (origin top left) that draws a tile now at `to` where it was, at `from`; null if it hasn't moved. */
export function flipInverse(from: Rect, to: Rect): { dx: number; dy: number; sx: number; sy: number } | null {
  if (from.w <= 0 || from.h <= 0 || to.w <= 0 || to.h <= 0) return null
  const dx = from.x - to.x
  const dy = from.y - to.y
  const sx = from.w / to.w
  const sy = from.h / to.h
  if (Math.abs(dx) < 0.5 && Math.abs(dy) < 0.5 && Math.abs(sx - 1) < 0.005 && Math.abs(sy - 1) < 0.005) return null
  return { dx, dy, sx, sy }
}
