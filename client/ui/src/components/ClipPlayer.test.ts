// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Message } from '../lib/protocol/Message'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import ClipPlayer from './ClipPlayer.svelte'
import { setNotch } from '../lib/mixer.svelte'
import { claim, playback, release } from '../lib/clipPlayback.svelte'
import { forgetClipLinks } from '../lib/clipLinks'

type Fake = HTMLVideoElement & { _t: number; _d: number; _ns: number; _paused: boolean }

// jsdom has no media pipeline: back the bits the player reads with plain fields.
const PATCHED = ['currentTime', 'duration', 'networkState', 'paused', 'play', 'pause']
const saved = new Map<string, PropertyDescriptor | undefined>()
afterEach(() => {
  for (const [k, d] of saved) d ? Object.defineProperty(HTMLMediaElement.prototype, k, d) : delete (HTMLMediaElement.prototype as unknown as Record<string, unknown>)[k]
  saved.clear()
  delete (document as unknown as Record<string, unknown>).fullscreenElement
  delete (document as unknown as Record<string, unknown>).exitFullscreen
  vi.restoreAllMocks()
})
beforeEach(() => {
  const proto = HTMLMediaElement.prototype as unknown as Record<string, unknown>
  for (const k of PATCHED) saved.set(k, Object.getOwnPropertyDescriptor(HTMLMediaElement.prototype, k))
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
  invoke.mockImplementation(async (cmd) => (cmd === 'clip_url' ? { url: `http://127.0.0.1:7895/clips/k.mp4?X-Amz-Expires=3600&sig=${++n}`, expires_at: new Date(Date.now() + 3600_000).toISOString() } : undefined))
  forgetClipLinks()
  localStorage.clear()
  release(claim('reset', () => {})); playback.volume = 100; playback.muted = false
})

const clipMsg = (id: string): Message => ({
  id, channel_id: 'C1', author_id: 'U1', kind: 'normal', content: '', reply_to_id: null,
  created_at: '2026-10-01T10:00:00.000Z', edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null,
  quote: null, qoty_card: null, embeds: [],
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

  it('asks for the link on the first play', async () => {
    const r = show()
    await start(r)
    expect(urlCalls()).toHaveLength(1)
    const v = video(r)!
    expect(v.getAttribute('src')).toContain('sig=1')
    expect(v.getAttribute('preload')).toBe('metadata')
    expect(v.hasAttribute('playsinline')).toBe(true)
  })

  it('Play after Pause carries on in place: same video, same link, same spot', async () => {
    const r = show()
    await start(r)
    const v = video(r)!
    v._t = 12.3; v.dispatchEvent(new Event('timeupdate')); flushSync()
    await press(r, 'Pause')
    v._t = 12.4 // the shown time trails the video a little: no seek back for it
    await press(r, 'Play')
    expect(urlCalls()).toHaveLength(1)
    expect(video(r)).toBe(v)
    expect(v.currentTime).toBe(12.4)
    expect(v._paused).toBe(false)
  })

  it('a replay after the end starts over on the same link, without reloading', async () => {
    const r = show()
    await start(r)
    const v = video(r)!
    v._t = 41; v._d = 41
    v.dispatchEvent(new Event('timeupdate'))
    v.pause(); v.dispatchEvent(new Event('ended')); flushSync()
    await press(r, 'Play')
    expect(urlCalls()).toHaveLength(1)
    expect(video(r)).toBe(v)
    expect(v.currentTime).toBe(0)
    expect(v._paused).toBe(false)
  })

  it('reuses the clip’s link after another clip took over, and in another player of the same clip', async () => {
    const a = show('M1')
    const b = show('M2')
    await start(a)
    await start(b) // displaces M1: its video goes
    expect(video(a)).toBeNull()
    await start(a)
    expect(video(a)!.getAttribute('src')).toContain('sig=1')
    const c = show('M1') // e.g. the clip opened large
    await start(c)
    expect(video(c)!.getAttribute('src')).toContain('sig=1')
    expect(urlCalls().map((x) => (x[1] as { messageId: string }).messageId)).toEqual(['M1', 'M2'])
  })

  it('a link close to expiring is not reused', async () => {
    let n = 0
    invoke.mockImplementation(async (cmd) => (cmd === 'clip_url' ? { url: `http://127.0.0.1:7895/k?X-Amz-Expires=60&sig=${++n}`, expires_at: '' } : undefined))
    const a = show('M1')
    const b = show('M2')
    await start(a)
    await start(b)
    await start(a)
    expect(urlCalls()).toHaveLength(3)
    expect(video(a)!.getAttribute('src')).toContain('sig=3')
  })

  it('Play after a failure asks for a new link', async () => {
    const r = show()
    await start(r)
    video(r)!.dispatchEvent(new Event('error'))
    await settle(); flushSync()
    video(r)!.dispatchEvent(new Event('loadedmetadata'))
    video(r)!.dispatchEvent(new Event('error'))
    await settle(); flushSync()
    expect(r.textContent).toContain("Couldn't play this clip")
    await start(r)
    expect(urlCalls()).toHaveLength(3)
    expect(video(r)!.getAttribute('src')).toContain('sig=3')
    expect(r.textContent).not.toContain("Couldn't play this clip")
  })

  it('a renewed link replaces the stored one for every player', async () => {
    const a = show('M1')
    await start(a)
    video(a)!.dispatchEvent(new Event('error'))
    await settle(); flushSync()
    const b = show('M1')
    await start(b)
    expect(video(b)!.getAttribute('src')).toContain('sig=2')
    expect(urlCalls()).toHaveLength(2)
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
    expect([slider.min, slider.max, slider.step]).toEqual(['0', '100', '1'])
    slider.value = '85'
    slider.dispatchEvent(new Event('input', { bubbles: true }))
    flushSync()
    expect(video(a)!.volume).toBeCloseTo(0.5, 2) // 85 % on the mixer curve is -6 dB
    expect(JSON.parse(localStorage.getItem('pulse.mixer')!).media.clips).toBe(85)
    const b = show('M2')
    await start(b)
    expect(video(b)!.volume).toBeCloseTo(0.5, 2)
    expect(b.querySelector<HTMLInputElement>('input[aria-label="Volume"]')!.value).toBe('85')
  })

  it('the mixer’s Clips notch moves the player’s slider and volume', async () => {
    const a = show('M1')
    await start(a)
    setNotch('media', 'clips', 50)
    flushSync()
    expect(a.querySelector<HTMLInputElement>('input[aria-label="Volume"]')!.value).toBe('50')
    expect(video(a)!.volume).toBeCloseTo(0.1, 4)
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

  it('a media error renews the link once and resumes at the same time', async () => {
    const r = show()
    await start(r)
    const v = video(r)!
    v._t = 17
    v._ns = 1 // a seek after a long pause fails mid-stream: IDLE, not NO_SOURCE
    v.dispatchEvent(new Event('error'))
    await settle(); flushSync()
    expect(urlCalls()).toHaveLength(2)
    const v2 = video(r)!
    expect(v2.getAttribute('src')).toContain('sig=2')
    v2.dispatchEvent(new Event('loadedmetadata'))
    expect(v2.currentTime).toBe(17)
    expect(v2._paused).toBe(false)
  })

  it('a renewed link that fails too shows the error, with no loop', async () => {
    const r = show()
    await start(r)
    video(r)!.dispatchEvent(new Event('error'))
    await settle(); flushSync()
    const v2 = video(r)!
    v2.dispatchEvent(new Event('loadedmetadata'))
    v2.dispatchEvent(new Event('error'))
    await settle(); flushSync()
    expect(urlCalls()).toHaveLength(2)
    expect(r.textContent).toContain("Couldn't play this clip")
  })

  it('a successful play earns the renewal back', async () => {
    const r = show()
    await start(r)
    video(r)!.dispatchEvent(new Event('error'))
    await settle(); flushSync()
    const v2 = video(r)!
    v2.dispatchEvent(new Event('loadedmetadata'))
    v2.dispatchEvent(new Event('playing'))
    v2.dispatchEvent(new Event('error'))
    await settle(); flushSync()
    expect(urlCalls()).toHaveLength(3)
  })

  it('a renewal while paused does not autoplay', async () => {
    const r = show()
    await start(r)
    await press(r, 'Pause')
    const v = video(r)!
    v._t = 9
    v.dispatchEvent(new Event('error'))
    await settle(); flushSync()
    const v2 = video(r)!
    v2.dispatchEvent(new Event('loadedmetadata'))
    expect(v2.currentTime).toBe(9)
    expect(v2._paused).not.toBe(false)
  })

  it('unmounting during the link fetch claims nothing and leaves the other clip playing', async () => {
    const a = show('M1')
    await start(a)
    let resolve!: (v: unknown) => void
    invoke.mockImplementationOnce(() => new Promise((r) => { resolve = r }))
    const b = show('M2')
    btn(b, 'Play').click()
    unmount(cmps.pop()!)
    resolve({ url: 'http://127.0.0.1:7895/x', expires_at: '' })
    await settle(); flushSync()
    expect(playback.nowPlaying).toBe('M1')
    expect(video(a)).not.toBeNull()
  })

  it('Pause cancels a Play still waiting for its link', async () => {
    const r = show()
    let resolve!: (v: unknown) => void
    invoke.mockImplementationOnce(() => new Promise((res) => { resolve = res }))
    btn(r, 'Play').click()
    flushSync()
    btn(r, 'Pause').click()
    resolve({ url: 'http://127.0.0.1:7895/x', expires_at: '' })
    await settle(); flushSync()
    expect(video(r)).toBeNull()
    expect(playback.nowPlaying).toBeNull()
  })

  it('two players of one message do not both play', async () => {
    const a = show('M1')
    const b = show('M1')
    await start(a)
    await start(b)
    expect(video(a)).toBeNull()
    expect(video(b)).not.toBeNull()
    unmount(cmps.shift()!) // the displaced one going away keeps the other's claim
    expect(playback.nowPlaying).toBe('M1')
  })

  it('after being displaced, Play resumes from the time still shown', async () => {
    const a = show('M1')
    const b = show('M2')
    await start(a)
    const v = video(a)!
    v._t = 8; v._d = 41
    v.dispatchEvent(new Event('timeupdate'))
    flushSync()
    await start(b)
    expect(video(a)).toBeNull()
    expect(a.textContent).toContain('0:08 / 0:41')
    await start(a)
    expect(video(a)!.currentTime).toBe(8)
  })

  it('autoplay failure does not leak a rejection', async () => {
    ;(HTMLMediaElement.prototype as unknown as Record<string, unknown>).play = vi.fn(async () => { throw new Error('blocked') })
    const r = show()
    await start(r)
    await settle()
    expect(video(r)).not.toBeNull()
  })

  it('the window goes back to how it was, not blindly windowed', async () => {
    const r = show()
    const box = r.querySelector<HTMLElement>('[role=group]')!
    let fs: Element | null = null
    Object.defineProperty(document, 'fullscreenElement', { configurable: true, get: () => fs })
    box.requestFullscreen = vi.fn(async () => { fs = box; document.dispatchEvent(new Event('fullscreenchange')) })
    document.exitFullscreen = vi.fn(async () => {})
    invoke.mockImplementation(async (cmd, args) => (cmd === 'set_fullscreen' && (args as { on: boolean }).on ? true : undefined))
    await press(r, 'Full screen')
    // Esc: the browser leaves fullscreen with no button press
    fs = null
    document.dispatchEvent(new Event('fullscreenchange'))
    await settle()
    expect(invoke).toHaveBeenLastCalledWith('set_fullscreen', { on: true })
  })

  it('unmounting while fullscreen restores the window', async () => {
    const r = show()
    const box = r.querySelector<HTMLElement>('[role=group]')!
    let fs: Element | null = null
    Object.defineProperty(document, 'fullscreenElement', { configurable: true, get: () => fs })
    box.requestFullscreen = vi.fn(async () => { fs = box; document.dispatchEvent(new Event('fullscreenchange')) })
    await press(r, 'Full screen')
    unmount(cmps.pop()!)
    await settle()
    expect(invoke).toHaveBeenLastCalledWith('set_fullscreen', { on: false })
  })

  describe('compact (the clips wall, #208)', () => {
    const vol = (r: Element) => r.querySelector<HTMLInputElement>('input[aria-label="Volume"]')!

    it('shows the speaker; clicking it toggles the shared mute', async () => {
      const r = show('M1', { compact: true })
      await start(r)
      await press(r, 'Mute')
      expect(playback.muted).toBe(true)
      expect(video(r)!.muted).toBe(true)
      await press(r, 'Unmute')
      expect(playback.muted).toBe(false)
    })

    it('has a vertical volume slider beside the speaker, on the same volume as the full player', async () => {
      const full = show('M1')
      const r = show('M2', { compact: true })
      await start(r)
      const slider = vol(r)
      expect(slider.closest('.vol-wrap')?.querySelector('button[aria-label="Mute"]')).not.toBeNull()
      expect(slider.closest('.vol-pop')).not.toBeNull()
      expect(slider.getAttribute('aria-orientation')).toBe('vertical')
      slider.value = '85'
      slider.dispatchEvent(new Event('input', { bubbles: true }))
      flushSync()
      expect(playback.volume).toBe(85)
      expect(video(r)!.volume).toBeCloseTo(0.5, 2)
      expect(vol(full).value).toBe('85')
    })

    it('moving the slider while muted unmutes', () => {
      const r = show('M1', { compact: true })
      playback.muted = true
      flushSync()
      const slider = vol(r)
      slider.value = '40'
      slider.dispatchEvent(new Event('input', { bubbles: true }))
      flushSync()
      expect(playback.muted).toBe(false)
      expect(playback.volume).toBe(40)
    })

    it('arrow keys on the speaker adjust the volume, without seeking', async () => {
      const r = show('M1', { compact: true })
      await start(r)
      const v = video(r)!
      v._d = 41; v._t = 10
      playback.volume = 50
      flushSync()
      const speaker = btn(r, 'Mute')
      const key = (k: string) => { speaker.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true })); flushSync() }
      key('ArrowUp')
      expect(playback.volume).toBe(55)
      key('ArrowDown'); key('ArrowDown')
      expect(playback.volume).toBe(45)
      key('ArrowRight')
      expect(playback.volume).toBe(50)
      key('ArrowLeft')
      expect(playback.volume).toBe(45)
      expect(v.currentTime).toBe(10)
      playback.volume = 98; flushSync()
      key('ArrowUp')
      expect(playback.volume).toBe(100) // clamped
    })
  })

  it('autoplay starts on mount', async () => {
    const r = show('M1', { autoplay: true })
    await settle(); flushSync()
    expect(video(r)).not.toBeNull()
    expect(urlCalls()).toHaveLength(1)
  })
})
