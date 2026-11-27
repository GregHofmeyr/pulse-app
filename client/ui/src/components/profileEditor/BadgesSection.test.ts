// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => undefined) }))

import BadgesSection from './BadgesSection.svelte'
import { badgeStore } from '../../lib/badges.svelte'
import { ProfileEditorState } from '../../lib/profileEditor.svelte'
import { app } from '../../lib/store.svelte'
import { emptyState } from '../../lib/state'
import type { Badge } from '../../lib/protocol/Badge'
import type { BadgeDef } from '../../lib/protocol/BadgeDef'
import type { BadgeRarity } from '../../lib/protocol/BadgeRarity'
import type { Profile } from '../../lib/protocol/Profile'
import type { User } from '../../lib/protocol/User'

const alex: User = { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const held = (key: string, rarity: BadgeRarity, n: number): Badge => ({ key, earned_at: `2026-10-0${n}T00:00:00Z`, first_earner: false, tier: 1, rarity, first_tiers: [], count: 0, hidden: false, seen: true })
const def = (key: string, name: string, holders: number, one = false): BadgeDef => ({
  key, name, description: '', icon: '⭐', hidden: false, hand_given: false, one_of_a_kind: one, stacking: false, unit: 'count' as never,
  tiers: [{ threshold: 1, rarity: 'common', holders }], progress: null, parts: [], art: null, hint: null,
})
// rarest first: f mythical, e legendary, d epic, c rare, b uncommon, a common
const BADGES = [held('a', 'common', 1), held('b', 'uncommon', 2), held('c', 'rare', 3), held('d', 'epic', 4), held('e', 'legendary', 5), held('f', 'mythical', 6)]
const profile = (showcase: string[] | null = ['a', 'b']): Profile => ({
  user: alex,
  card: { banner_hash: null, banner_still: null, theme_top: '#3a3d46', theme_bottom: '#16171b', pronouns: null, bio: null },
  stats: { voice_seconds: 0, member_since: '2026-10-03T00:00:00Z', top_reaction: null, times_renamed: 0 },
  badges: BADGES,
  name_archive: [],
  showcase: showcase ?? [],
  showcase_chosen: showcase !== null,
})

let cmp: ReturnType<typeof mount> | undefined
let st: ProfileEditorState
beforeEach(() => {
  app.state = { ...emptyState(), me: alex } as never
  badgeStore.catalogue = {
    people: 40,
    badges: [def('a', 'Alpha', 30), def('b', 'Bravo', 12), def('c', 'Charlie', 1), def('d', 'Delta', 7), def('e', 'Echo', 3), def('f', 'Foxtrot Very Long Badge Name Indeed', 1, true)],
  }
  badgeStore.owner = 'U1'
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  badgeStore.catalogue = null
  badgeStore.owner = null
  document.body.innerHTML = ''
})

function show(p = profile()) {
  st = new ProfileEditorState(p)
  cmp = mount(BadgesSection, { target: document.body, props: { state: st } })
  flushSync()
}
const q = <T extends Element = HTMLElement>(sel: string) => document.querySelector<T>(sel)!
const qa = (sel: string) => [...document.querySelectorAll<HTMLElement>(sel)]
const click = (el: Element) => { (el as HTMLElement).click(); flushSync() }
const slotKeys = () => qa('.slots [data-badge]').map((b) => b.dataset.badge)
const tile = (key: string) => q(`[data-tile="${key}"]`)
const chip = () => q('button.chip')

// jsdom has no PointerEvent or layout: a MouseEvent with a pointerId stands in, and rects are given by hand
function pointer(type: string, el: Element | Window, x: number, y = 0) {
  const e = new MouseEvent(type, { bubbles: true, cancelable: true, clientX: x, clientY: y, button: 0 })
  Object.defineProperty(e, 'pointerId', { value: 1 })
  el.dispatchEvent(e)
}
const rect = (left: number, top: number, w: number, h: number) => ({ left, top, right: left + w, bottom: top + h, width: w, height: h, x: left, y: top, toJSON: () => ({}) })
/** Slots at x = 0, 100, 200, 300, 400 (84 wide), y 0–84; the grid sits below at y 200+. */
function layout() {
  q('.slots').getBoundingClientRect = () => rect(0, 0, 484, 84)
  qa('.slots > li').forEach((li, i) => (li.getBoundingClientRect = () => rect(i * 100, 0, 84, 84)))
  qa('.slots [data-badge]').forEach((b, i) => (b.getBoundingClientRect = () => rect(i * 100, 0, 84, 84)))
  qa('[data-tile]').forEach((t, i) => (t.getBoundingClientRect = () => rect(i * 120, 200, 110, 120)))
}

describe('the Badges section', () => {
  it('five_slots_in_a_row_with_dashed_empty_slots', () => {
    show()
    const slots = qa('.slots > li')
    expect(slots).toHaveLength(5)
    expect(slotKeys()).toEqual(['a', 'b'])
    expect(slots.map((s) => s.classList.contains('empty'))).toEqual([false, false, true, true, true])
  })

  it('click_in_grid_adds_or_removes', () => {
    show()
    click(tile('c'))
    expect(slotKeys()).toEqual(['a', 'b', 'c'])
    expect(st.showcase).toEqual(['a', 'b', 'c'])
    click(tile('a'))
    expect(slotKeys()).toEqual(['b', 'c'])
    // a full row takes no more
    click(tile('a'))
    click(tile('d'))
    click(tile('e'))
    expect(slotKeys()).toEqual(['b', 'c', 'a', 'd', 'e'])
    expect((tile('f') as HTMLButtonElement).disabled).toBe(true)
    // clicking a slot takes it off
    click(q('.slots [data-badge="c"]'))
    expect(slotKeys()).toEqual(['b', 'a', 'd', 'e'])
  })

  it('drag_reorders_slots', () => {
    show()
    layout()
    const a = q('.slots [data-badge="a"]')
    pointer('pointerdown', a, 40, 40)
    pointer('pointermove', q('.slots'), 150, 40)
    pointer('pointerup', q('.slots'), 150, 40)
    flushSync()
    expect(st.showcase).toEqual(['b', 'a'])
  })

  it('a_vertical_wiggle_inside_the_row_keeps_the_badge', () => {
    show()
    layout()
    const a = q('.slots [data-badge="a"]')
    pointer('pointerdown', a, 40, 40)
    pointer('pointermove', q('.slots'), 40, 70)
    pointer('pointerup', q('.slots'), 40, 70)
    click(a)
    expect(st.showcase).toEqual(['a', 'b'])
    // a plain click afterwards still takes it off
    pointer('pointerdown', a, 40, 40)
    pointer('pointerup', a, 40, 40)
    click(a)
    expect(st.showcase).toEqual(['b'])
  })

  it('drag_out_removes', () => {
    show()
    layout()
    const a = q('.slots [data-badge="a"]')
    pointer('pointerdown', a, 40, 40)
    pointer('pointermove', q('.slots'), 40, 220)
    pointer('pointerup', q('.slots'), 40, 220)
    flushSync()
    expect(st.showcase).toEqual(['b'])
    // dropped back within the row it stays
    const b = q('.slots [data-badge="b"]')
    layout()
    pointer('pointerdown', b, 40, 40)
    pointer('pointermove', q('.slots'), 60, 50)
    pointer('pointerup', q('.slots'), 60, 50)
    expect(st.showcase).toEqual(['b'])
  })

  it('drag_from_the_grid_drops_into_the_row', () => {
    show()
    layout()
    // tile c over the second slot's place
    pointer('pointerdown', tile('c'), 250, 250)
    pointer('pointermove', window, 110, 40)
    pointer('pointerup', window, 110, 40)
    flushSync()
    expect(st.showcase).toEqual(['a', 'c', 'b'])
    // a drop that misses the row does nothing, and the click it ends with is swallowed
    layout()
    pointer('pointerdown', tile('d'), 370, 250)
    pointer('pointermove', window, 380, 260)
    pointer('pointerup', window, 380, 260)
    click(tile('d'))
    expect(st.showcase).toEqual(['a', 'c', 'b'])
  })

  it('rarest_five_chip_sets_auto', () => {
    show()
    expect(chip().getAttribute('aria-pressed')).toBe('false')
    click(chip())
    expect(st.showcase).toBe('auto')
    expect(chip().getAttribute('aria-pressed')).toBe('true')
    // the slots follow the rarest five
    expect(slotKeys()).toEqual(['f', 'e', 'd', 'c', 'b'])
    // an edit while auto starts from them; the chip lets go
    click(q('.slots [data-badge="f"]'))
    expect(st.showcase).toEqual(['e', 'd', 'c', 'b'])
    expect(chip().getAttribute('aria-pressed')).toBe('false')
    // pressing it again goes back to auto
    click(chip())
    expect(st.showcase).toBe('auto')
  })

  it('grid_tile_shows_name_and_holder_count_and_outline_when_on_card', () => {
    show()
    expect(tile('a').textContent).toContain('Alpha')
    expect(tile('a').textContent).toContain('30 have this')
    expect(tile('c').textContent).toContain('1 has this')
    expect(tile('f').textContent).toContain('1 of 1')
    expect(tile('a').classList.contains('on')).toBe(true)
    expect(tile('a').getAttribute('aria-pressed')).toBe('true')
    expect(tile('c').classList.contains('on')).toBe(false)
    expect(tile('c').getAttribute('aria-pressed')).toBe('false')
    // names ellipsise in their own element
    expect(tile('f').querySelector('.name')!.textContent).toBe('Foxtrot Very Long Badge Name Indeed')
  })

  it('without_a_catalogue_no_count_is_invented', () => {
    badgeStore.catalogue = null
    show()
    expect(tile('a').textContent).not.toMatch(/have this|has this/)
    expect(tile('a').textContent).toContain('A') // the fallback name
  })

  it('arrow_keys_reorder', () => {
    show(profile(['a', 'b', 'c']))
    const a = q<HTMLElement>('.slots [data-badge="a"]')
    a.focus()
    a.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true, cancelable: true }))
    flushSync()
    expect(st.showcase).toEqual(['b', 'a', 'c'])
    expect(document.activeElement).toBe(q('.slots [data-badge="a"]'))
    q('.slots [data-badge="a"]').dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowLeft', bubbles: true, cancelable: true }))
    flushSync()
    expect(st.showcase).toEqual(['a', 'b', 'c'])
    // the end stays the end
    q('.slots [data-badge="a"]').dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowLeft', bubbles: true, cancelable: true }))
    flushSync()
    expect(st.showcase).toEqual(['a', 'b', 'c'])
  })

  it('with_none_earned_it_says_so', () => {
    show({ ...profile(), badges: [], showcase: [], showcase_chosen: false })
    expect(document.body.textContent).toContain('No badges yet')
  })

  it('edits_stay_in_the_state_until_save', async () => {
    const { invoke } = await import('@tauri-apps/api/core')
    show()
    click(tile('c'))
    expect(st.dirty).toBe(true)
    expect(invoke).not.toHaveBeenCalledWith(expect.stringMatching(/showcase/i), expect.anything())
  })
})
