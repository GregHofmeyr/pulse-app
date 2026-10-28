import { describe, expect, it } from 'vitest'
import { canReopen, defaultQotyYear, hasNewReplies, leaders, leftLabel, plural, liveQoty, myVote, nameKey, quotePreview, score, speakersOf, speakerOf, stripMarks, timeLeft, voteAction } from './quotes'
import { applyEvent, applyReady, emptyState } from './state'
import type { Message } from './protocol/Message'
import type { QotyEvent } from './protocol/QotyEvent'
import type { QuoteLine } from './protocol/QuoteLine'
import type { Ready } from './protocol/Ready'

const me = { id: 'U1', username: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const sam = { id: 'U2', username: 'sam', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const line = (over: Partial<QuoteLine> = {}): QuoteLine => ({
  id: 'L1', speaker_id: null, speaker_name: 'Sam B', text: 'hi', media: null, caption: null, awards: 0, awarders: [], awarded: false, ...over,
})
const msg = (over: Partial<Message> = {}): Message => ({
  id: 'M1', channel_id: 'Q1', author_id: 'U1', kind: 'normal', content: '', reply_to_id: null, created_at: '2026-10-01T00:00:00.000Z',
  edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null, quote: null, qoty_card: null, clip: null, ...over,
})
const quote = (over: Partial<NonNullable<Message['quote']>> = {}): Message['quote'] => ({
  lines: [line()], year: 2026, said_on: null, reply_count: 0, last_reply_id: null, replies_seen: null, wins: [], ...over,
})
const opt = (id: string, voters: string[]) => ({ message: msg({ id }), added_by: 'U1', votes: voters.length, voters })
const qoty = (over: Partial<QotyEvent> = {}): QotyEvent => ({
  id: 'E1', server_id: 'S1', year: 2026, started_by: 'U1', started_at: '2026-10-01T00:00:00Z', ends_at: null, ended_at: null,
  ended_by: null, end_reason: null, reopen_until: null, options: [], voters: 0, members: 2, ...over,
})
const ready = (over: Partial<Ready> = {}): Ready => ({
  me, servers: [{ id: 'S1', name: 'Main', icon_hash: null }], channels: [{ id: 'Q1', server_id: 'S1', kind: 'quotes', name: 'quotes', position: 0 }],
  members: [{ server_id: 'S1', members: [{ user: me, nickname: null }, { user: sam, nickname: 'Big Dog' }] }],
  dm_members: [], voice: [], people: [], read_states: [], mutes: [], hidden: [], latest: [], streams: [], custom_emoji: [], frequent_emoji: [],
  qoty: [], speaker_links: [], thread_seen: [], games: [], clip_views: [], ...over,
})

describe('text helpers', () => {
  it('nameKey folds case and whitespace like the server', () => {
    expect(nameKey('  Sam   B ')).toBe('sam b')
    expect(nameKey('ÉMILE\tX')).toBe('émile x')
    // The server splits on Unicode White_Space: NEL and no-break space count, a BOM does not.
    expect(nameKey('a\u0085b\u00a0\u2003c')).toBe('a b c')
    expect(nameKey('a\ufeffb')).toBe('a\ufeffb')
  })
  it('stripMarks removes surrounding quote marks only', () => {
    expect(stripMarks('“hi”')).toBe('hi')
    expect(stripMarks('"hi"')).toBe('hi')
    expect(stripMarks("'tis great")).toBe("'tis great")
    expect(stripMarks('the dogs\'')).toBe('the dogs\'')
    expect(stripMarks('  “ hi ”  ')).toBe('hi')
  })
  it('stripMarks takes one matching pair that wraps the whole text, as the server does', () => {
    const cases: [string, string][] = [
      ['“x"', '“x"'],
      ["'x\"", "'x\""],
      ['"Yes" and "no"', '"Yes" and "no"'],
      ['“Yes” and “no”', '“Yes” and “no”'],
      ['""x""', '""x""'],
      ['“He said ‘hi’”', 'He said ‘hi’'],
      ['“‘x’”', '‘x’'],
      ["'don't do that'", "don't do that"],
      ['‘don’t’', 'don’t'],
      // Ruling (fix wave U1): a leading elision can't be told from an opening quote, so a line that
      // both opens and closes with a single quote is a wrapper and loses both.
      ["'tis the dogs'", 'tis the dogs'],
      ["'tis great", "'tis great"],
      ['«x»', 'x'],
      ['“”', ''],
      ['"', '"'],
    ]
    for (const [text, want] of cases) expect(stripMarks(text), text).toBe(want)
  })
})

describe('speakerOf', () => {
  const s = applyReady(emptyState(), ready({ speaker_links: [{ server_id: 'S1', name_key: 'sam b', name: 'Sam B', user_id: 'U2', linked_by: 'U1', linked_at: '2026-10-01T00:00:00Z' }] }))
  it('follows a link for a plain name, whatever its case', () => {
    expect(speakerOf(s, 'S1', line({ speaker_name: ' sam   B' }))).toEqual({ userId: 'U2', name: 'Big Dog' })
  })
  it('falls back to the plain name', () => {
    expect(speakerOf(s, 'S1', line({ speaker_name: 'Kim' }))).toEqual({ userId: null, name: 'Kim' })
  })
  it('a member speaker is shown by display name', () => {
    expect(speakerOf(s, 'S1', line({ speaker_id: 'U1', speaker_name: null }))).toEqual({ userId: 'U1', name: 'alex' })
  })
  it('links of another server do not apply', () => {
    expect(speakerOf(s, 'S2', line()).userId).toBeNull()
  })
})

describe('score and replies', () => {
  it('score sums reaction counts', () => {
    expect(score(msg({ reactions: [{ emoji: 'a', count: 2, me: false }, { emoji: 'b', count: 3, me: true }] }))).toBe(5)
    expect(score(msg())).toBe(0)
  })
  it('hasNewReplies compares the newest reply with the seen point', () => {
    expect(hasNewReplies(msg({ quote: quote() }))).toBe(false)
    expect(hasNewReplies(msg({ quote: quote({ last_reply_id: 'R2' }) }))).toBe(true)
    expect(hasNewReplies(msg({ quote: quote({ last_reply_id: 'R2', replies_seen: 'R1' }) }))).toBe(true)
    expect(hasNewReplies(msg({ quote: quote({ last_reply_id: 'R2', replies_seen: 'R2' }) }))).toBe(false)
    expect(hasNewReplies(msg())).toBe(false)
  })
})

describe('Quote of the Year helpers', () => {
  it('defaultQotyYear is last year in January', () => {
    expect(defaultQotyYear(new Date('2027-01-10T12:00:00'))).toBe(2026)
    expect(defaultQotyYear(new Date('2026-06-01T12:00:00'))).toBe(2026)
  })
  it('timeLeft', () => {
    const now = Date.parse('2026-10-01T00:00:00Z')
    const at = (ms: number) => new Date(now + ms).toISOString()
    expect(timeLeft(at(2 * 3600_000 + 5 * 60_000), now)).toBe('2h 5m')
    expect(timeLeft(at(4 * 60_000), now)).toBe('4m')
    expect(timeLeft(at(10_000), now)).toBe('<1m')
    expect(timeLeft(at(-5), now)).toBe('<1m')
    expect(timeLeft(null, now)).toBe('until ended')
  })
  it('myVote finds me among the voters', () => {
    const e = qoty({ options: [opt('A', ['U2']), opt('B', ['U1'])] })
    expect(myVote(e, 'U1')).toBe('B')
    expect(myVote(e, 'U9')).toBeNull()
  })
  it('voteAction', () => {
    expect(voteAction(null, 'A')).toBe('vote')
    expect(voteAction('A', 'B')).toBe('move')
    expect(voteAction('A', 'A')).toBe('unvote')
  })
  it('liveQoty is the open vote that has not run out', () => {
    const now = Date.parse('2026-10-01T00:00:00Z')
    let s = applyReady(emptyState(), ready({ qoty: [qoty()] }))
    expect(liveQoty(s, 'S1', now)?.id).toBe('E1')
    expect(liveQoty(s, 'S2', now)).toBeNull()
    s = applyEvent(s, { t: 'QotyUpdated', d: { event: qoty({ ends_at: new Date(now - 1).toISOString() }) } }, 0)
    expect(liveQoty(s, 'S1', now)).toBeNull()
    s = applyEvent(s, { t: 'QotyEnded', d: { event: qoty({ ended_at: '2026-10-01T00:00:00Z', end_reason: 'early' }) } }, 0)
    expect(liveQoty(s, 'S1', now)).toBeNull()
  })
})

describe('Quote of the Year labels', () => {
  const s = applyReady(emptyState(), ready())
  it('speakersOf names each speaker once, in line order', () => {
    const m = msg({ quote: quote({ lines: [line({ speaker_id: 'U2', speaker_name: null }), line({ id: 'L2' }), line({ id: 'L3', speaker_id: 'U2', speaker_name: null })] }) })
    expect(speakersOf(s, 'S1', m)).toBe('Big Dog & Sam B')
    expect(speakersOf(s, 'S1', msg())).toBe('')
    // several quotes: each person once across all of them
    const other = msg({ id: 'M2', quote: quote({ lines: [line({ speaker_id: 'U2', speaker_name: null })] }) })
    expect(speakersOf(s, 'S1', [other, m])).toBe('Big Dog & Sam B')
  })
  it('quotePreview is the first line and its speaker, with an ellipsis for more', () => {
    expect(quotePreview(s, 'S1', msg({ quote: quote() }))).toEqual({ said: '“hi”', by: 'Sam B' })
    expect(quotePreview(s, 'S1', msg({ quote: quote({ lines: [line(), line({ id: 'L2' })] }) }))).toEqual({ said: '“hi” …', by: 'Sam B' })
    expect(quotePreview(s, 'S1', msg({ quote: quote({ lines: [line({ text: null, caption: 'the chat' })] }) }))).toEqual({ said: 'the chat', by: 'Sam B' })
    expect(quotePreview(s, 'S1', msg({ content: 'plain' }))).toEqual({ said: 'plain', by: '' })
  })
  it('leftLabel says how long is left, or until ended', () => {
    const now = Date.parse('2026-10-01T00:00:00Z')
    expect(leftLabel('2026-10-01T03:00:00Z', now)).toBe('3h left')
    expect(leftLabel(null, now)).toBe('until ended')
  })
  it('canReopen: the same event, ended, within its window', () => {
    const now = Date.parse('2026-10-01T00:00:00Z')
    const ended = qoty({ ended_at: '2026-09-30T23:00:00Z', end_reason: 'early', reopen_until: '2026-10-01T23:00:00Z' })
    expect(canReopen(ended, 'E1', now)).toBe(true)
    expect(canReopen(ended, 'E2', now)).toBe(false)
    expect(canReopen(undefined, 'E1', now)).toBe(false)
    expect(canReopen({ ...ended, reopen_until: '2026-09-30T23:59:00Z' }, 'E1', now)).toBe(false)
    expect(canReopen({ ...ended, reopen_until: null }, 'E1', now)).toBe(false)
    expect(canReopen({ ...ended, ended_at: null }, 'E1', now)).toBe(false)
  })
})

describe('wording', () => {
  it('plural picks the singular for exactly one', () => {
    expect(plural(1, 'vote', 'votes')).toBe('1 vote')
    expect(plural(0, 'vote', 'votes')).toBe('0 votes')
    expect(plural(2, 'reply', 'replies')).toBe('2 replies')
  })
  it('leaders are every option tied for the most votes, none without votes', () => {
    expect(leaders(qoty({ options: [opt('A', []), opt('B', [])] }))).toEqual([])
    const e = qoty({ options: [opt('A', ['U1']), opt('B', ['U2', 'U3']), opt('C', ['U4', 'U5'])] })
    expect(leaders(e)).toEqual([e.options[1], e.options[2]])
  })
})
