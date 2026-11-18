// @vitest-environment jsdom
// Settings › Sports (#385): not set up without the server's key; the pings switch; following from a league's team
// search (each league's list asked for once) and unfollowing.
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import SportsSettings, { clearTeamCache } from './SportsSettings.svelte'
import { chooseOption, selectTrigger } from './selectHarness'
import type { SportsStatus } from '../lib/protocol/SportsStatus'

let cmp: ReturnType<typeof mount> | undefined
const settle = async () => {
  for (let i = 0; i < 6; i++) await tick()
  flushSync()
}
const coastal = { league: 'psl' as const, team_id: 10, name: 'Coastal City' }
const highveld = { league: 'psl' as const, team_id: 20, name: 'Highveld United' }
let status: SportsStatus

beforeEach(() => {
  clearTeamCache()
  status = { available: true, pings: true, follows: [], leagues: ['psl', 'urc', 'epl'], note: null }
  invoke.mockReset()
  invoke.mockImplementation(async (cmd: string, args?: unknown) => {
    const a = args as { on?: boolean; league?: string; teamId?: number }
    if (cmd === 'sports_status') return status
    if (cmd === 'sports_teams') return { teams: a.league === 'psl' ? [coastal, highveld] : [{ league: 'urc', team_id: 502, name: 'Highveld XV' }] }
    if (cmd === 'set_sports_pings') return (status = { ...status, pings: !!a.on })
    if (cmd === 'follow_team') return (status = { ...status, follows: [...status.follows, [coastal, highveld].find((t) => t.team_id === a.teamId)!] })
    if (cmd === 'unfollow_team') return (status = { ...status, follows: status.follows.filter((t) => t.team_id !== a.teamId) })
    return undefined
  })
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

async function open() {
  cmp = mount(SportsSettings, { target: document.body })
  await settle()
}
const byLabel = (l: string) => document.querySelector<HTMLElement>(`[aria-label="${l}"]`)

describe('SportsSettings', () => {
  it("says sports aren't set up when the server has no key, and offers nothing to follow", async () => {
    status = { ...status, available: false }
    await open()
    expect(document.querySelector('[data-not-set-up]')).not.toBeNull()
    expect(document.querySelector('input')).toBeNull()
    expect(invoke).not.toHaveBeenCalledWith('sports_teams', expect.anything())
  })

  it('the pings switch is on by default and turns off', async () => {
    await open()
    const sw = document.querySelector<HTMLInputElement>('[data-switch=sports-pings]')!
    expect(sw.checked).toBe(true)
    sw.click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('set_sports_pings', { on: false })
    expect(document.querySelector<HTMLInputElement>('[data-switch=sports-pings]')!.checked).toBe(false)
  })

  it('searches a league, follows and unfollows; each list is asked for once', async () => {
    await open()
    expect(invoke).toHaveBeenCalledWith('sports_teams', { league: 'psl' })
    const search = byLabel('Search teams') as HTMLInputElement
    search.value = 'coast'
    search.dispatchEvent(new Event('input', { bubbles: true }))
    await settle()
    const rows = [...document.querySelectorAll('[aria-label=Teams] li')].map((l) => l.textContent?.trim())
    expect(rows).toEqual(['Coastal City Follow'])
    byLabel('Follow Coastal City')!.click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('follow_team', { league: 'psl', teamId: 10 })
    expect(document.body.textContent).toContain('Following')
    expect(byLabel('Unfollow Coastal City')).not.toBeNull()
    // another league, then back: the PSL list isn't asked for again
    const league = selectTrigger('League')!
    chooseOption(league, 'urc')
    await settle()
    chooseOption(league, 'psl')
    await settle()
    expect(invoke.mock.calls.filter(([c, a]) => c === 'sports_teams' && (a as { league: string }).league === 'psl')).toHaveLength(1)
    byLabel('Unfollow Coastal City')!.click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('unfollow_team', { league: 'psl', teamId: 10 })
    expect(document.body.textContent).toContain('None yet.')
  })

  it("shows the server's note while scores are paused", async () => {
    status = { ...status, note: "Today's sports requests are used up: scores resume tomorrow." }
    await open()
    expect(document.querySelector('[role=status]')!.textContent).toContain('used up')
  })
})
