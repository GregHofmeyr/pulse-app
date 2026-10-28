// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import CropDialog from './CropDialog.svelte'

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
