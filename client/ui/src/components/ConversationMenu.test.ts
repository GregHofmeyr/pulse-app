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

// #203: the bell's popover offers a server channel's notification level, as its right-click menu's flyout does.
describe("the bell's popover: notification levels", () => {
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
  const radios = () => [...document.querySelectorAll<HTMLButtonElement>('[role=menu] [role=menuitemradio]')]
  const ticked = () => radios().filter((b) => b.getAttribute('aria-checked') === 'true').map((b) => b.textContent?.trim())
  const settle = () => new Promise((r) => setTimeout(r, 0))
  beforeEach(() => {
    invoke.mockReset()
    invoke.mockResolvedValue(undefined)
  })

  it('lists the channel menu’s choices under the mute durations, Follow server ticked by default', () => {
    open(general)
    expect([...document.querySelectorAll('.menu-label')].map((l) => l.textContent)).toEqual(['Mute', 'Notifications'])
    expect(radios().map((b) => b.textContent?.trim())).toEqual(['Follow server', 'All messages', 'Only @mentions', 'Nothing'])
    expect(ticked()).toEqual(['Follow server'])
    // the tick is the status menu's check icon, on the ticked one only
    expect(radios().map((b) => !!b.querySelector('svg'))).toEqual([true, false, false, false])
  })

  it("ticks the channel's own level, also while it's muted", () => {
    open(general, [{ target_kind: 'channel', target_id: 'C1', level: 'nothing' }, { target_kind: 'server', target_id: 'S1', level: 'mentions' }], true)
    expect(ticked()).toEqual(['Nothing'])
  })

  it('picking one sets it and closes; Follow server puts the default back', async () => {
    open(general)
    radios().find((b) => b.textContent?.trim() === 'Only @mentions')!.click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('set_notify_level', { targetKind: 'channel', targetId: 'C1', level: 'mentions' })
    expect(onClose).toHaveBeenCalled()
    unmount(cmp!)
    open(general, [{ target_kind: 'channel', target_id: 'C1', level: 'mentions' }])
    radios().find((b) => b.textContent?.trim() === 'Follow server')!.click()
    await settle()
    expect(invoke).toHaveBeenLastCalledWith('set_notify_level', { targetKind: 'channel', targetId: 'C1', level: null })
  })

  it('a DM or group has no levels', () => {
    open(dm)
    expect(radios()).toEqual([])
    expect([...document.querySelectorAll('.menu-label')].map((l) => l.textContent)).toEqual(['Mute'])
  })
})
