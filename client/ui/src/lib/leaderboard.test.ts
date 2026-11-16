import { describe, expect, it } from 'vitest'
import { boardLines, boardRefreshFor, ranks, shares, tabMove, widerPeriod } from './leaderboard'
import { emptyState, type AppState } from './state'
import type { Event } from './protocol/Event'
import type { Message } from './protocol/Message'

const state = (): AppState => ({
  ...emptyState(),
  channels: {
    C1: { id: 'C1', server_id: 'S1', kind: 'quotes', name: 'quotes', position: 0 },
    C2: { id: 'C2', server_id: 'S1', kind: 'text', name: 'general', position: 1 },
    C9: { id: 'C9', server_id: 'S2', kind: 'quotes', name: 'quotes', position: 0 },
  } as never,
})
const msg = (channel_id: string, quote: boolean) => ({ id: 'M', channel_id, quote: quote ? { lines: [] } : null }) as unknown as Message
const ev = (t: string, d: unknown) => ({ t, d }) as Event

describe('boardRefreshFor (#410)', () => {
  const s = state()
  it.each([
    ['a quote posted here', ev('MessageCreated', { message: msg('C1', true), nonce: null }), true],
    ['a quote edited here', ev('MessageUpdated', { message: msg('C1', true) }), true],
    ['a plain message', ev('MessageCreated', { message: msg('C2', false), nonce: null }), false],
    ['a quote in another server', ev('MessageCreated', { message: msg('C9', true), nonce: null }), false],
    ['a quote deleted', ev('MessageDeleted', { channel_id: 'C1', message_id: 'M', author_id: null, mentions: [], thread_root_id: null }), true],
    ['a reply deleted', ev('MessageDeleted', { channel_id: 'C1', message_id: 'M', author_id: null, mentions: [], thread_root_id: 'R' }), false],
    ['a message deleted elsewhere', ev('MessageDeleted', { channel_id: 'C2', message_id: 'M', author_id: null, mentions: [], thread_root_id: null }), false],
    ['a reaction in the quotes channel', ev('ReactionAdded', { channel_id: 'C1', message_id: 'M', emoji: 'x', user_id: 'U', count: 1 }), true],
    ['a reaction taken back', ev('ReactionRemoved', { channel_id: 'C1', message_id: 'M', emoji: 'x', user_id: 'U', count: 0 }), true],
    ['a reaction in a text channel', ev('ReactionAdded', { channel_id: 'C2', message_id: 'M', emoji: 'x', user_id: 'U', count: 1 }), false],
    ['a reaction in another server', ev('ReactionAdded', { channel_id: 'C9', message_id: 'M', emoji: 'x', user_id: 'U', count: 1 }), false],
    ['an award', ev('QuoteAwardChanged', { channel_id: 'C1' }), true],
    ['a speaker link', ev('SpeakerLinksChanged', { server_id: 'S1', links: [] }), true],
    ['another server\'s link', ev('SpeakerLinksChanged', { server_id: 'S2', links: [] }), false],
    ['a nickname', ev('MemberUpdated', { server_id: 'S1' }), true],
    ['a channel deleted', ev('ChannelDeleted', { server_id: 'S1', channel_id: 'C1' }), true],
    ['a channel restored', ev('ChannelRestored', { channel: { server_id: 'S1' } }), true],
    ['a vote ended', ev('QotyEnded', { event: { server_id: 'S1' } }), true],
    ['a vote reopened', ev('QotyUpdated', { event: { server_id: 'S1' } }), true],
    ['another server\'s vote', ev('QotyEnded', { event: { server_id: 'S2' } }), false],
    ['typing', ev('Typing', { channel_id: 'C1', user_ids: [] }), false],
  ])('%s → %s', (_, e, want) => {
    expect(boardRefreshFor(e, 'S1', s)).toBe(want)
  })
})

describe('tabs and counts', () => {
  it('arrows wrap, Home and End jump, other keys do nothing', () => {
    expect(tabMove('ArrowRight', 3, 4)).toBe(0)
    expect(tabMove('ArrowLeft', 0, 4)).toBe(3)
    expect(tabMove('ArrowDown', 1, 4)).toBe(2)
    expect(tabMove('ArrowUp', 1, 4)).toBe(0)
    expect(tabMove('Home', 2, 4)).toBe(0)
    expect(tabMove('End', 0, 4)).toBe(3)
    expect(tabMove('Enter', 0, 4)).toBeNull()
  })
  it('an empty period points at the next longer one', () => {
    expect(widerPeriod('week')).toBe('month')
    expect(widerPeriod('year')).toBe('all')
    expect(widerPeriod('all')).toBeNull()
  })
  it('ties share a rank; bars are a share of the leader', () => {
    expect(ranks([9, 5, 5, 2])).toEqual([1, 2, 2, 4])
    expect(shares([8, 4, 2])).toEqual([1, 0.5, 0.25])
    expect(shares([0, 0])).toEqual([0, 0])
    expect(shares([])).toEqual([])
  })
  it('a quote\'s lines: words, a caption, or Image, each with its speaker now', () => {
    const s = { ...state(), speakerLinks: { S1: [{ name_key: 'a friend', user_id: 'U2' }] } } as never as AppState
    const line = (o: object) => ({ id: 'L', speaker_id: null, speaker_name: null, text: null, media: null, caption: null, awards: 0, awarders: [], awarded: false, ...o })
    const m = { quote: { lines: [line({ speaker_name: 'A Friend', text: 'hi' }), line({ speaker_name: 'Someone', caption: 'look' }), line({ speaker_name: 'Someone', media: {} })] } } as unknown as Message
    const lines = boardLines(s, 'S1', m)
    expect(lines.map((l) => [l.said, l.userId])).toEqual([['hi', 'U2'], ['look', null], ['Image', null]])
    expect(boardLines(s, 'S1', { quote: null } as unknown as Message)).toEqual([])
  })
})
