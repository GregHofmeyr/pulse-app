// @vitest-environment jsdom
// Server settings › F1 (#386): the owner picks one of the server's text channels or none (the default); everyone else
// reads the choice.
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import F1ChannelSettings from './F1ChannelSettings.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import { f1Status, resetF1 } from '../lib/f1.svelte'
import type { Channel } from '../lib/protocol/Channel'

let cmp: ReturnType<typeof mount> | undefined
const ch = (id: string, kind: Channel['kind'], position: number, server_id = 'S1') => ({ id, server_id, kind, name: id.toLowerCase(), position }) as unknown as Channel
const status = (channel_id: string | null, picks_close = 'lights_out') => ({ channel_id, next: null, season_table: [], picks_close })
const settle = async () => {
  for (let i = 0; i < 4; i++) await tick()
  flushSync()
}

beforeEach(() => {
  resetF1()
  invoke.mockReset()
  invoke.mockImplementation(async (cmd: string, args?: unknown) => {
    if (cmd === 'f1_status') return status(null)
    if (cmd === 'set_f1_channel') {
      const a = args as { channelId: string | null; picksClose: string | null }
      return status(a.channelId, a.picksClose ?? 'lights_out')
    }
    return undefined
  })
  app.state = { ...emptyState(), me: { id: 'U1', username: 'alex', avatar_hash: null } as never, channels: { GENERAL: ch('GENERAL', 'text', 0), LOUNGE: ch('LOUNGE', 'voice', 1), OTHER: ch('OTHER', 'text', 0, 'S2'), PADDOCK: ch('PADDOCK', 'text', 3) } }
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

function open(owner: string) {
  cmp = mount(F1ChannelSettings, { target: document.body, props: { server: { id: 'S1', name: 'Paddock', icon_hash: null, owner_id: owner } as never } })
  flushSync()
}

describe('F1ChannelSettings', () => {
  it("the owner picks among this server's text channels; none by default", async () => {
    open('U1')
    await settle()
    const select = document.querySelector<HTMLSelectElement>('select')!
    expect([...select.options].map((o) => o.textContent)).toEqual(['None (no F1 cards)', '#general', '#paddock'])
    expect(select.value).toBe('')
    select.value = 'PADDOCK'
    select.dispatchEvent(new Event('change', { bubbles: true }))
    await settle()
    expect(invoke).toHaveBeenCalledWith('set_f1_channel', { serverId: 'S1', channelId: 'PADDOCK', picksClose: null })
    // the banner's copy follows at once
    expect(f1Status.byServer.S1.channel_id).toBe('PADDOCK')
    select.value = ''
    select.dispatchEvent(new Event('change', { bubbles: true }))
    await settle()
    expect(invoke).toHaveBeenLastCalledWith('set_f1_channel', { serverId: 'S1', channelId: null, picksClose: null })
  })

  it('with a channel, the owner picks when predictions close (#403): lights out by default, or qualifying', async () => {
    invoke.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === 'f1_status') return status('GENERAL')
      if (cmd === 'set_f1_channel') return status('GENERAL', (args as { picksClose: string | null }).picksClose ?? 'lights_out')
      return undefined
    })
    open('U1')
    await settle()
    const close = document.querySelector<HTMLSelectElement>('select[aria-label="Predictions close"]')!
    expect([...close.options].map((o) => o.textContent)).toEqual(['At lights out', 'When qualifying starts'])
    expect(close.value).toBe('lights_out')
    close.value = 'qualifying'
    close.dispatchEvent(new Event('change', { bubbles: true }))
    await settle()
    expect(invoke).toHaveBeenLastCalledWith('set_f1_channel', { serverId: 'S1', channelId: 'GENERAL', picksClose: 'qualifying' })
    expect(close.value).toBe('qualifying')
    expect(f1Status.byServer.S1.picks_close).toBe('qualifying')
  })

  it('with no channel there is no predictions choice', async () => {
    open('U1')
    await settle()
    expect(document.querySelector('select[aria-label="Predictions close"]')).toBeNull()
  })

  it('anyone else reads the choice', async () => {
    invoke.mockImplementation(async (cmd: string) => (cmd === 'f1_status' ? status('GENERAL') : undefined))
    open('U9')
    await settle()
    expect(document.querySelector('select')).toBeNull()
    expect(document.body.textContent).toContain('F1 cards go to #general. Predictions close at lights out.')
  })
})
