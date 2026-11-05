// @vitest-environment jsdom
// Collapsed sidebar sections, per device (#99), keyed by category since categories (#54). A section collapsed before
// the update (`serverId:kind`) stays collapsed: it carries over to the default category made for that kind.
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { Ready } from './protocol/Ready'
import type { Event } from './protocol/Event'

const hooks = vi.hoisted(() => [] as Array<(r: Ready) => void>)
const eventHooks = vi.hoisted(() => [] as Array<(e: Event) => void>)
const store = vi.hoisted(() => ({ app: { state: { layouts: {} } as { layouts: Record<string, unknown> } } }))
vi.mock('./store.svelte', () => ({
  app: store.app,
  onAppReady: (h: (r: Ready) => void) => (hooks.push(h), () => {}),
  onAppEvent: (h: (e: Event) => void) => (eventHooks.push(h), () => {}),
}))

import { carryOverCollapsed, isCollapsed, resetCollapsed, sections, setCollapsed, toggleSection } from './sidebarSections.svelte'
import type { Category } from './protocol/Category'
import type { ServerLayout } from './protocol/ServerLayout'

const KEY = 'pulse.collapsedSections'
const cat = (id: string, server_id: string, over: Partial<Category> = {}): Category => ({ id, server_id, name: id, position: 0, ...over })
const layout = (server_id: string, categories: Category[]): ServerLayout => ({ server_id, revision: 1, categories, channels: [] })
const stored = () => JSON.parse(localStorage.getItem(KEY) ?? '[]')

beforeEach(() => {
  localStorage.clear()
  resetCollapsed()
})

describe('collapse by key', () => {
  it('toggles, sets and persists', () => {
    toggleSection('cat:K1')
    expect(isCollapsed('cat:K1')).toBe(true)
    setCollapsed('cat:K2', true)
    setCollapsed('cat:K2', true)
    expect(stored()).toEqual(['cat:K1', 'cat:K2'])
    toggleSection('cat:K1')
    setCollapsed('cat:K2', false)
    expect(isCollapsed('cat:K1')).toBe(false)
    expect(isCollapsed('cat:K2')).toBe(false)
    resetCollapsed()
    expect(sections.collapsed).toEqual([])
  })
})

describe('carryOverCollapsed', () => {
  it('old collapsed sections carry over to their default categories', () => {
    localStorage.setItem(KEY, JSON.stringify(['S1:voice', 'S1:text', 'S2:voice']))
    resetCollapsed()
    carryOverCollapsed([
      // "Voice Channels" was renamed since: the match is by the kind it was made for, not its name.
      layout('S1', [cat('KT', 'S1', { default_kind: 'text' }), cat('KV', 'S1', { name: 'Hangout', default_kind: 'voice' }), cat('KM', 'S1')]),
      layout('S2', [cat('KV2', 'S2', { default_kind: 'voice' })]),
    ])
    expect(isCollapsed('cat:KV')).toBe(true)
    expect(isCollapsed('cat:KT')).toBe(true)
    expect(isCollapsed('cat:KV2')).toBe(true)
    expect(isCollapsed('cat:KM')).toBe(false)
    expect(isCollapsed('S1:voice')).toBe(false)
    expect(stored().sort()).toEqual(['cat:KT', 'cat:KV', 'cat:KV2'])
  })

  it('unknown keys are left alone', () => {
    // No layout for S3 (an older server), no default quotes category in S1, and a category key already moved.
    localStorage.setItem(KEY, JSON.stringify(['S3:voice', 'S1:quotes', 'cat:KX']))
    resetCollapsed()
    carryOverCollapsed([layout('S1', [cat('KV', 'S1', { default_kind: 'voice' })])])
    expect(sections.collapsed).toEqual(['S3:voice', 'S1:quotes', 'cat:KX'])
  })

  it('runs with every Ready', () => {
    localStorage.setItem(KEY, JSON.stringify(['S1:voice']))
    resetCollapsed()
    expect(hooks).toHaveLength(1)
    hooks[0]({ layouts: [layout('S1', [cat('KV', 'S1', { default_kind: 'voice' })])] } as unknown as Ready)
    expect(sections.collapsed).toEqual(['cat:KV'])
    hooks[0]({} as unknown as Ready) // an older server sends no layouts
    expect(sections.collapsed).toEqual(['cat:KV'])
  })
})

describe('keys of categories that are gone', () => {
  it('are pruned with each Ready that has layouts; other keys stay', () => {
    localStorage.setItem(KEY, JSON.stringify(['cat:K1', 'cat:GONE', 'S3:voice']))
    resetCollapsed()
    hooks[0]({ layouts: [layout('S1', [cat('K1', 'S1')]), layout('S2', [])] } as unknown as Ready)
    expect(sections.collapsed).toEqual(['cat:K1', 'S3:voice'])
    expect(stored()).toEqual(['cat:K1', 'S3:voice'])
  })

  it('stay when the Ready has no layouts (an older server)', () => {
    localStorage.setItem(KEY, JSON.stringify(['cat:K1']))
    resetCollapsed()
    hooks[0]({} as unknown as Ready)
    hooks[0]({ layouts: [] } as unknown as Ready)
    expect(sections.collapsed).toEqual(['cat:K1'])
  })

  it('are pruned when a layout changes, against every layout held', () => {
    localStorage.setItem(KEY, JSON.stringify(['cat:K1', 'cat:K2', 'cat:K9']))
    resetCollapsed()
    // K2 was deleted from S1; K9 is S2's, held from before
    store.app.state = { layouts: { S1: { revision: 2, categories: [cat('K1', 'S1')] }, S2: { revision: 1, categories: [cat('K9', 'S2')] } } }
    expect(eventHooks).toHaveLength(1)
    eventHooks[0]({ t: 'LayoutChanged', d: { layout: layout('S1', [cat('K1', 'S1')]) } } as unknown as Event)
    expect(sections.collapsed).toEqual(['cat:K1', 'cat:K9'])
    expect(stored()).toEqual(['cat:K1', 'cat:K9'])
  })

  it('nothing is written when nothing is pruned', () => {
    localStorage.setItem(KEY, JSON.stringify(['cat:K1']))
    resetCollapsed()
    const set = vi.spyOn(Storage.prototype, 'setItem')
    store.app.state = { layouts: { S1: { revision: 2, categories: [cat('K1', 'S1')] } } }
    eventHooks[0]({ t: 'LayoutChanged', d: { layout: layout('S1', [cat('K1', 'S1')]) } } as unknown as Event)
    hooks[0]({ layouts: [layout('S1', [cat('K1', 'S1')])] } as unknown as Ready)
    eventHooks[0]({ t: 'ChannelDeleted', d: {} } as unknown as Event)
    expect(set).not.toHaveBeenCalled()
    set.mockRestore()
  })
})
