// @vitest-environment jsdom
// The header's ⋯ / bell menu opens by its button (#162), kept inside the window.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import ConversationMenu from './ConversationMenu.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import type { Channel } from '../lib/protocol/Channel'

const group: Channel = { id: 'G1', server_id: null, kind: 'group', name: 'Squad', position: 0 }
const MENU_W = 230
const MENU_H = 120

let cmp: ReturnType<typeof mount> | undefined
let anchor: HTMLButtonElement
const rect = (r: { left: number; top: number; width: number; height: number }) =>
  ({ ...r, right: r.left + r.width, bottom: r.top + r.height, x: r.left, y: r.top, toJSON: () => r }) as DOMRect

beforeEach(() => {
  app.state = { ...emptyState(), me: { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } } } as never
  // jsdom lays nothing out: the menu's size, as a real one would have
  vi.spyOn(HTMLElement.prototype, 'offsetWidth', 'get').mockImplementation(function (this: HTMLElement) { return this.classList.contains('menu') ? MENU_W : 0 })
  vi.spyOn(HTMLElement.prototype, 'offsetHeight', 'get').mockImplementation(function (this: HTMLElement) { return this.classList.contains('menu') ? MENU_H : 0 })
  Object.assign(window, { innerWidth: 1200, innerHeight: 800 })
  anchor = document.createElement('button')
  document.body.append(anchor)
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
  vi.restoreAllMocks()
})

const show = (r: Parameters<typeof rect>[0]) => {
  anchor.getBoundingClientRect = () => rect(r)
  cmp = mount(ConversationMenu, { target: document.body, props: { channel: group, muted: false, show: 'manage', anchor, onClose: () => {}, onLeft: () => {} } })
  flushSync()
  const menu = document.querySelector<HTMLElement>('[role=menu]')!
  return { menu, left: parseFloat(menu.style.left), top: parseFloat(menu.style.top) }
}

describe('ConversationMenu anchored to its button', () => {
  it('opens just under the button, its right edge lined up with the button\'s', () => {
    const { menu, left, top } = show({ left: 700, top: 10, width: 32, height: 32 })
    expect(menu.classList.contains('anchored')).toBe(true)
    expect(left).toBe(732 - MENU_W)
    expect(top).toBe(42 + 4)
  })

  it('stays inside the window on the left', () => {
    const { left } = show({ left: 40, top: 10, width: 32, height: 32 })
    expect(left).toBe(8)
  })

  it('stays inside the window on the right', () => {
    const { left } = show({ left: 1190, top: 10, width: 32, height: 32 })
    expect(left).toBe(1200 - MENU_W - 8)
  })

  it('flips above the button when there is no room below', () => {
    const { top } = show({ left: 700, top: 740, width: 32, height: 32 })
    expect(top).toBe(740 - 4 - MENU_H)
  })
})

// #203, #236: the bell's popover is the right-click menu's items: Mute … as the durations flyout (Unmute … while
// muted) and, on a server channel, Notifications as the levels flyout with yours ticked, muted or not.
describe("the bell's popover", () => {
  const general: Channel = { id: 'C1', server_id: 'S1', kind: 'text', name: 'general', position: 0 }
  const dm: Channel = { id: 'D1', server_id: null, kind: 'dm', name: null, position: 0 }
  type Level = { target_kind: 'server' | 'channel'; target_id: string; level: 'all' | 'mentions' | 'nothing' }
  let onClose = vi.fn()
  const open = (channel: Channel, levels: Level[] = [], muted = false) => {
    app.state = { ...app.state, notifyLevels: levels } as never
    anchor.getBoundingClientRect = () => rect({ left: 700, top: 10, width: 32, height: 32 })
    onClose = vi.fn()
    cmp = mount(ConversationMenu, { target: document.body, props: { channel, muted, show: 'mute', anchor, onClose, onLeft: () => {} } })
    flushSync()
  }
  const menu = () => document.querySelector<HTMLElement>('.menu')!
  const items = () => [...menu().querySelectorAll<HTMLButtonElement>(':scope > [role=menuitem]')]
  const labels = () => items().map((b) => b.textContent?.trim())
  const item = (label: string) => items().find((b) => b.textContent?.trim() === label)!
  const hover = (el: Element) => { el.dispatchEvent(new Event('pointerover', { bubbles: true })); el.dispatchEvent(new Event('pointerenter')); flushSync() }
  const flyout = (name: string) => document.querySelector<HTMLElement>(`[role=menu][aria-label="${name}"]`)
  const choices = (name: string) => [...(flyout(name)?.querySelectorAll<HTMLButtonElement>('[role^=menuitem]') ?? [])]
  const ticked = (name: string) => choices(name).filter((b) => b.getAttribute('aria-checked') === 'true').map((b) => b.textContent?.trim())
  const settle = () => new Promise((r) => setTimeout(r, 0))
  beforeEach(() => {
    invoke.mockReset()
    invoke.mockResolvedValue(undefined)
  })

  it('no inline list: Mute channel and Notifications, each a flyout as in the channel menu', () => {
    open(general)
    expect(document.querySelectorAll('.menu-label')).toHaveLength(0)
    expect(labels()).toEqual(['Mute channel', 'Notifications'])
    expect(items().map((b) => b.getAttribute('aria-haspopup'))).toEqual(['menu', 'menu'])
    expect(flyout('Mute #general for')).toBeNull()
  })

  it('hovering Mute channel shows the durations; picking one mutes it and closes', async () => {
    open(general)
    hover(item('Mute channel'))
    expect(choices('Mute #general for').map((b) => b.textContent?.trim()))
      .toEqual(['For 15 minutes', 'For 1 hour', 'For 8 hours', 'For 24 hours', 'Until I turn it back on'])
    // the bell is near the window's right: the flyout opens to the left
    expect(flyout('Mute #general for')!.dataset.side).toBe('left')
    choices('Mute #general for')[4].click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('set_mute', { targetKind: 'channel', targetId: 'C1', until: null })
    expect(onClose).toHaveBeenCalled()
  })

  it('Notifications lists the channel menu’s levels, Follow server ticked by default', () => {
    open(general)
    hover(item('Notifications'))
    expect(choices('Notifications for #general').map((b) => b.textContent?.trim())).toEqual(['Follow server', 'All messages', 'Only @mentions', 'Nothing'])
    expect(ticked('Notifications for #general')).toEqual(['Follow server'])
    // hovering the other item closes it (after the hover-intent wait)
    vi.useFakeTimers()
    hover(item('Mute channel'))
    vi.advanceTimersByTime(500); flushSync()
    vi.useRealTimers()
    expect(flyout('Notifications for #general')).toBeNull()
  })

  it('muted (#236): Unmute channel, and the levels are still there with its own ticked', async () => {
    open(general, [{ target_kind: 'channel', target_id: 'C1', level: 'nothing' }, { target_kind: 'server', target_id: 'S1', level: 'mentions' }], true)
    expect(labels()).toEqual(['Unmute channel', 'Notifications'])
    hover(item('Notifications'))
    expect(ticked('Notifications for #general')).toEqual(['Nothing'])
    item('Unmute channel').click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('clear_mute', { targetKind: 'channel', targetId: 'C1' })
  })

  it('picking a level sets it and closes; Follow server puts the default back', async () => {
    open(general)
    hover(item('Notifications'))
    choices('Notifications for #general').find((b) => b.textContent?.trim() === 'Only @mentions')!.click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('set_notify_level', { targetKind: 'channel', targetId: 'C1', level: 'mentions' })
    expect(onClose).toHaveBeenCalled()
    unmount(cmp!)
    open(general, [{ target_kind: 'channel', target_id: 'C1', level: 'mentions' }])
    hover(item('Notifications'))
    choices('Notifications for #general').find((b) => b.textContent?.trim() === 'Follow server')!.click()
    await settle()
    expect(invoke).toHaveBeenLastCalledWith('set_notify_level', { targetKind: 'channel', targetId: 'C1', level: null })
  })

  it('Escape with a flyout open closes just the flyout', () => {
    open(general)
    hover(item('Mute channel'))
    item('Mute channel').focus()
    item('Mute channel').dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))
    flushSync()
    expect(flyout('Mute #general for')).toBeNull()
    expect(onClose).not.toHaveBeenCalled()
  })

  it('a DM or group: Mute conversation (or Unmute conversation), no levels', () => {
    open(dm)
    expect(labels()).toEqual(['Mute conversation'])
    unmount(cmp!)
    open(dm, [], true)
    expect(labels()).toEqual(['Unmute conversation'])
  })
})

// #121: the DM list's menu for a 1:1 carries the person's items, Mute join sound among them
describe('a DM in the list', () => {
  const dm: Channel = { id: 'D1', server_id: null, kind: 'dm', name: null, position: 0 }
  const sam = { id: 'U2', username: 'sam', display_name: 'sam', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
  const texts = () => [...document.querySelectorAll<HTMLElement>('[role=menu] .menu-item')].map((b) => b.textContent?.trim())

  it('Mute join sound for someone with a join sound, then Unmute', () => {
    app.state = {
      ...app.state,
      dmMembers: { D1: ['U1', 'U2'] },
      sounds: { A: { id: 'A', server_id: 'S1', name: 'airhorn', emoji: null, emoji_id: null, media_hash: 'hA', volume: 100, duration_ms: 6000, added_by: null, created_at: '2026-11-10T00:00:00.000Z', plays: 0 } },
      members: { S1: [{ user: sam, nickname: null, join_sound: 'A' }] },
    } as never
    anchor.getBoundingClientRect = () => rect({ left: 100, top: 100, width: 20, height: 20 })
    cmp = mount(ConversationMenu, { target: document.body, props: { channel: dm, muted: false, show: 'all', anchor, onClose: () => {}, onLeft: () => {} } })
    flushSync()
    expect(texts()).toContain('Mute join sound')
    expect(texts().indexOf('Mute join sound')).toBe(texts().indexOf('Notify me when they go live') + 1)
    ;[...document.querySelectorAll<HTMLElement>('[role=menu] .menu-item')].find((b) => b.textContent?.trim() === 'Mute join sound')!.click()
    flushSync()
    expect(JSON.parse(localStorage.getItem('pulse.joinSounds.mutes')!)).toEqual({ U2: true })
    unmount(cmp)
    cmp = mount(ConversationMenu, { target: document.body, props: { channel: dm, muted: false, show: 'all', anchor, onClose: () => {}, onLeft: () => {} } })
    flushSync()
    expect(texts()).toContain('Unmute join sound')
    ;[...document.querySelectorAll<HTMLElement>('[role=menu] .menu-item')].find((b) => b.textContent?.trim() === 'Unmute join sound')!.click()
  })
})
