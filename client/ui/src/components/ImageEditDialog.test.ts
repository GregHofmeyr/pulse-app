// @vitest-environment jsdom
// Editing a queued image before it's sent (#290): the Edit button on an image's chip, the editor's crop, turns and
// keys, and the edit taking the original's place in the queue with its spoiler mark.
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const calls: { cmd: string; args: unknown; opts?: unknown }[] = []
let answers: Record<string, (args: unknown) => unknown> = {}
vi.mock('@tauri-apps/api/core', () => ({
  invoke: async (cmd: string, args: unknown, opts?: unknown) => {
    calls.push({ cmd, args, opts })
    return answers[cmd]?.(args)
  },
}))

import Composer from './Composer.svelte'
import ComposerFiles from './ComposerFiles.svelte'
import ImageEditDialog from './ImageEditDialog.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import type { StagedFile } from '../lib/tauri'

const file = (id: string, name: string, extra: Partial<StagedFile> = {}): StagedFile => ({
  id, name, size: 1000, content_type: 'image/webp', kind: 'image', width: 400, height: 300, preview: 'data:image/png;base64,AA', ...extra,
})

/** What the fake canvas was asked to do. */
let drawn: { transforms: number[][]; size: { w: number; h: number } | null }
beforeEach(() => {
  calls.length = 0
  answers = { staged_image_bytes: () => new Uint8Array([1, 2, 3]).buffer }
  drawn = { transforms: [], size: null }
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockImplementation(function (this: HTMLCanvasElement) {
    const el = this
    return {
      canvas: el,
      setTransform: (...m: number[]) => drawn.transforms.push(m),
      clearRect: () => {},
      drawImage: () => (drawn.size = { w: el.width, h: el.height }),
      beginPath: () => {},
      moveTo: () => {},
      lineTo: () => {},
      stroke: () => {},
    } as unknown as CanvasRenderingContext2D
  } as never)
  vi.spyOn(HTMLCanvasElement.prototype, 'toBlob').mockImplementation((cb: BlobCallback) =>
    cb({ arrayBuffer: async () => new Uint8Array([9, 9]).buffer } as unknown as Blob),
  )
})

let cmp: ReturnType<typeof mount> | undefined
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
  vi.restoreAllMocks()
})

const settle = async () => {
  for (let i = 0; i < 5; i++) await tick()
  flushSync()
}

/** The picture arrives (the core's bytes, read as a data URL) and decodes at `w` × `h`. */
async function decoded(w = 400, h = 300) {
  await vi.waitFor(() => expect(document.querySelector<HTMLImageElement>('img.source')?.getAttribute('src')).toMatch(/^data:/))
  const img = document.querySelector<HTMLImageElement>('img.source')!
  Object.defineProperty(img, 'naturalWidth', { value: w })
  Object.defineProperty(img, 'naturalHeight', { value: h })
  img.dispatchEvent(new Event('load'))
  flushSync()
}
const dims = () => document.querySelector('.dims')?.textContent
const button = (label: string) => document.querySelector<HTMLButtonElement>(`[aria-label="${label}"]`)!

describe('the Edit button', () => {
  it('is on still images only, and opens the editor for that file', () => {
    const onEdit = vi.fn()
    cmp = mount(ComposerFiles, {
      target: document.body,
      props: {
        files: [file('a', 'shot.webp'), file('b', 'party.webp', { animated: true }), file('c', 'notes.pdf', { kind: 'file', preview: null })],
        onEdit, onRemove: () => {},
      },
    })
    flushSync()
    expect(document.querySelector('[aria-label="Edit party.webp"]')).toBeNull()
    expect(document.querySelector('[aria-label="Edit notes.pdf"]')).toBeNull()
    button('Edit shot.webp').click()
    expect(onEdit).toHaveBeenCalledWith('a')
  })

  it('is not offered on a message on its way', () => {
    cmp = mount(ComposerFiles, { target: document.body, props: { files: [file('a', 'shot.webp')], progress: 0.5 } })
    flushSync()
    expect(document.querySelector('[aria-label^="Edit"]')).toBeNull()
  })
})

describe('the image editor', () => {
  function open(onSave = vi.fn(), onCancel = vi.fn()) {
    cmp = mount(ImageEditDialog, { target: document.body, props: { file: file('a', 'shot.webp'), onSave, onCancel } })
    flushSync()
    return { onSave, onCancel }
  }

  it('reads the queued picture from the core and starts on the whole of it', async () => {
    open()
    await decoded()
    expect(calls.find((c) => c.cmd === 'staged_image_bytes')!.args).toEqual({ id: 'a' })
    expect(dims()).toBe('400 × 300')
    expect(document.querySelector('[role=dialog]')!.getAttribute('aria-label')).toBe('Edit shot.webp')
    // and says the copy it saves leaves the metadata out
    expect(document.querySelector('.note')!.textContent).toContain('EXIF')
  })

  it('Save with nothing changed just closes', async () => {
    const { onSave, onCancel } = open()
    await decoded()
    document.querySelector<HTMLButtonElement>('.btn-primary')!.click()
    await settle()
    expect(onCancel).toHaveBeenCalled()
    expect(onSave).not.toHaveBeenCalled()
  })

  it('crops to a ratio, turns the picture, and saves a PNG of the crop', async () => {
    const { onSave } = open()
    await decoded()
    document.querySelector<HTMLButtonElement>('.ratio:nth-child(2)')!.click()
    flushSync()
    expect(dims()).toBe('300 × 300')
    button('Rotate right').click()
    flushSync()
    // turned: 300 × 400, still a square crop
    expect(dims()).toBe('300 × 300')
    document.querySelector<HTMLButtonElement>('.btn-primary')!.click()
    await settle()
    await vi.waitFor(() => expect(onSave).toHaveBeenCalled())
    const [bytes, name] = onSave.mock.calls[0]
    expect(name).toBe('shot.png')
    expect([...bytes]).toEqual([9, 9])
    expect(drawn.size).toEqual({ w: 300, h: 300 })
    // drawn a quarter turn clockwise (the turned picture is 300 wide), moved up to the crop (50 px down it)
    expect(drawn.transforms.at(-1)).toEqual([0, 1, -1, 0, 300, -50])
  })

  it('moves and resizes the crop with the keyboard', async () => {
    open()
    await decoded()
    const stage = document.querySelector<HTMLElement>('[role=application]')!
    stage.dispatchEvent(new KeyboardEvent('keydown', { key: '-', bubbles: true }))
    flushSync()
    expect(dims()).toBe('364 × 273')
    const box = () => document.querySelector<HTMLElement>('.box')!.style.left
    const before = box()
    stage.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', shiftKey: true, bubbles: true }))
    flushSync()
    expect(box()).not.toBe(before)
    stage.dispatchEvent(new KeyboardEvent('keydown', { key: '+', bubbles: true }))
    stage.dispatchEvent(new KeyboardEvent('keydown', { key: '+', bubbles: true }))
    flushSync()
    expect(dims()).toBe('400 × 300')
  })

  it('switches to drawing, with theme pens and undo', async () => {
    open()
    await decoded()
    document.querySelector<HTMLButtonElement>('[role=radiogroup][aria-label=Tool] [role=radio]:nth-child(2)')!.click()
    flushSync()
    const pens = [...document.querySelectorAll<HTMLButtonElement>('.pen')]
    expect(pens.length).toBeGreaterThan(2)
    for (const p of pens) expect(p.style.background).toMatch(/^var\(--/)
    expect(button('Undo the last stroke').disabled).toBe(true)
  })

  it("says so when the picture can't be read", async () => {
    answers.staged_image_bytes = () => Promise.reject(new Error('gone'))
    open()
    await settle()
    expect(document.querySelector('[role=alert]')!.textContent).toContain("can't be edited")
  })
})

describe('in the composer', () => {
  it('puts the edit in the original\'s place, keeps its spoiler mark, and lets the original go', async () => {
    answers.pick_attachment_files = () => ['/home/alex/shot.png', '/home/alex/b.pdf']
    answers.stage_attachment_paths = () => [
      { status: 'staged', file: file('s1', 'shot.webp') },
      { status: 'staged', file: file('s2', 'b.pdf', { kind: 'file', preview: null }) },
    ]
    answers.stage_attachment_bytes = () => file('s3', 'shot.webp', { size: 900 })
    app.state = { ...emptyState(), me: { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } } }
    cmp = mount(Composer, {
      target: document.body,
      props: { placeholder: 'Message', draftKey: `K${Math.random()}`, replyingTo: null, onCancelReply: () => {}, onSend: () => {}, onTyping: () => {}, files: true },
    })
    flushSync()
    button('Attach files').click()
    await settle()
    button('Mark shot.webp as a spoiler').click()
    flushSync()
    button('Edit shot.webp').click()
    flushSync()
    await decoded()
    button('Rotate left').click()
    flushSync()
    document.querySelector<HTMLButtonElement>('.modal .btn-primary')!.click()
    await vi.waitFor(() => expect(document.querySelector('[role=dialog]')).toBeNull())
    await settle()
    const staged = calls.find((c) => c.cmd === 'stage_attachment_bytes')!
    expect((staged.opts as { headers: Record<string, string> }).headers['x-name']).toBe('shot.png')
    expect(calls.find((c) => c.cmd === 'unstage_attachment')!.args).toEqual({ id: 's1' })
    // same spot, still a spoiler
    expect([...document.querySelectorAll('.chip .name')].map((n) => n.textContent)).toEqual(['shot.webp', 'b.pdf'])
    expect(document.querySelector('.chip')!.classList.contains('spoiler')).toBe(true)
    expect(document.querySelector('[aria-label="Unmark shot.webp as a spoiler"]')).not.toBeNull()
  })
})
