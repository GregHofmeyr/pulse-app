// @vitest-environment jsdom
// The F1 countdown banner (#386): only in the server's F1 channel, with the next race weekend, the countdown, the next
// session and the season's leaders; every session on a click; hidden with "F1 race weekends" off.
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import F1Banner from './F1Banner.svelte'
import { f1Prefs } from '../lib/f1Prefs.svelte'
import { resetF1 } from '../lib/f1.svelte'

const NOW = Date.parse('2030-03-12T07:00:00.000Z')
const STATUS = {
  channel_id: 'F1',
  next: {
    season: 2030, round: 2, race_name: 'Harbour Grand Prix', circuit: 'Harbour Street Circuit', locality: 'Port Town', country: 'UK',
    race_at: '2030-03-15T07:00:00.000Z',
    sessions: [{ kind: 'practice1', starts_at: '2030-03-13T03:30:00.000Z' }, { kind: 'qualifying', starts_at: '2030-03-14T07:00:00.000Z' }, { kind: 'race', starts_at: '2030-03-15T07:00:00.000Z' }],
  },
  season_table: [{ user_id: 'U2', points: 12, races: 2 }, { user_id: 'U3', points: 4, races: 2 }],
}
const names: Record<string, string> = { U2: 'sam', U3: 'kim' }

let cmp: ReturnType<typeof mount> | undefined
async function show(channelId: string) {
  cmp = mount(F1Banner, { target: document.body, props: { serverId: 'S1', channelId, nameOf: (id: string | null) => names[id ?? ''] ?? '' } })
  for (let i = 0; i < 5; i++) await Promise.resolve()
  await tick()
  flushSync()
}

beforeEach(() => {
  vi.useFakeTimers()
  vi.setSystemTime(NOW)
  resetF1()
  f1Prefs.on = true
  invoke.mockReset()
  invoke.mockImplementation(async (cmd: string) => (cmd === 'f1_status' ? STATUS : undefined))
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
  vi.useRealTimers()
})

describe('F1Banner', () => {
  it('counts down to lights out in the F1 channel', async () => {
    await show('F1')
    expect(invoke).toHaveBeenCalledWith('f1_status', { serverId: 'S1' })
    const b = document.querySelector('.f1-banner')!
    expect(b.querySelector('.title')!.textContent).toBe('Harbour Grand Prix')
    expect(b.querySelector('.count strong')!.textContent).toBe('3d 0h')
    expect(b.querySelector('.next')!.textContent).toContain('Next: Practice 1')
    expect(b.querySelector('.leaders')!.textContent).toBe('Predictions: sam 12 pts · kim 4 pts')
    b.querySelector<HTMLButtonElement>('.row')!.click()
    flushSync()
    expect(b.querySelectorAll('.sessions li')).toHaveLength(3)
  })

  it('is nowhere else, and not with the setting off', async () => {
    await show('GENERAL')
    expect(document.querySelector('.f1-banner')).toBeNull()
    unmount(cmp!)
    f1Prefs.on = false
    await show('F1')
    expect(document.querySelector('.f1-banner')).toBeNull()
  })
})
