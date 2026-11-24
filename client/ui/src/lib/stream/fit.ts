// Which simulcast layer a stream preview (a grid tile or the mini-preview) needs, from its size on screen in device
// pixels (#582). Previews used to take the low layer whatever their size, so a big stage tile, or any tile on a
// high-DPI screen, showed 640 x 360 stretched up. The watch's `<video>` is observed wherever it's hosted.
import { LOW_LAYER, MID_LAYER } from './encodings'
import type { Layer } from './viewer'

/** Stretching a layer up by this much isn't visible; past it the next layer up is asked for. */
const SLACK = 1.1
/** Going down waits this long (going up is at once), so a resize drag or a tile gliding into place doesn't flap. */
export const DOWN_MS = 2_000

const RANK: Record<Layer, number> = { low: 0, medium: 1, high: 2 }
const fits = (l: { width: number; height: number }, w: number, h: number) => w <= l.width * SLACK && h <= l.height * SLACK

/** The smallest layer that covers `w` x `h` device pixels. */
export function layerForSize(w: number, h: number): Layer {
  if (fits(LOW_LAYER, w, h)) return 'low'
  if (fits(MID_LAYER, w, h)) return 'medium'
  return 'high'
}

/** Up (a soft picture now), down (wait a moment), or nothing to do. */
export function fitStep(current: Layer, wanted: Layer): 'up' | 'down' | 'same' {
  const d = RANK[wanted] - RANK[current]
  return d > 0 ? 'up' : d < 0 ? 'down' : 'same'
}

/** An observed size in device pixels, or null while it has none (out of the page, between hosts, hidden). */
export function devicePixels(e: ResizeObserverEntry, dpr: number): { w: number; h: number } | null {
  const d = e.devicePixelContentBoxSize?.[0]
  const w = d ? d.inlineSize : e.contentRect.width * dpr
  const h = d ? d.blockSize : e.contentRect.height * dpr
  return w > 0 && h > 0 ? { w, h } : null
}

/**
 * Follows `el`'s size and calls `onChange` with the layer that fits it whenever that changes (it starts at low). The
 * device-pixel box also changes with the screen's scale (the window moved to another monitor), so that's followed
 * too. Returns the stop. Without ResizeObserver it never changes from low.
 */
export function observeFit(el: Element, onChange: (layer: Layer) => void): () => void {
  if (typeof ResizeObserver === 'undefined') return () => {}
  let current: Layer = 'low'
  let wanted: Layer = 'low'
  let down: ReturnType<typeof setTimeout> | undefined
  const cancelDown = () => {
    clearTimeout(down)
    down = undefined
  }
  const settle = () => {
    cancelDown()
    if (wanted === current) return
    current = wanted
    onChange(current)
  }
  const ro = new ResizeObserver((entries) => {
    const e = entries[entries.length - 1]
    const size = e && devicePixels(e, globalThis.devicePixelRatio || 1)
    if (!size) return // keep what it has: it's on its way to another host, or about to go
    wanted = layerForSize(size.w, size.h)
    const step = fitStep(current, wanted)
    if (step === 'up') settle()
    else if (step === 'same') cancelDown()
    else down ??= setTimeout(settle, DOWN_MS) // settles on the size it has by then
  })
  try {
    ro.observe(el, { box: 'device-pixel-content-box' })
  } catch {
    ro.observe(el) // an engine without that box: CSS pixels times the scale
  }
  return () => {
    cancelDown()
    ro.disconnect()
  }
}
