// @vitest-environment jsdom
// Server settings → Members (#418): avatar, name and join date for everyone in the server, the owner marked, no
// buttons on the rows but the avatar's card; a person's actions are on their right-click menu (Remove from server and
// Make owner for the owner). Under them, for the owner, the people blocked from rejoining (#418), with Allow back on
// their right-click menu.
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import ServerSettings from './ServerSettings.svelte'
import { app, ui } from '../lib/store.svelte'
import { applyEvent, emptyState } from '../lib/state'

const user = (id: string, name: string) => ({ id, username: name, display_name: name, avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } })
const alex = user('U1', 'alex')
const sam = user('U2', 'sam')
const kim = user('U3', 'kim')
const jo = user('U4', 'jo')

let cmp: ReturnType<typeof mount> | undefined
function open(owner: string) {
  app.state = {
    ...emptyState(),
    me: alex,
    servers: [{ id: 'S1', name: 'Alpha', icon_hash: null, owner_id: owner }],
    members: { S1: [
      { user: alex, nickname: null, joined_at: '2026-10-03T08:00:00.000Z' },
      { user: sam, nickname: null, joined_at: '2026-11-01T08:00:00.000Z' },
      { user: kim, nickname: null },
    ] },
    people: Object.fromEntries([alex, sam, kim, jo].map((u) => [u.id, { user: u, online: true, last_seen_at: null }])),
  } as never
  cmp = mount(ServerSettings, { target: document.body, props: { get server() { return app.state.servers[0] }, onClose: vi.fn() } })
  flushSync()
  ;[...document.querySelectorAll<HTMLButtonElement>('nav button')].find((b) => b.textContent === 'Members')!.click()
  flushSync()
}
const rows = () => [...document.querySelectorAll<HTMLElement>('.content li > .row')]
const ids = () => [...document.querySelectorAll<HTMLElement>('[role=menu] > [data-item]')].map((i) => i.dataset.item)
const item = (id: string) => document.querySelector<HTMLElement>(`[role=menu] > [data-item="${id}"]`)
const rightClick = (el: HTMLElement) => {
  el.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 5, clientY: 5 }))
  flushSync()
}

beforeEach(() => {
  invoke.mockReset()
  invoke.mockResolvedValue(undefined)
  ui.serverAction = null
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('Server settings: Members', () => {
  it('lists everyone with their name and join date, the owner marked', () => {
    open('U1')
    const list = rows()
    expect(list.map((r) => r.dataset.user)).toEqual(['U1', 'U2', 'U3'])
    expect(list[0].textContent).toContain('alex')
    expect(list[0].textContent).toContain('Joined Oct 3, 2026')
    expect(list[0].querySelector('.owner')?.textContent).toBe('Owner')
    expect(list[1].textContent).toContain('Joined Nov 1, 2026')
    expect(list[1].querySelector('.owner')).toBeNull()
    // a member from an older server has no date
    expect(list[2].textContent).not.toContain('Joined')
  })

  it('has no action buttons on its rows', () => {
    open('U1')
    for (const r of rows()) {
      const buttons = [...r.querySelectorAll('button')].filter((b) => !b.closest('.avatar, [data-avatar]'))
      expect(buttons.map((b) => b.getAttribute('aria-label') ?? b.textContent)).not.toContain('Remove from server')
      expect(r.textContent).not.toContain('Make owner')
    }
  })

  it('the owner\'s right-click on someone offers Make owner and Remove from server, asked in the shell', () => {
    open('U1')
    rightClick(rows()[1])
    expect(ids()).toContain('make-owner')
    expect(ids().at(-1)).toBe('remove-server')
    expect(item('make-owner')!.classList.contains('danger')).toBe(true)
    item('make-owner')!.click()
    flushSync()
    expect(ui.serverAction).toEqual({ kind: 'transfer', serverId: 'S1', userId: 'U2' })
    rightClick(rows()[2])
    item('remove-server')!.click()
    flushSync()
    expect(ui.serverAction).toEqual({ kind: 'remove', serverId: 'S1', userId: 'U3' })
    expect(invoke).not.toHaveBeenCalledWith('transfer_server', expect.anything())
  })

  it('others get the person menu without them, and nobody gets them on themselves', () => {
    open('U2')
    rightClick(rows()[2])
    expect(ids()).not.toContain('make-owner')
    expect(ids()).not.toContain('remove-server')
    unmount(cmp!)
    document.body.innerHTML = ''
    open('U1')
    rightClick(rows()[0])
    expect(ids()).toContain('profile')
    expect(ids()).not.toContain('make-owner')
  })

  describe('blocked from rejoining (#418)', () => {
    const settle = async () => { for (let i = 0; i < 5; i++) await Promise.resolve(); await tick() }
    const blockedRows = () => [...document.querySelectorAll<HTMLElement>('[data-blocked]')]
    beforeEach(() => {
      invoke.mockImplementation(async (cmd: string) =>
        cmd === 'list_server_blocks' ? [{ user_id: 'U4', blocked_at: '2026-11-15T08:00:00.000Z' }] : undefined)
    })

    it('the owner sees who is blocked, and Allow back on their right-click lets them back', async () => {
      open('U1')
      await settle()
      expect(invoke).toHaveBeenCalledWith('list_server_blocks', { serverId: 'S1' })
      const list = blockedRows()
      expect(list.map((r) => r.dataset.blocked)).toEqual(['U4'])
      expect(list[0].textContent).toContain('jo')
      expect(list[0].textContent).toContain('Blocked Nov 15, 2026')
      rightClick(list[0])
      expect(ids()).toContain('allow-back')
      expect(ids()).not.toContain('remove-server')
      invoke.mockImplementation(async () => [])
      item('allow-back')!.click()
      flushSync()
      await settle()
      expect(invoke).toHaveBeenCalledWith('allow_back', { serverId: 'S1', userId: 'U4' })
      expect(blockedRows()).toHaveLength(0)
      expect(document.querySelector('.content h3')).toBeNull()
    })

    it('members get no Allow back, and others never ask', async () => {
      open('U1')
      await settle()
      rightClick(rows()[1])
      expect(ids()).not.toContain('allow-back')
      unmount(cmp!)
      document.body.innerHTML = ''
      invoke.mockClear()
      open('U2')
      await settle()
      expect(invoke).not.toHaveBeenCalledWith('list_server_blocks', expect.anything())
      expect(blockedRows()).toHaveLength(0)
    })

    it('a block change on another of the owner\'s devices asks again; other events don\'t (#431)', async () => {
      open('U1')
      await settle()
      expect(blockedRows()).toHaveLength(1)
      const asks = () => invoke.mock.calls.filter((c) => c[0] === 'list_server_blocks').length
      expect(asks()).toBe(1)
      invoke.mockImplementation(async () => [])
      app.state = applyEvent(app.state, { t: 'ServerBlocksChanged', d: { user_id: 'U1', server_id: 'S2' } }, 0)
      app.state = applyEvent(app.state, { t: 'Typing', d: { channel_id: 'C1', user_id: 'U2' } } as never, 0)
      flushSync()
      await settle()
      expect(asks()).toBe(1)
      app.state = applyEvent(app.state, { t: 'ServerBlocksChanged', d: { user_id: 'U1', server_id: 'S1' } }, 0)
      flushSync()
      await settle()
      expect(asks()).toBe(2)
      expect(blockedRows()).toHaveLength(0)
    })

    it('a refused Allow back says why', async () => {
      open('U1')
      await settle()
      invoke.mockImplementation(async (cmd: string) => {
        if (cmd === 'allow_back') throw { kind: 'Forbidden', message: "Only the server's owner can see or lift rejoin blocks." }
        return [{ user_id: 'U4', blocked_at: '2026-11-15T08:00:00.000Z' }]
      })
      rightClick(blockedRows()[0])
      item('allow-back')!.click()
      flushSync()
      await settle()
      expect(document.querySelector('.content [role=alert]')?.textContent).toContain('owner')
      expect(blockedRows()).toHaveLength(1)
    })
  })
})
