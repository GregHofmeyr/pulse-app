/** The part of a picture to keep, in source pixels. */
export type CropRect = { x: number; y: number; w: number; h: number }

/** Width over height of the crop for each kind. */
export const ASPECT = { avatar: 1, banner: 960 / 384 }

/** The shortest side a crop can shrink to, in source pixels. */
const MIN_SIDE = 32

const clamp = (v: number, lo: number, hi: number) => Math.min(Math.max(v, lo), hi)

/** The widest crop of `aspect` that fits the image. */
const maxWidth = (imgW: number, imgH: number, aspect: number) => Math.min(imgW, imgH * aspect)

/** Keeps the rectangle inside the image, in fractional pixels (so small drags and zoom steps add up). */
function fit(x: number, y: number, w: number, h: number, imgW: number, imgH: number): CropRect {
  const fw = clamp(w, 1, imgW)
  const fh = clamp(h, 1, imgH)
  return { x: clamp(x, 0, imgW - fw), y: clamp(y, 0, imgH - fh), w: fw, h: fh }
}

/** `c` in whole pixels, still inside the image: what is sent on Apply. */
export function roundCrop(c: CropRect, imgW: number, imgH: number): CropRect {
  const w = clamp(Math.round(c.w), 1, imgW)
  const h = clamp(Math.round(c.h), 1, imgH)
  return { x: clamp(Math.round(c.x), 0, imgW - w), y: clamp(Math.round(c.y), 0, imgH - h), w, h }
}

/** The largest centred rectangle of `aspect`, in whole pixels. */
export function initialCrop(imgW: number, imgH: number, aspect: number): CropRect {
  const w = Math.max(1, Math.floor(maxWidth(imgW, imgH, aspect)))
  const h = w / aspect
  return roundCrop(fit((imgW - w) / 2, (imgH - h) / 2, w, h, imgW, imgH), imgW, imgH)
}

/** Zooms in by `factor` (above 1 shrinks the rectangle, below 1 grows it) about `focus`, a point in the image. */
export function zoomCrop(c: CropRect, imgW: number, imgH: number, aspect: number, factor: number, focus: { x: number; y: number }): CropRect {
  const max = maxWidth(imgW, imgH, aspect)
  const min = Math.min(max, aspect >= 1 ? MIN_SIDE * aspect : MIN_SIDE)
  const w = clamp(c.w / factor, min, max)
  const h = w / aspect
  const x = focus.x - ((focus.x - c.x) * w) / c.w
  const y = focus.y - ((focus.y - c.y) * h) / c.h
  return fit(x, y, w, h, imgW, imgH)
}

/** Moves the rectangle by (`dx`, `dy`) source pixels, keeping it inside the image. */
export function panCrop(c: CropRect, imgW: number, imgH: number, dx: number, dy: number): CropRect {
  return fit(c.x + dx, c.y + dy, c.w, c.h, imgW, imgH)
}

const PNG_SIGNATURE = [137, 80, 78, 71, 13, 10, 26, 10]

/**
 * `bytes` without a PNG's eXIf chunks; anything else as it is. The server turns only a JPEG by
 * its EXIF orientation, which every webview does too. For a PNG's they differ (WebView2 turns it,
 * WebKitGTK doesn't), so the crop dialog shows a PNG without it: as stored, as the server crops it.
 */
export function withoutPngExif(bytes: Uint8Array<ArrayBuffer>): Uint8Array<ArrayBuffer> {
  if (bytes.length < 8 || PNG_SIGNATURE.some((b, i) => bytes[i] !== b)) return bytes
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength)
  const kept: Uint8Array<ArrayBuffer>[] = [bytes.subarray(0, 8)]
  let at = 8
  // chunk by chunk: length, tag, data, CRC; whatever can't be read as one is kept as it is
  while (at + 12 <= bytes.length) {
    const end = at + 12 + view.getUint32(at)
    if (end > bytes.length) break
    if (String.fromCharCode(...bytes.subarray(at + 4, at + 8)) !== 'eXIf') kept.push(bytes.subarray(at, end))
    at = end
  }
  kept.push(bytes.subarray(at))
  const length = kept.reduce((n, part) => n + part.length, 0)
  if (length === bytes.length) return bytes
  const out = new Uint8Array(length)
  kept.reduce((offset, part) => (out.set(part, offset), offset + part.length), 0)
  return out
}
