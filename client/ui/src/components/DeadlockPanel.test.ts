// @vitest-environment jsdom
// A member's Deadlock section on their profile card (#390): rank, last match, heroes, games with you and best duo;
// nothing at all when the server has nothing (or can't be asked).
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import DeadlockPanel from './DeadlockPanel.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import { settle } from '../lib/testSettle'

const HASH = 'ef'.repeat(32)
let cmp: ReturnType<typeof mount> | undefined
beforeEach(() => {
  invoke.mockReset()
  app.state = { ...emptyState(), people: { U3: { user: { id: 'U3', username: 'kim', display_name: '', avatar_hash: null } } } as never }
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})
const open = () => {
  cmp = mount(DeadlockPanel, { target: document.body, props: { userId: 'U1', serverId: 'S1' } })
  flushSync()
}

describe('DeadlockPanel', () => {
  it('shows what the member shares', async () => {
    invoke.mockResolvedValue({
      rank: { badge: 83, tier: 8, subrank: 3, name: 'Oracle', image: { hash: HASH, width: 32, height: 32 } },
      last_match: { hero: 'Infernus', hero_icon: null, kills: 12, deaths: 3, assists: 8, won: true, started_at: '2026-11-16T18:00:00Z' },
      heroes: [{ hero: 'Infernus', icon: null, matches: 40, wins: 22 }],
      with_you: { user_id: 'U2', matches: 10, wins: 6 },
      best_duo: { user_id: 'U3', matches: 5, wins: 4 },
    })
    open()
    await settle(4)
    expect(invoke).toHaveBeenCalledWith('deadlock_profile', { serverId: 'S1', userId: 'U1' })
    expect(document.querySelector('[data-deadlock-rank]')!.textContent).toContain('Oracle 3')
    expect(document.querySelector('[data-deadlock-last]')!.textContent).toContain('Infernus · 12 / 3 / 8 · Win')
    expect(document.querySelector('[data-deadlock-heroes]')!.textContent).toContain('Infernus')
    expect(document.querySelector('[data-deadlock-with-you]')!.textContent).toContain('6 of 10 won · 60 %')
    expect(document.querySelector('[data-deadlock-duo]')!.textContent).toContain('kim · 4 of 5 won · 80 %')
  })

  it('nothing to show, or no answer: no section', async () => {
    invoke.mockResolvedValue({ rank: null, last_match: null, heroes: [], with_you: null, best_duo: null })
    open()
    await settle(4)
    expect(document.querySelector('[data-deadlock-panel]')).toBeNull()
    unmount(cmp!)
    invoke.mockRejectedValue(new Error('older server'))
    open()
    await settle(4)
    expect(document.querySelector('[data-deadlock-panel]')).toBeNull()
  })
})
