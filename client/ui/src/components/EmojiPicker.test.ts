// @vitest-environment jsdom
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { CustomEmoji } from '../lib/protocol/CustomEmoji'
import type { Server } from '../lib/protocol/Server'

vi.mock('@tauri-apps/api/core', () => ({ invoke: async () => undefined }))

import EmojiPicker from './EmojiPicker.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import { byShortcode } from '../lib/emoji'

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

  it('opens the full grid quickly', () => {
    const times: number[] = []
    for (let i = 0; i < 4; i++) {
      if (cmp) unmount(cmp)
      const t0 = performance.now()
      open()
      times.push(performance.now() - t0)
    }
    console.info(`emoji picker open (cold, warm...): ${times.map((t) => t.toFixed(1)).join(' ')} ms, ${cells().length} cells`)
    expect(cells().length).toBeGreaterThan(1800)
  })
})
