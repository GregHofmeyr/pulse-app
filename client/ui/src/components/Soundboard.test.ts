// @vitest-environment jsdom
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Server } from '../lib/protocol/Server'
import type { Sound } from '../lib/protocol/Sound'

const invoke = vi.hoisted(() => vi.fn(async (_c: string, _a?: unknown): Promise<unknown> => undefined))
vi.mock('@tauri-apps/api/core', () => ({ invoke }))

import Soundboard from './Soundboard.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import { soundboard } from '../lib/soundboard.svelte'

const servers: Server[] = [
  { id: 'S1', name: 'Main', icon_hash: null },
  { id: 'S2', name: 'Other', icon_hash: null },
]
const snd = (id: string, server_id: string, name: string, over: Partial<Sound> = {}): Sound => ({
  id, server_id, name, emoji: '🎺', media_hash: 'h' + id, volume: 120, duration_ms: 2000, added_by: null,
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
  soundboard.volume_pct = 100
  invoke.mockClear()
  invoke.mockImplementation(async () => undefined)
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

  it('a_rate_limited_play_is_swallowed_but_other_errors_show', async () => {
    open()
    invoke.mockRejectedValueOnce('slow down a little, then try again')
    tile('Bruh').click()
    await settle()
    expect(document.querySelector('[role=alert]')).toBeNull()
    invoke.mockRejectedValueOnce("You're not in a call")
    tile('Bruh').click()
    await settle()
    expect(document.querySelector('[role=alert]')?.textContent).toBe("You're not in a call")
  })

  it('preview_plays_locally_only', async () => {
    open()
    expect(btn('Preview Bruh')).not.toBeNull()
    tile('Bruh').querySelector<HTMLElement>('.preview')!.click()
    await settle()
    expect(calls('soundboard_play')).toEqual([['soundboard_play', { hash: 'hb', soundboardPercent: 100, soundPercent: 120, output: null }]])
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
    r.value = '35'
    r.dispatchEvent(new Event('input', { bubbles: true }))
    flushSync()
    expect(JSON.parse(localStorage.getItem('pulse.soundboard')!)).toEqual({ volume_pct: 35 })
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
