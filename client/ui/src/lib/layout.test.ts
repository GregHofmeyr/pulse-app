import { describe, expect, it } from 'vitest'
import { NARROW_BODY, applyOrder, moveCategory, moveChannel, orderOf, searchMakesRoom, sidebarLayout, stepCategory, stepChannel, toRequest, type Order } from './layout'
import { emptyState, type AppState } from './state'
import type { Category } from './protocol/Category'
import type { Channel } from './protocol/Channel'

// #97: in a narrow window only one side panel shows. A replies panel (a quote's or a clip's) opened from search
// hides the results for as long as it's open; closing it brings them back as they were.
describe('searchMakesRoom', () => {
  it('hides the results only when narrow, searching and a replies panel is open', () => {
    expect(searchMakesRoom({ narrow: true, searchOpen: true, threadOpen: true })).toBe(true)
    expect(searchMakesRoom({ narrow: false, searchOpen: true, threadOpen: true })).toBe(false)
    expect(searchMakesRoom({ narrow: true, searchOpen: false, threadOpen: true })).toBe(false)
    expect(searchMakesRoom({ narrow: true, searchOpen: true, threadOpen: false })).toBe(false)
  })
  it('narrow means the window body (sidebar + chat + panels) is under 1100 px', () => {
    expect(NARROW_BODY).toBe(1100)
  })
})

// Categories (#54): the sidebar's channel list, and the moves made on it by drag and Alt+arrows.
const cat = (id: string, position: number, name = id): Category => ({ id, server_id: 'S1', name, position })
const ch = (id: string, position: number, category_id?: string, kind: Channel['kind'] = 'text', server_id: string | null = 'S1'): Channel => ({
  id, server_id, kind, name: id.toLowerCase(), position, ...(category_id ? { category_id } : {}),
})
const channels = [
  ch('A', 1), ch('B', 0), ch('X', 5, 'GONE'), // X's category is unknown: it sits at the top
  ch('D', 1, 'K1', 'voice'), ch('C', 0, 'K1'),
  ch('E', 0, 'K2', 'quotes'),
  ch('OTHER', 0, undefined, 'text', 'S2'), ch('DM', 0, undefined, 'dm', null),
]
function state(live = true): AppState {
  return {
    ...emptyState(),
    channels: Object.fromEntries(channels.map((c) => [c.id, c])),
    layouts: live ? { S1: { revision: 7, categories: [cat('K3', 2, 'Empty'), cat('K1', 0, 'Gaming'), cat('K2', 1, 'Chat')] } } : {},
  }
}
const ids = (cs: Channel[]) => cs.map((c) => c.id)
const base: Order = { top: ['B', 'A', 'X'], cats: [{ id: 'K1', channels: ['C', 'D'] }, { id: 'K2', channels: ['E'] }, { id: 'K3', channels: [] }] }

describe('sidebarLayout', () => {
  it('live: uncategorised channels on top, then each category in order, each by position then name', () => {
    const l = sidebarLayout(state(), 'S1')
    expect(l.live).toBe(true)
    expect(l.revision).toBe(7)
    expect(ids(l.top)).toEqual(['B', 'A', 'X'])
    expect(l.sections.map((x) => [x.key, x.name, x.kind, x.category?.id, ids(x.channels)])).toEqual([
      ['cat:K1', 'Gaming', null, 'K1', ['C', 'D']],
      ['cat:K2', 'Chat', null, 'K2', ['E']],
      ['cat:K3', 'Empty', null, 'K3', []],
    ])
  })

  it('a tie in position goes by name', () => {
    const s = state()
    s.channels = { ...s.channels, Z: { ...ch('Z', 0), name: 'aaa' } }
    expect(ids(sidebarLayout(s, 'S1').top)).toEqual(['Z', 'B', 'A', 'X'])
  })

  it('legacy (an older server, no layout): the four kind sections, always all four', () => {
    const l = sidebarLayout(state(false), 'S1')
    expect(l.live).toBe(false)
    expect(l.top).toEqual([])
    expect(l.sections.map((x) => [x.key, x.name, x.kind, x.category, ids(x.channels)])).toEqual([
      ['S1:text', 'Text Channels', 'text', null, ['B', 'C', 'A', 'X']],
      ['S1:quotes', 'Quotes', 'quotes', null, ['E']],
      ['S1:clips', 'Clips', 'clips', null, []],
      ['S1:voice', 'Voice Channels', 'voice', null, ['D']],
    ])
  })
})

describe('orderOf / applyOrder / toRequest', () => {
  it('orderOf reads the shown order', () => {
    expect(orderOf(sidebarLayout(state(), 'S1'))).toEqual(base)
  })

  it('applyOrder ∘ orderOf is the identity', () => {
    for (const live of [true, false]) {
      const l = sidebarLayout(state(live), 'S1')
      expect(applyOrder(l, orderOf(l))).toEqual(l)
    }
  })

  it('applyOrder re-sorts; what the order does not name keeps its place at the end', () => {
    const l = sidebarLayout(state(), 'S1')
    const out = applyOrder(l, { top: ['A', 'E'], cats: [{ id: 'K2', channels: ['C'] }, { id: 'NOPE', channels: ['B'] }] })
    expect(ids(out.top)).toEqual(['A', 'E', 'B', 'X'])
    expect(out.sections.map((x) => [x.key, ids(x.channels)])).toEqual([
      ['cat:K2', ['C']],
      ['cat:K1', ['D']],
      ['cat:K3', []],
    ])
  })

  it('toRequest is the save body', () => {
    expect(toRequest(base, 7)).toEqual({
      revision: 7,
      uncategorised: ['B', 'A', 'X'],
      categories: [{ id: 'K1', channels: ['C', 'D'] }, { id: 'K2', channels: ['E'] }, { id: 'K3', channels: [] }],
    })
  })
})

describe('moves', () => {
  it('within a category', () => {
    expect(moveChannel(base, 'D', { category: 'K1', index: 0 }).cats[0].channels).toEqual(['D', 'C'])
    expect(moveChannel(base, 'B', { category: null, index: 2 }).top).toEqual(['A', 'X', 'B'])
  })

  it('between categories', () => {
    const o = moveChannel(base, 'C', { category: 'K2', index: 1 })
    expect(o.cats[0].channels).toEqual(['D'])
    expect(o.cats[1].channels).toEqual(['E', 'C'])
  })

  it('out of a category to the top, and into one from the top', () => {
    const out = moveChannel(base, 'E', { category: null, index: 0 })
    expect(out.top).toEqual(['E', 'B', 'A', 'X'])
    expect(out.cats[1].channels).toEqual([])
    const into = moveChannel(base, 'A', { category: 'K1', index: 1 })
    expect(into.top).toEqual(['B', 'X'])
    expect(into.cats[0].channels).toEqual(['C', 'A', 'D'])
  })

  it('into an empty category; an index past the end goes last', () => {
    expect(moveChannel(base, 'C', { category: 'K3', index: 0 }).cats[2].channels).toEqual(['C'])
    expect(moveChannel(base, 'B', { category: 'K1', index: 99 }).cats[0].channels).toEqual(['C', 'D', 'B'])
  })

  it('an unknown channel or category changes nothing', () => {
    expect(moveChannel(base, 'NOPE', { category: 'K1', index: 0 })).toEqual(base)
    expect(moveChannel(base, 'C', { category: 'NOPE', index: 0 })).toEqual(base)
    expect(moveCategory(base, 'NOPE', 0)).toEqual(base)
  })

  it('does not change the order it was given', () => {
    const before = JSON.stringify(base)
    moveChannel(base, 'C', { category: 'K2', index: 0 })
    moveCategory(base, 'K3', 0)
    expect(JSON.stringify(base)).toBe(before)
  })

  it('a category up and down', () => {
    expect(moveCategory(base, 'K3', 0).cats.map((c) => c.id)).toEqual(['K3', 'K1', 'K2'])
    expect(moveCategory(base, 'K1', 2).cats.map((c) => c.id)).toEqual(['K2', 'K3', 'K1'])
    expect(moveCategory(base, 'K1', 2).cats[2].channels).toEqual(['C', 'D'])
  })
})

describe('steps', () => {
  it('a channel steps within its list', () => {
    expect(stepChannel(base, 'A', -1)!.top).toEqual(['A', 'B', 'X'])
    expect(stepChannel(base, 'C', 1)!.cats[0].channels).toEqual(['D', 'C'])
  })

  it('at a list edge it goes to the end of the previous list or the start of the next', () => {
    const up = stepChannel(base, 'C', -1)!
    expect(up.top).toEqual(['B', 'A', 'X', 'C'])
    expect(up.cats[0].channels).toEqual(['D'])
    const down = stepChannel(base, 'X', 1)!
    expect(down.top).toEqual(['B', 'A'])
    expect(down.cats[0].channels).toEqual(['X', 'C', 'D'])
    const intoEmpty = stepChannel(base, 'E', 1)!
    expect(intoEmpty.cats[1].channels).toEqual([])
    expect(intoEmpty.cats[2].channels).toEqual(['E'])
    expect(stepChannel(base, 'E', -1)!.cats[0].channels).toEqual(['C', 'D', 'E'])
  })

  it('up from the first category goes into an empty top area', () => {
    const o: Order = { top: [], cats: [{ id: 'K1', channels: ['C'] }] }
    expect(stepChannel(o, 'C', -1)).toEqual({ top: ['C'], cats: [{ id: 'K1', channels: [] }] })
  })

  it('null at the very top and bottom, or for an unknown id', () => {
    expect(stepChannel(base, 'B', -1)).toBeNull()
    expect(stepChannel({ ...base, cats: base.cats.slice(0, 2) }, 'E', 1)).toBeNull()
    expect(stepChannel(base, 'NOPE', 1)).toBeNull()
  })

  it('a category steps up and down, null at the ends', () => {
    expect(stepCategory(base, 'K2', -1)!.cats.map((c) => c.id)).toEqual(['K2', 'K1', 'K3'])
    expect(stepCategory(base, 'K2', 1)!.cats.map((c) => c.id)).toEqual(['K1', 'K3', 'K2'])
    expect(stepCategory(base, 'K1', -1)).toBeNull()
    expect(stepCategory(base, 'K3', 1)).toBeNull()
    expect(stepCategory(base, 'NOPE', 1)).toBeNull()
  })
})
