// @vitest-environment jsdom
// The F1 countdown banner (#386): only in the server's F1 channel, with the next race weekend, the countdown, the next
// session and the season's leaders; every session on a click; hidden with "F1 race weekends" off.
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import F1Banner from './F1Banner.svelte'
import { f1Prefs, setF1Strip, showF1WeekendAgain } from '../lib/f1Prefs.svelte'
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
  setF1Strip(true)
  showF1WeekendAgain()
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

  describe('right-click menu (#456)', () => {
    const menu = () => document.querySelector<HTMLElement>('[role=menu][aria-label="F1 strip"]')
    const item = (t: string) => [...document.querySelectorAll<HTMLButtonElement>('[role=menuitem]')].find((b) => b.textContent!.trim() === t)!
    function rightClick() {
      document.querySelector('.f1-banner')!.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 40, clientY: 50 }))
      flushSync()
    }

    it('opens on right-click with Hide for this weekend and Turn off F1 strip', async () => {
      await show('F1')
      rightClick()
      expect(menu()).not.toBeNull()
      expect([...menu()!.querySelectorAll('[role=menuitem]')].map((b) => b.textContent!.trim())).toEqual(['Hide for this weekend', 'Turn off F1 strip'])
      expect(document.activeElement).toBe(item('Hide for this weekend'))
    })

    it('Shift+F10 on the strip opens it too; Escape closes it and focus goes back', async () => {
      await show('F1')
      const row = document.querySelector<HTMLButtonElement>('.f1-banner .row')!
      row.dispatchEvent(new KeyboardEvent('keydown', { key: 'F10', shiftKey: true, bubbles: true, cancelable: true }))
      flushSync()
      expect(menu()).not.toBeNull()
      menu()!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))
      flushSync()
      expect(menu()).toBeNull()
      expect(document.activeElement).toBe(row)
    })

    it('Hide for this weekend hides it on this device until the next race weekend', async () => {
      await show('F1')
      rightClick()
      item('Hide for this weekend').click()
      flushSync()
      expect(document.querySelector('.f1-banner')).toBeNull()
      expect(menu()).toBeNull()
      expect(f1Prefs.hidden).toEqual({ key: '2030-2', name: 'Harbour Grand Prix' })
      expect(JSON.parse(localStorage.getItem('pulse.f1.hiddenWeekend')!)).toEqual({ key: '2030-2', name: 'Harbour Grand Prix' })
      // the next weekend (round 3) shows again by itself
      unmount(cmp!)
      resetF1()
      invoke.mockImplementation(async (cmd: string) => (cmd === 'f1_status' ? { ...STATUS, next: { ...STATUS.next, round: 3, race_name: 'Desert Grand Prix' } } : undefined))
      await show('F1')
      expect(document.querySelector('.f1-banner .title')!.textContent).toBe('Desert Grand Prix')
    })

    it('Turn off F1 strip flips its own switch (F1 cards untouched); Settings brings it back', async () => {
      await show('F1')
      rightClick()
      item('Turn off F1 strip').click()
      flushSync()
      expect(document.querySelector('.f1-banner')).toBeNull()
      expect(f1Prefs.strip).toBe(false)
      expect(f1Prefs.on).toBe(true)
      expect(localStorage.getItem('pulse.f1.strip')).toBe('off')
      setF1Strip(true)
      flushSync()
      expect(document.querySelector('.f1-banner')).not.toBeNull()
    })
  })
})
