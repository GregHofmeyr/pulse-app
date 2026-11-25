// Ctrl+K's builds (#619): `builds:` limits the switcher to builds, and a game named by the start of its name or its
// initials filters them.
import { describe, expect, it } from 'vitest'
import { buildsQuery, parseBuildsQuery, sortBuildHits } from './quickBuilds'
import type { Game } from './protocol/Game'
import type { Message } from './protocol/Message'

const game = (id: string, name: string, server = 'S1'): Game => ({ id, server_id: server, name, created_by: null, created_at: '2026-10-01T00:00:00Z' })
const games = [
  game('G1', 'The First Descendant'),
  game('G2', 'The First Descendant', 'S2'), // the same game in another server: one key
  game('G3', 'Warframe'),
  game('G4', 'The Division'),
  game('G5', 'Deadlock'),
]

describe('parseBuildsQuery', () => {
  it('a plain query searches builds as text', () => {
    expect(parseBuildsQuery('raven', games)).toEqual({ only: false, gameKey: null, text: 'raven' })
  })

  it('`>` (actions) never searches builds', () => {
    expect(parseBuildsQuery('> mute', games)).toBeNull()
  })

  it('`builds:` alone limits to builds, nothing to search yet', () => {
    expect(parseBuildsQuery('builds:', games)).toEqual({ only: true, gameKey: null, text: '' })
    expect(parseBuildsQuery('  Builds:  ', games)).toEqual({ only: true, gameKey: null, text: '' })
  })

  it('a game by its initials, the rest as text', () => {
    expect(parseBuildsQuery('builds: tfd', games)).toEqual({ only: true, gameKey: 'the first descendant', text: '' })
    expect(parseBuildsQuery('builds: tfd raven', games)).toEqual({ only: true, gameKey: 'the first descendant', text: 'raven' })
  })

  it('a game by the start of its name, the longest that picks one', () => {
    expect(parseBuildsQuery('builds: the first raven', games)).toEqual({ only: true, gameKey: 'the first descendant', text: 'raven' })
    expect(parseBuildsQuery('builds:war', games)).toEqual({ only: true, gameKey: 'warframe', text: '' })
  })

  it('an ambiguous prefix is searched as text', () => {
    expect(parseBuildsQuery('builds: the raven', games)).toEqual({ only: true, gameKey: null, text: 'the raven' })
  })

  it('no game matching: all text', () => {
    expect(parseBuildsQuery('builds: raven bunny', games)).toEqual({ only: true, gameKey: null, text: 'raven bunny' })
  })
})

describe('buildsQuery', () => {
  it('what to ask the search route, or null', () => {
    expect(buildsQuery({ only: false, gameKey: null, text: 'r' })).toBeNull()
    expect(buildsQuery({ only: false, gameKey: null, text: '!!' })).toBeNull()
    expect(buildsQuery({ only: true, gameKey: null, text: '' })).toBeNull()
    expect(buildsQuery({ only: true, gameKey: 'warframe', text: '' })).toEqual({ q: '', game: 'warframe' })
    expect(buildsQuery({ only: false, gameKey: null, text: 'raven' })).toEqual({ q: 'raven', game: null })
  })
})

describe('sortBuildHits', () => {
  it('archived builds come last, each group in the order the server gave', () => {
    const m = (id: string, archived = false) => ({ id, build: { archived_at: archived ? '2026-10-05T00:00:00Z' : null } }) as unknown as Message
    expect(sortBuildHits([m('a', true), m('b'), m('c', true), m('d')]).map((x) => x.id)).toEqual(['b', 'd', 'a', 'c'])
  })
})
