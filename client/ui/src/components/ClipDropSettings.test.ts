// @vitest-environment jsdom
// Settings › Chat › Clip drop (#382): its own switch, apart from Game activity (off there leaves it on and usable), the
// folders it watches, each change saved and told to the core. Windows only.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import ClipDropSettings from './ClipDropSettings.svelte'
import ChatSettings from './ChatSettings.svelte'
import { clipDropPrefs } from '../lib/clipDropPrefs.svelte'
import { gamePrefs } from '../lib/gamePrefs.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import { settle } from '../lib/testSettle'

let cmp: ReturnType<typeof mount> | undefined
const realUA = navigator.userAgent
const setUA = (value: string) => Object.defineProperty(navigator, 'userAgent', { configurable: true, value })
const sw = () => document.querySelector<HTMLInputElement>('input[data-switch="clip-drop"]')
const told = () => invoke.mock.calls.filter((c) => c[0] === 'set_clip_drop').map((c) => (c[1] as { settings: unknown }).settings)
const button = (text: string) => [...document.querySelectorAll('button')].find((b) => b.textContent?.trim() === text)
const folderPaths = () => [...document.querySelectorAll('.folders .path')].map((e) => e.textContent)
const defaults = ['C:\\Users\\alex\\Videos\\Captures', 'C:\\Users\\alex\\Videos']
function show(c: typeof ClipDropSettings | typeof ChatSettings = ClipDropSettings) {
  cmp = mount(c, { target: document.body })
  flushSync()
}

beforeEach(() => {
  invoke.mockReset()
  invoke.mockImplementation(async (cmd: string) => (cmd === 'clip_drop_defaults' ? defaults : cmd === 'pick_clip_folder' ? 'D:\\Clips' : undefined))
  localStorage.clear()
  Object.assign(clipDropPrefs, { on: true, folders: null })
  gamePrefs.enabled = true
  app.state = emptyState()
  setUA('Mozilla/5.0 (Windows NT 10.0; Win64; x64)')
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
  setUA(realUA)
})

describe('Clip drop settings', () => {
  it('a section of its own in Settings › Chat, headed "Clip drop"', () => {
    show(ChatSettings)
    const title = [...document.querySelectorAll('.settings-section-title')].find((h) => h.textContent === 'Clip drop')
    expect(title).toBeDefined()
    expect(sw()!.checked).toBe(true)
  })

  it('not shown outside Windows', () => {
    setUA('Mozilla/5.0 (X11; Linux x86_64)')
    show()
    expect(sw()).toBeNull()
  })

  it('Game activity off: the switch stays on and usable', () => {
    gamePrefs.enabled = false
    show()
    expect([sw()!.checked, sw()!.disabled]).toEqual([true, false])
    expect(document.querySelector('.folders')).not.toBeNull()
  })

  it('its switch is told to the core and saved; off hides the folders', () => {
    show()
    sw()!.click()
    flushSync()
    expect(told().at(-1)).toEqual({ on: false, folders: null })
    expect(JSON.parse(localStorage.getItem('pulse.clipDrop')!).on).toBe(false)
    expect(document.querySelector('.folders')).toBeNull()
  })

  it('the defaults show; adding one, removing one and going back to the defaults are told', async () => {
    show()
    await settle(3)
    expect(folderPaths()).toEqual(defaults)
    expect(button('Use the defaults')).toBeUndefined()

    button('Add a folder')!.click()
    await settle(3)
    expect(told().at(-1)).toEqual({ on: true, folders: [...defaults, 'D:\\Clips'] })
    expect(folderPaths()).toEqual([...defaults, 'D:\\Clips'])

    // (backslashes in a path: found by its label, not a CSS selector)
    ;[...document.querySelectorAll('button')].find((b) => b.getAttribute('aria-label') === `Stop watching ${defaults[0]}`)!.click()
    flushSync()
    expect(told().at(-1)).toEqual({ on: true, folders: [defaults[1], 'D:\\Clips'] })

    button('Use the defaults')!.click()
    flushSync()
    expect(told().at(-1)).toEqual({ on: true, folders: null })
    expect(folderPaths()).toEqual(defaults)
  })
})
