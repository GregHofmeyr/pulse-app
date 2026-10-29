// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import CropDialog from './CropDialog.svelte'
import { pngOf, pngTags } from '../lib/testPng'

let comp: ReturnType<typeof mount> | undefined
beforeEach(() => {
  Object.defineProperty(HTMLImageElement.prototype, 'naturalWidth', { configurable: true, get: () => 800 })
  Object.defineProperty(HTMLImageElement.prototype, 'naturalHeight', { configurable: true, get: () => 600 })
})
afterEach(() => {
  if (comp) unmount(comp)
  comp = undefined
  document.body.innerHTML = ''
  vi.restoreAllMocks()
})

const file = () => new File([new Uint8Array(10)], 'a.png', { type: 'image/png' })

async function open(kind: 'avatar' | 'banner' = 'avatar', f: File = file()) {
  const onApply = vi.fn()
  const onCancel = vi.fn()
  comp = mount(CropDialog, { target: document.body, props: { file: f, kind, onApply, onCancel } })
  const img = await vi.waitFor(() => {
    const i = document.querySelector('img')
    if (!i) throw new Error('no image yet')
    return i
  })
  img.dispatchEvent(new Event('load'))
  flushSync()
  return { onApply, onCancel }
}
const apply = () => [...document.querySelectorAll('button')].find((b) => b.textContent === 'Apply')!

describe('CropDialog', () => {
  it('Apply returns the current rect', async () => {
    const { onApply } = await open('avatar')
    apply().click()
    expect(onApply).toHaveBeenCalledWith({ x: 100, y: 0, w: 600, h: 600 })
  })

  it('arrow keys pan', async () => {
    const { onApply } = await open('avatar')
    const stage = document.querySelector<HTMLElement>('[aria-label="Crop area"]')!
    stage.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowLeft', bubbles: true }))
    flushSync()
    apply().click()
    expect(onApply).toHaveBeenLastCalledWith({ x: 92, y: 0, w: 600, h: 600 })
    stage.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowLeft', shiftKey: true, bubbles: true }))
    flushSync()
    apply().click()
    expect(onApply).toHaveBeenLastCalledWith({ x: 12, y: 0, w: 600, h: 600 })
  })

  const stage = () => document.querySelector<HTMLElement>('[aria-label="Crop area"]')!
  const box = () => document.querySelector<HTMLElement>('.box')!
  const integral = (c: { x: number; y: number; w: number; h: number }, w = 800, h = 600) =>
    [c.x, c.y, c.w, c.h].every(Number.isInteger) && c.x >= 0 && c.y >= 0 && c.x + c.w <= w && c.y + c.h <= h

  it('the wheel zooms about the pointer', async () => {
    const { onApply } = await open('avatar')
    const wheel = new WheelEvent('wheel', { deltaY: -100, clientX: 400, clientY: 300, bubbles: true, cancelable: true })
    stage().dispatchEvent(wheel)
    flushSync()
    expect(wheel.defaultPrevented).toBe(true)
    apply().click()
    const c = onApply.mock.lastCall![0]
    expect(c.w).toBe(Math.round(600 / 1.1))
    expect(c.w).toBe(c.h)
    expect(integral(c)).toBe(true)
    stage().dispatchEvent(new WheelEvent('wheel', { deltaY: 100, clientX: 400, clientY: 300, bubbles: true, cancelable: true }))
    flushSync()
    apply().click()
    expect(onApply.mock.lastCall![0]).toEqual({ x: 100, y: 0, w: 600, h: 600 })
  })

  it('the slider and + / - zoom', async () => {
    const { onApply } = await open('avatar')
    const slider = document.querySelector<HTMLInputElement>('input[type="range"]')!
    slider.value = '2'
    slider.dispatchEvent(new Event('input', { bubbles: true }))
    flushSync()
    apply().click()
    expect(onApply.mock.lastCall![0]).toEqual({ x: 250, y: 150, w: 300, h: 300 })
    stage().dispatchEvent(new KeyboardEvent('keydown', { key: '-', bubbles: true }))
    flushSync()
    apply().click()
    expect(onApply.mock.lastCall![0].w).toBe(330)
    stage().dispatchEvent(new KeyboardEvent('keydown', { key: '+', bubbles: true }))
    flushSync()
    apply().click()
    expect(onApply.mock.lastCall![0]).toEqual({ x: 250, y: 150, w: 300, h: 300 })
  })

  it('dragging pans, and sub-pixel moves add up', async () => {
    const { onApply } = await open('avatar')
    const at = (type: string, x: number) => stage().dispatchEvent(new MouseEvent(type, { clientX: x, clientY: 0, bubbles: true }))
    at('pointerdown', 100)
    for (let x = 100.4; x < 104; x += 0.4) at('pointermove', x)
    at('pointerup', 104)
    flushSync()
    apply().click()
    // dragged right by ~3.6 px (1 source px per screen px here): the crop moves left as much
    expect(onApply.mock.lastCall![0]).toEqual({ x: 96, y: 0, w: 600, h: 600 })
    at('pointermove', 50) // no longer dragging
    flushSync()
    apply().click()
    expect(onApply.mock.lastCall![0].x).toBe(96)
  })

  it('the mask is round for an avatar, square for a banner; a banner applies its own aspect', async () => {
    await open('avatar')
    expect(box().classList.contains('round')).toBe(true)
    unmount(comp!)
    comp = undefined
    document.body.innerHTML = ''
    const { onApply } = await open('banner')
    expect(box().classList.contains('round')).toBe(false)
    apply().click()
    expect(onApply).toHaveBeenCalledWith({ x: 0, y: 140, w: 800, h: 320 })
  })

  it('Apply hands over a plain snapshot', async () => {
    const { onApply } = await open('avatar')
    apply().click()
    const c = onApply.mock.lastCall![0]
    expect(Object.getPrototypeOf(c)).toBe(Object.prototype)
    expect(JSON.parse(JSON.stringify(c))).toEqual(c)
  })

  it('a file that is not a readable picture says so and leaves only Cancel', async () => {
    Object.defineProperty(HTMLImageElement.prototype, 'naturalWidth', { configurable: true, get: () => 0 })
    const onCancel = vi.fn()
    comp = mount(CropDialog, { target: document.body, props: { file: file(), kind: 'avatar', onApply: vi.fn(), onCancel } })
    const img = await vi.waitFor(() => {
      const i = document.querySelector('img')
      if (!i) throw new Error('no image yet')
      return i
    })
    img.dispatchEvent(new Event('error'))
    flushSync()
    expect(document.querySelector('[role="alert"]')!.textContent).toBe("That file isn't a picture Pulse can read (PNG, JPEG, GIF or WebP).")
    expect(document.querySelector('img')).toBeNull()
    expect([...document.querySelectorAll('button')].map((b) => b.textContent)).toEqual(['Cancel'])
  })

  it("shows a PNG without its eXIf (as the server crops it), and other files as they are", async () => {
    const shown = async (f: File) => {
      comp = mount(CropDialog, { target: document.body, props: { file: f, kind: 'avatar', onApply: vi.fn(), onCancel: vi.fn() } })
      const src = await vi.waitFor(() => {
        const s = document.querySelector('img')?.getAttribute('src')
        if (!s) throw new Error('no image yet')
        return s
      })
      unmount(comp)
      comp = undefined
      return Uint8Array.from(atob(src.slice(src.indexOf(',') + 1)), (c) => c.charCodeAt(0))
    }
    const png = await shown(new File([pngOf(['IHDR', 'eXIf', 'IDAT', 'IEND'])], 'a.png', { type: 'image/png' }))
    expect(pngTags(png)).toEqual(['IHDR', 'IDAT', 'IEND'])
    const jpeg = new Uint8Array([0xff, 0xd8, 0xff, 0xe1, 0, 8, 69, 120, 105, 102, 0, 0, 0xff, 0xd9])
    expect([...(await shown(new File([jpeg], 'a.jpg', { type: 'image/jpeg' })))]).toEqual([...jpeg])
  })

  it('a file the reader fails on says so too', async () => {
    vi.spyOn(FileReader.prototype, 'readAsDataURL').mockImplementation(function (this: FileReader) {
      queueMicrotask(() => this.onerror?.(new ProgressEvent('error') as ProgressEvent<FileReader>))
    })
    comp = mount(CropDialog, { target: document.body, props: { file: file(), kind: 'banner', onApply: vi.fn(), onCancel: vi.fn() } })
    await vi.waitFor(() => expect(document.querySelector('[role="alert"]')).not.toBeNull())
    expect(document.querySelector('[role="alert"]')!.textContent).toContain("isn't a picture Pulse can read")
    expect([...document.querySelectorAll('button')].map((b) => b.textContent)).toEqual(['Cancel'])
  })

  it('Escape cancels', async () => {
    const { onCancel } = await open('banner')
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
    expect(onCancel).toHaveBeenCalledTimes(1)
  })

  it('an oversized file is refused before reading', () => {
    const read = vi.spyOn(FileReader.prototype, 'readAsDataURL')
    const big = new File([new Uint8Array(8 * 1024 * 1024 + 1)], 'big.png', { type: 'image/png' })
    comp = mount(CropDialog, { target: document.body, props: { file: big, kind: 'avatar', onApply: vi.fn(), onCancel: vi.fn() } })
    flushSync()
    expect(read).not.toHaveBeenCalled()
    expect(document.body.textContent).toContain('Profile pictures and banners can be up to 8 MB.')
    expect(document.querySelector('img')).toBeNull()
  })
})
