import { describe, expect, it } from 'vitest'
import { applyEvent, applyReady, emptyState } from './state'
import { conversationName, conversations, homeBadge, isMuted, messageSound, serverBadge, shouldMarkRead } from './conversations'
import type { Ready } from './protocol/Ready'
import type { Message } from './protocol/Message'

const user = (id: string, username: string) => ({ id, username, avatar_hash: null })
const msg = (id: string, channel: string, author: string | null, extra: Partial<Message> = {}): Message => ({
  id, channel_id: channel, author_id: author, kind: 'normal', content: 'x', reply_to_id: null,
  created_at: '2026-10-08T10:00:00Z', edited_at: null, deleted: false, mentions: [], ...extra,
})
function ready(over: Partial<Ready> = {}): Ready {
  return {
    me: user('ME', 'greg'),
    servers: [{ id: 'S1', name: 'Main', icon_hash: null }],
    channels: [
      { id: 'DM1', server_id: null, kind: 'dm', name: null, position: 0 },
      { id: 'G1', server_id: null, kind: 'group', name: null, position: 0 },
      { id: 'T1', server_id: 'S1', kind: 'text', name: 'general', position: 0 },
    ],
    members: [{ server_id: 'S1', members: [{ user: user('ME', 'greg'), nickname: null }] }],
    dm_members: [{ channel_id: 'DM1', user_ids: ['ME', 'U1'] }, { channel_id: 'G1', user_ids: ['ME', 'U1', 'U2'] }],
    voice: [],
    people: [
      { user: user('ME', 'greg'), online: true, last_seen_at: null },
      { user: user('U1', 'sam'), online: true, last_seen_at: null },
      { user: user('U2', 'jo'), online: false, last_seen_at: '2026-10-08T08:00:00Z' },
    ],
    read_states: [
      { channel_id: 'DM1', last_read_message_id: 'M1', unread: 2, mentions: 0 },
      { channel_id: 'G1', last_read_message_id: null, unread: 1, mentions: 1 },
      { channel_id: 'T1', last_read_message_id: null, unread: 4, mentions: 0 },
    ],
    mutes: [],
    hidden: [],
    latest: [msg('M3', 'DM1', 'U1'), msg('M9', 'G1', 'U2')],
    ...over,
  }
}
const NOW = '2026-10-08T12:00:00Z'

describe('conversations', () => {
  it('names DMs after the other person and groups after members unless named', () => {
    const s = applyReady(emptyState(), ready())
    expect(conversationName(s, 'DM1')).toBe('sam')
    expect(conversationName(s, 'G1')).toBe('sam, jo')
    const named = applyEvent(s, { t: 'ChannelUpdated', d: { channel: { id: 'G1', server_id: null, kind: 'group', name: 'raid squad', position: 0 } } }, 0)
    expect(conversationName(named, 'G1')).toBe('raid squad')
  })

  it('lists newest activity first and hides closed ones', () => {
    const s = applyReady(emptyState(), ready())
    expect(conversations(s).map((c) => c.channel.id)).toEqual(['G1', 'DM1'])
    const closed = applyReady(emptyState(), ready({ hidden: ['G1'] }))
    expect(conversations(closed).map((c) => c.channel.id)).toEqual(['DM1'])
  })

  it('a new message reopens a closed conversation and counts as unread (not your own)', () => {
    let s = applyReady(emptyState(), ready({ hidden: ['DM1'] }))
    s = applyEvent(s, { t: 'MessageCreated', d: { message: msg('M10', 'DM1', 'U1'), nonce: null } }, 0)
    expect(s.hidden['DM1']).toBeUndefined()
    expect(s.reads['DM1'].unread).toBe(3)
    s = applyEvent(s, { t: 'MessageCreated', d: { message: msg('M11', 'DM1', 'ME'), nonce: null } }, 0)
    expect(s.reads['DM1'].unread).toBe(3)
    s = applyEvent(s, { t: 'MessageCreated', d: { message: msg('M12', 'DM1', null, { kind: 'system' }), nonce: null } }, 0)
    expect(s.reads['DM1'].unread).toBe(3)
  })

  it('mentions of me count; read state resets on sync', () => {
    let s = applyReady(emptyState(), ready())
    s = applyEvent(s, { t: 'MessageCreated', d: { message: msg('M20', 'T1', 'U1', { mentions: ['ME'] }), nonce: null } }, 0)
    expect(s.reads['T1']).toMatchObject({ unread: 5, mentions: 1 })
    s = applyEvent(s, { t: 'ReadStateUpdated', d: { user_id: 'ME', channel_id: 'T1', last_read_message_id: 'M20' } }, 0)
    expect(s.reads['T1']).toMatchObject({ unread: 0, mentions: 0, last_read_message_id: 'M20' })
  })

  it('badges: home sums unmuted DM unread + all mentions; servers show a dot and mentions', () => {
    const s = applyReady(emptyState(), ready())
    expect(homeBadge(s, NOW)).toBe(2 + 1 + 1)
    expect(serverBadge(s, 'S1', NOW)).toEqual({ dot: true, mentions: 0 })
    const muted = applyReady(emptyState(), ready({ mutes: [{ target_kind: 'channel', target_id: 'DM1', until: null }] }))
    expect(homeBadge(muted, NOW)).toBe(1 + 1)
  })

  it('mute_with_past_until_is_not_muted', () => {
    const s = applyReady(emptyState(), ready({ mutes: [{ target_kind: 'server', target_id: 'S1', until: '2026-10-08T11:00:00Z' }] }))
    expect(isMuted(s, s.channels['T1'], NOW)).toBe(false)
    expect(isMuted(s, s.channels['T1'], '2026-10-08T10:30:00Z')).toBe(true)
  })

  it('removal drops the conversation entirely', () => {
    let s = applyReady(emptyState(), ready())
    s = applyEvent(s, { t: 'ChannelRemoved', d: { channel_id: 'G1', user_id: 'ME' } }, 0)
    expect(s.channels['G1']).toBeUndefined()
    expect(s.reads['G1']).toBeUndefined()
    expect(conversations(s).map((c) => c.channel.id)).toEqual(['DM1'])
  })

  it('read rule needs open + focused + at bottom', () => {
    expect(shouldMarkRead({ open: true, focused: true, atBottom: true })).toBe(true)
    expect(shouldMarkRead({ open: true, focused: false, atBottom: true })).toBe(false)
    expect(shouldMarkRead({ open: true, focused: true, atBottom: false })).toBe(false)
  })

  it('sound: DMs, groups and mentions; never own/system/open-and-focused/muted (mentions ping through mute); 2 s limit', () => {
    const s = applyReady(emptyState(), ready())
    const base = { me: 'ME', open: false, focused: true, muted: false, lastPlayedAt: 0, now: 10_000 }
    const dm = s.channels['DM1'], text = s.channels['T1']
    expect(messageSound({ ...base, message: msg('A', 'DM1', 'U1'), channel: dm })).toBe(true)
    expect(messageSound({ ...base, message: msg('A', 'T1', 'U1'), channel: text })).toBe(false)
    expect(messageSound({ ...base, message: msg('A', 'T1', 'U1', { mentions: ['ME'] }), channel: text })).toBe(true)
    expect(messageSound({ ...base, message: msg('A', 'DM1', 'ME'), channel: dm })).toBe(false)
    expect(messageSound({ ...base, message: msg('A', 'DM1', null, { kind: 'system' }), channel: dm })).toBe(false)
    expect(messageSound({ ...base, open: true, message: msg('A', 'DM1', 'U1'), channel: dm })).toBe(false)
    expect(messageSound({ ...base, muted: true, message: msg('A', 'DM1', 'U1'), channel: dm })).toBe(false)
    expect(messageSound({ ...base, muted: true, message: msg('A', 'DM1', 'U1', { mentions: ['ME'] }), channel: dm })).toBe(true)
    expect(messageSound({ ...base, lastPlayedAt: 9_000, message: msg('A', 'DM1', 'U1'), channel: dm })).toBe(false)
  })

  it('presence and new users update the people map; group membership updates', () => {
    let s = applyReady(emptyState(), ready())
    s = applyEvent(s, { t: 'PresenceChanged', d: { user_id: 'U1', online: false, last_seen_at: NOW } }, 0)
    expect(s.people['U1']).toMatchObject({ online: false, last_seen_at: NOW })
    s = applyEvent(s, { t: 'UserCreated', d: { user: user('U3', 'riley') } }, 0)
    expect(s.people['U3'].online).toBe(false)
    s = applyEvent(s, { t: 'GroupMembersChanged', d: { channel_id: 'G1', user_ids: ['ME', 'U1', 'U3'] } }, 0)
    expect(s.dmMembers['G1']).toEqual(['ME', 'U1', 'U3'])
  })
})
