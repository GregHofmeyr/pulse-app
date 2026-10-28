// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Game } from '../lib/protocol/Game'
import type { ClipSource } from '../lib/tauri'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args), convertFileSrc: (s: string) => `asset://${s}` }))
const listeners: Record<string, (e: { payload: unknown }) => void> = {}
vi.mock('@tauri-apps/api/event', () => ({
  listen: (name: string, cb: (e: { payload: unknown }) => void) => { listeners[name] = cb; return Promise.resolve(() => { delete listeners[name] }) },
}))

import ClipEditor from './ClipEditor.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'

type Fake = HTMLVideoElement & { _t: number; _d: number; _paused: boolean }

// jsdom has no media pipeline: back the bits the editor reads with plain fields.
const PATCHED = ['currentTime', 'duration', 'paused', 'play', 'pause']
const saved = new Map<string, PropertyDescriptor | undefined>()
beforeEach(() => {
  const proto = HTMLMediaElement.prototype as unknown as Record<string, unknown>
  for (const k of PATCHED) saved.set(k, Object.getOwnPropertyDescriptor(HTMLMediaElement.prototype, k))
  const prop = (name: string, field: string, init: unknown) =>
    Object.defineProperty(proto, name, { configurable: true, get() { return (this as Record<string, unknown>)[field] ?? init }, set(v) { (this as Record<string, unknown>)[field] = v } })
  prop('currentTime', '_t', 0)
  prop('duration', '_d', NaN)
  Object.defineProperty(proto, 'paused', { configurable: true, get() { return (this as Fake)._paused ?? true } })
  proto.play = vi.fn(async function (this: Fake) { this._paused = false; this.dispatchEvent(new Event('play')) })
  proto.pause = vi.fn(function (this: Fake) { this._paused = true; this.dispatchEvent(new Event('pause')) })
  invoke.mockReset()
  invoke.mockImplementation(async (cmd) => {
    if (cmd === 'clip_filmstrip') return { path: '/tmp/film.jpg', tiles: 24 }
    if (cmd === 'prepare_preview') return '/tmp/preview.mp4'
    if (cmd === 'send_clip') return { id: 'J1' }
    return undefined
  })
  const game = (id: string, name: string): Game => ({ id, server_id: 'S1', name, created_by: null, created_at: '2026-10-01T10:00:00.000Z' })
  app.state = { ...emptyState(), games: { S1: [game('G1', 'Overwatch 2')] } }
})
afterEach(() => {
  for (const [k, d] of saved) d ? Object.defineProperty(HTMLMediaElement.prototype, k, d) : delete (HTMLMediaElement.prototype as unknown as Record<string, unknown>)[k]
  saved.clear()
  cmps.forEach((c) => unmount(c)); cmps = []; document.body.innerHTML = ''
  for (const k of Object.keys(listeners)) delete listeners[k]
})

const source = (over: Partial<ClipSource> = {}): ClipSource => ({
  id: 'S-1', path: '/home/alex/Videos/Overwatch 2/replay.mp4', file_name: 'replay.mp4', size: 1_300_000_000, duration_ms: 41_000,
  width: 2560, height: 1440, fps: 60, video_codec: 'h264', game_guess: 'overwatch 2', preview: 'direct', ...over,
})

let cmps: ReturnType<typeof mount>[] = []
const settle = () => new Promise((r) => setTimeout(r, 0))
async function show(src: ClipSource = source(), onClose = vi.fn()) {
  const target = document.createElement('div')
  document.body.append(target)
  cmps.push(mount(ClipEditor, { target, props: { source: src, channelId: 'C1', serverId: 'S1', onClose } }))
  flushSync()
  await settle(); flushSync()
  return { target, onClose }
}
const video = () => document.querySelector('video') as Fake
const at = (seconds: number) => {
  const v = video()
  v._t = seconds
  v.dispatchEvent(new Event('timeupdate'))
  flushSync()
}
const key = (k: string, init: KeyboardEventInit = {}) => { document.body.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, ...init })); flushSync() }
const btn = (label: string) => document.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`)!
const pieces = () => [...document.querySelectorAll<HTMLElement>('[data-piece]')]
const readout = () => document.querySelector('[data-readout]') as HTMLElement
const text = () => document.body.textContent ?? ''
const sendBtn = () => [...document.querySelectorAll('button')].find((b) => b.textContent?.includes('Trim & send')) as HTMLButtonElement

describe('ClipEditor', () => {
  it('opens with the file details and the guessed game', async () => {
    await show()
    expect(text()).toContain('replay.mp4')
    expect(text()).toContain('0:41')
    expect(text()).toContain('2560×1440')
    expect(text()).toContain('60 fps')
    expect(text()).toContain('1.3 GB')
    expect(video().getAttribute('src')).toBe('asset:///home/alex/Videos/Overwatch 2/replay.mp4')
    expect((document.querySelector('input[role=combobox]') as HTMLInputElement).value).toBe('Overwatch 2')
    expect(invoke).toHaveBeenCalledWith('clip_filmstrip', { source: 'S-1' })
    expect(invoke).not.toHaveBeenCalledWith('prepare_preview', expect.anything())
  })

  it('a hevc file prepares a preview first', async () => {
    let finish!: (p: string) => void
    invoke.mockImplementation(async (cmd) => {
      if (cmd === 'prepare_preview') return new Promise((r) => { finish = r })
      if (cmd === 'clip_filmstrip') return { path: '/tmp/film.jpg', tiles: 24 }
      return undefined
    })
    await show(source({ video_codec: 'hevc', preview: 'encode' }))
    expect(text()).toContain('Preparing preview…')
    expect(video()).toBeNull()
    listeners['clip://preview']?.({ payload: { source: 'S-1', percent: 40 } })
    flushSync()
    expect(text()).toContain('40%')
    finish('/tmp/preview.mp4')
    await settle(); flushSync()
    expect(invoke).toHaveBeenCalledWith('prepare_preview', { source: 'S-1' })
    expect(video().getAttribute('src')).toBe('asset:///tmp/preview.mp4')
  })

  it('a direct file the player rejects falls back to a prepared preview', async () => {
    await show()
    video().dispatchEvent(new Event('error'))
    await settle(); flushSync()
    expect(invoke).toHaveBeenCalledWith('prepare_preview', { source: 'S-1' })
    expect(video().getAttribute('src')).toBe('asset:///tmp/preview.mp4')
  })

  it('shows the error when the video cannot be read at all', async () => {
    invoke.mockImplementation(async (cmd) => { if (cmd === 'prepare_preview') throw "Pulse couldn't read that video."; return undefined })
    await show(source({ preview: 'remux' }))
    expect(document.querySelector('[role=alert]')?.textContent).toContain("Pulse couldn't read that video.")
  })

  it('C cuts at the playhead and Delete drops the selected piece', async () => {
    await show()
    at(10)
    key('c')
    expect(pieces()).toHaveLength(2)
    expect(readout().textContent).toContain('1 piece') // adjacent kept pieces count as one
    key('Delete')
    expect(pieces()[1].dataset.dropped).toBe('true')
    expect(readout().textContent).toContain('0:10 / 0:10 kept')
    expect(readout().textContent).toContain('1 piece')
  })

  it('clicking a dropped piece restores it', async () => {
    await show()
    at(10)
    key('c'); key('Delete')
    pieces()[1].click(); flushSync()
    expect(pieces()[1].dataset.dropped).toBeUndefined()
    expect(readout().textContent).toContain('/ 0:41 kept')
  })

  it('ctrl+z undoes', async () => {
    await show()
    at(10)
    key('c'); key('Delete')
    key('z', { ctrlKey: true })
    expect(pieces()[1].dataset.dropped).toBeUndefined()
    key('z', { ctrlKey: true })
    expect(pieces()).toHaveLength(1)
  })

  it('does not take keys while typing in the caption', async () => {
    await show()
    at(10)
    const caption = document.querySelector<HTMLTextAreaElement>('textarea')!
    caption.dispatchEvent(new KeyboardEvent('keydown', { key: 'c', bubbles: true }))
    flushSync()
    expect(pieces()).toHaveLength(1)
  })

  it('arrow keys and comma/period move the playhead', async () => {
    await show()
    at(10)
    key('ArrowRight'); expect(video()._t).toBe(15)
    key('ArrowLeft'); key('ArrowLeft'); expect(video()._t).toBe(5)
    key('.'); expect(video()._t).toBeCloseTo(5 + 1 / 60, 2)
    key(','); expect(video()._t).toBeCloseTo(5, 2)
  })

  it('play skips a dropped piece', async () => {
    await show()
    at(10); key('c')
    at(20); key('c'); // pieces: 0-10, 10-20, 20-41; the last cut selected the third
    pieces()[1].click(); flushSync(); key('Delete') // drop 10-20
    at(0)
    key(' ')
    expect(video()._paused).toBe(false)
    at(10.2)
    expect(video()._t).toBe(20)
  })

  it('over 1:30 the readout is red and send is disabled with the copy', async () => {
    await show(source({ duration_ms: 120_000 }))
    expect(readout().classList.contains('over')).toBe(true)
    expect(text()).toContain('Clips can be up to 1:30. Drop a bit more.')
    expect(sendBtn().disabled).toBe(true)
  })

  it('nothing kept disables send with the copy', async () => {
    await show()
    pieces()[0].click(); flushSync()
    key('Delete')
    expect(text()).toContain('Nothing kept. Bring a piece back.')
    expect(sendBtn().disabled).toBe(true)
  })

  it('presets show size estimates for the kept length', async () => {
    await show(source({ duration_ms: 90_000 }))
    const labels = [...document.querySelectorAll('[data-preset]')].map((l) => l.textContent?.replace(/\s+/g, ' ').trim())
    expect(labels).toEqual(['Light · 720p60 · ~48 MB', 'Sharp · 1080p60 · ~94 MB'])
  })

  it('trim and send sends the kept spans, preset, caption and game', async () => {
    const { onClose } = await show()
    at(10); key('c'); key('Delete') // keeps 0-10
    document.querySelector<HTMLElement>('[data-preset="sharp"] input')!.click()
    const caption = document.querySelector<HTMLTextAreaElement>('textarea')!
    caption.value = 'ace'
    caption.dispatchEvent(new Event('input', { bubbles: true }))
    flushSync()
    sendBtn().click()
    await settle(); flushSync()
    expect(invoke).toHaveBeenCalledWith('send_clip', {
      request: { source: 'S-1', channel_id: 'C1', spans: [{ start_ms: 0, end_ms: 10_000 }], preset: 'sharp', caption: 'ace', game_id: 'G1' },
    })
    // the source is released after a successful send too
    expect(invoke).toHaveBeenCalledWith('close_clip_source', { source: 'S-1' })
    expect(onClose).toHaveBeenCalledTimes(1)
  })

  it('a refused send stays open and shows the reason', async () => {
    invoke.mockImplementation(async (cmd) => { if (cmd === 'send_clip') throw 'One clip at a time.'; return undefined })
    const { onClose } = await show()
    sendBtn().click()
    await settle(); flushSync()
    expect(document.querySelector('[role=alert]')?.textContent).toContain('One clip at a time.')
    expect(onClose).not.toHaveBeenCalled()
    expect(invoke).not.toHaveBeenCalledWith('close_clip_source', expect.anything())
  })

  it('closing without sending releases the source', async () => {
    const { onClose } = await show()
    btn('Close').click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('close_clip_source', { source: 'S-1' })
    expect(invoke).not.toHaveBeenCalledWith('send_clip', expect.anything())
    expect(onClose).toHaveBeenCalledTimes(1)
  })

  it('dragging an edge is one undo step', async () => {
    await show()
    at(10); key('c')
    const edge = document.querySelector<HTMLElement>('[data-edge="1"]')!
    // jsdom has no layout: the timeline reports the time from the pointer, so give it a 1000 px wide track
    const track = document.querySelector<HTMLElement>('[data-track]')!
    track.getBoundingClientRect = () => ({ left: 0, width: 1000, top: 0, height: 60, right: 1000, bottom: 60, x: 0, y: 0, toJSON() {} }) as DOMRect
    edge.setPointerCapture = () => {}
    edge.releasePointerCapture = () => {}
    edge.dispatchEvent(new MouseEvent('pointerdown', { clientX: 244, bubbles: true }))
    edge.dispatchEvent(new MouseEvent('pointermove', { clientX: 300, bubbles: true }))
    edge.dispatchEvent(new MouseEvent('pointermove', { clientX: 400, bubbles: true }))
    edge.dispatchEvent(new MouseEvent('pointerup', { clientX: 400, bubbles: true }))
    flushSync()
    expect(readout().textContent).toContain('0:41')
    expect(pieces()[0].dataset.end).toBe('16400')
    key('z', { ctrlKey: true })
    expect(pieces()[0].dataset.end).toBe('10000')
  })

  it('Space on a focused button presses it once and does not also toggle playback', async () => {
    await show()
    const cutBtn = [...document.querySelectorAll('button')].find((b) => b.textContent?.includes('Cut here'))!
    cutBtn.focus()
    cutBtn.dispatchEvent(new KeyboardEvent('keydown', { key: ' ', bubbles: true }))
    flushSync()
    expect(video()._paused).not.toBe(false)
    expect(invoke).not.toHaveBeenCalledWith('send_clip', expect.anything())
  })

  it('typing in the game box does not trigger shortcuts', async () => {
    await show()
    at(10)
    const box = document.querySelector<HTMLInputElement>('input[role=combobox]')!
    for (const k of ['c', 'Delete', ' ', 'ArrowRight']) box.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true }))
    flushSync()
    expect(pieces()).toHaveLength(1)
    expect(video()._t).toBe(10)
  })

  it('a held key repeats nothing', async () => {
    await show()
    at(10)
    key('c')
    key('c', { repeat: true }) // would be a no-op anyway at the same spot; use a drop to tell
    key('Delete')
    key('Delete', { repeat: true })
    key('z', { ctrlKey: true })
    key('z', { ctrlKey: true, repeat: true })
    // one undo undid the drop; the repeat did not also undo the cut
    expect(pieces()).toHaveLength(2)
    expect(pieces()[1].dataset.dropped).toBeUndefined()
  })

  it('shows no size when nothing is kept', async () => {
    await show()
    pieces()[0].click(); flushSync(); key('Delete')
    const labels = [...document.querySelectorAll('[data-preset]')].map((l) => l.textContent?.replace(/\s+/g, ' ').trim())
    expect(labels).toEqual(['Light · 720p60 · —', 'Sharp · 1080p60 · —'])
  })

  it('a timeline edge is a slider that arrow keys nudge, one undo step each', async () => {
    await show()
    at(10); key('c')
    const edge = document.querySelector<HTMLElement>('[data-edge="1"]')!
    expect(edge.getAttribute('role')).toBe('slider')
    expect(edge.getAttribute('aria-label')).toBe('Start of piece 2')
    expect(edge.getAttribute('aria-valuenow')).toBe('10000')
    edge.focus()
    const press = (k: string, shiftKey = false) => { edge.dispatchEvent(new KeyboardEvent('keydown', { key: k, shiftKey, bubbles: true })); flushSync() }
    press('ArrowRight')
    expect(pieces()[0].dataset.end).toBe('10017') // one frame at 60 fps
    press('ArrowRight', true)
    expect(pieces()[0].dataset.end).toBe('11017')
    expect(video()._t).toBe(10) // the playhead did not move
    key('z', { ctrlKey: true })
    expect(pieces()[0].dataset.end).toBe('10017')
    key('z', { ctrlKey: true })
    expect(pieces()[0].dataset.end).toBe('10000')
  })

  it('runs one frame-callback chain, cancelled on pause and on unmount', async () => {
    const el = HTMLVideoElement.prototype as unknown as Record<string, unknown>
    let n = 0
    const live = new Set<number>()
    el.requestVideoFrameCallback = vi.fn(() => { live.add(++n); return n })
    el.cancelVideoFrameCallback = vi.fn((h: number) => { live.delete(h) })
    try {
      await show()
      video().dispatchEvent(new Event('play')); video().dispatchEvent(new Event('play'))
      expect(live.size).toBe(1)
      video().dispatchEvent(new Event('pause'))
      expect(live.size).toBe(0)
      video().dispatchEvent(new Event('play'))
      expect(live.size).toBe(1)
      cmps.forEach((c) => unmount(c)); cmps = []
      expect(live.size).toBe(0)
    } finally {
      delete el.requestVideoFrameCallback
      delete el.cancelVideoFrameCallback
    }
  })
})
