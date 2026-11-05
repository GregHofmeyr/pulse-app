// @vitest-environment jsdom
// Collapsed sidebar sections, per device (#99), keyed by category since categories (#54). A section collapsed before
// the update (`serverId:kind`) stays collapsed: it carries over to the default category made for that kind.
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { Ready } from './protocol/Ready'

const hooks = vi.hoisted(() => [] as Array<(r: Ready) => void>)
vi.mock('./store.svelte', () => ({ onAppReady: (h: (r: Ready) => void) => (hooks.push(h), () => {}) }))

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
