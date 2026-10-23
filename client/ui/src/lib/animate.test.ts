// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fakeMatchMedia, FakeIntersectionObserver } from './testMotion'

const onFocusChanged = vi.fn(async (_h: (e: { payload: boolean }) => void) => () => {})
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({ onFocusChanged }) }))

type Mod = typeof import('./animate.svelte')
// The watchers install once per module: each test gets a fresh one.
// (Svelte's runtime is fresh too, so flush through that one.)
let flushSync: () => void
async function fresh(): Promise<Mod> {
  vi.resetModules()
  const m = await import('./animate.svelte')
  flushSync = (await import('svelte')).flushSync
  return m
}

beforeEach(() => {
  localStorage.clear()
  onFocusChanged.mockClear()
  vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
  FakeIntersectionObserver.all = []
})
afterEach(() => {
  vi.unstubAllGlobals()
  vi.restoreAllMocks()
  delete (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__
})

describe('shouldAnimate', () => {
  it('needs focus, on-screen and no reduced motion; then the mode decides', async () => {
    const { shouldAnimate } = await fresh()
    const modes = ['always', 'hover', 'never'] as const
    for (const mode of modes)
      for (const focused of [true, false])
        for (const visible of [true, false])
          for (const hovered of [true, false])
            for (const reducedMotion of [true, false]) {
              const want = focused && visible && !reducedMotion && (mode === 'always' || (mode === 'hover' && hovered))
              expect(shouldAnimate({ mode, focused, visible, hovered, reducedMotion }), JSON.stringify({ mode, focused, visible, hovered, reducedMotion })).toBe(want)
            }
  })
})

describe('Animate emoji setting', () => {
  it('defaults to always and persists a choice', async () => {
    let m = await fresh()
    expect(m.motion.mode).toBe('always')
    m.setAnimateMode('hover')
    expect(m.motion.mode).toBe('hover')
    m = await fresh()
    expect(m.motion.mode).toBe('hover')
    m.setAnimateMode('never')
    expect((await fresh()).motion.mode).toBe('never')
  })

  it('an unknown stored value falls back to always', async () => {
    localStorage.setItem('pulse.animateEmoji', 'sometimes')
    expect((await fresh()).motion.mode).toBe('always')
  })
})

describe('window focus and reduced motion', () => {
  it('starts from document.hasFocus() and follows window focus/blur', async () => {
    vi.spyOn(document, 'hasFocus').mockReturnValue(false)
    const m = await fresh()
    m.watchMotion()
    expect(m.motion.focused).toBe(false)
    window.dispatchEvent(new Event('focus'))
    expect(m.motion.focused).toBe(true)
    window.dispatchEvent(new Event('blur'))
    expect(m.motion.focused).toBe(false)
  })

  it('also follows the Tauri window focus event when running in Tauri', async () => {
    ;(window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = {}
    vi.spyOn(document, 'hasFocus').mockReturnValue(true)
    const m = await fresh()
    m.watchMotion()
    m.watchMotion() // once only
    expect(onFocusChanged).toHaveBeenCalledOnce()
    const handler = onFocusChanged.mock.calls[0][0]
    handler({ payload: false })
    expect(m.motion.focused).toBe(false)
    handler({ payload: true })
    expect(m.motion.focused).toBe(true)
  })

  it('follows prefers-reduced-motion live', async () => {
    const mm = fakeMatchMedia(true)
    vi.stubGlobal('matchMedia', mm.matchMedia)
    const m = await fresh()
    m.watchMotion()
    expect(m.motion.reducedMotion).toBe(true)
    mm.set(false)
    expect(m.motion.reducedMotion).toBe(false)
  })

  it('without matchMedia, reduced motion is off', async () => {
    vi.stubGlobal('matchMedia', undefined)
    const m = await fresh()
    m.watchMotion()
    expect(m.motion.reducedMotion).toBe(false)
  })
})

describe('wireEmoji', () => {
  const STILL = 'pulse-media://localhost/still'
  const ANIM = 'pulse-media://localhost/anim'
  function img(inButton = false) {
    const i = document.createElement('img')
    i.setAttribute('src', STILL)
    if (inButton) {
      const b = document.createElement('button')
      b.append(i)
      document.body.append(b)
    } else document.body.append(i)
    return i
  }

  it('shares one IntersectionObserver and swaps src when on-screen', async () => {
    vi.spyOn(document, 'hasFocus').mockReturnValue(true)
    const m = await fresh()
    const a = img()
    const b = img()
    const stopA = m.wireEmoji(a, ANIM, STILL)
    const stopB = m.wireEmoji(b, ANIM, STILL)
    flushSync()
    expect(FakeIntersectionObserver.all).toHaveLength(1)
    expect(a.getAttribute('src')).toBe(STILL) // not yet reported on-screen
    FakeIntersectionObserver.set(a, true)
    flushSync()
    expect(a.getAttribute('src')).toBe(ANIM)
    expect(b.getAttribute('src')).toBe(STILL)
    FakeIntersectionObserver.set(a, false)
    flushSync()
    expect(a.getAttribute('src')).toBe(STILL)
    stopA()
    stopB()
    expect(FakeIntersectionObserver.watching()).toBe(0)
  })

  it('without IntersectionObserver, images count as on-screen', async () => {
    vi.stubGlobal('IntersectionObserver', undefined)
    vi.spyOn(document, 'hasFocus').mockReturnValue(true)
    const m = await fresh()
    const a = img()
    m.wireEmoji(a, ANIM, STILL)
    flushSync()
    expect(a.getAttribute('src')).toBe(ANIM)
  })

  it('on hover, animates while the pointer is over the containing button', async () => {
    vi.spyOn(document, 'hasFocus').mockReturnValue(true)
    const m = await fresh()
    m.setAnimateMode('hover')
    const a = img(true)
    m.wireEmoji(a, ANIM, STILL)
    FakeIntersectionObserver.set(a, true)
    flushSync()
    expect(a.getAttribute('src')).toBe(STILL)
    a.parentElement!.dispatchEvent(new Event('pointerenter'))
    flushSync()
    expect(a.getAttribute('src')).toBe(ANIM)
    a.parentElement!.dispatchEvent(new Event('pointerleave'))
    flushSync()
    expect(a.getAttribute('src')).toBe(STILL)
  })
})
