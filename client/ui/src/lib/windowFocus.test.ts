// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

type Handler = (e: { payload: boolean }) => void
let handler: Handler | undefined
let subscribe: () => Promise<() => void>
const onFocusChanged = vi.fn(async (h: Handler) => {
  handler = h
  return subscribe()
})
let currentlyFocused: () => Promise<boolean>
const isFocused = vi.fn(() => currentlyFocused())
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({ onFocusChanged, isFocused }) }))
const invoke = vi.hoisted(() => vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined))
vi.mock('@tauri-apps/api/core', () => ({ invoke }))
const logged = () => invoke.mock.calls.filter((c) => c[0] === 'ui_log').map((c) => (c[1] as { line: string }).line)

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
  currentlyFocused = async () => true
  onFocusChanged.mockClear()
  isFocused.mockClear()
  invoke.mockClear()
})
afterEach(() => {
  vi.restoreAllMocks()
  delete (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__
})

describe('window focus without Tauri (DOM fallback)', () => {
  it('test files start from a focused window (shared setup)', async () => {
    const m = await fresh()
    m.watchWindowFocus()
    expect(m.windowFocus.focused).toBe(true)
  })

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
    let established!: () => void
    subscribe = () => new Promise((done) => (established = () => done(() => {})))
    currentlyFocused = async () => false // the window really is in the background
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

  it('once subscribed, reads the window\'s current focus (hasFocus() may be wrong at startup)', async () => {
    inTauri()
    vi.spyOn(document, 'hasFocus').mockReturnValue(false)
    const m = await fresh()
    m.watchWindowFocus()
    expect(m.windowFocus.focused).toBe(false)
    await settle()
    expect(isFocused).toHaveBeenCalledOnce()
    expect(m.windowFocus.focused).toBe(true) // focused in the foreground, no focus event ever came
  })

  it('a focus event that lands first beats the current-focus read', async () => {
    inTauri()
    let answer!: (f: boolean) => void
    currentlyFocused = () => new Promise((done) => (answer = done))
    const m = await fresh()
    m.watchWindowFocus()
    await settle()
    handler!({ payload: false })
    answer(true) // read before the event, answered after it: stale
    await settle()
    expect(m.windowFocus.focused).toBe(false)
  })

  it('a failed current-focus read keeps the value', async () => {
    inTauri()
    currentlyFocused = async () => {
      throw new Error('not allowed')
    }
    const m = await fresh()
    m.watchWindowFocus()
    await settle()
    expect(m.windowFocus.focused).toBe(true)
  })
})

describe('what the log says about focus', () => {
  it('the focus the page starts from', async () => {
    inTauri()
    currentlyFocused = async () => false
    const m = await fresh()
    m.watchWindowFocus()
    for (let i = 0; i < 20 && !logged().length; i++) await settle()
    expect(logged()).toEqual(['focus: at start focused=false'])
  })

  it('that window focus events are missing', async () => {
    inTauri()
    subscribe = async () => Promise.reject(new Error('no permission'))
    const m = await fresh()
    m.watchWindowFocus()
    for (let i = 0; i < 20 && !logged().length; i++) await settle()
    expect(logged()).toEqual(['focus: no window focus events; following the page instead'])
  })
})
