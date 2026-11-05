// Your status and who you appear offline to (#45), as the state keeps them.
import { describe, expect, it } from 'vitest'
import { applyEvent, applyReady, emptyState } from './state'
import { testMe, testReady } from './testReady'
import type { Ready } from './protocol/Ready'

const sam = { id: 'U2', username: 'sam', display_name: 'sam', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const kim = { id: 'U3', username: 'kim', display_name: 'kim', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }

describe('status in Ready', () => {
  it('takes your status, DND end and hidden list', () => {
    const s = applyReady(emptyState(), testReady([], { status: { status: 'dnd', dnd_until: '2026-11-05T12:00:00Z' }, appear_offline_to: ['U2'] }))
    expect(s.myStatus).toEqual({ status: 'dnd', dnd_until: '2026-11-05T12:00:00Z' })
    expect(s.appearOfflineTo).toEqual(['U2'])
  })
  it('an older server sends neither: online, hidden from nobody', () => {
    const r = testReady([]) as Partial<Ready>
    delete r.status
    delete r.appear_offline_to
    const s = applyReady({ ...emptyState(), myStatus: { status: 'invisible', dnd_until: null }, appearOfflineTo: ['U9'] }, r as Ready)
    expect(s.myStatus).toEqual({ status: 'online', dnd_until: null })
    expect(s.appearOfflineTo).toEqual([])
  })
  it("a person's status comes along; an older server's (no status) goes by online", () => {
    const people = [
      { user: sam, online: true, last_seen_at: null, status: 'dnd' as const },
      { user: kim, online: true, last_seen_at: null } as never,
      { user: testMe, online: false, last_seen_at: '2026-11-01T00:00:00Z' } as never,
    ]
    const s = applyReady(emptyState(), testReady([], { people }))
    expect(s.people.U2.status).toBe('dnd')
    expect(s.people.U3.status).toBe('online')
    expect(s.people.U1.status).toBe('offline')
  })
})

describe('status events', () => {
  const base = () => applyReady(emptyState(), testReady([], { people: [{ user: sam, online: true, last_seen_at: null, status: 'online' }] }))

  it('PresenceChanged stores the status as this viewer may see it', () => {
    let s = applyEvent(base(), { t: 'PresenceChanged', d: { user_id: 'U2', online: true, last_seen_at: null, status: 'dnd' } }, 0)
    expect(s.people.U2).toMatchObject({ online: true, status: 'dnd' })
    s = applyEvent(s, { t: 'PresenceChanged', d: { user_id: 'U2', online: false, last_seen_at: '2026-11-05T10:00:00Z', status: 'offline' } }, 0)
    expect(s.people.U2).toMatchObject({ online: false, status: 'offline', last_seen_at: '2026-11-05T10:00:00Z' })
  })
  it('PresenceChanged from an older server (no status) goes by online', () => {
    let s = applyEvent(base(), { t: 'PresenceChanged', d: { user_id: 'U2', online: false, last_seen_at: null } as never }, 0)
    expect(s.people.U2.status).toBe('offline')
    s = applyEvent(s, { t: 'PresenceChanged', d: { user_id: 'U2', online: true, last_seen_at: null } as never }, 0)
    expect(s.people.U2.status).toBe('online')
  })
  it('a new user starts offline', () => {
    const s = applyEvent(base(), { t: 'UserCreated', d: { user: kim } }, 0)
    expect(s.people.U3).toMatchObject({ online: false, status: 'offline' })
  })
  it('StatusChanged for you sets your status; for anyone else it is ignored', () => {
    const own = { status: 'dnd' as const, dnd_until: null }
    const s = applyEvent(base(), { t: 'StatusChanged', d: { user_id: 'U1', status: own } }, 0)
    expect(s.myStatus).toEqual(own)
    const t = applyEvent(s, { t: 'StatusChanged', d: { user_id: 'U2', status: { status: 'invisible', dnd_until: null } } }, 0)
    expect(t.myStatus).toEqual(own)
  })
  it('AppearOfflineChanged adds and removes, once each; only yours counts', () => {
    let s = applyEvent(base(), { t: 'AppearOfflineChanged', d: { user_id: 'U1', target_id: 'U2', hidden: true } }, 0)
    s = applyEvent(s, { t: 'AppearOfflineChanged', d: { user_id: 'U1', target_id: 'U2', hidden: true } }, 0)
    expect(s.appearOfflineTo).toEqual(['U2'])
    s = applyEvent(s, { t: 'AppearOfflineChanged', d: { user_id: 'U9', target_id: 'U3', hidden: true } }, 0)
    expect(s.appearOfflineTo).toEqual(['U2'])
    s = applyEvent(s, { t: 'AppearOfflineChanged', d: { user_id: 'U1', target_id: 'U2', hidden: false } }, 0)
    expect(s.appearOfflineTo).toEqual([])
  })
})
