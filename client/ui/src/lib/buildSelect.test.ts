import { describe, expect, it } from 'vitest'
import { BULK_ACTIONS, barKeys, batchChunks, bulkPlan, bulkSummary, buildMenuItems, rangeSelected, toggleSelected } from './buildSelect'
import { applyReady, emptyState } from './state'
import { testReady } from './testReady'
import type { Message } from './protocol/Message'

const msg = (id: string, author: string, archived = false, favourite = false): Message =>
  ({ id, channel_id: 'B1', author_id: author, build: { title: id, archived_at: archived ? '2026-10-01T00:00:00Z' : null, favourite } }) as unknown as Message
const state = (owner: string | null = null) =>
  applyReady(emptyState(), testReady([], { servers: [{ id: 'S1', name: 'Main', icon_hash: null, ...(owner ? { owner_id: owner } : {}) } as never] }))
const action = (k: string) => BULK_ACTIONS.find((a) => a.key === k)!

describe('buildMenuItems', () => {
  it('offers Edit to the author, Archive and Delete to the author or owner, Open, Favourite and Select to anyone', () => {
    expect(buildMenuItems({ edit: true, archive: true, delete: true }, false, false)).toEqual(['open', 'favourite', 'edit', 'archive', 'delete', 'select'])
    expect(buildMenuItems({ edit: false, archive: true, delete: true }, true, true)).toEqual(['open', 'unfavourite', 'restore', 'delete', 'select'])
    expect(buildMenuItems({ edit: false, archive: false, delete: false }, false, false)).toEqual(['open', 'favourite', 'select'])
  })
})

describe('selection', () => {
  it('toggles a build in and out', () => {
    expect(toggleSelected(['a'], 'b')).toEqual(['a', 'b'])
    expect(toggleSelected(['a', 'b'], 'a')).toEqual(['b'])
  })
  it('a range adds every build from the anchor to the one clicked, either way, keeping what was selected', () => {
    const order = ['a', 'b', 'c', 'd', 'e']
    expect(rangeSelected(['a'], order, 'b', 'd')).toEqual(['a', 'b', 'c', 'd'])
    expect(rangeSelected([], order, 'd', 'b')).toEqual(['b', 'c', 'd'])
    // no anchor (or one no longer shown): just that build
    expect(rangeSelected(['a'], order, null, 'c')).toEqual(['a', 'c'])
    expect(rangeSelected(['a'], order, 'zz', 'c')).toEqual(['a', 'c'])
  })
})

describe('bulkPlan', () => {
  it('sends the builds you may change and counts the rest as skipped', () => {
    const s = state()
    const sel = [msg('a', 'U1'), msg('b', 'U2'), msg('c', 'U1', true)]
    expect(bulkPlan(s, 'S1', sel, action('archive'))).toEqual({ ids: ['a'], skipped: 1 })
    expect(bulkPlan(s, 'S1', sel, action('restore'))).toEqual({ ids: ['c'], skipped: 0 })
    expect(bulkPlan(s, 'S1', sel, action('delete'))).toEqual({ ids: ['a', 'c'], skipped: 1 })
  })
  it('a star is yours to give any build: nothing is skipped', () => {
    const sel = [msg('a', 'U2'), msg('b', 'U2', false, true), msg('c', 'U1')]
    expect(bulkPlan(state(), 'S1', sel, action('favourite'))).toEqual({ ids: ['a', 'c'], skipped: 0 })
    expect(bulkPlan(state(), 'S1', sel, action('unfavourite'))).toEqual({ ids: ['b'], skipped: 0 })
  })
  it('the bar shows an action when it applies to a selected build, and the base ones with none selected', () => {
    const keys = (sel: Message[]) => barKeys(sel)
    expect(keys([])).toEqual(['favourite', 'archive', 'delete'])
    expect(keys([msg('a', 'U1', true, true)])).toEqual(['unfavourite', 'restore', 'delete'])
    expect(keys([msg('a', 'U1'), msg('b', 'U1', true, true)])).toEqual(['favourite', 'unfavourite', 'archive', 'restore', 'delete'])
  })
  it("the server's owner may change everyone's", () => {
    expect(bulkPlan(state('U1'), 'S1', [msg('a', 'U1'), msg('b', 'U2')], action('delete'))).toEqual({ ids: ['a', 'b'], skipped: 0 })
  })
})

describe('batches', () => {
  it('splits into the server cap of 100', () => {
    const ids = Array.from({ length: 230 }, (_, i) => `M${i}`)
    expect(batchChunks(ids).map((c) => c.length)).toEqual([100, 100, 30])
    expect(batchChunks([])).toEqual([])
  })
  it('says what was done and how many were skipped', () => {
    expect(bulkSummary(action('archive'), 1, 0)).toBe('Archived 1 build.')
    expect(bulkSummary(action('delete'), 3, 2)).toBe("Deleted 3 builds. 2 skipped: they aren't yours to change.")
    expect(bulkSummary(action('favourite'), 2, 1)).toBe('Added 2 builds to your favourites. 1 skipped: gone meanwhile.')
  })
})
