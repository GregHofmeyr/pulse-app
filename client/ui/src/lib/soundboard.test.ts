// @vitest-environment jsdom
import { beforeEach, describe, expect, it } from 'vitest'
import type { Sound } from './protocol/Sound'
import { emptyState } from './state'
import {
  loadSoundboardPrefs, saveSoundboardPrefs, loadSoundMutes, saveSoundMutes,
  shouldPlay, soundSections, searchSounds, tidyCandidates,
} from './soundboard'

const snd = (id: string, name: string, over: Partial<Sound> = {}): Sound => ({
  id, server_id: 'S1', name, emoji: null, media_hash: 'h' + id, volume: 100, duration_ms: 1000,
  added_by: 'U1', created_at: '2026-10-01T00:00:00.000Z', plays: 0, ...over,
})
const servers = [{ id: 'S1', name: 'Main', icon_hash: null }, { id: 'S2', name: 'Side', icon_hash: null }, { id: 'S3', name: 'Other', icon_hash: null }]

describe('shouldPlay', () => {
  const ctx = { myChannel: 'V1', volume_pct: 100, mutes: {} as Record<string, true> }
  it('plays for my call, including my own sound', () => {
    expect(shouldPlay({ channel_id: 'V1', user_id: 'U1' }, ctx)).toBe(true)
  })
  it('ignores other calls and no call', () => {
    expect(shouldPlay({ channel_id: 'V2', user_id: 'U2' }, ctx)).toBe(false)
    expect(shouldPlay({ channel_id: 'V1', user_id: 'U2' }, { ...ctx, myChannel: null })).toBe(false)
  })
  it('is silent at volume 0 and for muted people', () => {
    expect(shouldPlay({ channel_id: 'V1', user_id: 'U2' }, { ...ctx, volume_pct: 0 })).toBe(false)
    expect(shouldPlay({ channel_id: 'V1', user_id: 'U2' }, { ...ctx, mutes: { U2: true } })).toBe(false)
    expect(shouldPlay({ channel_id: 'V1', user_id: 'U3' }, { ...ctx, mutes: { U2: true } })).toBe(true)
  })
})

describe('soundSections', () => {
  const sounds = Object.fromEntries([
    snd('A', 'zap'), snd('B', 'bruh'), snd('C', 'side one', { server_id: 'S2' }), snd('D', 'hidden', { server_id: 'S3' }),
  ].map((x) => [x.id, x]))
  it('orders favourites, frequent, then member servers by name; skips non-member servers', () => {
    const s = { ...emptyState(), sounds, soundFavourites: ['A', 'B'], frequentSounds: ['C'] }
    const out = soundSections(s, new Set(['S1', 'S2']), servers)
    expect(out.map((x) => x.id)).toEqual(['favourites', 'frequent', 'server:S1', 'server:S2'])
    expect(out[0].sounds.map((x) => x.id)).toEqual(['A', 'B'])
    expect(out[2].sounds.map((x) => x.name)).toEqual(['bruh', 'zap'])
    expect(out[2].server?.id).toBe('S1')
  })
  it('omits empty favourites and frequent but keeps an empty server', () => {
    const out = soundSections({ ...emptyState(), sounds: {} }, new Set(['S1']), servers)
    expect(out.map((x) => x.id)).toEqual(['server:S1'])
    expect(out[0].sounds).toEqual([])
  })
  it('caps frequent at 9 and drops unknown ids', () => {
    const many = Object.fromEntries(Array.from({ length: 12 }, (_, i) => [`X${i}`, snd(`X${i}`, `n${i}`)]))
    const s = { ...emptyState(), sounds: many, frequentSounds: ['gone', ...Object.keys(many)] }
    const out = soundSections(s, new Set(['S1']), servers)
    expect(out[0].id).toBe('frequent')
    expect(out[0].sounds).toHaveLength(9)
  })
})

describe('searchSounds', () => {
  it('matches case-insensitively with prefix matches first', () => {
    const l = [snd('A', 'big airhorn'), snd('B', 'Air raid'), snd('C', 'clap')]
    expect(searchSounds(l, 'AIR').map((x) => x.id)).toEqual(['B', 'A'])
    expect(searchSounds(l, '  ').map((x) => x.id)).toEqual(['A', 'B', 'C'])
  })
})

describe('tidyCandidates', () => {
  it('takes the 5 least played, oldest first on ties', () => {
    const l = [
      snd('A', 'a', { plays: 5 }), snd('B', 'b', { plays: 0, created_at: '2026-10-03T00:00:00.000Z' }),
      snd('C', 'c', { plays: 0, created_at: '2026-10-01T00:00:00.000Z' }), snd('D', 'd', { plays: 1 }),
      snd('E', 'e', { plays: 2 }), snd('F', 'f', { plays: 3 }), snd('G', 'g', { plays: 4 }),
    ]
    expect(tidyCandidates(l).map((x) => x.id)).toEqual(['C', 'B', 'D', 'E', 'F'])
  })
})

describe('prefs', () => {
  beforeEach(() => localStorage.clear())
  it('round-trips volume and mutes', () => {
    expect(loadSoundboardPrefs()).toEqual({ volume_pct: 100 })
    saveSoundboardPrefs({ volume_pct: 40 })
    expect(loadSoundboardPrefs()).toEqual({ volume_pct: 40 })
    saveSoundMutes({ U2: true })
    expect(loadSoundMutes()).toEqual({ U2: true })
  })
  it('falls back on corrupt values', () => {
    localStorage.setItem('pulse.soundboard', '{nope')
    localStorage.setItem('pulse.soundboard.mutes', '[1,"x"]')
    expect(loadSoundboardPrefs()).toEqual({ volume_pct: 100 })
    expect(loadSoundMutes()).toEqual({})
    localStorage.setItem('pulse.soundboard', JSON.stringify({ volume_pct: 'loud' }))
    expect(loadSoundboardPrefs()).toEqual({ volume_pct: 100 })
  })
})
