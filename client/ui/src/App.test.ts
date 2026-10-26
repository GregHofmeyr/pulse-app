// @vitest-environment jsdom
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn(async (cmd: string) => (cmd === 'restore_session' ? null : undefined))
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invoke(...(a as [string])), convertFileSrc: (s: string) => s }))
vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }))
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ onFocusChanged: async () => () => {}, isFocused: async () => true }),
}))

import App from './App.svelte'

let cmp: ReturnType<typeof mount> | undefined
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('animation freeze', () => {
  it('body_is_unfocused_while_the_window_is_blurred_even_on_the_login_screen', async () => {
    cmp = mount(App, { target: document.body })
    for (let i = 0; i < 5; i++) await tick()
    await new Promise((r) => setTimeout(r, 0))
    flushSync()
    expect(document.querySelector('.boot')).toBeNull() // past boot: the login screen
    window.dispatchEvent(new Event('focus'))
    flushSync()
    expect(document.body.classList.contains('unfocused')).toBe(false)
    window.dispatchEvent(new Event('blur'))
    flushSync()
    expect(document.body.classList.contains('unfocused')).toBe(true)
    window.dispatchEvent(new Event('focus'))
    flushSync()
    expect(document.body.classList.contains('unfocused')).toBe(false)
  })
})
