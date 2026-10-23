// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

type Handler = (e: { payload: boolean }) => void
let handler: Handler | undefined
let subscribe: () => Promise<() => void>
const onFocusChanged = vi.fn(async (h: Handler) => {
  handler = h
  return subscribe()
})
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({ onFocusChanged }) }))

type Mod = typeof import('./windowFocus.svelte')
// The watcher installs once per module: each test gets a fresh one.
async function fresh(): Promise<Mod> {
  vi.resetModules()
  return import('./windowFocus.svelte')
}
const fire = (type: 'focus' | 'blur') => window.dispatchEvent(new Event(type))
const settle = () => new Promise((r) => setTimeout(r, 0))
const inTauri = () => ((window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = {})

beforeEach(() => {
  handler = undefined
  subscribe = async () => () => {}
  onFocusChanged.mockClear()
})
afterEach(() => {
  vi.restoreAllMocks()
  delete (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__
})

describe('window focus without Tauri (DOM fallback)', () => {
  it('starts from document.hasFocus() and follows window focus/blur', async () => {
    vi.spyOn(document, 'hasFocus').mockReturnValue(false)
    const m = await fresh()
    m.watchWindowFocus()
    expect(m.windowFocus.focused).toBe(false)
    expect(onFocusChanged).not.toHaveBeenCalled()
    fire('focus')
    expect(m.windowFocus.focused).toBe(true)
    fire('blur')
    expect(m.windowFocus.focused).toBe(false)
  })
})

describe('window focus in Tauri (C8: the OS window is the authority)', () => {
  it('once subscribed, DOM focus/blur no longer change it; Tauri events do', async () => {
    inTauri()
    vi.spyOn(document, 'hasFocus').mockReturnValue(true)
    const m = await fresh()
    m.watchWindowFocus()
    m.watchWindowFocus() // once only
    expect(onFocusChanged).toHaveBeenCalledOnce()
    await settle() // subscription established
    expect(m.windowFocus.focused).toBe(true) // hasFocus() holds until the first Tauri event
    fire('blur') // e.g. keyboard focus leaving the webview while the window stays active
    expect(m.windowFocus.focused).toBe(true)
    handler!({ payload: false })
    expect(m.windowFocus.focused).toBe(false)
    fire('focus')
    expect(m.windowFocus.focused).toBe(false)
    handler!({ payload: true })
    expect(m.windowFocus.focused).toBe(true)
  })

  it('DOM events count until the subscription is established', async () => {
    inTauri()
    vi.spyOn(document, 'hasFocus').mockReturnValue(true)
    let established!: () => void
    subscribe = () => new Promise((done) => (established = () => done(() => {})))
    const m = await fresh()
    m.watchWindowFocus()
    fire('blur')
    expect(m.windowFocus.focused).toBe(false)
    established()
    await settle()
    fire('focus')
    expect(m.windowFocus.focused).toBe(false) // the DOM no longer decides
    handler!({ payload: true })
    expect(m.windowFocus.focused).toBe(true)
  })

  it('a failed subscription falls back to the DOM events', async () => {
    inTauri()
    vi.spyOn(document, 'hasFocus').mockReturnValue(true)
    subscribe = async () => {
      throw new Error('not allowed')
    }
    const m = await fresh()
    m.watchWindowFocus()
    await settle()
    fire('blur')
    expect(m.windowFocus.focused).toBe(false)
    fire('focus')
    expect(m.windowFocus.focused).toBe(true)
  })
})
