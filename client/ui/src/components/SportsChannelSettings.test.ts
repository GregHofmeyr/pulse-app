// @vitest-environment jsdom
// Server settings › Sports (#385): the owner picks one of the server's text channels or none (the default); everyone
// else reads the choice.
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import SportsChannelSettings from './SportsChannelSettings.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import type { Channel } from '../lib/protocol/Channel'

let cmp: ReturnType<typeof mount> | undefined
const ch = (id: string, kind: Channel['kind'], position: number, server_id = 'S1') => ({ id, server_id, kind, name: id.toLowerCase(), position }) as unknown as Channel
const settle = async () => {
  for (let i = 0; i < 4; i++) await tick()
  flushSync()
}

beforeEach(() => {
  invoke.mockReset()
  invoke.mockImplementation(async (cmd: string, args?: unknown) => {
    if (cmd === 'sports_channel') return { channel_id: null, leagues: null }
    if (cmd === 'set_sports_channel') return { channel_id: (args as { channelId: string | null }).channelId, leagues: null }
    return undefined
  })
  app.state = { ...emptyState(), me: { id: 'U1', username: 'alex', avatar_hash: null } as never, channels: { GENERAL: ch('GENERAL', 'text', 0), LOUNGE: ch('LOUNGE', 'voice', 1), OTHER: ch('OTHER', 'text', 0, 'S2'), SPORT: ch('SPORT', 'text', 2) } }
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

function open(owner: string) {
  cmp = mount(SportsChannelSettings, { target: document.body, props: { server: { id: 'S1', name: 'Braai crew', icon_hash: null, owner_id: owner } as never } })
  flushSync()
}

describe('SportsChannelSettings', () => {
  it("the owner picks among this server's text channels; none by default", async () => {
    open('U1')
    await settle()
    const select = document.querySelector<HTMLSelectElement>('select')!
    expect([...select.options].map((o) => o.textContent)).toEqual(['None (no cards)', '#general', '#sport'])
    expect(select.value).toBe('')
    select.value = 'SPORT'
    select.dispatchEvent(new Event('change', { bubbles: true }))
    await settle()
    expect(invoke).toHaveBeenCalledWith('set_sports_channel', { serverId: 'S1', channelId: 'SPORT', leagues: null })
    select.value = ''
    select.dispatchEvent(new Event('change', { bubbles: true }))
    await settle()
    expect(invoke).toHaveBeenLastCalledWith('set_sports_channel', { serverId: 'S1', channelId: null, leagues: null })
  })

  it('anyone else reads the choice', async () => {
    invoke.mockImplementation(async (cmd: string) => (cmd === 'sports_channel' ? { channel_id: 'GENERAL', leagues: null } : undefined))
    open('U9')
    await settle()
    expect(document.querySelector('select')).toBeNull()
    expect(document.body.textContent).toContain('Cards go to #general.')
    expect(document.querySelector('[data-league]')).toBeNull()
  })

  it('with a channel the owner limits the leagues (#406): every league by default, never none', async () => {
    let leagues: string[] | null = null
    invoke.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === 'sports_channel') return { channel_id: 'GENERAL', leagues }
      if (cmd === 'set_sports_channel') {
        const pick = (args as { leagues: string[] | null }).leagues
        leagues = pick && pick.length < 3 ? pick : null
        return { channel_id: 'GENERAL', leagues }
      }
      return undefined
    })
    open('U1')
    await settle()
    const box = (l: string) => document.querySelector<HTMLInputElement>(`[data-league="${l}"]`)!
    expect(['psl', 'urc', 'epl'].map((l) => box(l).checked)).toEqual([true, true, true])
    box('psl').click()
    await settle()
    expect(invoke).toHaveBeenLastCalledWith('set_sports_channel', { serverId: 'S1', channelId: 'GENERAL', leagues: ['urc', 'epl'] })
    expect(box('psl').checked).toBe(false)
    box('epl').click()
    await settle()
    expect(invoke).toHaveBeenLastCalledWith('set_sports_channel', { serverId: 'S1', channelId: 'GENERAL', leagues: ['urc'] })
    // the last one on stays on
    expect(box('urc').disabled).toBe(true)
    box('psl').click()
    await settle()
    expect(invoke).toHaveBeenLastCalledWith('set_sports_channel', { serverId: 'S1', channelId: 'GENERAL', leagues: ['psl', 'urc'] })
  })

  it('anyone else reads a league limit', async () => {
    invoke.mockImplementation(async (cmd: string) => (cmd === 'sports_channel' ? { channel_id: 'GENERAL', leagues: ['urc'] } : undefined))
    open('U9')
    await settle()
    expect(document.body.textContent).toContain('Cards go to #general. Only United Rugby Championship.')
  })
})
