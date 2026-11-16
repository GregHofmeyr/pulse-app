// @vitest-environment jsdom
// Settings → Privacy → Game activity (#380): the master switch and the finer ones (on by default, the finer ones
// disabled while the master is off), each change saved and told to the core, and games added by hand by picking an
// exe and naming it.
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import GameActivitySettings from './GameActivitySettings.svelte'
import { gameActivity, gamePrefs } from '../lib/gamePrefs.svelte'

let cmp: ReturnType<typeof mount> | undefined
const sw = (k: string) => document.querySelector<HTMLInputElement>(`input[data-switch="${k}"]`)!
const told = () => invoke.mock.calls.filter((c) => c[0] === 'set_game_activity').map((c) => (c[1] as { settings: unknown }).settings)
const button = (text: string) => [...document.querySelectorAll('button')].find((b) => b.textContent?.trim() === text)
const settle = async () => {
  for (let i = 0; i < 3; i++) await tick()
  flushSync()
}

beforeEach(() => {
  invoke.mockReset()
  invoke.mockResolvedValue(undefined)
  localStorage.clear()
  Object.assign(gamePrefs, { enabled: true, art: true, time: true, games: [] })
  Object.assign(gameActivity, { detecting: false, playing: null })
  cmp = mount(GameActivitySettings, { target: document.body })
  flushSync()
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('switches', () => {
  it('all on by default', () => {
    for (const k of ['enabled', 'art', 'time']) expect([k, sw(k).checked, sw(k).disabled]).toEqual([k, true, false])
  })

  it('master off: told to the core, saved, and the finer ones disabled', () => {
    sw('enabled').click()
    flushSync()
    expect(told().at(-1)).toEqual({ enabled: false, art: true, time: true, games: [] })
    expect(JSON.parse(localStorage.getItem('pulse.games')!).enabled).toBe(false)
    expect(sw('art').disabled && sw('time').disabled).toBe(true)
  })

  it('art and play time each switch on their own', () => {
    sw('art').click()
    flushSync()
    expect(told().at(-1)).toMatchObject({ enabled: true, art: false, time: true })
    sw('time').click()
    flushSync()
    expect(told().at(-1)).toMatchObject({ art: false, time: false })
  })

  it('what the core sees shows while it looks', () => {
    Object.assign(gameActivity, { detecting: true, playing: 'Space Game' })
    flushSync()
    expect(document.querySelector('.seen')!.textContent).toBe('Pulse sees you playing Space Game')
  })
})

describe('games added by hand', () => {
  it('pick an exe, name it, add it; remove it again', async () => {
    invoke.mockImplementation(async (cmd: string) => (cmd === 'pick_game_exe' ? { exe: 'D:\\Games\\Old\\old.exe', name: 'old' } : undefined))
    button('Add a game')!.click()
    await settle()
    const name = document.querySelector<HTMLInputElement>('input[aria-label="Game name"]')!
    expect(name.value).toBe('old')
    name.value = 'Old Game'
    name.dispatchEvent(new Event('input'))
    flushSync()
    button('Add')!.click()
    flushSync()
    expect(told().at(-1)).toEqual({ enabled: true, art: true, time: true, games: [{ name: 'Old Game', exe: 'D:\\Games\\Old\\old.exe' }] })
    expect(document.querySelector('.settings-list-item .nm')!.textContent).toBe('Old Game')
    expect(document.querySelector('.settings-list-item small')!.textContent).toBe('old.exe')
    document.querySelector<HTMLButtonElement>('button[aria-label="Remove Old Game"]')!.click()
    flushSync()
    expect(told().at(-1)).toMatchObject({ games: [] })
    expect(document.querySelector('.settings-list-item')).toBeNull()
  })

  it('cancelling the dialog adds nothing', async () => {
    invoke.mockImplementation(async (cmd: string) => (cmd === 'pick_game_exe' ? null : undefined))
    button('Add a game')!.click()
    await settle()
    expect(document.querySelector('input[aria-label="Game name"]')).toBeNull()
    expect(told()).toEqual([])
  })
})
