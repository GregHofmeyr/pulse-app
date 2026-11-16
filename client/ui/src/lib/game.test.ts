// "Playing" (#380): wording, art, elapsed time, saved switches, and how the state keeps it.
import { describe, expect, it } from 'vitest'
import { elapsedLabel, gameArt, playingLine, playingOf } from './game'
import { parseSaved } from './gamePrefs.svelte'
import { applyEvent, applyReady, emptyState } from './state'
import { testReady } from './testReady'
import type { Playing } from './protocol/Playing'

const thumb = (c: string, width = 480, height = 155) => ({ hash: c.repeat(64), width, height })
const game = (over: Partial<Playing> = {}): Playing => ({ name: 'Space Game', steam_app_id: null, started_at: null, art: null, ...over })
const sam = { id: 'U2', username: 'sam', display_name: 'sam', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }

describe('how it reads', () => {
  it('a line', () => expect(playingLine(game())).toBe('Playing Space Game'))

  it('the wide art is the hero, else the header; only media hashes', () => {
    expect(gameArt(game())).toEqual({ wide: null, logo: null })
    const full = gameArt(game({ art: { hero: thumb('a'), header: thumb('b'), logo: thumb('c') } }))
    expect(full.wide).toContain('a'.repeat(64))
    expect(full.logo).toContain('c'.repeat(64))
    expect(gameArt(game({ art: { hero: null, header: thumb('b'), logo: null } })).wide).toContain('b'.repeat(64))
    expect(gameArt(game({ art: { hero: { hash: '../x', width: 1, height: 1 }, header: null, logo: null } })).wide).toBeNull()
  })

  it('elapsed time', () => {
    const start = '2026-11-16T18:00:00.000Z'
    const at = (mins: number) => Date.parse(start) + mins * 60000
    expect(elapsedLabel(null, at(5))).toBeNull()
    expect(elapsedLabel('nonsense', at(5))).toBeNull()
    expect(elapsedLabel(start, at(0.5))).toBe('less than a minute')
    expect(elapsedLabel(start, at(-2))).toBe('less than a minute')
    expect(elapsedLabel(start, at(42))).toBe('42m')
    expect(elapsedLabel(start, at(65))).toBe('1h 05m')
    expect(elapsedLabel(start, at(60 * 26 + 30))).toBe('26h 30m')
  })
})

describe('saved switches', () => {
  it('all on by default; an older save fills in; junk is the defaults', () => {
    expect(parseSaved(null)).toEqual({ enabled: true, art: true, time: true, games: [], clips: true, clip_folders: null })
    expect(parseSaved('{oops')).toEqual(parseSaved(null))
    expect(parseSaved(JSON.stringify({ enabled: false, games: [{ name: 'Old', exe: 'C:\\old.exe' }, { name: '', exe: 'x' }, 7] }))).toEqual({
      enabled: false, art: true, time: true, games: [{ name: 'Old', exe: 'C:\\old.exe' }], clips: true, clip_folders: null,
    })
  })

  it('clip drop: its switch and chosen folders are kept; junk folders are dropped', () => {
    expect(parseSaved(JSON.stringify({ clips: false, clip_folders: ['D:\\Clips', '', 3, '  '] }))).toMatchObject({ clips: false, clip_folders: ['D:\\Clips'] })
    expect(parseSaved(JSON.stringify({ clip_folders: 'D:\\Clips' })).clip_folders).toBeNull()
  })
})

describe('the state', () => {
  const base = () => applyReady(emptyState(), testReady([], { people: [{ user: sam, online: true, last_seen_at: null, status: 'online', playing: game() }] }))

  it('Ready carries it; an older server sends none', () => {
    expect(playingOf(base(), 'U2')?.name).toBe('Space Game')
    const old = applyReady(emptyState(), testReady([], { people: [{ user: sam, online: true, last_seen_at: null, status: 'online' }] }))
    expect(playingOf(old, 'U2')).toBeNull()
  })

  it('PlayingChanged sets and clears it, beside listening; going offline drops it', () => {
    let s = applyEvent(base(), { t: 'ListeningChanged', d: { user_id: 'U2', listening: { title: 'Song', artist: null, album: null, spotify_id: null, art: null } } }, 0)
    s = applyEvent(s, { t: 'PlayingChanged', d: { user_id: 'U2', playing: game({ name: 'Next' }) } }, 0)
    expect(playingOf(s, 'U2')?.name).toBe('Next')
    expect(s.people.U2.listening?.title).toBe('Song')
    s = applyEvent(s, { t: 'PlayingChanged', d: { user_id: 'U2', playing: null } }, 0)
    expect(playingOf(s, 'U2')).toBeNull()
    expect(s.people.U2.listening?.title).toBe('Song')
    s = applyEvent(s, { t: 'PlayingChanged', d: { user_id: 'U2', playing: game() } }, 0)
    s = applyEvent(s, { t: 'PresenceChanged', d: { user_id: 'U2', online: false, last_seen_at: '2026-11-16T10:00:00Z', status: 'offline' } }, 0)
    expect(playingOf(s, 'U2')).toBeNull()
    const same = applyEvent(s, { t: 'PlayingChanged', d: { user_id: 'U9', playing: game() } }, 0)
    expect(same).toBe(s)
  })
})
