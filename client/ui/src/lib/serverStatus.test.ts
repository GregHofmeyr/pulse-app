// Your status in one server (#154): the menu's choices, how someone looks in a server where they set one, your own
// dot there, and what DND in a server means for your alerts.
import { describe, expect, it } from 'vitest'
import { SERVER_STATUS_CHOICES, dndIn, myStatusIn } from './serverStatus'
import { byPresence, presenceOf } from './presence'
import { applyEvent, emptyState, type AppState } from './state'

const NOW = '2026-11-19T12:00:00.000Z'
const me = { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const sam = { ...me, id: 'U2', username: 'sam', display_name: 'sam' }

const state = (over: Partial<AppState> = {}): AppState =>
  ({
    ...emptyState(),
    me,
    people: {
      U2: { user: sam, online: true, last_seen_at: null, status: 'online', in_servers: [{ server_id: 'S1', status: 'offline', last_seen_at: '2026-11-19T11:00:00.000Z' }] },
    },
    ...over,
  }) as AppState

describe('My status here', () => {
  it('offers Same as everywhere, then the three statuses', () => {
    expect(SERVER_STATUS_CHOICES.map((c) => c.label)).toEqual(['Same as everywhere', 'Online', 'Do Not Disturb', 'Invisible'])
    expect(SERVER_STATUS_CHOICES[0].status).toBeNull()
  })

  it('someone shows their status in that server only, their main one everywhere else', () => {
    const s = state()
    expect(presenceOf(s, 'U2', NOW).kind).toBe('online')
    expect(presenceOf(s, 'U2', NOW, 'S2').kind).toBe('online')
    const here = presenceOf(s, 'U2', NOW, 'S1')
    expect(here.kind).toBe('offline')
    expect(here.lastSeen).toBe('2026-11-19T11:00:00.000Z')
    // the member list's split follows it too
    expect(byPresence(s, ['U2'], (x) => x, NOW, 'S1').offline).toEqual(['U2'])
    expect(byPresence(s, ['U2'], (x) => x, NOW).online).toEqual(['U2'])
  })

  it('your own dot in a server is your status there', () => {
    const s = state({ myServerStatuses: [{ server_id: 'S1', status: 'invisible' }] })
    expect(myStatusIn(s, 'S1')).toBe('invisible')
    expect(myStatusIn(s, 'S2')).toBeNull()
    expect(presenceOf(s, 'U1', NOW, 'S1').kind).toBe('invisible')
    expect(presenceOf(s, 'U1', NOW).kind).toBe('online')
  })

  it('DND in a server holds back its alerts; Online there lets them through while DND everywhere else', () => {
    const s = state({ myServerStatuses: [{ server_id: 'S1', status: 'dnd' }] })
    expect(dndIn(s, 'S1', NOW)).toBe(true)
    expect(dndIn(s, 'S2', NOW)).toBe(false)
    expect(dndIn(s, null, NOW)).toBe(false)
    const t = state({ myStatus: { status: 'dnd', dnd_until: null }, myServerStatuses: [{ server_id: 'S1', status: 'online' }] })
    expect(dndIn(t, 'S1', NOW)).toBe(false)
    expect(dndIn(t, 'S2', NOW)).toBe(true)
    expect(dndIn(t, null, NOW)).toBe(true)
  })

  it('MyServerStatusesChanged and PresenceChanged keep it', () => {
    const r = state({ myServerStatuses: [{ server_id: 'S1', status: 'dnd' }] })
    expect(applyEvent(r, { t: 'MyServerStatusesChanged', d: { user_id: 'U9', list: [] } }, 0)).toBe(r)
    const t = applyEvent(r, { t: 'MyServerStatusesChanged', d: { user_id: 'U1', list: [] } }, 0)
    expect(t.myServerStatuses).toEqual([])
    const u = applyEvent(t, { t: 'PresenceChanged', d: { user_id: 'U2', online: true, last_seen_at: null, status: 'online', in_servers: [{ server_id: 'S1', status: 'dnd', last_seen_at: null }] } }, 0)
    expect(presenceOf(u, 'U2', NOW, 'S1').kind).toBe('dnd')
    const v = applyEvent(u, { t: 'PresenceChanged', d: { user_id: 'U2', online: true, last_seen_at: null, status: 'online' } }, 0)
    expect(presenceOf(v, 'U2', NOW, 'S1').kind).toBe('online')
  })
})
