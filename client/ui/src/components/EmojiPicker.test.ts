// @vitest-environment jsdom
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { CustomEmoji } from '../lib/protocol/CustomEmoji'
import type { Server } from '../lib/protocol/Server'

vi.mock('@tauri-apps/api/core', () => ({ invoke: async () => undefined }))

import EmojiPicker from './EmojiPicker.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import { byShortcode, standard } from '../lib/emoji'

const servers: Server[] = [
  { id: 'S1', name: 'Main', icon_hash: null },
  { id: 'S2', name: 'Other', icon_hash: null },
]
const custom = (id: string, server_id: string, name: string, hash: string | null = 'h' + id): CustomEmoji => ({
  id, server_id, name, hash, still_hash: null, animated: false, added_by: null, created_at: '2026-10-01T00:00:00.000Z',
})

let cmp: ReturnType<typeof mount> | undefined
const scrollIntoView = vi.fn()
const onPick = vi.fn()
const onClose = vi.fn()

function open(memberOf = new Set(['S1'])) {
  cmp = mount(EmojiPicker, { target: document.body, props: { onPick, onClose, servers, memberOf } })
  flushSync()
}
const key = (el: Element, k: string) => {
  el.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true }))
  flushSync()
}
const search = () => document.querySelector('.search') as HTMLInputElement
const type = (v: string) => {
  search().value = v
  search().dispatchEvent(new Event('input', { bubbles: true }))
  flushSync()
}
const cells = () => [...document.querySelectorAll('.cell')]

beforeEach(() => {
  app.state = emptyState()
  Element.prototype.scrollIntoView = scrollIntoView
  localStorage.clear()
  onPick.mockClear(); onClose.mockClear(); scrollIntoView.mockClear()
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('EmojiPicker', () => {
  it('search_filters_and_enter_picks_first', () => {
    open()
    type('thumbs')
    key(search(), 'Enter')
    expect(onPick).toHaveBeenCalledTimes(1)
    expect(onPick.mock.calls[0][0].u).toBe('👍')
  })

  it('rail_click_scrolls_to_section', async () => {
    open()
    const btn = document.querySelector('[data-rail="group:objects"]') as HTMLElement
    btn.click()
    await tick()
    expect(scrollIntoView).toHaveBeenCalled()
    expect(scrollIntoView.mock.contexts.at(-1)).toBe(document.querySelector('[data-section="group:objects"]'))
  })

  it('arrows_move_highlight', () => {
    open()
    const second = cells()[1].getAttribute('data-key')
    key(search(), 'ArrowRight')
    key(search(), 'Enter')
    expect(onPick.mock.calls[0][0].u).toBe(second)
  })

  it('escape_closes', () => {
    open()
    key(search(), 'Escape')
    expect(onClose).toHaveBeenCalledTimes(1)
  })

  it('custom_from_non_member_servers_hidden', () => {
    app.state = {
      ...emptyState(),
      customEmoji: { a: custom('a', 'S1', 'mine'), b: custom('b', 'S2', 'theirs'), c: custom('c', 'S1', 'gone', null) },
    }
    open()
    const keys = cells().map((c) => c.getAttribute('data-key'))
    expect(keys).toContain('custom:a')
    expect(keys).not.toContain('custom:b')
    expect(keys).not.toContain('custom:c')
    expect(document.querySelector('[data-rail="server:S2"]')).toBeNull()
    expect(document.querySelector('[data-rail="server:S1"]')).not.toBeNull()
  })

  it('frequently_used_first', () => {
    app.state = { ...emptyState(), frequentEmoji: ['🔥', 'custom:a'], customEmoji: { a: custom('a', 'S1', 'mine') } }
    open()
    const first = document.querySelector('section')!
    expect(first.getAttribute('data-section')).toBe('frequent')
    expect([...first.querySelectorAll('.cell')].map((c) => c.getAttribute('data-key'))).toEqual(['🔥', 'custom:a'])
  })

  it('frequent_tones_of_one_emoji_do_not_collide', () => {
    app.state = { ...emptyState(), frequentEmoji: ['👍', '👍🏽'] }
    open()
    const first = document.querySelector('[data-section="frequent"]')!
    expect([...first.querySelectorAll('.cell')].map((c) => c.getAttribute('data-key'))).toEqual(['👍', '👍🏽'])
  })

  it('frequent_dedupes_fe0f_variants', () => {
    app.state = { ...emptyState(), frequentEmoji: ['❤️', '❤', '👍', '👍'] }
    open()
    const first = document.querySelector('[data-section="frequent"]')!
    const cells = [...first.querySelectorAll('.cell')]
    expect(cells.map((c) => c.getAttribute('data-key'))).toEqual(['❤', '👍'])
    expect(cells[0].querySelector('img')!.getAttribute('src')).toBe('/emoji/2764.svg')
  })

  it('frequent_fills_to_27_past_unusable_keys', () => {
    const smileys = Array.from({ length: 30 }, (_, i) => String.fromCodePoint(0x1f600 + i))
    app.state = {
      ...emptyState(),
      frequentEmoji: ['custom:gone', 'custom:b', 'custom:zzz', ...smileys],
      customEmoji: { gone: custom('gone', 'S1', 'gone', null), b: custom('b', 'S2', 'theirs') },
    }
    open()
    const first = document.querySelector('[data-section="frequent"]')!
    expect([...first.querySelectorAll('.cell')].map((c) => c.getAttribute('data-key'))).toEqual(smileys.slice(0, 27))
  })

  it('skin_tone_applies_to_toned_emoji', () => {
    open()
    const wave = byShortcode('wave')!
    ;(document.querySelector('[aria-label="Skin tone 3"]') as HTMLElement).click()
    flushSync()
    expect(localStorage.getItem('pulse.skinTone')).toBe('3')
    type('wave')
    key(search(), 'Enter')
    expect(onPick.mock.calls[0][0].u).toBe(wave.s![2])
  })

  it('frequent_caps_at_27_of_usable_mixed_keys', () => {
    const ids = Array.from({ length: 10 }, (_, i) => 'c' + i)
    const smileys = Array.from({ length: 30 }, (_, i) => String.fromCodePoint(0x1f600 + i))
    app.state = {
      ...emptyState(),
      frequentEmoji: [...ids.map((i) => 'custom:' + i), ...smileys],
      customEmoji: Object.fromEntries(ids.map((i) => [i, custom(i, 'S1', 'e' + i)])),
    }
    open()
    const keys = [...document.querySelectorAll('[data-section="frequent"] .cell')].map((c) => c.getAttribute('data-key'))
    expect(keys).toEqual([...ids.map((i) => 'custom:' + i), ...smileys.slice(0, 17)])
  })

  it('left_right_move_the_caret_in_a_typed_search_until_the_grid_is_entered', () => {
    open()
    type('thumbs')
    const ev = (k: string) => {
      const e = new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true })
      search().dispatchEvent(e)
      flushSync()
      return e.defaultPrevented
    }
    expect(ev('ArrowLeft')).toBe(false)
    expect(ev('ArrowRight')).toBe(false)
    expect(ev('ArrowDown')).toBe(true) // enters the grid
    expect(ev('ArrowRight')).toBe(true) // now navigating it
  })

  it('left_right_navigate_the_grid_when_the_search_is_empty', () => {
    open()
    const e = new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true, cancelable: true })
    search().dispatchEvent(e)
    expect(e.defaultPrevented).toBe(true)
  })

  it('enter_on_a_tab_focused_cell_picks_that_cell', () => {
    open()
    const cell = cells()[5] as HTMLElement
    cell.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }))
    expect(onPick).toHaveBeenCalledTimes(1)
    expect(onPick.mock.calls[0][0].u).toBe(cell.getAttribute('data-key'))
  })

  it('enter_with_nothing_highlighted_and_no_search_does_nothing', () => {
    open()
    key(search(), 'Enter')
    expect(onPick).not.toHaveBeenCalled()
  })

  it('active_is_clamped_when_a_section_collapses', () => {
    open()
    for (let i = 0; i < 150; i++) key(search(), 'ArrowDown')
    const heads = [...document.querySelectorAll('.head')] as HTMLElement[]
    for (const h of heads.slice(0, -1)) h.click()
    flushSync()
    key(search(), 'Enter')
    const last = cells().at(-1)!.getAttribute('data-key')
    expect(onPick).toHaveBeenCalledTimes(1)
    expect(onPick.mock.calls[0][0].u).toBe(last)
    expect(document.querySelector('.cell.active')).toBe(cells().at(-1))
  })

  it('enter_on_other_controls_does_not_pick_the_highlight', () => {
    open()
    key(search(), 'ArrowDown')
    for (const sel of ['.tone', '.head', '.rail-btn']) {
      key(document.querySelector(sel)!, 'Enter')
    }
    expect(onPick).not.toHaveBeenCalled()
  })

  it('rail_click_during_search_clears_it_and_scrolls', async () => {
    open()
    type('wave')
    expect(cells().length).toBeLessThan(60)
    ;(document.querySelector('[data-rail="group:objects"]') as HTMLElement).click()
    await tick()
    expect(search().value).toBe('')
    expect(document.querySelector('[data-section="group:objects"] .cell')).not.toBeNull()
    expect(scrollIntoView.mock.contexts.at(-1)).toBe(document.querySelector('[data-section="group:objects"]'))
  })

  it('renders_every_standard_emoji_with_lazy_images', () => {
    open()
    expect(cells().length).toBe(standard.length)
    const imgs = [...document.querySelectorAll('.cell img')]
    expect(imgs.every((i) => i.getAttribute('loading') === 'lazy')).toBe(true)
  })
})
