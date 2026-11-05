import { describe, expect, it } from 'vitest'
import { emptyState, type AppState } from './state'
import { byPresence, presenceOf } from './presence'

const user = (id: string, name: string) => ({ id, username: name, display_name: name, avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } })
const alex = user('U1', 'alex')
const NOW = '2026-11-05T10:00:00.000Z'
const state = (over: Partial<AppState> = {}): AppState => ({
  ...emptyState(),
  me: alex,
  people: {
    U1: { user: alex, online: true, last_seen_at: null, status: 'online' },
    U2: { user: user('U2', 'sam'), online: true, last_seen_at: null, status: 'online' },
    U3: { user: user('U3', 'kim'), online: true, last_seen_at: null, status: 'dnd' },
    U4: { user: user('U4', 'jo'), online: false, last_seen_at: '2026-11-05T09:00:00Z', status: 'offline' },
  },
  ...over,
})

describe('presenceOf (#45): what to draw for a person', () => {
  it('others: as the server lets you see them', () => {
    expect(presenceOf(state(), 'U2', NOW)).toEqual({ kind: 'online', label: 'Online', lastSeen: null })
    expect(presenceOf(state(), 'U3', NOW)).toEqual({ kind: 'dnd', label: 'Do Not Disturb', lastSeen: null })
    expect(presenceOf(state(), 'U4', NOW)).toEqual({ kind: 'offline', label: 'Offline', lastSeen: '2026-11-05T09:00:00Z' })
  })
  it('an older server sends no status: go by online', () => {
    const s = state()
    s.people.U2 = { user: user('U2', 'sam'), online: true, last_seen_at: null } as never
    s.people.U4 = { user: user('U4', 'jo'), online: false, last_seen_at: '2026-11-05T09:00:00Z' } as never
    expect(presenceOf(s, 'U2', NOW).kind).toBe('online')
    expect(presenceOf(s, 'U4', NOW)).toMatchObject({ kind: 'offline', lastSeen: '2026-11-05T09:00:00Z' })
  })
  it('someone unknown is offline, never seen', () => {
    expect(presenceOf(state(), 'U9', NOW)).toEqual({ kind: 'offline', label: 'Offline', lastSeen: null })
  })
  it('yourself: your own status, Invisible drawn as itself', () => {
    expect(presenceOf(state(), 'U1', NOW).kind).toBe('online')
    expect(presenceOf(state({ myStatus: { status: 'invisible', dnd_until: null } }), 'U1', NOW)).toEqual({ kind: 'invisible', label: 'Invisible', lastSeen: null })
    expect(presenceOf(state({ myStatus: { status: 'dnd', dnd_until: null } }), 'U1', NOW)).toMatchObject({ kind: 'dnd', label: 'Do Not Disturb' })
  })
  it('yourself: a DND timer that has run out is Online', () => {
    const s = state({ myStatus: { status: 'dnd', dnd_until: '2026-11-05T10:30:00.000Z' } })
    expect(presenceOf(s, 'U1', NOW).kind).toBe('dnd')
    expect(presenceOf(s, 'U1', '2026-11-05T10:30:00.000Z').kind).toBe('online')
  })
  it('never invisible for anyone else', () => {
    const s = state()
    s.people.U2 = { ...s.people.U2, status: 'invisible' as never }
    expect(presenceOf(s, 'U2', NOW).kind).not.toBe('invisible')
  })
})

describe('byPresence (#170): a member list split into Online and Offline', () => {
  it('online and DND are Online; offline and unknown are Offline; each keeps the list order', () => {
    const { online, offline } = byPresence(state(), ['U4', 'U3', 'U9', 'U2', 'U1'], (id) => id, NOW)
    expect(online).toEqual(['U3', 'U2', 'U1'])
    expect(offline).toEqual(['U4', 'U9'])
  })
  it('only what the viewer is told: someone sent as offline (Invisible, or hiding from you) is Offline', () => {
    const s = state()
    s.people.U2 = { ...s.people.U2, online: false, status: 'offline' }
    expect(byPresence(s, ['U2'], (id) => id, NOW).offline).toEqual(['U2'])
    // even a stray 'invisible' for someone else
    s.people.U2 = { ...s.people.U2, status: 'invisible' as never }
    expect(byPresence(s, ['U2'], (id) => id, NOW).offline).toEqual(['U2'])
  })
  it('yourself while Invisible: Offline, where everyone else sees you', () => {
    const s = state({ myStatus: { status: 'invisible', dnd_until: null } })
    expect(byPresence(s, ['U1'], (id) => id, NOW)).toEqual({ online: [], offline: ['U1'] })
  })
})
