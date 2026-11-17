// Editing a queued image before it's sent (#290): crop (free or a fixed ratio), quarter turns, and simple pen
// strokes. The geometry lives here (tested in imageEdit.test.ts); ImageEditDialog.svelte draws it on a canvas.
//
// Three spaces: the source picture (W × H, as queued), the turned picture (after `q` clockwise quarter turns: H × W
// for an odd `q`), and the screen. Strokes are kept in source pixels, so turning never moves them on the picture;
// the crop is kept in turned pixels, as the person sees it.
import { zoomCrop, type CropRect } from './crop'

export type { CropRect }
export type Quarter = 0 | 1 | 2 | 3
export type Point = { x: number; y: number }
export type Stroke = { color: string; width: number; points: Point[] }
export type Corner = 'nw' | 'ne' | 'sw' | 'se'

/** The ratios offered, width over height; `null` is free. */
export const RATIOS: readonly { label: string; value: number | null }[] = [
  { label: 'Free', value: null },
  { label: '1:1', value: 1 },
  { label: '4:3', value: 4 / 3 },
  { label: '3:2', value: 3 / 2 },
  { label: '16:9', value: 16 / 9 },
]

/** The smallest crop side, in pixels (less on a picture smaller than that). */
const MIN_SIDE = 16

const clamp = (v: number, lo: number, hi: number) => Math.min(Math.max(v, lo), hi)

export const turn = (q: Quarter, by: 1 | -1): Quarter => (((q + by) % 4) + 4) % 4 as Quarter

/** The picture's size after `q` quarter turns. */
export const turnedSize = (w: number, h: number, q: Quarter): { w: number; h: number } => (q % 2 ? { w: h, h: w } : { w, h })

/** A source point, on the picture turned `q` quarters clockwise. */
export function toTurned(p: Point, w: number, h: number, q: Quarter): Point {
  switch (q) {
    case 1: return { x: h - p.y, y: p.x }
    case 2: return { x: w - p.x, y: h - p.y }
    case 3: return { x: p.y, y: w - p.x }
    default: return { x: p.x, y: p.y }
  }
}

/** A point on the turned picture, back in source pixels. */
export function fromTurned(p: Point, w: number, h: number, q: Quarter): Point {
  switch (q) {
    case 1: return { x: p.y, y: h - p.x }
    case 2: return { x: w - p.x, y: h - p.y }
    case 3: return { x: w - p.y, y: p.x }
    default: return { x: p.x, y: p.y }
  }
}

/** The canvas transform that draws the source picture turned `q` quarters: `[a, b, c, d, e, f]` for `setTransform`
 *  (before any scale), so (x, y) lands where [`toTurned`] says. */
export function turnMatrix(w: number, h: number, q: Quarter): [number, number, number, number, number, number] {
  switch (q) {
    case 1: return [0, 1, -1, 0, h, 0]
    case 2: return [-1, 0, 0, -1, w, h]
    case 3: return [0, -1, 1, 0, 0, w]
    default: return [1, 0, 0, 1, 0, 0]
  }
}

/** A crop on a picture `w` × `h` (turned space) after one more quarter turn (`by` 1 clockwise, -1 back). */
export function turnCrop(c: CropRect, w: number, h: number, by: 1 | -1): CropRect {
  return by === 1 ? { x: h - (c.y + c.h), y: c.x, w: c.h, h: c.w } : { x: c.y, y: w - (c.x + c.w), w: c.h, h: c.w }
}

/** The whole picture, or the largest centred crop of `ratio`, in whole pixels. */
export function fullCrop(w: number, h: number, ratio: number | null): CropRect {
  if (ratio === null) return { x: 0, y: 0, w, h }
  const cw = Math.max(1, Math.floor(Math.min(w, h * ratio)))
  const ch = Math.max(1, Math.min(h, Math.round(cw / ratio)))
  return { x: Math.floor((w - cw) / 2), y: Math.floor((h - ch) / 2), w: cw, h: ch }
}

/** The crop with one corner dragged to `p` (turned pixels): the opposite corner stays put, the crop stays on the
 *  picture and at least `MIN_SIDE` a side, and keeps `ratio` when there is one. */
export function dragCorner(c: CropRect, w: number, h: number, corner: Corner, p: Point, ratio: number | null): CropRect {
  const sx = corner.endsWith('e') ? 1 : -1
  const sy = corner.startsWith('s') ? 1 : -1
  const ax = sx > 0 ? c.x : c.x + c.w
  const ay = sy > 0 ? c.y : c.y + c.h
  const maxW = sx > 0 ? w - ax : ax
  const maxH = sy > 0 ? h - ay : ay
  const minSide = Math.min(MIN_SIDE, w, h)
  let nw: number
  let nh: number
  if (ratio === null) {
    nw = clamp(sx * (p.x - ax), Math.min(minSide, maxW), maxW)
    nh = clamp(sy * (p.y - ay), Math.min(minSide, maxH), maxH)
  } else {
    const most = Math.min(maxW, maxH * ratio)
    const least = Math.min(most, ratio >= 1 ? minSide * ratio : minSide)
    nw = clamp(Math.max(sx * (p.x - ax), sy * (p.y - ay) * ratio), least, most)
    nh = nw / ratio
  }
  return { x: sx > 0 ? ax : ax - nw, y: sy > 0 ? ay : ay - nh, w: nw, h: nh }
}

/** The crop grown (`factor` above 1) or shrunk about its centre, keeping its shape: the keyboard's + and −. */
export function growCrop(c: CropRect, w: number, h: number, factor: number): CropRect {
  // zoomCrop zooms *in* by its factor (a smaller rectangle), so growing is its inverse
  return zoomCrop(c, w, h, c.w / c.h, 1 / factor, { x: c.x + c.w / 2, y: c.y + c.h / 2 })
}

/** The crop in whole pixels, on the picture: what the edit is cut to. */
export function wholeCrop(c: CropRect, w: number, h: number): CropRect {
  const cw = clamp(Math.round(c.w), 1, w)
  const ch = clamp(Math.round(c.h), 1, h)
  return { x: clamp(Math.round(c.x), 0, w - cw), y: clamp(Math.round(c.y), 0, h - ch), w: cw, h: ch }
}

/** Nothing would change: the whole picture, unturned, nothing drawn (Save then just closes). */
export const unchanged = (c: CropRect, w: number, h: number, q: Quarter, strokes: readonly Stroke[]): boolean =>
  q === 0 && strokes.length === 0 && Math.round(c.x) === 0 && Math.round(c.y) === 0 && Math.round(c.w) === w && Math.round(c.h) === h

/** The edited picture's file name: the same name, as a PNG (the composer converts it to WebP again on queueing, so
 *  saving it losslessly here means it's compressed only once more). */
export function editedName(name: string): string {
  const dot = name.lastIndexOf('.')
  return `${dot > 0 ? name.slice(0, dot) : name || 'image'}.png`
}
