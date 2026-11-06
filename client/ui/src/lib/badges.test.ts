// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invoke(...a) }))

import { badgeStore, defaultShowcase, fallbackName, handleBadgeEvent, logoClick, progressText, rarityLabel, rarityVar, resetEgg, shareText } from './badges.svelte'
import { app } from './store.svelte'
import { emptyState, setProfile } from './state'
import type { Badge } from './protocol/Badge'
import type { Profile } from './protocol/Profile'
import type { BadgeDef } from './protocol/BadgeDef'

const def = (over: Partial<BadgeDef> = {}): BadgeDef => ({
  key: 'the_yapper', name: 'The Yapper', description: 'Send messages.', icon: '💬', hidden: false, hand_given: false, stacking: false, unit: 'count',
  tiers: [{ threshold: 100, rarity: 'common', holders: 3 }, { threshold: 500, rarity: 'uncommon', holders: 1 }], progress: 412, parts: [], art: null, hint: null, ...over,
})
const badge = (key: string, rarity: Badge['rarity'], earned_at: string): Badge => ({ key, earned_at, first_earner: false, tier: 1, rarity, first_tiers: [], count: 0, hidden: false, seen: true })
const calls = (cmd: string) => invoke.mock.calls.filter((c) => c[0] === cmd)
const alex = { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const ownProfile = (badges: Badge[]) => ({ user: alex, badges, showcase: [], showcase_chosen: false }) as unknown as Profile
const earned = () => handleBadgeEvent({ t: 'BadgeEarned', d: { user_id: 'U1', badge: badge('the_yapper', 'common', '2026-11-03T00:00:00Z'), name: 'The Yapper', icon: '💬' } })
/** A promise and the functions that settle it. */
function deferred<T>() {
  let resolve!: (v: T) => void
  let reject!: (e: unknown) => void
  const promise = new Promise<T>((a, b) => ((resolve = a), (reject = b)))
  return { promise, resolve, reject }
}
const flush = () => new Promise((r) => setTimeout(r, 0))

beforeEach(() => {
  invoke.mockReset()
  invoke.mockResolvedValue(undefined)
})

describe('badge text', () => {
  it('shares read "2 of 8 have this"', () => {
    expect(shareText(2, 8)).toBe('2 of 8 have this')
    expect(shareText(1, 8)).toBe('1 of 8 has this')
  })

  it('progress counts toward the next tier, with the thing counted', () => {
    expect(progressText(def(), 412)).toBe('412 / 500 messages')
    expect(progressText(def(), 40)).toBe('40 / 100 messages')
    // every tier reached: the top one
    expect(progressText(def(), 700)).toBe('700 / 500 messages')
  })

  it('hours read as h', () => {
    const regular = def({ key: 'the_regular', unit: 'hours', tiers: [{ threshold: 10, rarity: 'common', holders: 0 }, { threshold: 50, rarity: 'uncommon', holders: 0 }] })
    expect(progressText(regular, 12)).toBe('12 / 50 h')
  })

  it('a badge without a known noun is just the numbers', () => {
    expect(progressText(def({ key: 'something_new' }), 3)).toBe('3 / 100')
  })

  it('rarity label and colour token', () => {
    expect(rarityLabel('legendary')).toBe('Legendary')
    expect(rarityVar('epic')).toBe('var(--rarity-epic)')
  })

  it('an unknown key reads in title case', () => {
    expect(fallbackName('early-bird_2026')).toBe('Early Bird 2026')
    expect(fallbackName('__')).toBe('Badge')
  })
})

describe('defaultShowcase', () => {
  it('rarest first, then the older, top 5 (the server rule)', () => {
    const held = [
      badge('a', 'common', '2026-10-01T00:00:00Z'),
      badge('b', 'mythical', '2026-10-05T00:00:00Z'),
      badge('c', 'rare', '2026-10-03T00:00:00Z'),
      badge('d', 'rare', '2026-10-02T00:00:00Z'),
      badge('e', 'epic', '2026-10-04T00:00:00Z'),
      badge('f', 'uncommon', '2026-10-06T00:00:00Z'),
    ]
    expect(defaultShowcase(held)).toEqual(['b', 'e', 'd', 'c', 'f'])
  })
})

describe('badgeStore', () => {
  it('loads the catalogue once, and again after invalidate', async () => {
    badgeStore.reset()
    invoke.mockResolvedValue({ people: 4, badges: [def()] })
    await Promise.all([badgeStore.load(), badgeStore.load()])
    expect(calls('get_badge_catalogue')).toHaveLength(1)
    expect(badgeStore.def('the_yapper')?.name).toBe('The Yapper')
    await badgeStore.load()
    expect(calls('get_badge_catalogue')).toHaveLength(1)
    await badgeStore.invalidate()
    expect(calls('get_badge_catalogue')).toHaveLength(2)
  })

  it('markSeen clears the unseen count and tells the server what was shown', async () => {
    badgeStore.reset()
    badgeStore.bumped = 2
    expect(badgeStore.unseen).toBe(2)
    badgeStore.markSeen([{ ...badge('the_yapper', 'common', '2026-11-03T00:00:00Z'), tier: 2, seen: false }])
    expect(badgeStore.unseen).toBe(0)
    expect(calls('mark_badges_seen')).toEqual([['mark_badges_seen', { badges: [{ key: 'the_yapper', tier: 2 }] }]])
  })

  it('markSeen marks only the shown badges seen in your cached profile (#129)', () => {
    badgeStore.reset()
    app.state = { ...emptyState(), me: alex } as never
    const shown = { ...badge('night_owl', 'rare', '2026-11-01T00:00:00Z'), seen: false }
    const later = { ...badge('ghost', 'rare', '2026-11-02T00:00:00Z'), seen: false }
    app.state = setProfile(app.state, ownProfile([shown, later]))
    badgeStore.markSeen([shown])
    const own = app.state.profiles['U1']!.badges
    expect(own.find((b) => b.key === 'night_owl')!.seen).toBe(true)
    expect(own.find((b) => b.key === 'ghost')!.seen).toBe(false)
    expect(badgeStore.unseen).toBe(1)
    expect(calls('mark_badges_seen')).toEqual([['mark_badges_seen', { badges: [{ key: 'night_owl', tier: 1 }] }]])
  })

  it('markSeen with nothing shown tells the server nothing', () => {
    badgeStore.reset()
    badgeStore.bumped = 1
    badgeStore.markSeen([])
    expect(calls('mark_badges_seen')).toHaveLength(0)
  })

  it('a catalogue that answers after sign-out is dropped', async () => {
    badgeStore.reset()
    const slow = deferred<unknown>()
    invoke.mockReturnValueOnce(slow.promise)
    const p = badgeStore.load()
    badgeStore.reset()
    slow.resolve({ people: 4, badges: [def()] })
    await p
    expect(badgeStore.catalogue).toBeNull()
  })

  it('your profile reloading drops the bump (another device may have marked them seen)', () => {
    badgeStore.reset()
    app.state = { ...emptyState(), me: alex } as never
    app.state = setProfile(app.state, ownProfile([]))
    earned()
    expect(badgeStore.unseen).toBe(1)
    // The reload finds it already seen.
    app.state = setProfile(app.state, ownProfile([{ ...badge('the_yapper', 'common', '2026-11-03T00:00:00Z'), seen: true }]))
    expect(badgeStore.unseen).toBe(0)
    app.state = emptyState() as never
  })

  it('a burst of unlocks refetches at most twice, the last after the burst', async () => {
    badgeStore.reset()
    app.state = { ...emptyState(), me: alex } as never
    invoke.mockImplementation(async (cmd: string) => (cmd === 'get_badge_catalogue' ? { people: 4, badges: [def()] } : cmd === 'get_profile' ? ownProfile([]) : undefined))
    await badgeStore.load()
    invoke.mockClear()
    for (let i = 0; i < 5; i++) earned()
    for (let i = 0; i < 5; i++) await flush()
    expect(calls('get_badge_catalogue')).toHaveLength(2)
    expect(calls('get_profile')).toHaveLength(2)
    app.state = emptyState() as never
  })
})

describe('the logo egg', () => {
  beforeEach(() => resetEgg())

  it('7 clicks within 3 s report it, once a session', () => {
    for (let i = 0; i < 7; i++) logoClick(1000 + i * 400)
    expect(calls('report_egg')).toEqual([['report_egg', { egg: 'logo' }]])
    for (let i = 0; i < 7; i++) logoClick(10_000 + i * 100)
    expect(calls('report_egg')).toHaveLength(1)
  })

  it('a failed report can be tried again; a reported one waits for the next session', async () => {
    invoke.mockRejectedValueOnce(new Error('offline'))
    for (let i = 0; i < 7; i++) logoClick(1000 + i * 100)
    await flush()
    for (let i = 0; i < 7; i++) logoClick(5000 + i * 100)
    await flush()
    expect(calls('report_egg')).toHaveLength(2)
    for (let i = 0; i < 7; i++) logoClick(9000 + i * 100)
    expect(calls('report_egg')).toHaveLength(2)
    // Signing out (badgeStore.reset) is a new session.
    badgeStore.reset()
    for (let i = 0; i < 7; i++) logoClick(13_000 + i * 100)
    expect(calls('report_egg')).toHaveLength(3)
  })

  it('slower clicks never do', () => {
    for (let i = 0; i < 20; i++) logoClick(i * 600)
    expect(calls('report_egg')).toHaveLength(0)
  })
})
