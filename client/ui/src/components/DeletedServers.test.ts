// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string, args?: never) => (cmd === 'ui_log' ? Promise.resolve() : invoke(cmd, args)),
}))

import DeletedServers from './DeletedServers.svelte'
import { app, clock } from '../lib/store.svelte'
import { windowFocus } from '../lib/windowFocus.svelte'
import { emptyState } from '../lib/state'
import type { DeletedServer } from '../lib/protocol/DeletedServer'

const DAY = 86_400_000
const gone = (id: string, name: string, o: Partial<DeletedServer> = {}): DeletedServer => ({
  server: { id, name, icon_hash: null, owner_id: 'U1' },
  deleted_at: new Date(Date.now() - 4 * DAY).toISOString(),
  purge_at: new Date(Date.now() + 3 * DAY - 60_000).toISOString(),
  can_restore: true,
  ...o,
})

let cmp: ReturnType<typeof mount> | undefined
const settle = () => new Promise((r) => setTimeout(r, 0))
const button = (label: string) => document.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`)
async function show(list: DeletedServer[]) {
  invoke.mockImplementation(async (cmd: string) => (cmd === 'list_deleted_servers' ? list : undefined))
  app.state = emptyState()
  cmp = mount(DeletedServers, { target: document.body })
  flushSync()
  await settle()
  flushSync()
}

beforeEach(() => invoke.mockReset())
afterEach(() => { if (cmp) unmount(cmp); cmp = undefined; document.body.innerHTML = '' })

describe('DeletedServers', () => {
  it('lists each deleted server with the days left', async () => {
    await show([gone('S1', 'Main'), gone('S2', 'Old', { purge_at: new Date(Date.now() + DAY - 60_000).toISOString() })])
    const rows = [...document.querySelectorAll('li')].map((li) => li.textContent ?? '')
    expect(rows).toHaveLength(2)
    expect(rows[0]).toContain('Main')
    expect(rows[0]).toContain('3 days left')
    expect(rows[1]).toContain('1 day left')
  })

  it('says when there are none', async () => {
    await show([])
    expect(document.body.textContent).toContain('No deleted servers.')
  })

  it('restores one, which leaves the list', async () => {
    await show([gone('S1', 'Main'), gone('S2', 'Old')])
    button('Restore Main')!.click()
    await settle()
    flushSync()
    expect(invoke).toHaveBeenCalledWith('restore_server', { serverId: 'S1' })
    expect([...document.querySelectorAll('li')].map((li) => li.textContent)).toEqual([expect.stringContaining('Old')])
  })

  it('offers no restore once it is being deleted for good', async () => {
    await show([gone('S1', 'Main', { can_restore: false, purge_at: new Date(Date.now() - 60_000).toISOString() })])
    expect(button('Restore Main')).toBeNull()
    expect(document.body.textContent).toContain('Being deleted for good')
  })

  it('shows a failed restore', async () => {
    await show([gone('S1', 'Main')])
    invoke.mockRejectedValueOnce('nope')
    button('Restore Main')!.click()
    await settle()
    flushSync()
    expect(document.querySelector('[role=alert]')).not.toBeNull()
  })

  // #287: a purge has no event (the server is already off everyone's rail), so the list asks again itself.
  const listCalls = () => invoke.mock.calls.filter((c) => c[0] === 'list_deleted_servers').length
  const tickClock = async () => {
    clock.nowIso = new Date(Date.parse(clock.nowIso) + 30_000).toISOString()
    flushSync()
    await settle()
    flushSync()
  }

  it('one being deleted for good leaves the list once its purge is done, with Settings open', async () => {
    await show([gone('S1', 'Main', { can_restore: false, purge_at: new Date(Date.now() - 60_000).toISOString() })])
    invoke.mockImplementation(async (cmd: string) => (cmd === 'list_deleted_servers' ? [] : undefined))
    await tickClock()
    expect(document.body.textContent).toContain('No deleted servers.')
  })

  it('one whose 7 days run out while open is asked about again (its Restore goes)', async () => {
    await show([gone('S1', 'Main', { purge_at: new Date(Date.parse(clock.nowIso) + 10_000).toISOString() })])
    expect(button('Restore Main')).not.toBeNull()
    invoke.mockImplementation(async (cmd: string) => (cmd === 'list_deleted_servers'
      ? [gone('S1', 'Main', { can_restore: false, purge_at: new Date(Date.now() - 1000).toISOString() })] : undefined))
    await tickClock()
    expect(button('Restore Main')).toBeNull()
  })

  it("doesn't ask again on every tick while nothing is due", async () => {
    await show([gone('S1', 'Main')])
    const n = listCalls()
    await tickClock()
    await tickClock()
    expect(listCalls()).toBe(n)
  })

  it('asks again when the window comes back into focus', async () => {
    await show([gone('S1', 'Main')])
    const n = listCalls()
    invoke.mockImplementation(async (cmd: string) => (cmd === 'list_deleted_servers' ? [] : undefined))
    windowFocus.focused = false
    flushSync()
    await settle()
    expect(listCalls()).toBe(n)
    windowFocus.focused = true
    flushSync()
    await settle()
    flushSync()
    expect(listCalls()).toBe(n + 1)
    expect(document.body.textContent).toContain('No deleted servers.')
  })
})
