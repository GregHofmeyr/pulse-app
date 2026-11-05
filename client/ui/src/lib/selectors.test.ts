import { describe, expect, it } from 'vitest'
import { applyEvent, applyReady, emptyState } from './state'
import { roleColorFor, userOf, callRoster, channelsFor, displayName, groupedWith, isLive, isMember, liveIn, mentionablesFor, nameOfIn, pillNamesFor, serverPeople, typingNames, voiceOccupants } from './selectors'
import type { Message } from './protocol/Message'
import type { Ready } from './protocol/Ready'
import type { StreamInfo } from './protocol/StreamInfo'

const me = { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const sam = { id: 'U2', username: 'sam', display_name: 'sam', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const r: Ready = {
  me,
  servers: [{ id: 'S1', name: 'Main', icon_hash: null }, { id: 'S2', name: 'Other', icon_hash: null }],
  channels: [
    { id: 'V1', server_id: 'S1', kind: 'voice', name: 'Lounge', position: 0 },
    { id: 'C2', server_id: 'S1', kind: 'text', name: 'memes', position: 1 },
    { id: 'C1', server_id: 'S1', kind: 'text', name: 'general', position: 0 },
    { id: 'X1', server_id: 'S2', kind: 'text', name: 'general', position: 0 },
    { id: 'D1', server_id: null, kind: 'dm', name: null, position: 0 },
  ],
  members: [{ server_id: 'S1', members: [{ user: me, nickname: null }, { user: sam, nickname: 'Big Dog' }] }, { server_id: 'S2', members: [{ user: sam, nickname: null }] }],
  dm_members: [],
  voice: [{ channel_id: 'V1', members: [{ user_id: 'U2', flags: { muted: true, deafened: false } }] }],
  people: [],
  read_states: [],
  mutes: [],
  hidden: [],
  latest: [],
  streams: [],
  custom_emoji: [],
  frequent_emoji: [], qoty: [], speaker_links: [], thread_seen: [], games: [], clip_views: [], private_nicknames: [], sounds: [], sound_favourites: [], frequent_sounds: [], server_voice: [], status: { status: 'online', dnd_until: null }, appear_offline_to: [], live_notify: [],
}
const s = applyReady(emptyState(), r)

describe('selectors', () => {
  it('channelsFor splits text/voice, sorted by position, only that server', () => {
    const c = channelsFor(s, 'S1')
    expect(c.text.map((x) => x.id)).toEqual(['C1', 'C2'])
    expect(c.voice.map((x) => x.id)).toEqual(['V1'])
    expect(c.quotes).toEqual([])
  })

  it('channelsFor lists quotes channels in their own group', () => {
    const q = applyReady(emptyState(), { ...r, channels: [...r.channels, { id: 'Q2', server_id: 'S1', kind: 'quotes', name: 'b', position: 1 }, { id: 'Q1', server_id: 'S1', kind: 'quotes', name: 'a', position: 0 }, { id: 'QX', server_id: 'S2', kind: 'quotes', name: 'x', position: 0 }] })
    const c = channelsFor(q, 'S1')
    expect(c.quotes.map((x) => x.id)).toEqual(['Q1', 'Q2'])
    expect(c.text.map((x) => x.id)).toEqual(['C1', 'C2'])
  })

  it('isMember', () => {
    expect(isMember(s, 'S1')).toBe(true)
    expect(isMember(s, 'S2')).toBe(false)
  })

  it('displayName prefers the server nickname', () => {
    expect(displayName(s, 'S1', 'U2')).toBe('Big Dog')
    expect(displayName(s, 'S2', 'U2')).toBe('sam')
    expect(displayName(s, 'S1', 'U404')).toBe('unknown')
  })

  describe('displayName precedence (#47, #48)', () => {
    const named = { ...sam, display_name: 'Sam 🎸' }
    const base = applyReady(emptyState(), {
      ...r,
      members: [{ server_id: 'S1', members: [{ user: me, nickname: null }, { user: named, nickname: 'Big Dog' }] }, { server_id: 'S2', members: [{ user: named, nickname: null }] }],
      people: [{ user: named, online: true, last_seen_at: null, status: 'online' as const }],
      dm_members: [{ channel_id: 'D1', user_ids: ['U1', 'U2'] }],
    })
    const priv = { ...base, privateNicks: { U2: 'Sammy' } }

    it('in a server: server nickname, else private nickname, else display name', () => {
      expect(displayName(priv, 'S1', 'U2')).toBe('Big Dog')
      expect(displayName(priv, 'S2', 'U2')).toBe('Sammy')
      expect(displayName(base, 'S2', 'U2')).toBe('Sam 🎸')
    })

    it('outside a server (DMs, groups, friends): private nickname, else display name', () => {
      expect(displayName(priv, null, 'U2')).toBe('Sammy')
      expect(displayName(base, null, 'U2')).toBe('Sam 🎸')
    })

    it('a server they are not in: private nickname, else display name, never another server’s nickname', () => {
      const lonely = { ...base, members: { S1: base.members.S1.filter((m) => m.user.id !== 'U2') } }
      expect(displayName(lonely, 'S1', 'U2')).toBe('Sam 🎸')
      expect(displayName({ ...lonely, privateNicks: { U2: 'Sammy' } }, 'S1', 'U2')).toBe('Sammy')
    })

    it('never shows the username once there is a display name; an older server sends none', () => {
      expect(displayName(base, null, 'U2')).not.toBe('sam')
      const old = applyReady(emptyState(), { ...r, people: [{ user: { ...sam, display_name: '' }, online: true, last_seen_at: null, status: 'online' as const }] })
      expect(displayName(old, null, 'U2')).toBe('sam')
    })

    it('me: my display name (no private nickname for myself)', () => {
      const mine = applyReady(emptyState(), { ...r, me: { ...me, display_name: 'Alex A' } })
      expect(displayName(mine, null, 'U1')).toBe('Alex A')
    })

    it('MemberUpdated and PrivateNicknameChanged change the shown name live', () => {
      let t = applyEvent(base, { t: 'MemberUpdated', d: { server_id: 'S1', member: { user: named, nickname: 'Captain' } } }, 0)
      expect(displayName(t, 'S1', 'U2')).toBe('Captain')
      t = applyEvent(t, { t: 'PrivateNicknameChanged', d: { user_id: 'U1', target_id: 'U2', nickname: 'Sammy' } }, 0)
      expect(displayName(t, null, 'U2')).toBe('Sammy')
      t = applyEvent(t, { t: 'PrivateNicknameChanged', d: { user_id: 'U1', target_id: 'U2', nickname: null } }, 0)
      expect(displayName(t, null, 'U2')).toBe('Sam 🎸')
      expect(t.privateNicks).toEqual({})
    })

    it('Ready loads my private nicknames', () => {
      const t = applyReady(emptyState(), { ...r, private_nicknames: [{ user_id: 'U2', nickname: 'Sammy' }] })
      expect(t.privateNicks).toEqual({ U2: 'Sammy' })
    })

    it('mentionables carry the shown name and every name they can be found by', () => {
      const st = { ...priv, people: { ...priv.people, U1: { user: me, online: true, last_seen_at: null, status: 'online' as const } } }
      expect(mentionablesFor(st, 'C1')).toEqual([{ id: 'U2', name: 'Big Dog', username: 'sam', alts: ['Sam 🎸', 'Big Dog', 'Sammy'] }])
      expect(mentionablesFor(st, 'D1')).toEqual([{ id: 'U2', name: 'Sammy', username: 'sam', alts: ['Sam 🎸', 'Sammy'] }])
    })
  })

  it('voiceOccupants resolves names and flags', () => {
    expect(voiceOccupants(s, 'S1', 'V1')).toEqual([{ userId: 'U2', name: 'Big Dog', muted: true, deafened: false }])
    expect(voiceOccupants(s, 'S1', 'nope')).toEqual([])
  })

  it('voiceOccupants carry who server muted or deafened them (#140)', () => {
    const t = { ...s, serverVoice: { S1: { U2: { server_id: 'S1', user_id: 'U2', muted_by: 'U1', deafened_by: 'U3' } } } }
    const [o] = voiceOccupants(t, 'S1', 'V1')
    expect(o.serverMutedBy).toBe('U1')
    expect(o.serverDeafenedBy).toBe('U3')
    // another server's mute doesn't show here, nor in a DM call
    expect(voiceOccupants(t, 'S2', 'V1')[0].serverMutedBy).toBeUndefined()
    expect(voiceOccupants(t, null, 'V1')[0].serverMutedBy).toBeUndefined()
  })

  it('typingNames excludes me and expired entries', () => {
    let t = applyEvent(s, { t: 'Typing', d: { channel_id: 'C1', user_id: 'U2' } }, 1000)
    t = applyEvent(t, { t: 'Typing', d: { channel_id: 'C1', user_id: 'U1' } }, 1000)
    expect(typingNames(t, 'S1', 'C1', 2000)).toEqual(['Big Dog'])
    expect(typingNames(t, 'S1', 'C1', 8000)).toEqual([])
  })
})

describe('stableList', () => {
  it('keeps the old array when nothing changed', async () => {
    const { stableList } = await import('./selectors')
    const a = ['sam', 'alex']
    expect(stableList(a, ['sam', 'alex'])).toBe(a)
    const b = ['sam']
    expect(stableList(a, b)).toBe(b)
    expect(stableList(a, ['alex', 'sam'])).not.toBe(a)
  })
  it('keepStable remembers the list it last handed out', async () => {
    const { keepStable } = await import('./selectors')
    const same = keepStable<string>()
    const a = same(['sam', 'alex'])
    expect(same(['sam', 'alex'])).toBe(a)
    const b = same(['sam'])
    expect(b).toEqual(['sam'])
    expect(same(['sam'])).toBe(b)
  })
})

describe('nextTypingExpiry', () => {
  it('is the earliest live expiry, or null when nobody is typing', async () => {
    const { nextTypingExpiry } = await import('./selectors')
    let t = applyEvent(applyReady(emptyState(), r), { t: 'Typing', d: { channel_id: 'C1', user_id: 'U2' } }, 3000)
    t = applyEvent(t, { t: 'Typing', d: { channel_id: 'C1', user_id: 'U3' } }, 1000) // added later, runs out first
    expect(nextTypingExpiry(t, 'C1', 2000)).toBe(7000)
    expect(nextTypingExpiry(t, 'C1', 7000)).toBe(9000)
    expect(nextTypingExpiry(t, 'C1', 9000)).toBeNull()
    expect(nextTypingExpiry(t, 'C2', 0)).toBeNull()
  })

  it('ignores your own typing, which is never shown', async () => {
    const { nextTypingExpiry } = await import('./selectors')
    let t = applyEvent(applyReady(emptyState(), r), { t: 'Typing', d: { channel_id: 'C1', user_id: 'U2' } }, 3000)
    t = applyEvent(t, { t: 'Typing', d: { channel_id: 'C1', user_id: 'U1' } }, 1000)
    expect(nextTypingExpiry(t, 'C1', 2000)).toBe(9000)
    t = applyEvent(applyReady(emptyState(), r), { t: 'Typing', d: { channel_id: 'C1', user_id: 'U1' } }, 1000)
    expect(nextTypingExpiry(t, 'C1', 2000)).toBeNull()
  })
})

describe('stream selectors', () => {
  const stream: StreamInfo = { channel_id: 'V1', user_id: 'U2', settings: { height: 720, fps: 60, audio: true, allow_ghosts: true }, status: 'live', viewers: [], ghosts: 0, started_at: '2026-10-23T10:00:00.000Z' }
  it('liveIn lists a channel streams and isLive checks one user', () => {
    const s = { ...emptyState(), streams: { V1: { U2: stream } } }
    expect(liveIn(s, 'V1')).toEqual({ U2: stream })
    expect(liveIn(s, 'V9')).toEqual({})
    expect(isLive(s, 'V1', 'U2')).toBe(true)
    expect(isLive(s, 'V1', 'U3')).toBe(false)
    expect(isLive(s, 'V9', 'U2')).toBe(false)
  })

  describe('callRoster', () => {
    const none = { channelId: null, joining: false, leaving: null }
    it('roster_adds_you_greyed_while_joining', () => {
      const r = callRoster(s, 'S1', 'V1', { ...none, channelId: 'V1', joining: true })
      expect(r.map((x) => [x.userId, x.pending])).toEqual([['U2', false], ['U1', true]])
      expect(r[1].name).toBe('alex')
      expect(callRoster(s, 'S1', 'V1', none).map((x) => x.userId)).toEqual(['U2'])
    })
    it('roster_marks_you_pending_when_the_server_already_lists_you', () => {
      const listed = applyEvent(s, { t: 'VoiceJoined', d: { channel_id: 'V1', user_id: 'U1', flags: { muted: false, deafened: false } } }, 0)
      const r = callRoster(listed, 'S1', 'V1', { ...none, channelId: 'V1', joining: true })
      expect(r.filter((x) => x.userId === 'U1')).toHaveLength(1)
      expect(r.find((x) => x.userId === 'U1')!.pending).toBe(true)
      expect(callRoster(listed, 'S1', 'V1', { ...none, channelId: 'V1' }).find((x) => x.userId === 'U1')!.pending).toBe(false)
    })
    it('roster_drops_you_from_the_channel_you_are_leaving', () => {
      const listed = applyEvent(s, { t: 'VoiceJoined', d: { channel_id: 'V1', user_id: 'U1', flags: { muted: false, deafened: false } } }, 0)
      const r = callRoster(listed, 'S1', 'V1', { ...none, leaving: 'V1' })
      expect(r.map((x) => x.userId)).toEqual(['U2'])
    })
    it('roster_never_marks_others_pending', () => {
      const r = callRoster(s, 'S1', 'V1', { ...none, channelId: 'V1', joining: true })
      expect(r.filter((x) => x.userId !== 'U1').every((x) => !x.pending)).toBe(true)
    })
  })
})

describe('groupedWith', () => {
  const msg = (id: string, author: string, at: string, extra: Partial<Message> = {}) =>
    ({ id, author_id: author, kind: 'normal', reply_to_id: null, created_at: at, ...extra }) as Message
  const list = [
    msg('1', 'U1', '2026-10-01T10:00:00Z'),
    msg('2', 'U1', '2026-10-01T10:02:00Z'),
    msg('3', 'U1', '2026-10-01T10:09:00Z'),
    msg('4', 'U2', '2026-10-01T10:09:30Z'),
    msg('5', 'U2', '2026-10-01T10:10:00Z', { reply_to_id: '1' }),
    msg('6', 'U2', '2026-10-01T10:10:30Z'),
  ]
  it('groups the same author within five minutes', () => {
    expect(groupedWith(list, 0)).toBe(false)
    expect(groupedWith(list, 1)).toBe(true)
  })
  it('starts a group after five minutes, another author, or an inline reply', () => {
    expect(groupedWith(list, 2)).toBe(false)
    expect(groupedWith(list, 3)).toBe(false)
    expect(groupedWith(list, 4)).toBe(false)
    expect(groupedWith(list, 5)).toBe(true)
  })
  it('does not group after a system message', () => {
    const l = [msg('1', 'U1', '2026-10-01T10:00:00Z', { kind: 'system' }), msg('2', 'U1', '2026-10-01T10:01:00Z')]
    expect(groupedWith(l, 1)).toBe(false)
  })
})

describe('serverPeople', () => {
  it("lists a server's members by the name shown there, findable by every name", () => {
    const st = { ...s, privateNicks: { U2: 'Sammy' } }
    expect(serverPeople(st, 'S1')).toEqual([
      { id: 'U1', name: 'alex', username: 'alex', alts: ['alex'] },
      { id: 'U2', name: 'Big Dog', username: 'sam', alts: ['sam', 'Big Dog', 'Sammy'] },
    ])
    expect(serverPeople(st, 'S2')).toEqual([{ id: 'U2', name: 'Sammy', username: 'sam', alts: ['sam', 'Sammy'] }])
  })
})

describe('pillNamesFor', () => {
  it('is the mentionable usernames plus mine: a mention of you shows your name too (#47)', () => {
    const st = { ...s, people: { U1: { user: me, online: true }, U2: { user: sam, online: false } }, dmMembers: { D1: ['U1', 'U2'] } } as unknown as typeof s
    expect(pillNamesFor(st, 'C1')).toEqual(['sam', 'alex'])
    expect(pillNamesFor(st, 'D1')).toEqual(['sam', 'alex'])
  })
})

describe('mentionablesFor', () => {
  it('is everyone but me for a server channel, and the members for a DM', () => {
    const st = { ...s, people: { U1: { user: me, online: true }, U2: { user: sam, online: false } }, dmMembers: { D1: ['U1', 'U2'] } } as unknown as typeof s
    expect(mentionablesFor(st, 'C1')).toEqual([{ id: 'U2', name: 'Big Dog', username: 'sam', alts: ['sam', 'Big Dog'] }])
    expect(mentionablesFor(st, 'D1')).toEqual([{ id: 'U2', name: 'sam', username: 'sam', alts: ['sam'] }])
  })
})

describe('nameOfIn', () => {
  it('names a user by the server, and no author as Pulse', () => {
    const nameOf = nameOfIn(() => s, 'S1')
    expect(nameOf('U2')).toBe('Big Dog')
    expect(nameOf(null)).toBe('Pulse')
  })
})

describe('channelsFor clips', () => {
  it('clips channels are their own group', () => {
    const st = applyReady(emptyState(), { ...r, channels: [...r.channels, { id: 'K1', server_id: 'S1', kind: 'clips', name: 'clips', position: 0 }] })
    const g = channelsFor(st, 'S1')
    expect(g.clips.map((c) => c.id)).toEqual(['K1'])
    expect(g.text.map((c) => c.id)).not.toContain('K1')
  })
})

describe('userOf and roleColorFor', () => {
  it('userOf prefers people', () => {
    const fresher = { ...sam, avatar_hash: 'new' }
    const s = applyReady(emptyState(), { ...r, people: [{ user: fresher, online: false, last_seen_at: null, status: 'offline' as const }] })
    expect(userOf(s, 'U2')).toEqual(fresher)
    expect(userOf(applyReady(emptyState(), r), 'U2')).toEqual(sam) // members
    expect(userOf(applyReady(emptyState(), { ...r, members: [] }), 'U1')).toEqual(me) // me
    expect(userOf(s, 'nobody')).toBeNull()
  })

  it('userOf order: people beat members, members beat me', () => {
    const tag = (u: typeof me, h: string) => ({ ...u, avatar_hash: h })
    const base = { ...r, me: tag(me, 'me'), members: [{ server_id: 'S1', members: [{ user: tag(me, 'mem'), nickname: null }] }], people: [{ user: tag(me, 'ppl'), online: false, last_seen_at: null, status: 'offline' as const }] }
    expect(userOf(applyReady(emptyState(), base), 'U1')?.avatar_hash).toBe('ppl')
    expect(userOf(applyReady(emptyState(), { ...base, people: [] }), 'U1')?.avatar_hash).toBe('mem')
    expect(userOf(applyReady(emptyState(), { ...base, people: [], members: [] }), 'U1')?.avatar_hash).toBe('me')
  })

  it('roleColorFor reads the fixture map', () => {
    const s = { ...applyReady(emptyState(), r), roleColors: { S1: { U2: '#ff0000' } } }
    expect(roleColorFor(s, 'S1', 'U2')).toBe('#ff0000')
    expect(roleColorFor(s, 'S1', 'U1')).toBeNull()
    expect(roleColorFor(s, 'S2', 'U2')).toBeNull()
    expect(roleColorFor(s, null, 'U2')).toBeNull()
  })
})
