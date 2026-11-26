// @vitest-environment jsdom
// "Who played it": a soundboard sound lights the player's tile ring like speaking does, without being speaking. The
// ring itself is SpeakingRing (#651), inside a ring box the tile reserves whatever the style, so nothing jumps or spills.
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { flushSync, mount, unmount } from 'svelte'
import { compile } from 'svelte/compiler'
import { afterEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({ invoke: async () => undefined }))

import UserTile from './UserTile.svelte'
import { voice } from '../lib/voice.svelte'
import { callChanged, lightRing } from '../lib/soundboard.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import { tileScale } from '../lib/stagelayout'

let cmp: ReturnType<typeof mount> | undefined
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
  callChanged(null)
  voice.speaking = new Set()
  app.state = emptyState()
})
const entry = { userId: 'U2', name: 'sam', muted: false, deafened: false, pending: false }

it("a playing sound lights the player's ring and leaves speaking alone", () => {
  vi.useFakeTimers()
  try {
    cmp = mount(UserTile, { target: document.body, props: { entry, onMenu: () => {} } })
    flushSync()
    const tile = document.querySelector('.tile')!
    const ring = document.querySelector<HTMLElement>('.ring-box')!
    expect(tile.classList.contains('speaking')).toBe(false)
    expect(ring.dataset.ring).toBe('off')
    lightRing('U2', 800)
    flushSync()
    expect(tile.classList.contains('speaking')).toBe(true)
    expect(ring.dataset.ring).toBe('on')
    expect(voice.speaking.has('U2')).toBe(false)
    vi.advanceTimersByTime(900)
    flushSync()
    expect(tile.classList.contains('speaking')).toBe(false)
    expect(ring.dataset.ring).toBe('off')
  } finally {
    vi.useRealTimers()
  }
})

describe('the ring on the stage and strip (#651)', () => {
  const scope = 'svelte-test'
  const css = compile(readFileSync(resolve(__dirname, 'UserTile.svelte'), 'utf8'), { filename: 'UserTile.svelte', css: 'external', cssHash: () => scope }).css!.code
  /** Every declaration block whose selector list has exactly `sel` (scoping stripped). */
  const ruleBodies = (sel: string) =>
    [...css.matchAll(/([^{}]+)\{([^}]*)\}/g)]
      .filter((m) => m[1].replace(/\/\*[\s\S]*?\*\//g, '').split(',').some((s) => s.replaceAll(`:where(.${scope})`, '').replaceAll(`.${scope}`, '').trim() === sel))
      .map((m) => m[2])
      .join(';')

  it('the_tile_draws_its_ring_through_speaking_ring', () => {
    voice.speaking = new Set(['U2'])
    cmp = mount(UserTile, { target: document.body, props: { entry, height: 300, onMenu: () => {} } })
    flushSync()
    const ring = document.querySelector<HTMLElement>('.avatar-btn > .ring-box')!
    expect(ring).not.toBeNull()
    expect(ring.style.getPropertyValue('--d')).toBe(`${tileScale(300).avatar}px`)
    expect(ring.dataset.place).toBe('stage')
    expect(document.querySelector<HTMLElement>('.av')!.style.boxShadow).toBe('')
    expect(css).not.toMatch(/:global\(\.av\)\s*\{[^}]*box-shadow/)
    expect(css).not.toMatch(/\.av[^{]*\{[^}]*box-shadow/)
  })

  it('the_strip_tile_draws_the_strip_ring', () => {
    cmp = mount(UserTile, { target: document.body, props: { entry, compact: true, onMenu: () => {} } })
    flushSync()
    const ring = document.querySelector<HTMLElement>('.ring-box')!
    expect(ring.dataset.place).toBe('strip')
    expect(ring.style.getPropertyValue('--d')).toBe('36px')
  })

  it('the_ring_box_is_reserved_whatever_the_style', () => {
    expect(ruleBodies('.avatar-btn')).toMatch(/margin:\s*calc\(var\(--d\)\s*\*\s*\.16\)/)
    expect(ruleBodies('.compact .avatar-btn')).toMatch(/margin:\s*max\(6px,\s*calc\(var\(--d\)\s*\*\s*\.16\)\)/)
    cmp = mount(UserTile, { target: document.body, props: { entry, height: 300, onMenu: () => {} } })
    flushSync()
    const btn = document.querySelector<HTMLElement>('.avatar-btn')!
    const tile = document.querySelector<HTMLElement>('.tile')!
    expect(btn.style.getPropertyValue('--d')).toBe(`${tileScale(300).avatar}px`)
    const before = [btn.getAttribute('style'), tile.getAttribute('style')]
    voice.speaking = new Set(['U2'])
    flushSync()
    expect([btn.getAttribute('style'), tile.getAttribute('style')]).toEqual(before)
  })

  it('the_strip_slot_leaves_room_for_the_ring_box', () => {
    // FullView's strip slot: its 16:9 tile, less the 2px borders, holds the 36px avatar with its 6px margins, the gap
    // and the 12px name row
    const slot = Number(/\.slot\s*\{\s*flex:\s*0 0 (\d+)px/.exec(readFileSync(resolve(__dirname, 'FullView.svelte'), 'utf8'))![1])
    expect(slot / (16 / 9) - 4).toBeGreaterThanOrEqual(36 + 2 * 6 + 4 + Math.ceil(12 * 1.4))
  })

  it('the_tile_clips_on_purpose', () => {
    expect(ruleBodies('.tile')).toMatch(/overflow:\s*hidden/)
  })

  it('the_border_lights_in_the_ring_colour', () => {
    expect(ruleBodies('.tile.speaking')).toMatch(/border-color:\s*var\(--speak\)/)
    app.state = { ...emptyState(), me: { id: 'U2', username: 'sam', display_name: 'sam', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] }, ring_color: 'gold' } }
    cmp = mount(UserTile, { target: document.body, props: { entry, onMenu: () => {} } })
    flushSync()
    expect(document.querySelector<HTMLElement>('.tile')!.style.getPropertyValue('--speak')).toBe('var(--speak-gold)')
  })

  it('the_badge_stays_above_the_ring', () => {
    expect(ruleBodies('.badge')).toMatch(/z-index:\s*2/)
    expect(ruleBodies('.avatar-btn')).toMatch(/isolation:\s*isolate/)
  })
})
