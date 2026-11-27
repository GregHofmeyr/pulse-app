// @vitest-environment jsdom
// Server settings → Members (#418): avatar, name and join date for everyone in the server, the owner marked, no
// buttons on the rows but the avatar's card; a person's actions are on their right-click menu (Remove from server and
// Make owner for the owner). The rejoin-block list is gone (#80: servers are invite-only, and bans keep people out).
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import ServerSettings from './ServerSettings.svelte'
import { app, ui } from '../lib/store.svelte'
import { emptyState } from '../lib/state'

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
    expect(ids().slice(-2)).toEqual(['remove-server', 'ban'])
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

  it('there is no list of people blocked from rejoining any more (#80: bans replace it)', () => {
    open('U1')
    flushSync()
    expect(document.querySelector('[data-blocked]')).toBeNull()
    expect(invoke).not.toHaveBeenCalledWith('list_server_blocks', expect.anything())
  })
})
