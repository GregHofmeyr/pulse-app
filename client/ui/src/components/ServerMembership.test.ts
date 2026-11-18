// @vitest-environment jsdom
// Leaving a server and removing someone from it (#371): Leave server on the tab's menu (not the owner's) and in Server
// settings, Remove from server on a person's menu (the owner's), each asked first in the shell's dialog.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import ServerMenu from './ServerMenu.svelte'
import MemberMenu from './MemberMenu.svelte'
import ServerActionDialog from './ServerActionDialog.svelte'
import ServerSettings from './ServerSettings.svelte'
import { app, ui } from '../lib/store.svelte'
import { emptyState } from '../lib/state'

const alex = { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const sam = { id: 'U2', username: 'sam', display_name: 'sam', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }

let cmp: ReturnType<typeof mount> | undefined
function state(owner: string) {
  app.state = {
    ...emptyState(),
    me: alex,
    servers: [{ id: 'S1', name: 'Alpha', icon_hash: null, owner_id: owner }],
    members: { S1: [{ user: alex, nickname: null }, { user: sam, nickname: null }] },
    people: { U1: { user: alex, online: true, last_seen_at: null }, U2: { user: sam, online: true, last_seen_at: null } },
  } as never
}
const opener = () => {
  const b = document.createElement('button')
  document.body.append(b)
  return b
}
const ids = () => [...document.querySelectorAll<HTMLElement>('[role=menu] > [data-item]')].map((i) => i.dataset.item)
const item = (id: string) => document.querySelector<HTMLElement>(`[role=menu] > [data-item="${id}"]`)

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

describe('Leave server on the tab menu', () => {
  const show = () => {
    cmp = mount(ServerMenu, { target: document.body, props: { serverId: 'S1', name: 'Alpha', x: 10, y: 10, opener: opener(), onClose: vi.fn(), onSettings: vi.fn(), onError: vi.fn() } })
    flushSync()
  }

  it('is there for a member who isn\'t the owner, in the danger colour, and asks first', () => {
    state('U2')
    show()
    expect(ids()).toContain('leave')
    expect(item('leave')!.classList.contains('danger')).toBe(true)
    item('leave')!.click()
    flushSync()
    expect(ui.serverAction).toEqual({ kind: 'leave', serverId: 'S1' })
    expect(invoke).not.toHaveBeenCalledWith('leave_server', expect.anything())
  })

  it('isn\'t there for the owner', () => {
    state('U1')
    show()
    expect(ids()).not.toContain('leave')
  })
})

describe('Remove from server on a person menu', () => {
  const show = (userId = 'U2') => {
    cmp = mount(MemberMenu, { target: document.body, props: { serverId: 'S1', userId, x: 10, y: 10, opener: opener(), onClose: vi.fn() } })
    flushSync()
  }

  it('the owner gets it on someone else, asked first', () => {
    state('U1')
    show()
    expect(ids().slice(-2)).toEqual(['remove-server', 'ban'])
    item('remove-server')!.click()
    flushSync()
    expect(ui.serverAction).toEqual({ kind: 'remove', serverId: 'S1', userId: 'U2' })
  })

  it('not on themselves, and nobody else gets it', () => {
    state('U1')
    show('U1')
    expect(ids()).not.toContain('remove-server')
    unmount(cmp!)
    state('U2')
    show('U2')
    expect(ids()).not.toContain('remove-server')
  })
})

describe('the confirm dialog', () => {
  it('Cancel does nothing; Remove removes them', async () => {
    state('U1')
    const onClose = vi.fn()
    cmp = mount(ServerActionDialog, { target: document.body, props: { action: { kind: 'remove', serverId: 'S1', userId: 'U2' }, onClose } })
    flushSync()
    const buttons = [...document.querySelectorAll<HTMLButtonElement>('[role=dialog] button')]
    expect(document.querySelector('[role=dialog]')!.getAttribute('aria-label')).toBe('Remove sam')
    expect(buttons.map((b) => b.textContent?.trim())).toEqual(['Cancel', 'Remove'])
    expect(buttons[1].classList.contains('btn-danger')).toBe(true)
    buttons[0].click()
    expect(invoke).not.toHaveBeenCalled()
    expect(onClose).toHaveBeenCalledTimes(1)
    buttons[1].click()
    await vi.waitFor(() => expect(onClose).toHaveBeenCalledTimes(2))
    expect(invoke).toHaveBeenCalledWith('remove_server_member', { serverId: 'S1', userId: 'U2' })
  })

  it('a refused leave shows the server\'s words and stays open', async () => {
    state('U2')
    invoke.mockRejectedValue({ kind: 'Forbidden', message: 'nope' })
    const onClose = vi.fn()
    cmp = mount(ServerActionDialog, { target: document.body, props: { action: { kind: 'leave', serverId: 'S1' }, onClose } })
    flushSync()
    document.querySelectorAll<HTMLButtonElement>('[role=dialog] button')[1].click()
    await vi.waitFor(() => expect(document.querySelector('[role=alert]')).not.toBeNull())
    expect(invoke).toHaveBeenCalledWith('leave_server', { serverId: 'S1' })
    expect(onClose).not.toHaveBeenCalled()
  })
})

describe('Server settings: Leave server', () => {
  const open = () => {
    const onClose = vi.fn()
    cmp = mount(ServerSettings, { target: document.body, props: { server: app.state.servers[0], onClose } })
    flushSync()
    const nav = [...document.querySelectorAll<HTMLButtonElement>('nav button')]
    nav.find((b) => b.textContent === 'Leave server')!.click()
    flushSync()
    return onClose
  }

  it('a member leaves from it, and settings close', async () => {
    state('U2')
    const onClose = open()
    const btn = [...document.querySelectorAll<HTMLButtonElement>('.content button')].find((b) => b.textContent === 'Leave server')!
    btn.click()
    await vi.waitFor(() => expect(onClose).toHaveBeenCalled())
    expect(invoke).toHaveBeenCalledWith('leave_server', { serverId: 'S1' })
  })

  it('the owner is told they can\'t, with no button', () => {
    state('U1')
    open()
    expect(document.querySelector('.content')!.textContent).toContain("you can't leave it")
    expect([...document.querySelectorAll('.content button')].some((b) => b.textContent === 'Leave server')).toBe(false)
  })
})

describe('handing a server over (#418)', () => {
  const openSection = (name: string) => {
    // live, as the shell passes it
    cmp = mount(ServerSettings, { target: document.body, props: { get server() { return app.state.servers[0] }, onClose: vi.fn() } })
    flushSync()
    const nav = [...document.querySelectorAll<HTMLButtonElement>('nav button')]
    const b = nav.find((x) => x.textContent === name)
    b?.click()
    flushSync()
    return b
  }

  it('Transfer ownership is the owner\'s only, in the danger colour', () => {
    state('U2')
    expect(openSection('Transfer ownership')).toBeUndefined()
    unmount(cmp!)
    state('U1')
    const b = openSection('Transfer ownership')!
    expect(b.classList.contains('danger')).toBe(true)
  })

  it('picking someone and Transfer ownership asks first in the shell', () => {
    state('U1')
    openSection('Transfer ownership')
    const go = [...document.querySelectorAll<HTMLButtonElement>('.content button')].find((b) => b.textContent === 'Transfer ownership')!
    expect(go.disabled).toBe(true)
    expect(go.classList.contains('btn-danger')).toBe(true)
    document.querySelector<HTMLButtonElement>('.content [aria-label="New owner"]')!.click()
    flushSync()
    const opt = [...document.querySelectorAll<HTMLElement>('[role=option]')].find((o) => o.textContent?.trim() === 'sam')!
    opt.dispatchEvent(new MouseEvent('mousedown', { bubbles: true }))
    opt.click()
    flushSync()
    expect(go.disabled).toBe(false)
    go.click()
    expect(ui.serverAction).toEqual({ kind: 'transfer', serverId: 'S1', userId: 'U2' })
    expect(invoke).not.toHaveBeenCalled()
  })

  it('the dialog wants the server\'s name before it hands it over', async () => {
    state('U1')
    const onClose = vi.fn()
    cmp = mount(ServerActionDialog, { target: document.body, props: { action: { kind: 'transfer', serverId: 'S1', userId: 'U2' }, onClose } })
    flushSync()
    expect(document.querySelector('[role=dialog]')!.getAttribute('aria-label')).toBe('Make sam the owner')
    const go = document.querySelectorAll<HTMLButtonElement>('[role=dialog] .actions button')[1]
    expect(go.textContent?.trim()).toBe('Transfer ownership')
    expect(go.classList.contains('btn-danger')).toBe(true)
    expect(go.disabled).toBe(true)
    const input = document.querySelector<HTMLInputElement>('[role=dialog] input')!
    input.value = 'Alph'
    input.dispatchEvent(new Event('input'))
    flushSync()
    expect(go.disabled).toBe(true)
    input.value = 'Alpha'
    input.dispatchEvent(new Event('input'))
    flushSync()
    go.click()
    await vi.waitFor(() => expect(onClose).toHaveBeenCalled())
    expect(invoke).toHaveBeenCalledWith('transfer_server', { serverId: 'S1', userId: 'U2' })
  })

  it('once it\'s handed over, settings move to Leave server', () => {
    state('U1')
    openSection('Transfer ownership')
    app.state = { ...app.state, servers: [{ ...app.state.servers[0], owner_id: 'U2' }] }
    flushSync()
    expect(document.querySelector('nav button.on')?.textContent).toBe('Leave server')
  })
})
