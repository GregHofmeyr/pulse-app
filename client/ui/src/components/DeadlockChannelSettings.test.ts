// @vitest-environment jsdom
// Server settings › Deadlock (#390): the owner picks one of the server's text channels or none (the default) and,
// once there is one, which kinds of card post (all on); everyone else reads the choice.
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import DeadlockChannelSettings from './DeadlockChannelSettings.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import type { Channel } from '../lib/protocol/Channel'

let cmp: ReturnType<typeof mount> | undefined
const ch = (id: string, kind: Channel['kind'], position: number, server_id = 'S1') => ({ id, server_id, kind, name: id.toLowerCase(), position }) as unknown as Channel
const settle = async () => {
  for (let i = 0; i < 4; i++) await tick()
  flushSync()
}
let stored = { channel_id: null as string | null, match_cards: true, rank_movers: true, patch_notes: true }

beforeEach(() => {
  stored = { channel_id: null, match_cards: true, rank_movers: true, patch_notes: true }
  invoke.mockReset()
  invoke.mockImplementation(async (cmd: string, args?: unknown) => {
    if (cmd === 'deadlock_channel') return { ...stored }
    if (cmd === 'set_deadlock_channel') {
      const r = (args as { request: Partial<typeof stored> }).request
      stored = { ...stored, ...r }
      return { ...stored }
    }
    return undefined
  })
  app.state = { ...emptyState(), me: { id: 'U1', username: 'alex', avatar_hash: null } as never, channels: { GENERAL: ch('GENERAL', 'text', 0), LOUNGE: ch('LOUNGE', 'voice', 1), OTHER: ch('OTHER', 'text', 0, 'S2') } }
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

function open(owner: string) {
  cmp = mount(DeadlockChannelSettings, { target: document.body, props: { server: { id: 'S1', name: 'Squad', icon_hash: null, owner_id: owner } as never } })
  flushSync()
}

describe('DeadlockChannelSettings', () => {
  it('none by default; picking a channel shows the kinds, each switchable', async () => {
    open('U1')
    await settle()
    const select = document.querySelector<HTMLSelectElement>('select')!
    expect([...select.options].map((o) => o.textContent)).toEqual(['None (no cards)', '#general'])
    expect(select.value).toBe('')
    expect(document.querySelector('[data-kind]')).toBeNull()
    select.value = 'GENERAL'
    select.dispatchEvent(new Event('change', { bubbles: true }))
    await settle()
    expect(invoke).toHaveBeenCalledWith('set_deadlock_channel', { serverId: 'S1', request: { channel_id: 'GENERAL' } })
    const patch = document.querySelector<HTMLInputElement>('[data-kind="patch_notes"]')!
    expect(patch.checked).toBe(true)
    patch.click()
    await settle()
    expect(invoke).toHaveBeenLastCalledWith('set_deadlock_channel', { serverId: 'S1', request: { patch_notes: false, channel_id: 'GENERAL' } })
    expect(document.querySelector<HTMLInputElement>('[data-kind="patch_notes"]')!.checked).toBe(false)
  })

  it('anyone else reads the choice', async () => {
    stored.channel_id = 'GENERAL'
    open('U9')
    await settle()
    expect(document.querySelector('select')).toBeNull()
    expect(document.body.textContent).toContain('Cards go to #general.')
  })
})
