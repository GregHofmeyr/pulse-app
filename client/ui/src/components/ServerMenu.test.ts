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

let onCreateCategory = vi.fn()
const me = { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }

function render(muted = false, levels: { target_kind: 'server' | 'channel'; target_id: string; level: 'all' | 'mentions' | 'nothing' }[] = [], { member = false, live = true } = {}) {
  app.state = {
    ...emptyState(),
    me,
    servers: [{ id: 'S1', name: 'Alpha', icon_hash: null }],
    members: member ? { S1: [{ user: me, nickname: null }] } : {},
    layouts: live ? { S1: { revision: 1, categories: [] } } : {},
    mutes: muted ? [{ target_kind: 'server', target_id: 'S1', until: null }] : [],
    notifyLevels: levels,
  } as never
  opener = document.createElement('button')
  document.body.append(opener)
  onClose = vi.fn()
  onSettings = vi.fn()
  onError = vi.fn()
  onCreateCategory = vi.fn()
  cmp = mount(ServerMenu, { target: document.body, props: { serverId: 'S1', name: 'Alpha', x: 20, y: 50, opener, onClose, onSettings, onCreateCategory, onError } })
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
    expect(labels()).toEqual(['Server settings', 'Mute server', 'Notifications'])
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
    expect(labels()).toEqual(['Server settings', 'Unmute server', 'Notifications'])
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

  it('members get Create category (#54), which hands back to the shell and closes', () => {
    render(false, [], { member: true })
    expect(labels()).toEqual(['Server settings', 'Create category', 'Mute server', 'Notifications'])
    item('Create category').click()
    flushSync()
    expect(onCreateCategory).toHaveBeenCalledTimes(1)
    expect(onClose).toHaveBeenCalled()
  })

  it('no Create category outside the server, or on a server without categories', () => {
    render(false, [], { member: false })
    expect(labels()).toEqual(['Server settings', 'Mute server', 'Notifications'])
    unmount(cmp!)
    render(false, [], { member: true, live: false })
    expect(labels()).toEqual(['Server settings', 'Mute server', 'Notifications'])
  })
})

describe('ServerMenu notifications (#195)', () => {
  const levels = () => document.querySelector<HTMLElement>('[role=menu][aria-label="Notifications for Alpha"]')
  const choices = () => [...(levels()?.querySelectorAll<HTMLButtonElement>('[role=menuitemradio]') ?? [])]
  const choice = (label: string) => choices().find((b) => b.textContent?.trim() === label)!
  const ticked = () => choices().filter((b) => b.getAttribute('aria-checked') === 'true').map((b) => b.textContent?.trim())

  it('All messages is ticked by default; a picked level is ticked instead', () => {
    render()
    expect(item('Notifications').getAttribute('aria-haspopup')).toBe('menu')
    hover(item('Notifications'))
    expect(choices().map((b) => b.textContent?.trim())).toEqual(['All messages', 'Only @mentions', 'Nothing'])
    expect(ticked()).toEqual(['All messages'])
    expect(levels()!.dataset.side).toBe('right')
    unmount(cmp!)
    cmp = undefined
    document.body.innerHTML = ''
    render(false, [{ target_kind: 'server', target_id: 'S1', level: 'mentions' }])
    hover(item('Notifications'))
    expect(ticked()).toEqual(['Only @mentions'])
  })

  it('picking a level sets it; All messages puts the default back', async () => {
    render(false, [{ target_kind: 'server', target_id: 'S1', level: 'nothing' }])
    hover(item('Notifications'))
    choice('Only @mentions').click()
    await Promise.resolve()
    expect(invoke).toHaveBeenCalledWith('set_notify_level', { targetKind: 'server', targetId: 'S1', level: 'mentions' })
    expect(onClose).toHaveBeenCalled()
    unmount(cmp!)
    cmp = undefined
    document.body.innerHTML = ''
    render(false, [{ target_kind: 'server', target_id: 'S1', level: 'nothing' }])
    hover(item('Notifications'))
    choice('All messages').click()
    await Promise.resolve()
    expect(invoke).toHaveBeenCalledWith('set_notify_level', { targetKind: 'server', targetId: 'S1', level: null })
  })

  it('is offered while the server is muted too, and the keyboard reaches it', () => {
    render(true)
    key(item('Server settings'), 'End')
    expect(document.activeElement).toBe(item('Notifications'))
    key(item('Notifications'), 'ArrowRight')
    expect(document.activeElement).toBe(choice('All messages'))
    key(document.activeElement!, 'ArrowLeft')
    expect(levels()).toBeNull()
    expect(document.activeElement).toBe(item('Notifications'))
  })

  it('Escape with a hover-opened flyout closes only that flyout, then the menu', () => {
    render()
    hover(item('Notifications'))
    expect(levels()).not.toBeNull()
    key(item('Server settings'), 'Escape')
    expect(levels()).toBeNull()
    expect(onClose).not.toHaveBeenCalled()
    key(item('Server settings'), 'Escape')
    expect(onClose).toHaveBeenCalled()
  })

  it('a failed pick is handed back once the menu has gone', async () => {
    render()
    invoke.mockRejectedValueOnce('no network')
    hover(item('Notifications'))
    choice('Nothing').click()
    await vi.waitFor(() => expect(onError).toHaveBeenCalledWith('no network'))
  })
})
