// @vitest-environment jsdom
// Settings › Chat › F1 countdown strip (#456): the strip's own switch, and the way back for a weekend hidden from the
// strip's right-click menu.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => undefined) }))

import ChatSettings from './ChatSettings.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import { f1Prefs, hideF1Weekend, setF1Strip, showF1WeekendAgain } from '../lib/f1Prefs.svelte'

let cmp: ReturnType<typeof mount> | undefined
const field = () => [...document.querySelectorAll<HTMLElement>('.settings-field')].find((f) => f.textContent!.includes('F1 COUNTDOWN STRIP'))!
const pick = (label: string) => [...field().querySelectorAll<HTMLButtonElement>('button[role=radio]')].find((b) => b.textContent === label)!

beforeEach(() => {
  setF1Strip(true)
  showF1WeekendAgain()
  app.state = emptyState()
  cmp = mount(ChatSettings, { target: document.body })
  flushSync()
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('F1 countdown strip setting', () => {
  it('on by default; Off turns the strip off on this device', () => {
    expect(pick('On').getAttribute('aria-checked')).toBe('true')
    expect(field().textContent).toContain('This device only.')
    pick('Off').click()
    flushSync()
    expect(f1Prefs.strip).toBe(false)
    expect(localStorage.getItem('pulse.f1.strip')).toBe('off')
    pick('On').click()
    flushSync()
    expect(f1Prefs.strip).toBe(true)
  })

  it('a weekend hidden from the strip says so, with Show it again', () => {
    expect(field().textContent).not.toContain('Hidden for')
    hideF1Weekend({ season: 2030, round: 2, race_name: 'Harbour Grand Prix' })
    flushSync()
    expect(field().textContent).toContain('Hidden for the Harbour Grand Prix weekend.')
    const again = [...field().querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent === 'Show it again')!
    again.click()
    flushSync()
    expect(f1Prefs.hidden).toBeNull()
    expect(localStorage.getItem('pulse.f1.hiddenWeekend')).toBeNull()
    expect(field().textContent).not.toContain('Hidden for')
  })
})
