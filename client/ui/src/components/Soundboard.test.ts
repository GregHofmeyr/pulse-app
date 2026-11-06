// @vitest-environment jsdom
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Server } from '../lib/protocol/Server'
import type { Sound } from '../lib/protocol/Sound'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'

const invoke = vi.hoisted(() => vi.fn(async (_c: string, _a?: unknown): Promise<unknown> => undefined))
vi.mock('@tauri-apps/api/core', () => ({ invoke }))

import Soundboard from './Soundboard.svelte'
import { mixer, reloadMixer, setNotch } from '../lib/mixer.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import { resetSoundCooldown, soundboard } from '../lib/soundboard.svelte'
import { SOUND_COOLDOWN_MS } from '../lib/soundboard'

const servers: Server[] = [
  { id: 'S1', name: 'Main', icon_hash: null },
  { id: 'S2', name: 'Other', icon_hash: null },
]
const snd = (id: string, server_id: string, name: string, over: Partial<Sound> = {}): Sound => ({
  id, server_id, name, emoji: '🎺', emoji_id: null, media_hash: 'h' + id, volume: 120, duration_ms: 2000, added_by: null,
  created_at: '2026-10-01T00:00:00.000Z', plays: 0, ...over,
})

let cmp: ReturnType<typeof mount> | undefined
const scrollIntoView = vi.fn()
const onClose = vi.fn()
const onAdd = vi.fn()
const onEdit = vi.fn()
const onTidy = vi.fn()

function open(memberOf = new Set(['S1'])) {
  cmp = mount(Soundboard, { target: document.body, props: { onClose, onAdd, onEdit, onTidy, servers, memberOf } })
  flushSync()
}
const settle = async () => {
  for (let i = 0; i < 4; i++) await tick()
  flushSync()
}
const tiles = () => [...document.querySelectorAll<HTMLElement>('.tile')]
const tile = (name: string) => tiles().find((t) => t.textContent?.includes(name))!
const calls = (c: string) => invoke.mock.calls.filter((x) => x[0] === c)
const btn = (label: string) => document.querySelector<HTMLElement>(`[aria-label="${label}"]`)!
const type = (v: string) => {
  const el = document.querySelector<HTMLInputElement>('input[placeholder="Find a sound"]')!
  el.value = v
  el.dispatchEvent(new Event('input', { bubbles: true }))
  flushSync()
}
const rightClick = (el: Element) => {
  el.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 30, clientY: 40 }))
  flushSync()
}
const menuItem = (text: string) => [...document.querySelectorAll<HTMLElement>('[role=menuitem]')].find((b) => b.textContent?.trim() === text)

beforeEach(() => {
  app.state = emptyState()
  app.state.sounds = {
    a: snd('a', 'S1', 'Airhorn'),
    b: snd('b', 'S1', 'Bruh'),
    c: snd('c', 'S1', 'Crickets'),
    d: snd('d', 'S2', 'Drumroll'),
  }
  Element.prototype.scrollIntoView = scrollIntoView
  localStorage.clear()
  reloadMixer()
  invoke.mockClear()
  invoke.mockImplementation(async () => undefined)
  resetSoundCooldown()
  onClose.mockClear(); onAdd.mockClear(); onEdit.mockClear(); onTidy.mockClear(); scrollIntoView.mockClear()
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('Soundboard', () => {
  it('renders_a_three_column_grid_of_tiles_per_member_server', () => {
    open()
    expect(document.querySelector('[data-section="server:S1"]')).not.toBeNull()
    expect(document.querySelector('[data-section="server:S2"]')).toBeNull()
    expect(tiles().map((t) => t.querySelector('.name')?.textContent)).toEqual(['Airhorn', 'Bruh', 'Crickets'])
    expect(document.querySelector<HTMLElement>('.tiles')!.dataset.cols).toBe('3')
  })

  it('favourites_and_frequent_sections_lead_when_present', () => {
    app.state.soundFavourites = ['b']
    app.state.frequentSounds = ['c']
    open()
    expect([...document.querySelectorAll('[data-section]')].map((s) => s.getAttribute('data-section'))).toEqual(['favourites', 'frequent', 'server:S1'])
  })

  it('search_filters_and_hides_sections', () => {
    open()
    type('bru')
    expect(tiles().map((t) => t.querySelector('.name')?.textContent)).toEqual(['Bruh'])
    expect(document.querySelector('[data-section]')).toBeNull()
    type('zzz')
    expect(document.body.textContent).toContain('No sounds found')
  })

  it('rail_click_scrolls_to_the_section', async () => {
    open(new Set(['S1', 'S2']))
    ;(document.querySelector('[data-rail="server:S2"]') as HTMLElement).click()
    await settle()
    expect(scrollIntoView).toHaveBeenCalled()
    expect(scrollIntoView.mock.contexts.at(-1)).toBe(document.querySelector('[data-section="server:S2"]'))
  })

  it('tile_click_plays_to_the_call_and_not_locally', async () => {
    open()
    tile('Bruh').click()
    await settle()
    expect(calls('play_sound_to_call')).toEqual([['play_sound_to_call', { id: 'b' }]])
    expect(calls('soundboard_play')).toHaveLength(0)
  })

  // #214: click-spam can't play over itself: after a play, a short cooldown, shown on the tiles
  it('a_second_click_inside_the_cooldown_plays_nothing_and_the_tiles_say_so', async () => {
    vi.useFakeTimers({ toFake: ['Date', 'setTimeout', 'clearTimeout'] })
    try {
      open()
      tile('Bruh').click()
      flushSync()
      tile('Airhorn').click()
      tile('Bruh').click()
      flushSync()
      expect(calls('play_sound_to_call')).toEqual([['play_sound_to_call', { id: 'b' }]])
      expect(tiles().every((t) => t.classList.contains('cooling') && t.getAttribute('aria-disabled') === 'true')).toBe(true)
      vi.advanceTimersByTime(SOUND_COOLDOWN_MS)
      flushSync()
      expect(tiles().some((t) => t.classList.contains('cooling'))).toBe(false)
      tile('Airhorn').click()
      flushSync()
      expect(calls('play_sound_to_call').map((c) => c[1])).toEqual([{ id: 'b' }, { id: 'a' }])
    } finally {
      vi.useRealTimers()
    }
  })

  it('the_cooldown_is_about_a_second_and_fades_on_a_motion_token', () => {
    expect(SOUND_COOLDOWN_MS).toBe(1000)
    const css = readFileSync(resolve(__dirname, 'Soundboard.svelte'), 'utf8').match(/<style>([\s\S]*)<\/style>/)![1]
    expect(css).toMatch(/\.tile\.cooling \{[^}]*opacity: \.55;[^}]*cursor: not-allowed/)
    expect(css).toMatch(/\.tile \{[^}]*transition: opacity var\(--dur-fast\) var\(--ease\)/)
  })

  it('a_rate_limited_play_is_swallowed_but_other_errors_show', async () => {
    open()
    invoke.mockRejectedValueOnce('slow down a little, then try again')
    tile('Bruh').click()
    await settle()
    expect(document.querySelector('[role=alert]')).toBeNull()
    invoke.mockRejectedValueOnce("You're not in a call")
    resetSoundCooldown()
    tile('Bruh').click()
    await settle()
    expect(document.querySelector('[role=alert]')?.textContent).toBe("You're not in a call")
  })

  it('a_play_of_a_sound_deleted_meanwhile_is_quiet', async () => {
    open()
    invoke.mockRejectedValueOnce('not found')
    tile('Bruh').click()
    await settle()
    expect(document.querySelector('[role=alert]')).toBeNull()
  })

  it('deleting_a_sound_someone_else_just_deleted_is_quiet', async () => {
    open()
    rightClick(tile('Bruh'))
    menuItem('Delete')!.click()
    flushSync()
    invoke.mockRejectedValueOnce('not found')
    ;[...document.querySelectorAll<HTMLElement>('[role=alertdialog] button')].find((b) => b.textContent === 'Delete')!.click()
    await settle()
    expect(calls('delete_sound')).toHaveLength(1)
    expect(document.querySelector('[role=alert]')).toBeNull()
  })

  it('preview_at_soundboard_volume_0_plays_at_100', async () => {
    soundboard.volume_pct = 0
    open()
    tile('Bruh').querySelector<HTMLElement>('.preview')!.click()
    await settle()
    expect(calls('soundboard_play')).toEqual([['soundboard_play', { hash: 'hb', callsPercent: 100, soundboardPercent: 100, soundPercent: 120, output: null, soundId: null, player: null }]])
  })

  it('preview_follows_the_calls_master_and_soundboard_notch', async () => {
    setNotch('calls', 'master', 60)
    soundboard.volume_pct = 150
    open()
    tile('Bruh').querySelector<HTMLElement>('.preview')!.click()
    await settle()
    expect(calls('soundboard_play')).toEqual([['soundboard_play', { hash: 'hb', callsPercent: 60, soundboardPercent: 150, soundPercent: 120, output: null, soundId: null, player: null }]])
  })

  it('preview_plays_locally_only', async () => {
    open()
    expect(btn('Preview Bruh')).not.toBeNull()
    tile('Bruh').querySelector<HTMLElement>('.preview')!.click()
    await settle()
    expect(calls('soundboard_play')).toEqual([['soundboard_play', { hash: 'hb', callsPercent: 100, soundboardPercent: 100, soundPercent: 120, output: null, soundId: null, player: null }]])
    expect(calls('play_sound_to_call')).toHaveLength(0)
  })

  it('context_menu_favourite_toggles_on', async () => {
    open()
    rightClick(tile('Bruh'))
    menuItem('Favourite')!.click()
    await settle()
    expect(calls('set_sound_favourite')).toEqual([['set_sound_favourite', { id: 'b', on: true }]])
  })

  it('context_menu_unfavourite_for_a_favourite', async () => {
    app.state.soundFavourites = ['b']
    open()
    rightClick(document.querySelector('[data-section="server:S1"]')!.querySelectorAll('.tile')[1])
    menuItem('Unfavourite')!.click()
    await settle()
    expect(calls('set_sound_favourite')).toEqual([['set_sound_favourite', { id: 'b', on: false }]])
  })

  it('context_menu_edit_hands_the_sound_to_onEdit', () => {
    open()
    rightClick(tile('Bruh'))
    menuItem('Edit')!.click()
    flushSync()
    expect(onEdit).toHaveBeenCalledWith(expect.objectContaining({ id: 'b' }))
  })

  it('delete_asks_first_then_deletes', async () => {
    open()
    rightClick(tile('Bruh'))
    menuItem('Delete')!.click()
    flushSync()
    expect(calls('delete_sound')).toHaveLength(0)
    const ask = document.querySelector('[role=alertdialog]')!
    expect(ask.textContent).toContain('Delete Bruh?')
    ;[...ask.querySelectorAll('button')].find((b) => b.textContent === 'Delete')!.click()
    await settle()
    expect(calls('delete_sound')).toEqual([['delete_sound', { id: 'b' }]])
  })

  it('cancelling_the_delete_does_nothing', async () => {
    open()
    rightClick(tile('Bruh'))
    menuItem('Delete')!.click()
    flushSync()
    ;[...document.querySelectorAll<HTMLElement>('[role=alertdialog] button')].find((b) => b.textContent === 'Cancel')!.click()
    await settle()
    expect(calls('delete_sound')).toHaveLength(0)
    expect(document.querySelector('[role=alertdialog]')).toBeNull()
  })

  it('volume_slider_writes_the_pref', () => {
    open()
    expect(document.querySelector('input[type=range]')).toBeNull()
    btn('Soundboard volume').click()
    flushSync()
    const r = document.querySelector<HTMLInputElement>('input[type=range]')!
    expect(r.max).toBe('200')
    r.value = '135'
    r.dispatchEvent(new Event('input', { bubbles: true }))
    flushSync()
    expect(JSON.parse(localStorage.getItem('pulse.mixer')!).calls.soundboard).toBe(135)
    expect(mixer.calls.soundboard).toBe(135)
    expect(document.body.textContent).toContain('135%')
  })

  it('volume_slider_follows_the_mixer', () => {
    open()
    btn('Soundboard volume').click()
    flushSync()
    setNotch('calls', 'soundboard', 42)
    flushSync()
    expect(document.querySelector<HTMLInputElement>('input[type=range]')!.value).toBe('42')
  })

  it('add_sound_tile_per_member_server_calls_onAdd', () => {
    open(new Set(['S1', 'S2']))
    const adds = [...document.querySelectorAll<HTMLElement>('.add-tile')]
    expect(adds).toHaveLength(2)
    adds[1].click()
    expect(onAdd).toHaveBeenCalledWith('S2')
  })

  it('tidy_up_shows_only_past_24_sounds', () => {
    for (let i = 0; i < 21; i++) app.state.sounds['x' + i] = snd('x' + i, 'S1', 'Extra ' + i)
    open() // 3 + 21 = 24: not yet
    expect(document.body.textContent).not.toContain('Tidy up?')
    unmount(cmp!)
    cmp = undefined
    document.body.innerHTML = ''
    app.state.sounds.y = snd('y', 'S1', 'One more')
    open()
    const link = [...document.querySelectorAll<HTMLElement>('button')].find((b) => b.textContent?.trim() === 'Tidy up?')!
    link.click()
    expect(onTidy).toHaveBeenCalledWith('S1')
  })

  it('escape_closes', () => {
    open()
    document.querySelector('[role=dialog]')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
    expect(onClose).toHaveBeenCalled()
  })
})

describe('sound emoji on tiles', () => {
  const pog = { id: 'E1', server_id: 'S2', name: 'pog', hash: 'a'.repeat(64), still_hash: null, animated: false, added_by: null, created_at: '2026-10-01T00:00:00.000Z' }
  it('shows a custom emoji, and the soundboard icon once that emoji is deleted', () => {
    app.state.customEmoji = { E1: pog }
    app.state.sounds = { ...app.state.sounds, a: snd('a', 'S1', 'Airhorn', { emoji: null, emoji_id: 'E1' }) }
    open()
    expect(tile('Airhorn').querySelector<HTMLImageElement>('img')?.alt).toBe(':pog:')
    expect(tile('Bruh').textContent).toContain('🎺')
    app.state = { ...app.state, customEmoji: { E1: { ...pog, hash: null } } }
    flushSync()
    expect(tile('Airhorn').querySelector('img')).toBeNull()
    expect(tile('Airhorn').querySelector('svg')).not.toBeNull()
  })
})
