import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { applyReady, emptyState } from './state'
import { NOTIFY_WINDOW_MS, NotifyThrottle, notificationBody, notificationTitle, notifyMessage, type Notice } from './notify'
import type { Ready } from './protocol/Ready'
import type { Message } from './protocol/Message'

const user = (id: string, username: string) => ({ id, username, avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } })
const msg = (channel: string, author: string | null, extra: Partial<Message> = {}): Message => ({
  id: 'M1', channel_id: channel, author_id: author, kind: 'normal', content: 'hello', reply_to_id: null,
  created_at: '2026-10-30T10:00:00Z', edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null, quote: null, qoty_card: null, clip: null, ...extra,
})
function ready(): Ready {
  return {
    me: user('ME', 'alex'),
    servers: [{ id: 'S1', name: 'Main', icon_hash: null }],
    channels: [
      { id: 'DM1', server_id: null, kind: 'dm', name: null, position: 0 },
      { id: 'G1', server_id: null, kind: 'group', name: 'Friends', position: 0 },
      { id: 'G2', server_id: null, kind: 'group', name: null, position: 0 },
      { id: 'T1', server_id: 'S1', kind: 'text', name: 'general', position: 0 },
      { id: 'Q1', server_id: 'S1', kind: 'quotes', name: 'quotes', position: 1 },
    ],
    members: [{ server_id: 'S1', members: [{ user: user('ME', 'alex'), nickname: null }, { user: user('U1', 'sam'), nickname: null }] }],
    dm_members: [{ channel_id: 'DM1', user_ids: ['ME', 'U1'] }, { channel_id: 'G1', user_ids: ['ME', 'U1', 'U2'] }, { channel_id: 'G2', user_ids: ['ME', 'U1', 'U2'] }],
    voice: [],
    people: [
      { user: user('ME', 'alex'), online: true, last_seen_at: null },
      { user: user('U1', 'sam'), online: true, last_seen_at: null },
      { user: user('U2', 'jo'), online: true, last_seen_at: null },
    ],
    read_states: [], mutes: [], hidden: [], latest: [], streams: [], custom_emoji: [],
    frequent_emoji: [], qoty: [], speaker_links: [], thread_seen: [], games: [], clip_views: [],
  }
}
const s = applyReady(emptyState(), ready())
const on = { enabled: true, showText: true }
const v = (m: Message, over: Partial<{ open: boolean; focused: boolean; muted: boolean; openThread: string | null }> = {}) =>
  ({ message: m, me: 'ME', channel: s.channels[m.channel_id], open: false, openThread: null, focused: false, muted: false, ...over })

describe('notifyMessage: the decision', () => {
  it('a DM from someone else while the window is unfocused notifies', () => {
    expect(notifyMessage(s, v(msg('DM1', 'U1')), on)).toEqual({ title: 'sam', body: 'hello', channelId: 'DM1' })
  })
  it('never while the window is focused, even for a conversation that is not open', () => {
    expect(notifyMessage(s, v(msg('DM1', 'U1'), { focused: true }), on)).toBeNull()
  })
  it('never when desktop notifications are off', () => {
    expect(notifyMessage(s, v(msg('DM1', 'U1')), { enabled: false, showText: true })).toBeNull()
  })
  it('server channels: only mentions', () => {
    expect(notifyMessage(s, v(msg('T1', 'U1')), on)).toBeNull()
    expect(notifyMessage(s, v(msg('T1', 'U1', { mentions: ['ME'] })), on)).not.toBeNull()
  })
  it('muted: only mentions', () => {
    expect(notifyMessage(s, v(msg('G1', 'U1'), { muted: true }), on)).toBeNull()
    expect(notifyMessage(s, v(msg('G1', 'U1', { mentions: ['ME'] }), { muted: true }), on)).not.toBeNull()
  })
  it('never your own, system or deleted messages', () => {
    expect(notifyMessage(s, v(msg('DM1', 'ME')), on)).toBeNull()
    expect(notifyMessage(s, v(msg('DM1', null, { kind: 'system' })), on)).toBeNull()
    expect(notifyMessage(s, v(msg('DM1', 'U1', { deleted: true })), on)).toBeNull()
  })
  it('a reply carries its quote as the thread; other messages carry none', () => {
    const reply = msg('Q1', 'U1', { mentions: ['ME'], thread_root_id: 'R1' })
    expect(notifyMessage(s, v(reply), on)).toMatchObject({ channelId: 'Q1', threadId: 'R1' })
    expect(notifyMessage(s, v(msg('DM1', 'U1')), on)).not.toHaveProperty('threadId')
  })
})

describe('notificationTitle', () => {
  it('a DM: just the name', () => expect(notificationTitle(s, msg('DM1', 'U1'), s.channels['DM1'], 0)).toBe('sam'))
  it('a named group: "in" its name', () => expect(notificationTitle(s, msg('G1', 'U1'), s.channels['G1'], 0)).toBe('sam in Friends'))
  it('an unnamed group: "in" its members', () => expect(notificationTitle(s, msg('G2', 'U1'), s.channels['G2'], 0)).toBe('sam in sam, jo'))
  it('a server channel: "in #channel (Server)"', () =>
    expect(notificationTitle(s, msg('T1', 'U1'), s.channels['T1'], 0)).toBe('sam in #general (Main)'))
  it("a server member's nickname", () => {
    const nick = { ...s, members: { S1: [{ user: user('U1', 'sam'), nickname: 'Sammy' }] } }
    expect(notificationTitle(nick, msg('T1', 'U1'), s.channels['T1'], 0)).toBe('Sammy in #general (Main)')
  })
  it('"and N more" after the name', () => {
    expect(notificationTitle(s, msg('T1', 'U1'), s.channels['T1'], 3)).toBe('sam and 3 more in #general (Main)')
    expect(notificationTitle(s, msg('DM1', 'U1'), s.channels['DM1'], 1)).toBe('sam and 1 more')
  })
})

describe('notificationBody', () => {
  it('the text on one line, no author prefix', () =>
    expect(notificationBody(msg('G1', 'U1', { content: '  hi\n\nthere\tyou ' }), 'ME', true)).toBe('hi there you'))
  it('custom emoji as :name:, mentions as @name', () =>
    expect(notificationBody(msg('DM1', 'U1', { content: 'gg <:party:E1> <a:dance:E2> @alex' }), 'ME', true)).toBe('gg :party: :dance: @alex'))
  it('cut at 140 characters', () => {
    const b = notificationBody(msg('DM1', 'U1', { content: 'a'.repeat(200) }), 'ME', true)
    expect(b).toHaveLength(140)
    expect(b.endsWith('…')).toBe(true)
  })
  it('never splits an emoji when cutting', () => {
    const b = notificationBody(msg('DM1', 'U1', { content: '😀'.repeat(200) }), 'ME', true)
    expect(Array.from(b)).toHaveLength(140)
    expect(Array.from(b).slice(0, 139).every((c) => c === '😀')).toBe(true)
  })
  it('never splits a combined emoji (a family, a flag, a skin tone) when cutting', () => {
    for (const e of ['👨‍👩‍👧', '🇿🇦', '👍🏽']) {
      const b = notificationBody(msg('DM1', 'U1', { content: e.repeat(200) }), 'ME', true)
      expect(b).toBe(e.repeat(139) + '…')
    }
  })
  it('140 combined emoji exactly stay whole', () =>
    expect(notificationBody(msg('DM1', 'U1', { content: '👨‍👩‍👧'.repeat(140) }), 'ME', true)).toBe('👨‍👩‍👧'.repeat(140)))
  it('140 characters exactly stay whole', () =>
    expect(notificationBody(msg('DM1', 'U1', { content: 'b'.repeat(140) }), 'ME', true)).toBe('b'.repeat(140)))
  it('a message with no text reads by what it carries', () => {
    const clip = { id: 'C1' } as never
    const quote = { lines: [] } as never
    expect(notificationBody(msg('T1', 'U1', { content: '', clip }), 'ME', true)).toBe('sent a clip')
    expect(notificationBody(msg('T1', 'U1', { content: 'my best one', clip }), 'ME', true)).toBe('my best one')
    expect(notificationBody(msg('Q1', 'U1', { content: '', quote }), 'ME', true)).toBe('shared a quote')
    expect(notificationBody(msg('Q1', 'U1', { content: 'sam: no way', quote }), 'ME', true)).toBe('sam: no way')
    expect(notificationBody(msg('Q1', null, { content: '', qoty_card: {} as never }), 'ME', true)).toBe('posted the Quote of the Year card')
  })
  it('message text off: "New message", or "Mentioned you" for a mention, whatever it carries', () => {
    expect(notificationBody(msg('DM1', 'U1', { content: 'secret' }), 'ME', false)).toBe('New message')
    expect(notificationBody(msg('T1', 'U1', { content: 'secret @alex', mentions: ['ME'] }), 'ME', false)).toBe('Mentioned you')
    expect(notificationBody(msg('T1', 'U1', { content: '', clip: { id: 'C1' } as never }), 'ME', false)).toBe('New message')
  })
  it('text off reaches the payload', () => {
    expect(notifyMessage(s, v(msg('DM1', 'U1', { content: 'secret' })), { enabled: true, showText: false })?.body).toBe('New message')
  })
})

describe('NotifyThrottle', () => {
  beforeEach(() => vi.useFakeTimers())
  afterEach(() => vi.useRealTimers())
  const notice = (key: string, text: string) => (more: number): Notice => ({ title: more ? `sam and ${more} more` : 'sam', body: text, channelId: key })
  function setup() {
    const sent: Notice[] = []
    return { sent, t: new NotifyThrottle((n) => sent.push(n)) }
  }

  it('the window is 4 s', () => expect(NOTIFY_WINDOW_MS).toBe(4000))

  it('a single message notifies at once and nothing follows', () => {
    const { sent, t } = setup()
    t.offer('DM1', 'one', notice('DM1', 'one'))
    expect(sent).toEqual([{ title: 'sam', body: 'one', channelId: 'DM1' }])
    vi.advanceTimersByTime(20_000)
    expect(sent).toHaveLength(1)
  })

  it('a burst of 10: the first at once, then one replacement with the newest and "and 9 more"', () => {
    const { sent, t } = setup()
    for (let i = 1; i <= 10; i++) {
      t.offer('DM1', `m${i}`, notice('DM1', `m${i}`))
      vi.advanceTimersByTime(100)
    }
    expect(sent).toHaveLength(1)
    vi.advanceTimersByTime(NOTIFY_WINDOW_MS)
    expect(sent).toHaveLength(2)
    expect(sent[1]).toEqual({ title: 'sam and 9 more', body: 'm10', channelId: 'DM1' })
    vi.advanceTimersByTime(NOTIFY_WINDOW_MS * 3)
    expect(sent).toHaveLength(2)
  })

  it('a replacement opens a new window; the count keeps growing while the chain goes on', () => {
    const { sent, t } = setup()
    t.offer('G1', 'a', notice('G1', 'a'))
    t.offer('G1', 'b', notice('G1', 'b'))
    t.offer('G1', 'c', notice('G1', 'c'))
    vi.advanceTimersByTime(NOTIFY_WINDOW_MS)
    expect(sent.at(-1)).toMatchObject({ title: 'sam and 2 more', body: 'c' })
    t.offer('G1', 'd', notice('G1', 'd'))
    t.offer('G1', 'e', notice('G1', 'e'))
    expect(sent).toHaveLength(2)
    vi.advanceTimersByTime(NOTIFY_WINDOW_MS)
    expect(sent.at(-1)).toMatchObject({ title: 'sam and 4 more', body: 'e' })
    vi.advanceTimersByTime(NOTIFY_WINDOW_MS)
    // the chain ended: the next one stands alone and goes at once
    t.offer('G1', 'f', notice('G1', 'f'))
    expect(sent.at(-1)).toMatchObject({ title: 'sam', body: 'f' })
    expect(sent).toHaveLength(4)
  })

  it('conversations are throttled separately', () => {
    const { sent, t } = setup()
    t.offer('DM1', 'x', notice('DM1', 'x'))
    t.offer('G1', 'y', notice('G1', 'y'))
    expect(sent.map((n) => n.channelId)).toEqual(['DM1', 'G1'])
  })

  it('the newest was deleted: the replacement shows the newest one left, and counts only those', () => {
    const { sent, t } = setup()
    for (const x of ['a', 'b', 'c']) t.offer('DM1', x, notice('DM1', x))
    t.forget('DM1', 'c')
    vi.advanceTimersByTime(NOTIFY_WINDOW_MS)
    expect(sent.at(-1)).toEqual({ title: 'sam and 1 more', body: 'b', channelId: 'DM1' })
  })

  it('everything after the first was deleted: no replacement, and the next message goes at once', () => {
    const { sent, t } = setup()
    for (const x of ['a', 'b', 'c']) t.offer('DM1', x, notice('DM1', x))
    t.forget('DM1', 'b')
    t.forget('DM1', 'c')
    vi.advanceTimersByTime(NOTIFY_WINDOW_MS)
    expect(sent).toHaveLength(1)
    t.offer('DM1', 'd', notice('DM1', 'd'))
    expect(sent.at(-1)).toMatchObject({ title: 'sam', body: 'd' })
  })

  it("a deletion in another conversation, or of a message it never had, changes nothing", () => {
    const { sent, t } = setup()
    for (const x of ['a', 'b']) t.offer('DM1', x, notice('DM1', x))
    t.forget('G1', 'b')
    t.forget('DM1', 'zzz')
    vi.advanceTimersByTime(NOTIFY_WINDOW_MS)
    expect(sent.at(-1)).toMatchObject({ title: 'sam and 1 more', body: 'b' })
  })

  it('clear() cancels a pending replacement; the next message goes at once', () => {
    const { sent, t } = setup()
    t.offer('DM1', 'a', notice('DM1', 'a'))
    t.offer('DM1', 'b', notice('DM1', 'b'))
    t.clear()
    vi.advanceTimersByTime(NOTIFY_WINDOW_MS * 2)
    expect(sent).toHaveLength(1)
    t.offer('DM1', 'c', notice('DM1', 'c'))
    expect(sent.at(-1)).toMatchObject({ title: 'sam', body: 'c' })
  })
})
