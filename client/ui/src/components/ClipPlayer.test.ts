// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Message } from '../lib/protocol/Message'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import ClipPlayer from './ClipPlayer.svelte'
import { playback } from '../lib/clipPlayback.svelte'

type Fake = HTMLVideoElement & { _t: number; _d: number; _ns: number; _paused: boolean }

// jsdom has no media pipeline: back the bits the player reads with plain fields.
beforeEach(() => {
  const proto = HTMLMediaElement.prototype as unknown as Record<string, unknown>
  const prop = (name: string, field: string, init: unknown) =>
    Object.defineProperty(proto, name, { configurable: true, get() { return (this as Record<string, unknown>)[field] ?? init }, set(v) { (this as Record<string, unknown>)[field] = v } })
  prop('currentTime', '_t', 0)
  prop('duration', '_d', NaN)
  prop('networkState', '_ns', 1)
  Object.defineProperty(proto, 'paused', { configurable: true, get() { return (this as Fake)._paused ?? true } })
  proto.play = vi.fn(async function (this: Fake) { this._paused = false; this.dispatchEvent(new Event('play')) })
  proto.pause = vi.fn(function (this: Fake) { this._paused = true; this.dispatchEvent(new Event('pause')) })
  invoke.mockReset()
  let n = 0
  invoke.mockImplementation(async (cmd) => (cmd === 'clip_url' ? { url: `http://127.0.0.1:7895/clips/k.mp4?sig=${++n}`, expires_at: '2026-10-28T11:00:00Z' } : undefined))
  localStorage.clear()
  playback.nowPlaying = null; playback.volume = 1; playback.muted = false
})

const clipMsg = (id: string): Message => ({
  id, channel_id: 'C1', author_id: 'U1', kind: 'normal', content: '', reply_to_id: null,
  created_at: '2026-10-01T10:00:00.000Z', edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null,
  quote: null, qoty_card: null,
  clip: { thumb_hash: 'a'.repeat(64), duration_ms: 41000, width: 1280, height: 720, size: 1000, game_id: null, reply_count: 0, last_reply_id: null, replies_seen: null },
}) as Message

let cmps: ReturnType<typeof mount>[] = []
function show(id = 'M1', props: Record<string, unknown> = {}) {
  const target = document.createElement('div')
  document.body.append(target)
  cmps.push(mount(ClipPlayer, { target, props: { message: clipMsg(id), ...props } }))
  flushSync()
  return target
}
afterEach(() => { cmps.forEach((c) => unmount(c)); cmps = []; document.body.innerHTML = '' })
const settle = () => new Promise((r) => setTimeout(r, 0))
const urlCalls = () => invoke.mock.calls.filter((c) => c[0] === 'clip_url')
const btn = (root: Element, label: string) => root.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`)!
const video = (root: Element) => root.querySelector('video') as Fake | null
const press = async (root: Element, label: string) => { btn(root, label).click(); flushSync(); await settle(); flushSync() }
// Pressing Play fetches the link; the video starts once its metadata is in (jsdom never loads one).
const start = async (root: Element) => {
  await press(root, 'Play')
  video(root)?.dispatchEvent(new Event('loadedmetadata'))
  flushSync()
}

describe('ClipPlayer', () => {
  it('draws no video until played, only a lazy thumbnail', () => {
    const r = show()
    expect(video(r)).toBeNull()
    expect(r.querySelector('img')?.getAttribute('loading')).toBe('lazy')
    expect(urlCalls()).toHaveLength(0)
  })

  it('asks for a fresh link on each play', async () => {
    const r = show()
    await start(r)
    expect(urlCalls()).toHaveLength(1)
    const v = video(r)!
    expect(v.getAttribute('src')).toContain('sig=1')
    expect(v.getAttribute('preload')).toBe('metadata')
    expect(v.hasAttribute('playsinline')).toBe(true)
    await press(r, 'Pause')
    await start(r)
    expect(urlCalls()).toHaveLength(2)
    expect(video(r)!.getAttribute('src')).toContain('sig=2')
  })

  it('does not cache the link in the message', async () => {
    const m = clipMsg('M1')
    const before = JSON.stringify(m)
    const target = document.createElement('div'); document.body.append(target)
    cmps.push(mount(ClipPlayer, { target, props: { message: m } }))
    flushSync()
    await start(target)
    expect(JSON.stringify(m)).toBe(before)
  })

  it('only one clip plays at a time', async () => {
    const a = show('M1')
    const b = show('M2')
    await start(a)
    expect(playback.nowPlaying).toBe('M1')
    await start(b)
    expect(playback.nowPlaying).toBe('M2')
    expect(video(a)).toBeNull() // the first went back to its thumbnail
    expect(video(b)).not.toBeNull()
    expect(document.querySelectorAll('video')).toHaveLength(1)
  })

  it('volume is remembered across players', async () => {
    const a = show('M1')
    await start(a)
    const slider = a.querySelector<HTMLInputElement>('input[aria-label="Volume"]')!
    slider.value = '0.3'
    slider.dispatchEvent(new Event('input', { bubbles: true }))
    flushSync()
    expect(video(a)!.volume).toBeCloseTo(0.3)
    expect(JSON.parse(localStorage.getItem('pulse.clipVolume')!).volume).toBeCloseTo(0.3)
    const b = show('M2')
    await start(b)
    expect(video(b)!.volume).toBeCloseTo(0.3)
    expect(b.querySelector<HTMLInputElement>('input[aria-label="Volume"]')!.value).toBe('0.3')
  })

  it('shows the time and seeks from the bar', async () => {
    const r = show()
    expect(r.textContent).toContain('0:00 / 0:41')
    await start(r)
    const v = video(r)!
    v._d = 41; v._t = 12
    v.dispatchEvent(new Event('timeupdate'))
    flushSync()
    expect(r.textContent).toContain('0:12 / 0:41')
    const bar = r.querySelector<HTMLInputElement>('input[aria-label="Seek"]')!
    bar.value = '30'
    bar.dispatchEvent(new Event('input', { bubbles: true }))
    flushSync()
    expect(v.currentTime).toBe(30)
  })

  it('space and arrows work when focused', async () => {
    const r = show()
    const box = r.querySelector<HTMLElement>('[role=group]')!
    box.focus()
    const key = (k: string) => { box.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true })); flushSync() }
    key(' ')
    await settle(); flushSync()
    const v = video(r)!
    v.dispatchEvent(new Event('loadedmetadata'))
    expect(v._paused).toBe(false)
    v._d = 41; v._t = 10
    key('ArrowRight')
    expect(v.currentTime).toBe(15)
    key('ArrowLeft'); key('ArrowLeft'); key('ArrowLeft')
    expect(v.currentTime).toBe(0) // clamped
    key(' ')
    expect(v._paused).toBe(true)
  })

  it('fullscreen asks the window too', async () => {
    const r = show()
    const box = r.querySelector<HTMLElement>('[role=group]')!
    let fs: Element | null = null
    Object.defineProperty(document, 'fullscreenElement', { configurable: true, get: () => fs })
    box.requestFullscreen = vi.fn(async () => { fs = box; document.dispatchEvent(new Event('fullscreenchange')) })
    document.exitFullscreen = vi.fn(async () => { fs = null; document.dispatchEvent(new Event('fullscreenchange')) })
    await press(r, 'Full screen')
    expect(box.requestFullscreen).toHaveBeenCalled()
    expect(invoke).toHaveBeenCalledWith('set_fullscreen', { on: true })
    await press(r, 'Exit full screen')
    expect(document.exitFullscreen).toHaveBeenCalled()
    expect(invoke).toHaveBeenLastCalledWith('set_fullscreen', { on: false })
    // another player's fullscreen change is not ours
    invoke.mockClear()
    fs = document.body
    document.dispatchEvent(new Event('fullscreenchange'))
    expect(invoke).not.toHaveBeenCalled()
  })

  it('an expired link is renewed once and resumes', async () => {
    let now = 1_000_000
    const spy = vi.spyOn(Date, 'now').mockImplementation(() => now)
    const r = show()
    await start(r)
    const v = video(r)!
    v._t = 17
    v.dispatchEvent(new Event('timeupdate'))
    now += 51 * 60_000
    v._ns = 3
    v.dispatchEvent(new Event('error'))
    await settle(); flushSync()
    expect(urlCalls()).toHaveLength(2)
    const v2 = video(r)!
    expect(v2.getAttribute('src')).toContain('sig=2')
    v2.dispatchEvent(new Event('loadedmetadata'))
    expect(v2.currentTime).toBe(17)
    expect(v2._paused).toBe(false)
    // failing again right away does not loop
    v2._ns = 3
    now += 51 * 60_000
    v2.dispatchEvent(new Event('error'))
    await settle(); flushSync()
    expect(urlCalls()).toHaveLength(2)
    spy.mockRestore()
  })

  it('a young link that errors is not renewed', async () => {
    const r = show()
    await start(r)
    const v = video(r)!
    v._ns = 3
    v.dispatchEvent(new Event('error'))
    await settle(); flushSync()
    expect(urlCalls()).toHaveLength(1)
    expect(r.textContent).toContain("Couldn't play this clip")
  })

  it('autoplay starts on mount', async () => {
    const r = show('M1', { autoplay: true })
    await settle(); flushSync()
    expect(video(r)).not.toBeNull()
    expect(urlCalls()).toHaveLength(1)
  })
})
