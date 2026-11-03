// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string, args?: never) => (cmd === 'ui_log' ? Promise.resolve() : invoke(cmd, args)),
}))

import RecentlyDeleted from './RecentlyDeleted.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import type { DeletedChannel } from '../lib/protocol/DeletedChannel'
import type { ChannelKind } from '../lib/protocol/ChannelKind'

const server = { id: 'S1', name: 'Home', icon_hash: null }
const DAY = 86_400_000
const user = (id: string, username: string) => ({ user: { id, username, display_name: username, avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }, nickname: null })
const gone = (id: string, name: string, kind: ChannelKind, o: Partial<DeletedChannel> = {}): DeletedChannel => ({
  channel: { id, server_id: 'S1', kind, name, position: 3 },
  deleted_by: 'U2',
  deleted_at: new Date(Date.now() - 2 * DAY).toISOString(),
  purge_at: new Date(Date.now() + 5 * DAY - 60_000).toISOString(),
  can_restore: true,
  can_purge: false,
  held: null,
  ...o,
})

let cmp: ReturnType<typeof mount> | undefined
const settle = () => new Promise((r) => setTimeout(r, 0))
const button = (label: string) => document.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`)
const text = (t: string) => [...document.querySelectorAll('button')].find((b) => b.textContent === t)
async function show(list: DeletedChannel[]) {
  invoke.mockImplementation(async (cmd: string) => (cmd === 'list_deleted_channels' ? list : undefined))
  app.state = { ...emptyState(), members: { S1: [user('U1', 'alex'), user('U2', 'sam')] } }
  cmp = mount(RecentlyDeleted, { target: document.body, props: { server } })
  flushSync()
  await settle()
  flushSync()
}

beforeEach(() => invoke.mockReset())
afterEach(() => { if (cmp) unmount(cmp); cmp = undefined; document.body.innerHTML = '' })

describe('RecentlyDeleted', () => {
  it('lists each channel with its kind, who deleted it, when and the days left', async () => {
    await show([gone('T1', 'old-stuff', 'text'), gone('V1', 'Lounge 2', 'voice', { deleted_by: 'U1' })])
    expect(invoke).toHaveBeenCalledWith('list_deleted_channels', { serverId: 'S1' })
    const rows = [...document.querySelectorAll('li')].map((li) => li.textContent ?? '')
    expect(rows).toHaveLength(2)
    expect(rows[0]).toContain('#old-stuff')
    expect(rows[0]).toContain('Text')
    expect(rows[0]).toContain('Deleted by sam')
    expect(rows[0]).toContain('5 days left')
    expect(rows[1]).toContain('Lounge 2')
    expect(rows[1]).toContain('Voice')
    expect(rows[1]).toContain('Deleted by alex')
  })

  it('says when there is nothing in it', async () => {
    await show([])
    expect(document.body.textContent).toContain('No deleted channels')
  })

  it('offers Restore and Delete now only to whoever may', async () => {
    await show([gone('T1', 'mine', 'text'), gone('T2', 'theirs', 'text', { can_restore: false }), gone('T3', 'owned', 'text', { can_purge: true })])
    expect(button('Restore #mine')).not.toBeNull()
    expect(button('Delete #mine now')).toBeNull()
    expect(button('Restore #theirs')).toBeNull()
    expect(button('Delete #owned now')).not.toBeNull()
  })

  it('restores and drops the row', async () => {
    await show([gone('T1', 'old-stuff', 'text')])
    button('Restore #old-stuff')!.click()
    await settle()
    flushSync()
    expect(invoke).toHaveBeenCalledWith('restore_channel', { channelId: 'T1' })
    expect(document.querySelectorAll('li')).toHaveLength(0)
  })

  it('deletes for good only once confirmed', async () => {
    await show([gone('T1', 'old-stuff', 'text', { can_purge: true })])
    button('Delete #old-stuff now')!.click()
    flushSync()
    expect(invoke).not.toHaveBeenCalledWith('purge_channel', expect.anything())
    expect(document.body.textContent).toContain("Delete #old-stuff for good? Everything in it goes, and it can't be undone.")
    text('Cancel')!.click()
    flushSync()
    expect(invoke).not.toHaveBeenCalledWith('purge_channel', expect.anything())
    button('Delete #old-stuff now')!.click()
    flushSync()
    text('Delete for good')!.click()
    await settle()
    flushSync()
    expect(invoke).toHaveBeenCalledWith('purge_channel', { channelId: 'T1' })
    expect(document.querySelectorAll('li')).toHaveLength(0)
  })

  it('a refusal stays and says why', async () => {
    await show([gone('T1', 'quotes', 'quotes', { can_purge: true })])
    invoke.mockRejectedValueOnce("It holds a Quote of the Year winner, so it can't be deleted for good.")
    button('Delete #quotes now')!.click()
    flushSync()
    text('Delete for good')!.click()
    await settle()
    flushSync()
    expect(document.querySelector('[role=alert]')?.textContent).toContain('Quote of the Year')
    expect(document.querySelectorAll('li')).toHaveLength(1)
  })

  it('a held channel says why instead of its days left', async () => {
    await show([gone('Q1', 'quotes', 'quotes', { held: 'It holds a Quote of the Year winner, so it can\'t be deleted for good.' })])
    const row = document.querySelector('li')!.textContent!
    expect(row).toContain('It holds a Quote of the Year winner')
    expect(row).not.toContain('days left')
  })
})
