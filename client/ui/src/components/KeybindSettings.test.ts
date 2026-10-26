// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

type Applied = { binds: unknown; enabled: boolean; problems: string[]; chosen: boolean }
const F13 = { code: 'F13', ctrl: false, shift: false, alt: false, meta: false }
let state: Applied
const invoke = vi.fn(async (cmd: string, args?: unknown): Promise<unknown> => {
  if (cmd === 'get_keybinds') return state
  if (cmd === 'set_keybinds_enabled') {
    state = { ...state, enabled: (args as { enabled: boolean }).enabled }
    return state
  }
  return undefined
})
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: unknown) => invoke(cmd, args) }))

import KeybindSettings from './KeybindSettings.svelte'

const settle = async () => {
  for (let i = 0; i < 5; i++) await Promise.resolve()
  flushSync()
}
const master = () => document.querySelector<HTMLButtonElement>('button.switch[aria-label="Global keybinds"]')!
const editors = () => [...document.querySelectorAll<HTMLButtonElement>('button.key, button.clear')]
const calls = (name: string) => invoke.mock.calls.filter(([cmd]) => cmd === name).map(([, a]) => a)

describe('KeybindSettings master switch', () => {
  let c: ReturnType<typeof mount> | null = null
  beforeEach(() => {
    invoke.mockClear()
    state = { binds: { toggle_mute: F13, toggle_deafen: null }, enabled: true, problems: [], chosen: true }
  })
  afterEach(() => {
    if (c) unmount(c)
    c = null
    document.body.innerHTML = ''
  })

  it('shows the switch on by default, with editors usable, and says what the hooks are for', async () => {
    c = mount(KeybindSettings, { target: document.body })
    await settle()
    expect(master().getAttribute('aria-pressed')).toBe('true')
    expect(editors().find((b) => b.classList.contains('key'))!.disabled).toBe(false)
    const text = document.body.textContent ?? ''
    expect(text).toContain('nothing is recorded or sent')
    expect(text).toContain('removes the hooks entirely')
  })

  it('turning it off calls the command, reflects the new state and disables the bind editors', async () => {
    c = mount(KeybindSettings, { target: document.body })
    await settle()
    master().click()
    await settle()
    expect(calls('set_keybinds_enabled')).toEqual([{ enabled: false }])
    expect(master().getAttribute('aria-pressed')).toBe('false')
    expect(editors().length).toBeGreaterThan(0)
    expect(editors().every((b) => b.disabled)).toBe(true)
    // and back on
    master().click()
    await settle()
    expect(calls('set_keybinds_enabled')).toEqual([{ enabled: false }, { enabled: true }])
    expect(master().getAttribute('aria-pressed')).toBe('true')
    expect(editors().find((b) => b.classList.contains('key'))!.disabled).toBe(false)
  })

  it('saved off: starts off, and the editors can’t start a recording', async () => {
    state.enabled = false
    c = mount(KeybindSettings, { target: document.body })
    await settle()
    expect(master().getAttribute('aria-pressed')).toBe('false')
    editors()[0].click()
    await settle()
    expect(calls('pause_keybinds')).toEqual([])
  })

  it('a failed switch keeps showing what is actually in use', async () => {
    c = mount(KeybindSettings, { target: document.body })
    await settle()
    invoke.mockImplementationOnce(async () => {
      throw "couldn't save keybinds: disk full"
    })
    master().click()
    await settle()
    expect(master().getAttribute('aria-pressed')).toBe('true')
    expect(document.querySelector('[role=alert]')?.textContent).toContain('disk full')
  })
})
