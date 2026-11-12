// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { QuickAction } from './quickActions'
import { emptyState, type AppState } from './state'
import type { Channel } from './protocol/Channel'
import type { User } from './protocol/User'
import {
  buildResults, isTyping, loadRecents, matchScore, parseQuery, placesFrom, pushRecent, rankPlaces, recentPlaces, RECENTS_KEY, RECENTS_SHOWN, saveRecents, switcherKey, type Place,
} from './quickSwitch'

const user = (id: string, display_name: string): User => ({ id, username: id, display_name, avatar_hash: null, avatar_still: null, style: {} as User['style'] })
const ch = (id: string, kind: Channel['kind'], name: string | null, server_id: string | null = null): Channel => ({ id, kind, name, server_id, position: 0 })

function state(): AppState {
  const s = emptyState()
  return {
    ...s,
    me: user('u-me', 'Me'),
    servers: [{ id: 's1', name: 'Game Night', icon_hash: null }, { id: 's2', name: 'Work', icon_hash: null }],
    channels: {
      c1: ch('c1', 'text', 'general', 's1'),
      c2: ch('c2', 'voice', 'Lounge', 's1'),
      c3: ch('c3', 'text', 'game-plans', 's2'),
      c4: ch('c4', 'quotes', 'quotes', 's1'),
      d1: ch('d1', 'dm', null),
      g1: ch('g1', 'group', 'Weekend crew'),
      d2: ch('d2', 'dm', null),
      x1: ch('x1', 'text', 'orphan', 'gone'),
    },
    dmMembers: { d1: ['u-me', 'u-alex'], g1: ['u-me', 'u-sam', 'u-kim'], d2: ['u-me', 'u-jo'] },
    people: {
      'u-alex': { user: user('u-alex', 'Alex') } as AppState['people'][string],
      'u-sam': { user: user('u-sam', 'Sam') } as AppState['people'][string],
      'u-kim': { user: user('u-kim', 'Kim') } as AppState['people'][string],
      'u-jo': { user: user('u-jo', 'Jo') } as AppState['people'][string],
    },
    privateNicks: { 'u-alex': 'Ace' },
    hidden: { d2: true },
  }
}

const key = (o: Partial<KeyboardEvent> & { target?: EventTarget | null }) =>
  ({ key: '', code: '', ctrlKey: false, metaKey: false, altKey: false, shiftKey: false, repeat: false, defaultPrevented: false, target: document.body, ...o }) as KeyboardEvent

describe('opening the switcher', () => {
  beforeEach(() => (document.body.innerHTML = ''))
  const closed = { open: false, blocked: false }

  it('Ctrl+K (and Cmd+K) toggles it from anywhere, a text field included', () => {
    document.body.innerHTML = '<input id="f" /><div id="box" contenteditable="plaintext-only" role="textbox"></div>'
    expect(switcherKey(key({ key: 'k', ctrlKey: true }), closed)).toBe('toggle')
    expect(switcherKey(key({ key: 'K', metaKey: true }), closed)).toBe('toggle')
    expect(switcherKey(key({ key: 'k', ctrlKey: true, target: document.getElementById('f') }), closed)).toBe('toggle')
    expect(switcherKey(key({ key: 'k', ctrlKey: true, target: document.getElementById('box') }), closed)).toBe('toggle')
    expect(switcherKey(key({ key: 'k', ctrlKey: true }), { open: true, blocked: false })).toBe('toggle')
  })
  it('goes by the physical key too (another layout), but not with Shift or Alt, or a held key repeating', () => {
    expect(switcherKey(key({ key: 'л', code: 'KeyK', ctrlKey: true }), closed)).toBe('toggle')
    expect(switcherKey(key({ key: 'K', ctrlKey: true, shiftKey: true }), closed)).toBeNull()
    expect(switcherKey(key({ key: 'k', ctrlKey: true, altKey: true }), closed)).toBeNull()
    expect(switcherKey(key({ key: 'k', ctrlKey: true, repeat: true }), closed)).toBeNull()
    expect(switcherKey(key({ key: 'k' }), closed)).toBeNull()
  })
  it('`/` opens it only when nothing that takes text has focus', () => {
    document.body.innerHTML = '<button id="b">x</button><input id="f" /><input id="r" type="range" /><textarea id="t"></textarea><div id="box" contenteditable="plaintext-only" role="textbox"><span id="in">hi</span></div><div role="menu"><button id="m">x</button></div>'
    const at = (id: string) => document.getElementById(id)
    expect(switcherKey(key({ key: '/' }), closed)).toBe('open')
    expect(switcherKey(key({ key: '/', target: at('b') }), closed)).toBe('open')
    expect(switcherKey(key({ key: '/', target: at('r') }), closed)).toBe('open')
    // the composer's slash commands (#262) keep their `/`
    expect(switcherKey(key({ key: '/', target: at('box') }), closed)).toBeNull()
    expect(switcherKey(key({ key: '/', target: at('in') }), closed)).toBeNull()
    expect(switcherKey(key({ key: '/', target: at('f') }), closed)).toBeNull()
    expect(switcherKey(key({ key: '/', target: at('t') }), closed)).toBeNull()
    expect(switcherKey(key({ key: '/', target: at('m') }), closed)).toBeNull()
  })
  it('`/` leaves alone a key something else took, a modified `/`, and an open switcher (it types there)', () => {
    expect(switcherKey(key({ key: '/', defaultPrevented: true }), closed)).toBeNull()
    expect(switcherKey(key({ key: '/', ctrlKey: true }), closed)).toBeNull()
    expect(switcherKey(key({ key: '/' }), { open: true, blocked: false })).toBeNull()
  })
  it('another dialog keeps its keys: neither opens it over one', () => {
    expect(switcherKey(key({ key: '/' }), { open: false, blocked: true })).toBeNull()
    expect(switcherKey(key({ key: 'k', ctrlKey: true }), { open: false, blocked: true })).toBeNull()
  })
  it('isTyping: inputs, text areas, editables and textboxes', () => {
    document.body.innerHTML = '<input id="c" type="checkbox" /><div id="d"></div><div contenteditable="false" id="ce"></div>'
    expect(isTyping(document.getElementById('c'))).toBe(false)
    expect(isTyping(document.getElementById('d'))).toBe(false)
    expect(isTyping(document.getElementById('ce'))).toBe(false)
    expect(isTyping(null)).toBe(false)
  })
})

describe('placesFrom', () => {
  it('lists servers, their channels with the server as context, and your open DMs and groups', () => {
    const p = placesFrom(state())
    const by = (k: string) => p.find((x) => x.key === k)
    expect(by('s:s1')).toMatchObject({ kind: 'server', name: 'Game Night', context: null })
    expect(by('c:c1')).toMatchObject({ kind: 'text', name: 'general', context: 'Game Night', serverId: 's1' })
    expect(by('c:c2')).toMatchObject({ kind: 'voice', name: 'Lounge' })
    expect(by('c:c4')).toMatchObject({ kind: 'quotes' })
    expect(by('c:g1')).toMatchObject({ kind: 'group', name: 'Weekend crew' })
    // a channel of a server you can't see, and a closed DM, aren't listed
    expect(by('c:x1')).toBeUndefined()
    expect(by('c:d2')).toBeUndefined()
  })
  it('names a DM as you see the other person, and finds it by any of their names', () => {
    const dm = placesFrom(state()).find((x) => x.key === 'c:d1')!
    expect(dm).toMatchObject({ kind: 'dm', name: 'Ace' })
    expect(dm.keys).toEqual(expect.arrayContaining(['ace', 'alex', 'u-alex']))
  })
})

describe('matchScore', () => {
  it('ranks whole name > prefix > word start > inside > fuzzy, and no match is 0', () => {
    const whole = matchScore('lounge', 'Lounge')
    const prefix = matchScore('lou', 'Lounge')
    const word = matchScore('pla', 'game-plans')
    const inside = matchScore('ung', 'Lounge')
    const fuzzy = matchScore('lng', 'Lounge')
    expect(whole).toBeGreaterThan(prefix)
    expect(prefix).toBeGreaterThan(word)
    expect(word).toBeGreaterThan(inside)
    expect(inside).toBeGreaterThan(fuzzy)
    expect(fuzzy).toBeGreaterThan(0)
    expect(matchScore('xyz', 'Lounge')).toBe(0)
    expect(matchScore('', 'Lounge')).toBe(0)
  })
  it('several words each starting a word, in order', () => {
    expect(matchScore('ga ni', 'Game Night')).toBeGreaterThan(matchScore('gni', 'Game Night'))
    expect(matchScore('ga ni', 'Game Night')).toBeLessThan(matchScore('game', 'Game Night'))
  })
  it('a shorter name wins within a band', () => {
    expect(matchScore('gen', 'general')).toBeGreaterThan(matchScore('gen', 'general-chat-archive'))
  })
})

describe('rankPlaces', () => {
  const places = placesFrom(state())
  it('puts the best match first and drops what does not match', () => {
    const r = rankPlaces(places, 'game')
    expect(r.map((p) => p.key)).toEqual(['s:s1', 'c:c3'])
  })
  it('a recent place lifts within its band, never past a better one', () => {
    // two prefix matches ("general" and "game-plans" for "g"): the recent one first
    const plain = rankPlaces(places, 'g').map((p) => p.key)
    const lifted = rankPlaces(places, 'g', ['c:c3']).map((p) => p.key)
    expect(lifted.indexOf('c:c3')).toBeLessThan(plain.indexOf('c:c3'))
    // a recent fuzzy match stays below a prefix match
    const r = rankPlaces(places, 'lo', ['c:g1']).map((p) => p.key)
    expect(r[0]).toBe('c:c2')
  })
  it('finds DMs and groups by the people in them', () => {
    expect(rankPlaces(places, 'alex')[0].key).toBe('c:d1')
    expect(rankPlaces(places, 'kim')[0].key).toBe('c:g1')
  })
  it('stays instant over a few hundred places', () => {
    const many: Place[] = Array.from({ length: 600 }, (_, i) => ({ key: `c:${i}`, kind: 'text', id: `${i}`, name: `channel-${i}-chat`, serverId: 's', context: 'S', keys: [`channel-${i}-chat`] }))
    const t0 = performance.now()
    for (let i = 0; i < 10; i++) rankPlaces(many, 'ch 4')
    expect((performance.now() - t0) / 10).toBeLessThan(16)
  })
})

describe('recents', () => {
  beforeEach(() => localStorage.clear())
  it('newest first, each once, kept on this device', () => {
    let r = pushRecent([], 'c:a')
    r = pushRecent(r, 'c:b')
    r = pushRecent(r, 'c:a')
    expect(r).toEqual(['c:a', 'c:b'])
    saveRecents(r)
    expect(JSON.parse(localStorage.getItem(RECENTS_KEY)!)).toEqual(['c:a', 'c:b'])
    expect(loadRecents()).toEqual(['c:a', 'c:b'])
  })
  it('a broken stored value reads as none', () => {
    localStorage.setItem(RECENTS_KEY, '{nope')
    expect(loadRecents()).toEqual([])
    localStorage.setItem(RECENTS_KEY, '{"a":1}')
    expect(loadRecents()).toEqual([])
  })
  it('lists the ones that still exist, without where you are, at most RECENTS_SHOWN', () => {
    const places = placesFrom(state())
    expect(recentPlaces(places, ['c:c1', 'c:gone', 's:s2', 'c:c2'], 'c:c1').map((p) => p.key)).toEqual(['s:s2', 'c:c2'])
    const keys = places.map((p) => p.key)
    expect(recentPlaces(places, keys, null)).toHaveLength(RECENTS_SHOWN)
  })
})

describe('parseQuery', () => {
  it('a leading `>` asks for actions only', () => {
    expect(parseQuery('> join lou')).toEqual({ actions: true, text: 'join lou' })
    expect(parseQuery('>')).toEqual({ actions: true, text: '' })
    expect(parseQuery(' gen ')).toEqual({ actions: false, text: 'gen' })
  })
})

describe('buildResults', () => {
  const places = placesFrom(state())
  const run = vi.fn()
  const actions: QuickAction[] = [
    { id: 'join:c2', label: 'Join Lounge', name: 'Lounge', icon: 'speaker', run },
    { id: 'join:v9', label: 'Join Gaming', name: 'Gaming', icon: 'speaker', run },
    { id: 'settings', label: 'Open Settings', icon: 'gear', run },
  ]
  const ids = (r: ReturnType<typeof buildResults>) => r.map((sec) => [sec.id, sec.rows.map((x) => (x.type === 'place' ? x.place.key : x.type === 'action' ? x.action.id : x.hit.message.id))])
  it('nothing typed: the recents (without where you are)', () => {
    expect(ids(buildResults({ places, actions, raw: '', recents: ['c:c1', 'c:c2'], here: 'c:c1' }))).toEqual([['recent', ['c:c2']]])
    expect(buildResults({ places, actions, raw: '  ', recents: [], here: null })).toEqual([])
  })
  it('typed: the places, then the actions that match', () => {
    expect(ids(buildResults({ places, actions, raw: 'gam', recents: [], here: null }))).toEqual([['places', ['s:s1', 'c:c3']], ['actions', ['join:v9']]])
    // a voice channel listed above offers its Join on its row: not again below
    expect(ids(buildResults({ places, actions, raw: 'lou', recents: [], here: null }))).toEqual([['places', ['c:c2']]])
    expect(ids(buildResults({ places, actions, raw: 'general', recents: [], here: null }))).toEqual([['places', ['c:c1']]])
  })
  it('`>`: actions only, all of them with nothing after it', () => {
    expect(ids(buildResults({ places, actions, raw: '>', recents: [], here: null }))).toEqual([['actions', ['join:c2', 'join:v9', 'settings']]])
    expect(ids(buildResults({ places, actions, raw: '> sett', recents: [], here: null }))).toEqual([['actions', ['settings']]])
  })
})
