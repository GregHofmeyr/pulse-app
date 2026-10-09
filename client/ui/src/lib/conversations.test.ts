import { describe, expect, it } from 'vitest'
import { applyEvent, applyReady, emptyState } from './state'
import { conversationName, conversations, homeBadge, inVoice, isMuted, messageSound, serverBadge, shouldMarkRead } from './conversations'
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
    expect(conversations(s, NOW).map((c) => c.channel.id)).toEqual(['G1', 'DM1'])
    const closed = applyReady(emptyState(), ready({ hidden: ['G1'] }))
    expect(conversations(closed, NOW).map((c) => c.channel.id)).toEqual(['DM1'])
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
    expect(homeBadge(s, NOW)).toBe(2 + 1) // DM1 2 unread + G1 1 unread (its mention is that same message)
    expect(serverBadge(s, 'S1', NOW)).toEqual({ dot: true, mentions: 0 })
    const muted = applyReady(emptyState(), ready({ mutes: [{ target_kind: 'channel', target_id: 'DM1', until: null }] }))
    expect(homeBadge(muted, NOW)).toBe(1) // DM1 muted (no mentions) + G1 1 unread
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
    expect(conversations(s, NOW).map((c) => c.channel.id)).toEqual(['DM1'])
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

import { previewText, relativeTime } from './conversations'

describe('home helpers', () => {
  const now = Date.parse('2026-10-08T12:00:00Z')
  it('relative time: just now, minutes, hours, days', () => {
    expect(relativeTime('2026-10-08T11:59:40Z', now)).toBe('just now')
    expect(relativeTime('2026-10-08T11:45:00Z', now)).toBe('15m ago')
    expect(relativeTime('2026-10-08T09:00:00Z', now)).toBe('3h ago')
    expect(relativeTime('2026-10-05T12:00:00Z', now)).toBe('3d ago')
  })

  it('preview: you/others prefixes, system italics flag, deleted', () => {
    const s = applyReady(emptyState(), ready())
    expect(previewText(s, msg('A', 'DM1', 'ME', { content: 'yo' }))).toEqual({ text: 'You: yo', system: false })
    expect(previewText(s, msg('A', 'DM1', 'U1', { content: 'hi' }))).toEqual({ text: 'hi', system: false })
    expect(previewText(s, msg('A', 'G1', 'U1', { content: 'hi' }))).toEqual({ text: 'sam: hi', system: false })
    expect(previewText(s, msg('A', 'G1', null, { kind: 'system', content: 'sam added jo' }))).toEqual({ text: 'sam added jo', system: true })
    expect(previewText(s, msg('A', 'DM1', 'U1', { deleted: true, content: '' }))).toEqual({ text: 'message deleted', system: true })
  })
})

import { firstUnreadIndex, reanchor } from './conversations'

describe('NEW divider', () => {
  const list = [msg('M1', 'DM1', 'U1'), msg('M2', 'DM1', 'ME'), msg('M3', 'DM1', 'U1'), msg('M4', 'DM1', 'U1')]
  it('marks the first message after the read point written by someone else', () => {
    expect(firstUnreadIndex(list, 'M1', 'ME')).toBe(2) // M2 is mine
    expect(firstUnreadIndex(list, 'M4', 'ME')).toBe(-1)
    const theirs = list.filter((m) => m.author_id !== 'ME')
    expect(firstUnreadIndex(theirs, null, 'ME')).toBe(0) // never read anything
    expect(firstUnreadIndex([msg('S', 'DM1', null, { kind: 'system' }), ...theirs], null, 'ME')).toBe(1)
  })

  it('goes away once you reply below it', () => {
    expect(firstUnreadIndex([...list, msg('M5', 'DM1', 'ME')], 'M1', 'ME')).toBe(-1)
    expect(firstUnreadIndex([...list, msg('M5', 'DM1', 'ME'), msg('M6', 'DM1', 'U1')], 'M1', 'ME')).toBe(-1)
  })
})

describe('review fixes', () => {
  it('I1: a brand-new DM counts unread before any reconnect', () => {
    let s = applyReady(emptyState(), ready())
    s = applyEvent(s, { t: 'ChannelCreated', d: { channel: { id: 'DM9', server_id: null, kind: 'dm', name: null, position: 0 } } }, 0)
    s = applyEvent(s, { t: 'MessageCreated', d: { message: msg('M50', 'DM9', 'U1', { mentions: ['ME'] }), nonce: null } }, 0)
    expect(s.reads['DM9']).toMatchObject({ unread: 1, mentions: 1 })
  })

  it('I1: a server channel counts only if you are a member of that server', () => {
    let s = applyReady(emptyState(), ready({
      servers: [{ id: 'S1', name: 'Main', icon_hash: null }, { id: 'S2', name: 'Other', icon_hash: null }],
      channels: [{ id: 'T1', server_id: 'S1', kind: 'text', name: 'general', position: 0 }, { id: 'X1', server_id: 'S2', kind: 'text', name: 'general', position: 0 }],
      read_states: [],
    }))
    s = applyEvent(s, { t: 'MessageCreated', d: { message: msg('M60', 'T1', 'U1'), nonce: null } }, 0)
    s = applyEvent(s, { t: 'MessageCreated', d: { message: msg('M61', 'X1', 'U1'), nonce: null } }, 0)
    expect(s.reads['T1']?.unread).toBe(1)
    expect(s.reads['X1']).toBeUndefined()
  })

  it('I2: a read point older than newer messages keeps counting them', () => {
    let s = applyReady(emptyState(), ready())
    s = applyEvent(s, { t: 'MessageCreated', d: { message: msg('M70', 'DM1', 'ME'), nonce: null } }, 0)
    s = applyEvent(s, { t: 'MessageCreated', d: { message: msg('M71', 'DM1', 'U1', { mentions: ['ME'] }), nonce: null } }, 0)
    s = applyEvent(s, { t: 'ReadStateUpdated', d: { user_id: 'ME', channel_id: 'DM1', last_read_message_id: 'M70' } }, 0)
    expect(s.reads['DM1']).toMatchObject({ unread: 1, mentions: 1, last_read_message_id: 'M70' })
  })

  it('badge: one DM message that mentions you counts once', () => {
    const s = applyReady(emptyState(), ready({
      read_states: [{ channel_id: 'DM1', last_read_message_id: null, unread: 1, mentions: 1 }],
    }))
    expect(homeBadge(s, NOW)).toBe(1)
  })
})

import { watchOpen } from './conversations'

describe('I3: removal detection', () => {
  it('only fires when a channel you were seeing disappears, never for one not yet arrived', () => {
    let w = watchOpen(null, 'NEW', false) // opened before ChannelCreated landed
    expect(w.removed).toBe(false)
    w = watchOpen(w.state, 'NEW', true) // it arrives
    expect(w.removed).toBe(false)
    w = watchOpen(w.state, 'NEW', false) // then it's taken away
    expect(w.removed).toBe(true)
    w = watchOpen(w.state, 'OTHER', false) // switching to another not-yet-arrived channel
    expect(w.removed).toBe(false)
  })
})

describe('inVoice', () => {
  it('counts people across a server’s voice channels only', () => {
    const base = ready({
      servers: [{ id: 'S1', name: 'Main', icon_hash: null }, { id: 'S2', name: 'Other', icon_hash: null }],
      channels: [
        { id: 'V1', server_id: 'S1', kind: 'voice', name: 'a', position: 0 },
        { id: 'V2', server_id: 'S1', kind: 'voice', name: 'b', position: 1 },
        { id: 'V3', server_id: 'S2', kind: 'voice', name: 'c', position: 0 },
      ],
    })
    const s = applyReady(emptyState(), base)
    expect(inVoice(s, 'S1')).toBe(0)
    const flags = { muted: false, deafened: false }
    const busy = applyReady(emptyState(), {
      ...base,
      voice: [
        { channel_id: 'V1', members: [{ user_id: 'U1', flags }] },
        { channel_id: 'V2', members: [{ user_id: 'U2', flags }, { user_id: 'ME', flags }] },
      ],
    } as Ready)
    expect(inVoice(busy, 'S1')).toBe(3)
    expect(inVoice(busy, 'S2')).toBe(0)
  })

  it('stays hidden on servers you have not joined', () => {
    const flags = { muted: false, deafened: false }
    const s = applyReady(emptyState(), ready({
      servers: [{ id: 'S1', name: 'Main', icon_hash: null }, { id: 'S2', name: 'Other', icon_hash: null }],
      channels: [{ id: 'V3', server_id: 'S2', kind: 'voice', name: 'c', position: 0 }],
      members: [
        { server_id: 'S1', members: [{ user: user('ME', 'greg'), nickname: null }] },
        { server_id: 'S2', members: [{ user: user('U1', 'sam'), nickname: null }] },
      ],
      voice: [{ channel_id: 'V3', members: [{ user_id: 'U1', flags }] }],
    }))
    expect(inVoice(s, 'S2')).toBe(0)
  })
})

describe('closed conversations', () => {
  it('do not count towards the Home badge until a new message reopens them', () => {
    let s = applyReady(emptyState(), ready({ hidden: ['DM1'] }))
    expect(homeBadge(s, NOW)).toBe(1) // only G1; closed DM1's 2 unread are out of sight
    s = applyEvent(s, { t: 'MessageCreated', d: { message: msg('M9', 'DM1', 'U1'), nonce: null } }, 0)
    expect(homeBadge(s, NOW)).toBe(3 + 1)
  })
})

describe('NEW divider anchor', () => {
  it('moves to the newest message when you stop watching, and only then', () => {
    expect(reanchor('M1', { was: true, now: false, newest: 'M7' })).toBe('M7') // looked away
    expect(reanchor('M1', { was: true, now: true, newest: 'M7' })).toBe('M1') // still watching
    expect(reanchor('M1', { was: false, now: true, newest: 'M7' })).toBe('M1') // came back: keep it so the line shows
    expect(reanchor('M1', { was: true, now: false, newest: null })).toBe('M1') // empty chat
  })

  it('three-person round: each away stretch gets its own line', () => {
    // greg watched up to M2, then switched windows; sam and jo wrote M3, M4
    const msgs = [msg('M1', 'G1', 'U1'), msg('M2', 'G1', 'ME'), msg('M3', 'G1', 'U1'), msg('M4', 'G1', 'U2')]
    let anchor = reanchor('M1', { was: true, now: false, newest: 'M2' })
    expect(firstUnreadIndex(msgs, anchor, 'ME')).toBe(2)
    // greg replies (M5): line goes; looks away again, U1 writes M6
    const more = [...msgs, msg('M5', 'G1', 'ME'), msg('M6', 'G1', 'U1')]
    expect(firstUnreadIndex(more.slice(0, 5), anchor, 'ME')).toBe(-1)
    anchor = reanchor(anchor, { was: true, now: false, newest: 'M5' })
    expect(firstUnreadIndex(more, anchor, 'ME')).toBe(5)
  })
})
