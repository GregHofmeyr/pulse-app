// @vitest-environment jsdom
// The server tab's right-click menu (#161): Server settings, and Mute server as a durations flyout (the channel
// menu's, #131).
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invoke(...a) }))

import ServerMenu from './ServerMenu.svelte'
import { app, clock } from '../lib/store.svelte'
import { emptyState } from '../lib/state'

let cmp: ReturnType<typeof mount> | undefined
let opener: HTMLButtonElement
let onClose = vi.fn()
let onSettings = vi.fn()
let onError = vi.fn()

function render(muted = false) {
  app.state = {
    ...emptyState(),
    servers: [{ id: 'S1', name: 'Alpha', icon_hash: null }],
    mutes: muted ? [{ target_kind: 'server', target_id: 'S1', until: null }] : [],
  } as never
  opener = document.createElement('button')
  document.body.append(opener)
  onClose = vi.fn()
  onSettings = vi.fn()
  onError = vi.fn()
  cmp = mount(ServerMenu, { target: document.body, props: { serverId: 'S1', name: 'Alpha', x: 20, y: 50, opener, onClose, onSettings, onError } })
  flushSync()
}
const menu = () => document.querySelector<HTMLElement>('[role=menu][aria-label="Actions for Alpha"]')!
const items = () => [...menu().querySelectorAll<HTMLButtonElement>(':scope > [role=menuitem]')]
const labels = () => items().map((b) => b.textContent?.trim())
const item = (label: string) => items().find((b) => b.textContent?.trim() === label)!
const flyout = () => document.querySelector<HTMLElement>('[role=menu][aria-label="Mute Alpha for"]')
const flyoutItem = (label: string) => [...(flyout()?.querySelectorAll<HTMLButtonElement>('[role=menuitem]') ?? [])].find((b) => b.textContent?.trim() === label)
const key = (el: Element, k: string) => {
  el.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true }))
  flushSync()
}
const hover = (el: Element) => { el.dispatchEvent(new Event('pointerover', { bubbles: true })); el.dispatchEvent(new Event('pointerenter')); flushSync() }

beforeEach(() => {
  invoke.mockReset()
  invoke.mockResolvedValue(undefined)
  clock.nowIso = '2026-11-05T10:00:00.000Z'
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('ServerMenu', () => {
  it('lists Server settings and one Mute server item, focus on the first', () => {
    render()
    expect(labels()).toEqual(['Server settings', 'Mute server'])
    expect(item('Mute server').getAttribute('aria-haspopup')).toBe('menu')
    expect(flyout()).toBeNull()
    expect(document.activeElement).toBe(item('Server settings'))
  })

  it('Server settings opens the settings and closes the menu', () => {
    render()
    item('Server settings').click()
    flushSync()
    expect(onSettings).toHaveBeenCalledTimes(1)
    expect(onClose).toHaveBeenCalled()
  })

  it('hovering Mute server opens the durations; picking one mutes the server', async () => {
    render()
    hover(item('Mute server'))
    expect([...flyout()!.querySelectorAll('[role=menuitem]')].map((b) => b.textContent?.trim()))
      .toEqual(['For 15 minutes', 'For 1 hour', 'For 8 hours', 'For 24 hours', 'Until I turn it back on'])
    // near the window's left edge (the tabs sit there): it opens to the right
    expect(flyout()!.dataset.side).toBe('right')
    flyoutItem('Until I turn it back on')!.click()
    await Promise.resolve()
    expect(invoke).toHaveBeenCalledWith('set_mute', { targetKind: 'server', targetId: 'S1', until: null })
    expect(onClose).toHaveBeenCalled()
  })

  it('the keyboard opens and leaves the flyout like the channel menu', () => {
    render()
    key(item('Server settings'), 'ArrowDown')
    expect(document.activeElement).toBe(item('Mute server'))
    key(item('Mute server'), 'ArrowRight')
    expect(document.activeElement).toBe(flyoutItem('For 15 minutes'))
    key(document.activeElement!, 'ArrowLeft')
    expect(flyout()).toBeNull()
    expect(document.activeElement).toBe(item('Mute server'))
    key(item('Mute server'), 'Escape')
    expect(onClose).toHaveBeenCalled()
    expect(document.activeElement).toBe(opener)
  })

  it('a muted server offers Unmute server instead', async () => {
    render(true)
    expect(labels()).toEqual(['Server settings', 'Unmute server'])
    item('Unmute server').click()
    await Promise.resolve()
    expect(invoke).toHaveBeenCalledWith('clear_mute', { targetKind: 'server', targetId: 'S1' })
  })

  it('a failed mute is handed back once the menu has gone', async () => {
    render()
    invoke.mockRejectedValueOnce('no network')
    hover(item('Mute server'))
    flyoutItem('For 1 hour')!.click()
    await vi.waitFor(() => expect(onError).toHaveBeenCalledWith('no network'))
  })
})
