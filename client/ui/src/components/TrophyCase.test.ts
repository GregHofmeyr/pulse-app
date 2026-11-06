// @vitest-environment jsdom
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invoke(...a) }))

import TrophyCase from './TrophyCase.svelte'
import { badgeStore } from '../lib/badges.svelte'
import { app, resetState } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import type { Badge } from '../lib/protocol/Badge'
import type { BadgeCatalogue } from '../lib/protocol/BadgeCatalogue'
import type { BadgeDef } from '../lib/protocol/BadgeDef'

const alex = { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const def = (key: string, over: Partial<BadgeDef> = {}): BadgeDef => ({
  key, name: key, description: `How to earn ${key}.`, icon: '⭐', hidden: false, hand_given: false, stacking: false, unit: 'count',
  tiers: [{ threshold: 1, rarity: 'common', holders: 0 }], progress: null, parts: [], art: null, hint: null, ...over,
})
const CATALOGUE: BadgeCatalogue = {
  people: 8,
  badges: [
    def('the_yapper', { name: 'The Yapper', icon: '💬', tiers: [{ threshold: 100, rarity: 'common', holders: 2 }, { threshold: 500, rarity: 'uncommon', holders: 1 }], progress: 412 }),
    def('the_regular', { name: 'The Regular', icon: '🎧', unit: 'hours', tiers: [{ threshold: 10, rarity: 'common', holders: 3 }, { threshold: 50, rarity: 'uncommon', holders: 0 }], progress: 12 }),
    def('quote_of_the_year', { name: 'Quote of the Year', icon: '🏆', stacking: true, tiers: [{ threshold: 1, rarity: 'legendary', holders: 1 }], progress: 2 }),
    def('night_owl', { name: 'Night Owl', icon: '🦉', tiers: [{ threshold: 1, rarity: 'rare', holders: 0 }], progress: 0 }),
    def('hidden:0', { name: '???', description: '', icon: '❔', hidden: true, hint: 'The logo up top is ticklish. Keep at it.', tiers: [{ threshold: 0, rarity: 'epic', holders: 1 }] }),
    def('founder', { name: 'Founder', icon: '👑', hand_given: true, tiers: [{ threshold: 1, rarity: 'mythical', holders: 1 }] }),
  ],
}
const badge = (key: string, over: Partial<Badge> = {}): Badge => ({ key, earned_at: '2026-10-03T00:00:00Z', first_earner: false, tier: 1, rarity: 'common', first_tiers: [], count: 0, hidden: false, seen: true, ...over })
const calls = (cmd: string) => invoke.mock.calls.filter((c) => c[0] === cmd)

let cmp: ReturnType<typeof mount> | undefined
async function show(props: { badges: Badge[]; mine?: boolean; showcase?: string[]; showcaseChosen?: boolean }) {
  cmp = mount(TrophyCase, { target: document.body, props: { mine: false, showcase: [], ...props } })
  flushSync()
  await vi.waitFor(() => expect(document.querySelector('.tile')).not.toBeNull())
  flushSync()
}
const tile = (name: string) => [...document.querySelectorAll<HTMLElement>('.tile')].find((t) => t.querySelector('.nm')?.textContent === name)
const button = (text: string) => [...document.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent?.trim() === text)!

beforeEach(() => {
  resetState()
  app.state = { ...emptyState(), me: alex } as never
  badgeStore.reset()
  invoke.mockReset()
  invoke.mockImplementation(async (cmd: string) => (cmd === 'get_badge_catalogue' ? CATALOGUE : undefined))
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('TrophyCase', () => {
  it('earned first: tier, rarity, date, First!, the share', async () => {
    await show({ badges: [badge('the_yapper', { tier: 2, rarity: 'uncommon', first_earner: true, first_tiers: [2] })] })
    const tiles = [...document.querySelectorAll('.tile')]
    expect(tiles[0].querySelector('.nm')!.textContent).toBe('The Yapper')
    const t = tile('The Yapper')!
    expect(t.classList.contains('locked')).toBe(false)
    expect(t.textContent).toContain('Uncommon')
    expect(t.querySelector('.tier')!.textContent).toBe('II')
    expect(t.textContent).toContain('Oct 3, 2026')
    expect(t.textContent).toContain('First!')
    expect(t.textContent).toContain('1 of 8 has this')
  })

  it('hidden badges not yet earned ("???") come last, after earned and locked; the rest keep their order (#131)', async () => {
    invoke.mockImplementation(async (cmd: string) => (cmd === 'get_badge_catalogue'
      ? { ...CATALOGUE, badges: [
          def('hidden:0', { name: '???', description: '', icon: '❔', hidden: true, tiers: [{ threshold: 0, rarity: 'epic', holders: 1 }] }),
          CATALOGUE.badges[0], CATALOGUE.badges[1],
          def('hidden:1', { name: '???', description: '', icon: '❔', hidden: true, tiers: [{ threshold: 0, rarity: 'rare', holders: 0 }] }),
          CATALOGUE.badges[2], CATALOGUE.badges[3],
        ] }
      : undefined))
    await show({ badges: [badge('night_owl', { rarity: 'rare' })] })
    const order = [...document.querySelectorAll('.tile')].map((t) => t.querySelector('.badge-icon')!.getAttribute('aria-label') + (t.querySelector('.rar')!.textContent === 'Rare' ? ':rare' : ''))
    expect(order).toEqual(['Night Owl:rare', 'The Yapper', 'The Regular', 'Quote of the Year', '???', '???:rare'])
  })

  it('a stacking badge shows ×N', async () => {
    await show({ badges: [badge('quote_of_the_year', { rarity: 'legendary', count: 2 })] })
    expect(tile('Quote of the Year')!.textContent).toContain('×2')
  })

  it('locked: greyed, how to earn, and on your own a progress bar', async () => {
    await show({ badges: [], mine: true })
    const y = tile('The Yapper')!
    expect(y.classList.contains('locked')).toBe(true)
    expect(y.textContent).toContain('How to earn the_yapper.')
    expect(y.textContent).toContain('412 / 500 messages')
    expect(y.querySelector('progress, [role="progressbar"]')).not.toBeNull()
    expect(tile('The Regular')!.textContent).toContain('12 / 50 h')
    expect(tile('The Regular')!.textContent).toContain('3 of 8 have this')
  })

  it('progress bars say which badge they belong to', async () => {
    await show({ badges: [], mine: true })
    expect(tile('The Yapper')!.querySelector('progress')!.getAttribute('aria-label')).toBe('The Yapper progress')
  })

  it("a masked badge on someone else's profile shows no date and no First!", async () => {
    await show({ badges: [badge('hidden:0', { hidden: true, rarity: 'epic', earned_at: '' })] })
    const t = [...document.querySelectorAll<HTMLElement>('.tile')].find((x) => !x.classList.contains('locked') && x.querySelector('.nm')?.textContent === '???')!
    expect(t.querySelector('.when')).toBeNull()
    expect(t.textContent).not.toContain('First!')
    expect(t.textContent).not.toContain('Invalid')
  })

  it("the showcase editor keeps the default unless you change it", async () => {
    const held = [badge('the_yapper'), badge('night_owl', { rarity: 'rare' })]
    await show({ badges: held, mine: true, showcase: ['night_owl', 'the_yapper'], showcaseChosen: false })
    button('Edit showcase').click()
    flushSync()
    button('Save').click()
    await tick()
    // Still "rarest five": it follows later unlocks instead of pinning today's pick.
    expect(calls('set_badge_showcase')).toEqual([['set_badge_showcase', { keys: [] }]])
  })

  it('a showcase you chose stays chosen on a save without changes', async () => {
    const held = [badge('the_yapper'), badge('night_owl', { rarity: 'rare' })]
    await show({ badges: held, mine: true, showcase: ['the_yapper'], showcaseChosen: true })
    button('Edit showcase').click()
    flushSync()
    button('Save').click()
    await tick()
    expect(calls('set_badge_showcase')).toEqual([['set_badge_showcase', { keys: ['the_yapper'] }]])
  })

  describe('Combo: one tier at a time, each stat with its own bar (#191)', () => {
    const combo = (parts: BadgeDef['parts']) => def('combo', {
      name: 'Combo', icon: '🔗', description: 'Messages, hours in voice and clips, all at once.',
      tiers: [{ threshold: 100, rarity: 'uncommon', holders: 1 }, { threshold: 500, rarity: 'epic', holders: 0 }, { threshold: 1000, rarity: 'mythical', holders: 0 }],
      parts,
    })
    const next = [{ label: 'Messages', value: 1200, target: 500 }, { label: 'Hours in voice', value: 42, target: 500 }, { label: 'Clips', value: 3, target: 25 }]
    const withCombo = (parts: BadgeDef['parts']) =>
      invoke.mockImplementation(async (cmd: string) => (cmd === 'get_badge_catalogue' ? { ...CATALOGUE, badges: [...CATALOGUE.badges, combo(parts)] } : undefined))
    const lines = (t: HTMLElement) => [...t.querySelectorAll('.part .prog')].map((p) => p.textContent?.trim())

    it('your own: the next tier only, as a short list, each with a bar like the others', async () => {
      withCombo(next)
      await show({ badges: [badge('combo', { tier: 1, rarity: 'uncommon' })], mine: true })
      const t = tile('Combo')!
      expect(t.textContent).toContain('Messages, hours in voice and clips, all at once.')
      expect(lines(t)).toEqual(['Messages — 1,200 / 500', 'Hours in voice — 42 / 500', 'Clips — 3 / 25'])
      const bars = [...t.querySelectorAll<HTMLProgressElement>('.part progress')]
      expect(bars.map((b) => [b.value, b.max])).toEqual([[500, 500], [42, 500], [3, 25]])
      expect(bars.map((b) => b.getAttribute('aria-label'))).toEqual(['Combo progress: Messages', 'Combo progress: Hours in voice', 'Combo progress: Clips'])
    })

    it('locked, on your own: the first tier', async () => {
      withCombo(next.map((p) => ({ ...p, target: p.label === 'Clips' ? 10 : 100 })))
      await show({ badges: [], mine: true })
      expect(lines(tile('Combo')!)).toEqual(['Messages — 1,200 / 100', 'Hours in voice — 42 / 100', 'Clips — 3 / 10'])
    })

    it('every tier earned: complete, no list, like other maxed badges', async () => {
      withCombo([])
      await show({ badges: [badge('combo', { tier: 3, rarity: 'mythical' })], mine: true })
      expect(tile('Combo')!.querySelector('.part, progress')).toBeNull()
    })

    it("someone else's case: no list (the numbers are yours)", async () => {
      withCombo(next)
      await show({ badges: [] })
      expect(tile('Combo')!.querySelector('.part, progress')).toBeNull()
    })
  })

  it("someone else's case shows no progress (the numbers are yours)", async () => {
    await show({ badges: [] })
    expect(tile('The Yapper')!.textContent).not.toContain('412')
  })

  it('a masked badge is ??? with its rarity; hand-given only when held', async () => {
    await show({ badges: [] })
    const masked = tile('???')!
    expect(masked.textContent).toContain('Epic')
    expect(tile('Founder')).toBeUndefined()
    unmount(cmp!)
    cmp = undefined
    await show({ badges: [badge('founder', { rarity: 'mythical' })] })
    expect(tile('Founder')).not.toBeUndefined()
  })

  it('your own "???" tile shows its hint, styled as any description; nothing else gives it away (#211)', async () => {
    await show({ badges: [], mine: true })
    const masked = tile('???')!
    const desc = masked.querySelector('.desc')!
    expect(desc.textContent).toBe('The logo up top is ticklish. Keep at it.')
    expect(masked.className).toBe(tile('Night Owl')!.className)
    expect(masked.querySelector('.nm')!.textContent).toBe('???')
  })

  it("someone else's case keeps the hint to you: Hidden until earned", async () => {
    await show({ badges: [] })
    expect(tile('???')!.querySelector('.desc')!.textContent).toBe('Hidden until earned')
  })

  it('opening your own case marks badges seen, keeping [NEW] while open', async () => {
    await show({ badges: [badge('the_yapper', { seen: false, tier: 2 }), badge('night_owl', { rarity: 'rare' })], mine: true })
    // Only what this case showed [NEW], at the tier shown (#129).
    expect(calls('mark_badges_seen')).toEqual([['mark_badges_seen', { badges: [{ key: 'the_yapper', tier: 2 }] }]])
    expect(tile('The Yapper')!.querySelector('.new')).not.toBeNull()
    expect(tile('Night Owl')!.querySelector('.new')).toBeNull()
  })

  it("someone else's case never marks anything seen", async () => {
    await show({ badges: [badge('the_yapper', { seen: false })] })
    expect(calls('mark_badges_seen')).toHaveLength(0)
  })

  it('the showcase editor saves the chosen order', async () => {
    await show({ badges: [badge('the_yapper'), badge('night_owl', { rarity: 'rare' }), badge('quote_of_the_year', { rarity: 'legendary', count: 1 })], mine: true, showcase: ['night_owl'] })
    button('Edit showcase').click()
    flushSync()
    // click an earned badge to add it; then move it first
    tile('The Yapper')!.querySelector<HTMLButtonElement>('button.pick')!.click()
    flushSync()
    const chosen = () => [...document.querySelectorAll('.chosen li')].map((li) => li.getAttribute('data-key'))
    expect(chosen()).toEqual(['night_owl', 'the_yapper'])
    document.querySelector<HTMLButtonElement>('.chosen li[data-key="the_yapper"] button[aria-label="Move The Yapper left"]')!.click()
    flushSync()
    expect(chosen()).toEqual(['the_yapper', 'night_owl'])
    button('Save').click()
    await tick()
    expect(calls('set_badge_showcase')).toEqual([['set_badge_showcase', { keys: ['the_yapper', 'night_owl'] }]])
  })

  it('the showcase holds at most 5', async () => {
    const many = ['a', 'b', 'c', 'd', 'e', 'f']
    invoke.mockImplementation(async (cmd: string) => (cmd === 'get_badge_catalogue' ? { people: 2, badges: many.map((k) => def(k)) } : undefined))
    await show({ badges: many.map((k) => badge(k)), mine: true, showcase: [] })
    button('Edit showcase').click()
    flushSync()
    for (const k of many) tile(k)!.querySelector<HTMLButtonElement>('button.pick')!.click()
    flushSync()
    expect(document.querySelectorAll('.chosen li')).toHaveLength(5)
  })
})
