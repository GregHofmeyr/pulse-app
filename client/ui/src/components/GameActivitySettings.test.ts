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
  Object.assign(gamePrefs, { enabled: true, art: true, time: true, games: [], clips: true, clip_folders: null })
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
    for (const k of ['enabled', 'art', 'time', 'clips']) expect([k, sw(k).checked, sw(k).disabled]).toEqual([k, true, false])
  })

  it('master off: told to the core, saved, and the finer ones disabled', () => {
    sw('enabled').click()
    flushSync()
    expect(told().at(-1)).toEqual({ enabled: false, art: true, time: true, games: [], clips: true, clip_folders: null, steam_achievements: false })
    expect(JSON.parse(localStorage.getItem('pulse.games')!).enabled).toBe(false)
    expect(sw('art').disabled && sw('time').disabled && sw('clips').disabled).toBe(true)
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
    expect(told().at(-1)).toMatchObject({ enabled: true, art: true, time: true, games: [{ name: 'Old Game', exe: 'D:\\Games\\Old\\old.exe' }] })
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

// #382: "Offer new recordings as clips" under the master switch, and the folders it watches (Windows only).
describe('clip drop', () => {
  const realUA = navigator.userAgent
  const remount = () => {
    if (cmp) unmount(cmp)
    document.body.innerHTML = ''
    cmp = mount(GameActivitySettings, { target: document.body })
    flushSync()
  }
  const folderPaths = () => [...document.querySelectorAll('.folders .path')].map((e) => e.textContent)
  afterEach(() => Object.defineProperty(navigator, 'userAgent', { configurable: true, value: realUA }))

  it('its switch is told to the core and saved', () => {
    sw('clips').click()
    flushSync()
    expect(told().at(-1)).toMatchObject({ enabled: true, clips: false })
    expect(JSON.parse(localStorage.getItem('pulse.games')!).clips).toBe(false)
  })

  it('no folder list outside Windows', () => {
    expect(document.querySelector('.folders')).toBeNull()
  })

  it('on Windows: the defaults show; adding one, removing one and going back to the defaults are told', async () => {
    Object.defineProperty(navigator, 'userAgent', { configurable: true, value: 'Mozilla/5.0 (Windows NT 10.0; Win64; x64)' })
    const defaults = ['C:\\Users\\alex\\Videos\\Captures', 'C:\\Users\\alex\\Videos']
    invoke.mockImplementation(async (cmd: string) => (cmd === 'clip_drop_defaults' ? defaults : cmd === 'pick_clip_folder' ? 'D:\\Clips' : undefined))
    remount()
    await settle()
    expect(folderPaths()).toEqual(defaults)
    expect(button('Use the defaults')).toBeUndefined()

    button('Add a folder')!.click()
    await settle()
    expect(told().at(-1)).toMatchObject({ clip_folders: [...defaults, 'D:\\Clips'] })
    expect(folderPaths()).toEqual([...defaults, 'D:\\Clips'])

    // (backslashes in a path: found by its label, not a CSS selector)
    ;[...document.querySelectorAll('button')].find((b) => b.getAttribute('aria-label') === `Stop watching ${defaults[0]}`)!.click()
    flushSync()
    expect(told().at(-1)).toMatchObject({ clip_folders: [defaults[1], 'D:\\Clips'] })

    button('Use the defaults')!.click()
    flushSync()
    expect(told().at(-1)).toMatchObject({ clip_folders: null })
    expect(folderPaths()).toEqual(defaults)

    // the switch off (or the master one): no folder list
    sw('clips').click()
    flushSync()
    expect(document.querySelector('.folders')).toBeNull()
  })
})

describe('Steam account (#379)', () => {
  const linked = (over: Record<string, unknown> = {}) => ({
    linking_available: true,
    achievements_available: true,
    link: { steam_id: '76500000000000001', persona: 'alex on steam', share_achievements: true, private: false, backfill_done: true, completion_rate: 62.5, perfect_games: 3, ...over },
  })
  const steamRows = () => document.querySelector('[data-steam]')!
  async function withStatus(status: unknown) {
    const { setSteamStatus } = await import('../lib/steam.svelte')
    setSteamStatus(status as never)
    flushSync()
  }

  it('not linked: Link Steam account opens the browser sign-in; the share switch waits for a link', async () => {
    await withStatus({ linking_available: true, achievements_available: true, link: null })
    expect(steamRows().textContent).toContain("Pulse never sees your Steam password")
    expect(sw('steam').disabled).toBe(true)
    button('Link Steam account')!.click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('link_steam', undefined)
  })

  it('linked: the persona, Unlink, the share switch on by default, and the completion rate', async () => {
    await withStatus(linked())
    expect(steamRows().textContent).toContain('alex on steam')
    expect(sw('steam').checked && !sw('steam').disabled).toBe(true)
    expect(steamRows().textContent).toContain('62.5 % completion · 3 perfect games')
    invoke.mockImplementation(async (cmd: string) => (cmd === 'unlink_steam' ? { linking_available: true, achievements_available: true, link: null } : undefined))
    button('Unlink')!.click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('unlink_steam', undefined)
    expect(button('Link Steam account')).toBeTruthy()
  })

  it('sharing off is the server setting; on also tells the core to send the app id for achievements', async () => {
    await withStatus(linked())
    expect(gamePrefs.steam_achievements).toBe(true)
    expect(told().at(-1)).toMatchObject({ steam_achievements: true })
    invoke.mockImplementation(async (cmd: string) => (cmd === 'set_steam_share' ? linked({ share_achievements: false }) : undefined))
    sw('steam').click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('set_steam_share', { on: false })
    expect(gamePrefs.steam_achievements).toBe(false)
    expect(told().at(-1)).toMatchObject({ steam_achievements: false })
  })

  it('obeys the master switch', async () => {
    await withStatus(linked())
    sw('enabled').click()
    flushSync()
    expect(sw('steam').disabled).toBe(true)
  })

  it('no key, private game details, or no linking on this server each say so', async () => {
    await withStatus(linked({}) && { ...linked(), achievements_available: false })
    expect(steamRows().textContent).toContain('Achievements need the server owner to add a Steam key.')
    expect(sw('steam').disabled).toBe(true)
    await withStatus(linked({ private: true }))
    expect(steamRows().textContent).toContain('Make your Steam game details public to share achievements.')
    await withStatus({ linking_available: false, achievements_available: true, link: null })
    expect(steamRows().textContent).toContain("Linking Steam isn't set up on this server.")
    expect(button('Link Steam account')!.disabled).toBe(true)
  })

  it('a SteamLinkChanged updates the rows', async () => {
    await withStatus({ linking_available: true, achievements_available: true, link: null })
    const { handleSteamEvent } = await import('../lib/steam.svelte')
    handleSteamEvent({ t: 'SteamLinkChanged', d: { user_id: 'U1', status: linked() } } as never)
    flushSync()
    expect(steamRows().textContent).toContain('alex on steam')
  })
})
