import { describe, expect, it } from 'vitest'
import { applyEvent, applyReady, countsAsUnread, emptyState } from './state'
import { callDuration, callLineText, ringAlerts, ringNoticeTag, ringPlace } from './calls'
import { testReady } from './testReady'
import type { Message } from './protocol/Message'
import type { CallRing } from './protocol/CallRing'

const user = (id: string, username: string) => ({ id, username, display_name: username, avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } })
const DM = { id: 'DM1', server_id: null, kind: 'dm' as const, name: null, position: 0 }
const GROUP = { id: 'G1', server_id: null, kind: 'group' as const, name: 'Friends', position: 0 }
const state = () =>
  applyReady(emptyState(), testReady([DM, GROUP], {
    me: user('U1', 'alex'),
    dm_members: [{ channel_id: 'DM1', user_ids: ['U1', 'U2'] }, { channel_id: 'G1', user_ids: ['U1', 'U2', 'U3'] }],
    people: [
      { user: user('U1', 'alex'), online: true, last_seen_at: null, status: 'online' },
      { user: user('U2', 'sam'), online: true, last_seen_at: null, status: 'online' },
      { user: user('U3', 'jo'), online: true, last_seen_at: null, status: 'online' },
    ],
  }))
const line = (started_by: string, over: { ended_at?: string | null; joined?: string[]; channel?: string } = {}): Message => ({
  id: 'L1', channel_id: over.channel ?? 'DM1', author_id: null, kind: 'system', content: 'sam started a call', reply_to_id: null,
  created_at: '2026-11-05T10:00:00.000Z', edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null, quote: null,
  qoty_card: null, clip: null, embeds: [], call: { call_id: 'C1', started_by, ended_at: over.ended_at ?? null, joined: over.joined ?? [] },
})
const ring = (channel = 'DM1'): CallRing => ({ call_id: 'C1', channel_id: channel, caller_id: 'U2', expires_at: '2026-11-05T10:00:30.000Z' })

describe('the call line', () => {
  it('reads "started a call" while it is going, with the duration once it ends', () => {
    const s = state()
    expect(callLineText(s, line('U2'))).toBe('sam started a call')
    expect(callLineText(s, line('U2', { ended_at: '2026-11-05T10:12:10.000Z', joined: ['U1', 'U2'] }))).toBe('sam started a call · 12 min')
    expect(callLineText(s, line('U1', { ended_at: '2026-11-05T10:00:20.000Z', joined: ['U1'] }))).toBe('You started a call · 1 min')
  })
  it('is a missed call for a member who never joined, never for the caller', () => {
    const s = state()
    const ended = { ended_at: '2026-11-05T10:05:00.000Z', joined: ['U2', 'U3'] }
    expect(callLineText(s, line('U2', ended))).toBe('Missed call from sam')
    expect(callLineText(s, line('U1', { ended_at: '2026-11-05T10:05:00.000Z', joined: [] }))).toBe('You started a call · 5 min')
    // still going: nobody has missed it yet
    expect(callLineText(s, line('U2', { joined: [] }))).toBe('sam started a call')
  })
  it('the conversation list shows the same words', async () => {
    const { previewText } = await import('./conversations')
    expect(previewText(state(), line('U2', { ended_at: '2026-11-05T10:05:00.000Z' }))).toEqual({ text: 'Missed call from sam', system: true })
  })
  it('durations', () => {
    const at = '2026-11-05T10:00:00.000Z'
    expect(callDuration(at, '2026-11-05T10:00:01.000Z')).toBe('1 min')
    expect(callDuration(at, '2026-11-05T10:59:59.000Z')).toBe('60 min')
    expect(callDuration(at, '2026-11-05T11:30:00.000Z')).toBe('1 h 30 min')
    expect(callDuration(at, '2026-11-05T12:00:00.000Z')).toBe('2 h')
  })
})

describe('unread', () => {
  it('a call line is unread for everyone but the caller, and never a mention', () => {
    expect(countsAsUnread(line('U2'), 'U1')).toBe(true)
    expect(countsAsUnread(line('U1'), 'U1')).toBe(false)
    const s = applyEvent(state(), { t: 'MessageCreated', d: { message: line('U2'), nonce: null } }, 0)
    expect(s.reads.DM1).toMatchObject({ unread: 1, mentions: 0 })
    const mine = applyEvent(state(), { t: 'MessageCreated', d: { message: line('U1'), nonce: null } }, 0)
    expect(mine.reads.DM1?.unread ?? 0).toBe(0)
    // other system lines still don't count
    const other = { ...line('U2'), call: undefined }
    expect(countsAsUnread(other, 'U1')).toBe(false)
  })
})

describe('calls in state', () => {
  const call = { id: 'C1', channel_id: 'DM1', started_by: 'U2', started_at: '2026-11-05T10:00:00.000Z', line_id: 'L1' }
  it('come from Ready and the call events', () => {
    const s = applyReady(emptyState(), testReady([DM], { calls: [call] }))
    expect(s.calls.DM1).toEqual(call)
    const ended = applyEvent(s, { t: 'CallEnded', d: { channel_id: 'DM1', call_id: 'C1' } }, 0)
    expect(ended.calls.DM1).toBeUndefined()
    const again = applyEvent(ended, { t: 'CallStarted', d: { call: { ...call, id: 'C2' } } }, 0)
    expect(again.calls.DM1.id).toBe('C2')
    // a late end of the old call leaves the new one alone
    expect(applyEvent(again, { t: 'CallEnded', d: { channel_id: 'DM1', call_id: 'C1' } }, 0).calls.DM1.id).toBe('C2')
    // an older server's Ready has none
    const { calls: _c, ...old } = testReady([DM])
    expect(applyReady(emptyState(), old).calls).toEqual({})
  })
})

describe('ringing', () => {
  it('a popover outside the conversation, the band inside it', () => {
    expect(ringPlace(ring(), null)).toBe('popover')
    expect(ringPlace(ring(), 'G1')).toBe('popover')
    expect(ringPlace(ring(), 'DM1')).toBe('band')
  })
  it('alerts (popover, ringtone, notification) unless DND or the conversation is muted', () => {
    const now = '2026-11-05T10:00:05.000Z'
    const s = state()
    expect(ringAlerts(s, ring(), now)).toBe(true)
    expect(ringAlerts({ ...s, myStatus: { status: 'dnd', dnd_until: null } }, ring(), now)).toBe(false)
    // a DND that ran out rings again
    expect(ringAlerts({ ...s, myStatus: { status: 'dnd', dnd_until: '2026-11-05T10:00:00.000Z' } }, ring(), now)).toBe(true)
    // invisible rings as normal
    expect(ringAlerts({ ...s, myStatus: { status: 'invisible', dnd_until: null } }, ring(), now)).toBe(true)
    const muted = { ...s, mutes: [{ target_kind: 'channel' as const, target_id: 'DM1', until: null }] }
    expect(ringAlerts(muted, ring(), now)).toBe(false)
    expect(ringAlerts(muted, ring('G1'), now)).toBe(true)
  })
})

describe('the ring notification tag (#176)', () => {
  it("is the call's own, apart from the conversation's message notifications, and fits a Windows tag", () => {
    const id = '01JC0000000000000000000000'
    expect(ringNoticeTag(id)).toBe(`call-${id}`)
    expect(ringNoticeTag(id)).not.toBe('G1')
    expect(ringNoticeTag(id).length).toBeLessThanOrEqual(64)
  })
})
