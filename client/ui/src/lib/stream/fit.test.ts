// @vitest-environment jsdom
// A preview's layer from its size on screen in device pixels (#582).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { devicePixels, DOWN_MS, fitStep, layerForSize, observeFit } from './fit'

describe('layerForSize', () => {
  it('the_mini_preview_at_100_and_200_percent_scale_takes_the_low_layer', () => {
    expect(layerForSize(320, 180)).toBe('low')
    expect(layerForSize(640, 360)).toBe('low')
  })
  it('a_typical_tile_on_a_scaled_screen_takes_the_middle_layer', () => {
    // a 480 x 270 tile at 150 %
    expect(layerForSize(720, 405)).toBe('medium')
    expect(layerForSize(1280, 720)).toBe('medium')
  })
  it('stretching_a_layer_up_by_a_tenth_is_still_that_layer', () => {
    expect(layerForSize(704, 396)).toBe('low')
    expect(layerForSize(705, 396)).toBe('medium')
    expect(layerForSize(1408, 792)).toBe('medium')
  })
  it('a_big_stage_tile_takes_the_top_layer', () => {
    expect(layerForSize(1500, 844)).toBe('high')
    expect(layerForSize(2560, 1440)).toBe('high')
  })
  it('either_side_too_big_counts', () => {
    // a tall, narrow box (a portrait window share letterboxed in it) still needs the height
    expect(layerForSize(400, 500)).toBe('medium')
  })
})

describe('fitStep', () => {
  it('up_down_or_same', () => {
    expect(fitStep('low', 'medium')).toBe('up')
    expect(fitStep('low', 'high')).toBe('up')
    expect(fitStep('high', 'medium')).toBe('down')
    expect(fitStep('medium', 'medium')).toBe('same')
  })
})

type Entry = Pick<ResizeObserverEntry, 'contentRect' | 'devicePixelContentBoxSize'>
const entry = (w: number, h: number, device?: [number, number]): Entry => ({
  contentRect: { width: w, height: h } as DOMRectReadOnly,
  devicePixelContentBoxSize: (device ? [{ inlineSize: device[0], blockSize: device[1] }] : []) as unknown as ResizeObserverSize[],
})

describe('devicePixels', () => {
  it('prefers_the_device_pixel_box_and_falls_back_to_css_times_the_scale', () => {
    expect(devicePixels(entry(480, 270, [720, 405]) as ResizeObserverEntry, 1)).toEqual({ w: 720, h: 405 })
    expect(devicePixels(entry(480, 270) as ResizeObserverEntry, 1.5)).toEqual({ w: 720, h: 405 })
  })
  it('no_size_is_none', () => {
    expect(devicePixels(entry(0, 0, [0, 0]) as ResizeObserverEntry, 1)).toBeNull()
    expect(devicePixels(entry(0, 0) as ResizeObserverEntry, 2)).toBeNull()
  })
})

describe('observeFit', () => {
  let observers: { cb: (e: Entry[]) => void; box?: string; disconnected: boolean }[]
  beforeEach(() => {
    vi.useFakeTimers()
    observers = []
    vi.stubGlobal(
      'ResizeObserver',
      class {
        o: (typeof observers)[number]
        constructor(cb: (e: Entry[]) => void) {
          this.o = { cb, disconnected: false }
          observers.push(this.o)
        }
        observe(_el: Element, opts?: { box?: string }) {
          this.o.box = opts?.box
        }
        disconnect() {
          this.o.disconnected = true
        }
      },
    )
  })
  afterEach(() => {
    vi.unstubAllGlobals()
    vi.useRealTimers()
  })
  const resize = (w: number, h: number) => observers[0].cb([entry(w, h, [w, h])])

  it('watches_the_device_pixel_box', () => {
    observeFit(document.createElement('video'), () => {})
    expect(observers[0].box).toBe('device-pixel-content-box')
  })

  it('switches_up_at_once_when_the_tile_grows', () => {
    const seen: string[] = []
    observeFit(document.createElement('video'), (l) => seen.push(l))
    resize(600, 338)
    expect(seen).toEqual([]) // still low
    resize(900, 506)
    expect(seen).toEqual(['medium'])
    resize(1600, 900)
    expect(seen).toEqual(['medium', 'high'])
  })

  it('switches_down_only_once_it_has_stayed_smaller', () => {
    const seen: string[] = []
    observeFit(document.createElement('video'), (l) => seen.push(l))
    resize(1600, 900)
    resize(900, 506)
    vi.advanceTimersByTime(DOWN_MS - 1)
    expect(seen).toEqual(['high'])
    resize(500, 281) // shrank further meanwhile: lands on where it is by then
    vi.advanceTimersByTime(1)
    expect(seen).toEqual(['high', 'low'])
  })

  it('a_shrink_that_grows_back_in_time_changes_nothing', () => {
    const seen: string[] = []
    observeFit(document.createElement('video'), (l) => seen.push(l))
    resize(1600, 900)
    resize(500, 281)
    resize(1600, 900)
    vi.advanceTimersByTime(DOWN_MS * 2)
    expect(seen).toEqual(['high'])
  })

  it('no_size_keeps_the_layer_it_has', () => {
    const seen: string[] = []
    observeFit(document.createElement('video'), (l) => seen.push(l))
    resize(900, 506)
    observers[0].cb([entry(0, 0, [0, 0])]) // moved out of the page between hosts
    vi.advanceTimersByTime(DOWN_MS * 2)
    expect(seen).toEqual(['medium'])
  })

  it('stopping_disconnects_and_drops_a_pending_switch_down', () => {
    const seen: string[] = []
    const stop = observeFit(document.createElement('video'), (l) => seen.push(l))
    resize(1600, 900)
    resize(500, 281)
    stop()
    vi.advanceTimersByTime(DOWN_MS * 2)
    expect(seen).toEqual(['high'])
    expect(observers[0].disconnected).toBe(true)
  })
})
