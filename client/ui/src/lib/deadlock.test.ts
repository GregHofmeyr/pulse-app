// Deadlock (#390): how ranks, matches and squad stats read, and the member list's rank reads.
import { beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import { BOT_WARNING, DEADLOCK_BOT_PAGE, deltaText, durationText, kdaText, rankText, stepsText, winRateText } from './deadlock'
import { RANKS_TTL_MS, loadDeadlockRanks, rankOf, resetDeadlockRanks } from './deadlock.svelte'

const rank = (tier: number, subrank: number, name: string) => ({ badge: tier * 10 + subrank, tier, subrank, name, image: null })

describe('deadlock text', () => {
  it('ranks, K/D/A, durations, deltas, win rates and steps', () => {
    expect(rankText(rank(8, 3, 'Oracle'))).toBe('Oracle 3')
    expect(rankText(rank(0, 0, 'Obscurus'))).toBe('Obscurus')
    expect(kdaText(12, 3, 8)).toBe('12 / 3 / 8')
    expect(durationText(1864)).toBe('31:04')
    expect(durationText(3729)).toBe('1:02:09')
    expect([deltaText(112), deltaText(-90), deltaText(0)]).toEqual(['+112', '−90', '±0'])
    expect(winRateText(6, 10)).toBe('6 of 10 won · 60 %')
    expect(winRateText(0, 0)).toBe('')
    expect([stepsText(2), stepsText(-3)]).toEqual(['▲ 2', '▼ 3'])
  })

  it("the bot link is deadlock-api's own site, with the copycat warning", () => {
    expect(new URL(DEADLOCK_BOT_PAGE).host).toBe('deadlock-api.com')
    expect(DEADLOCK_BOT_PAGE.startsWith('https://')).toBe(true)
    expect(BOT_WARNING).toContain('never asks for logins, codes or trades')
  })
})

describe('member list ranks', () => {
  beforeEach(() => {
    invoke.mockReset()
    resetDeadlockRanks()
  })

  it('reads a server once, again only after a while, and an error shows nothing', async () => {
    invoke.mockResolvedValue({ entries: [{ user_id: 'U1', rank: rank(8, 3, 'Oracle') }] })
    await loadDeadlockRanks('S1', 1000)
    expect(rankOf('S1', 'U1')?.name).toBe('Oracle')
    expect(rankOf('S1', 'U2')).toBeNull()
    await loadDeadlockRanks('S1', 2000)
    expect(invoke).toHaveBeenCalledTimes(1)
    await loadDeadlockRanks('S1', 1000 + RANKS_TTL_MS)
    expect(invoke).toHaveBeenCalledTimes(2)
    invoke.mockRejectedValue(new Error('older server'))
    await loadDeadlockRanks('S2', 1000)
    expect(rankOf('S2', 'U1')).toBeNull()
  })
})
