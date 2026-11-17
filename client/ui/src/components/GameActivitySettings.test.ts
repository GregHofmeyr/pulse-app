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
    expect(told().at(-1)).toEqual({ enabled: false, art: true, time: true, games: [], steam_achievements: false })
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

// #382: clip drop is its own feature now (ClipDropSettings, Settings › Chat): nothing of it here.
it('no clip drop switch or folders in Game activity', () => {
  expect(document.querySelector('input[data-switch="clips"]')).toBeNull()
  expect(document.querySelector('.folders')).toBeNull()
  expect(document.body.textContent).not.toContain('Offer new recordings')
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

  it('Steam hours (#383): on by default, the hours shown, and off is the server setting', async () => {
    await withStatus(linked({ share_hours: true, recent_minutes: 1290 }))
    expect(sw('steam-hours').checked && !sw('steam-hours').disabled).toBe(true)
    expect(steamRows().querySelector('[data-steam-hours]')!.textContent).toBe('21.5 h in Steam games over the last two weeks')
    invoke.mockImplementation(async (cmd: string) => (cmd === 'set_steam_hours' ? linked({ share_hours: false }) : undefined))
    sw('steam-hours').click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('set_steam_hours', { on: false })
    expect(sw('steam-hours').checked).toBe(false)
    expect(steamRows().querySelector('[data-steam-hours]')).toBeNull()
  })

  it('an older server that never says is on; no link is off and disabled', async () => {
    await withStatus(linked())
    expect(sw('steam-hours').checked).toBe(true)
    await withStatus({ linking_available: true, achievements_available: true, link: null })
    expect(sw('steam-hours').checked || !sw('steam-hours').disabled).toBe(false)
  })

  it('Join (#381): on by default (an older server never says), and off is the server setting', async () => {
    await withStatus(linked())
    expect(sw('steam-join').checked && !sw('steam-join').disabled).toBe(true)
    invoke.mockImplementation(async (cmd: string) => (cmd === 'set_steam_join' ? linked({ share_join: false }) : undefined))
    sw('steam-join').click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('set_steam_join', { on: false })
    expect(sw('steam-join').checked).toBe(false)
  })

  it('obeys the master switch', async () => {
    await withStatus(linked())
    sw('enabled').click()
    flushSync()
    expect(sw('steam').disabled && sw('steam-hours').disabled && sw('steam-join').disabled).toBe(true)
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

describe('Deadlock (#390)', () => {
  const linked = (over: Record<string, unknown> = {}, status: Record<string, unknown> = {}) => ({
    linking_available: true,
    achievements_available: true,
    deadlock_available: true,
    link: { steam_id: '76500000000000001', persona: 'alex on steam', share_achievements: true, private: false, backfill_done: true, completion_rate: null, perfect_games: 0, ...over },
    ...status,
  })
  const rows = () => document.querySelector('[data-deadlock]')!
  async function withStatus(status: unknown) {
    const { setSteamStatus } = await import('../lib/steam.svelte')
    setSteamStatus(status as never)
    flushSync()
  }

  it('three switches, on by default (an older server never says), each saved on the server', async () => {
    await withStatus(linked())
    for (const k of ['matches', 'rank', 'squad']) expect([k, sw(`deadlock-${k}`).checked, sw(`deadlock-${k}`).disabled]).toEqual([k, true, false])
    invoke.mockImplementation(async (cmd: string) => (cmd === 'set_deadlock_switch' ? linked({ deadlock_rank: false }) : undefined))
    sw('deadlock-rank').click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('set_deadlock_switch', { key: 'rank', on: false })
    expect(sw('deadlock-rank').checked).toBe(false)
  })

  it("need a linked account, not game detection; the bot link is deadlock-api's site with the copycat warning", async () => {
    await withStatus({ linking_available: true, achievements_available: true, link: null })
    expect(sw('deadlock-matches').disabled).toBe(true)
    await withStatus(linked())
    sw('enabled').click()
    flushSync()
    expect(sw('deadlock-matches').disabled).toBe(false)
    const a = rows().querySelector<HTMLAnchorElement>('a')!
    expect(a.getAttribute('href')).toBe('https://deadlock-api.com/patron')
    expect(a.getAttribute('target')).toBe('_blank')
    expect(rows().querySelector('[data-bot-warning]')!.textContent).toBe('The real bot never asks for logins, codes or trades — ignore copycats.')
  })

  it('deadlock-api unreachable: one quiet line, no error', async () => {
    await withStatus(linked({}, { deadlock_available: false }))
    expect(rows().querySelector('[data-deadlock-down]')).toBeTruthy()
    expect(rows().querySelector('[role=alert]')).toBeNull()
  })
})
