// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string, args?: never) => (cmd === 'ui_log' ? Promise.resolve() : invoke(cmd, args)),
}))

import DeleteServer from './DeleteServer.svelte'
import ServerSettings from './ServerSettings.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'

const me = { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const server = { id: 'S1', name: 'Main Hall', icon_hash: null, owner_id: 'U1' }

let cmp: ReturnType<typeof mount> | undefined
const settle = () => new Promise((r) => setTimeout(r, 0))
const deleteButton = () => [...document.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent === 'Delete server')!
function type(value: string) {
  const input = document.querySelector<HTMLInputElement>('input[aria-label="Server name"]')!
  input.value = value
  input.dispatchEvent(new Event('input', { bubbles: true }))
  flushSync()
}

beforeEach(() => {
  invoke.mockReset()
  app.state = { ...emptyState(), me }
})
afterEach(() => { if (cmp) unmount(cmp); cmp = undefined; document.body.innerHTML = '' })

describe('DeleteServer', () => {
  it('stays locked until the name is typed as it is', () => {
    cmp = mount(DeleteServer, { target: document.body, props: { server, onDeleted: vi.fn() } })
    flushSync()
    expect(deleteButton().disabled).toBe(true)
    type('main hall')
    expect(deleteButton().disabled).toBe(true)
    type('Main Hall ')
    expect(deleteButton().disabled).toBe(false)
  })

  it('deletes with the typed name, then closes', async () => {
    const onDeleted = vi.fn()
    cmp = mount(DeleteServer, { target: document.body, props: { server, onDeleted } })
    flushSync()
    type('Main Hall')
    deleteButton().click()
    await settle()
    flushSync()
    expect(invoke).toHaveBeenCalledWith('delete_server', { serverId: 'S1', name: 'Main Hall' })
    expect(onDeleted).toHaveBeenCalledTimes(1)
  })

  it('says why when the server refuses, and stays open', async () => {
    invoke.mockRejectedValueOnce("Type the server's name to delete it.")
    const onDeleted = vi.fn()
    cmp = mount(DeleteServer, { target: document.body, props: { server, onDeleted } })
    flushSync()
    type('Main Hall')
    deleteButton().click()
    await settle()
    flushSync()
    expect(document.querySelector('[role=alert]')).not.toBeNull()
    expect(onDeleted).not.toHaveBeenCalled()
  })
})

describe('Server settings → Delete server', () => {
  const sectionButton = () => [...document.querySelectorAll<HTMLButtonElement>('nav button')].find((b) => b.textContent === 'Delete server')
  it('is there for the owner only', () => {
    cmp = mount(ServerSettings, { target: document.body, props: { server, onClose: vi.fn() } })
    flushSync()
    expect(sectionButton()).toBeDefined()
    unmount(cmp)
    cmp = mount(ServerSettings, { target: document.body, props: { server: { ...server, owner_id: 'U2' }, onClose: vi.fn() } })
    flushSync()
    expect(sectionButton()).toBeUndefined()
    unmount(cmp)
    // an older server that doesn't say who owns it
    cmp = mount(ServerSettings, { target: document.body, props: { server: { id: 'S1', name: 'Main Hall', icon_hash: null }, onClose: vi.fn() } })
    flushSync()
    expect(sectionButton()).toBeUndefined()
  })
})
