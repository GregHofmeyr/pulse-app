import { describe, expect, it } from 'vitest'
import { emptyState, type AppState } from './state'
import type { Message } from './protocol/Message'
import { localDay, quotable, quoteRequest, quotesChannelsFor } from './quoteFromMessage'

const user = (id: string, name: string) => ({ id, username: name, display_name: name, avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } })
const msg = (over: Partial<Message> = {}): Message => ({
  id: 'M1', channel_id: 'T1', author_id: 'U2', kind: 'normal', content: '  the floor is lava  ', reply_to_id: null,
  created_at: '2026-11-09T09:00:00.000Z', edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null,
  quote: null, qoty_card: null, clip: null, embeds: [], ...over,
})
const state = (): AppState => ({
  ...emptyState(),
  channels: {
    T1: { id: 'T1', server_id: 'S1', kind: 'text', name: 'general', position: 0 },
    Q2: { id: 'Q2', server_id: 'S1', kind: 'quotes', name: 'old-quotes', position: 5 },
    Q1: { id: 'Q1', server_id: 'S1', kind: 'quotes', name: 'quotes', position: 2 },
    QX: { id: 'QX', server_id: 'S2', kind: 'quotes', name: 'elsewhere', position: 0 },
    D1: { id: 'D1', server_id: null, kind: 'dm', name: null, position: 0 },
  },
  members: { S1: [{ user: user('U1', 'alex'), nickname: null }, { user: user('U2', 'sam'), nickname: 'Sammy' }] },
  people: { U3: { user: user('U3', 'kim'), online: false, last_seen_at: null } },
} as never)

describe('Add to quotes (#316)', () => {
  it("offers the message's own server's quotes channels, in list order; none in a DM", () => {
    expect(quotesChannelsFor(state(), msg()).map((c) => c.id)).toEqual(['Q1', 'Q2'])
    expect(quotesChannelsFor(state(), msg({ channel_id: 'D1' }))).toEqual([])
  })

  it('only words someone typed are quotable', () => {
    expect(quotable(msg())).toBe(true)
    for (const m of [
      msg({ deleted: true }), msg({ kind: 'system' }), msg({ author_id: null }), msg({ content: '   ' }),
      msg({ caption_is_fallback: true }), msg({ gif: { id: 'g', title: 't', width: 1, height: 1 } as never }),
      msg({ poll: {} as never }),
    ]) expect(quotable(m)).toBe(false)
    expect(quotesChannelsFor(state(), msg({ content: '' }))).toEqual([])
  })

  it('a member author is the speaker; the text is the content', () => {
    expect(quoteRequest(state(), msg(), 'S1', 'n1', '2026-11-12')).toEqual({
      lines: [{ id: null, speaker_id: 'U2', speaker_name: null, text: 'the floor is lava', media_hash: null, caption: null }],
      today: '2026-11-12', nonce: 'n1',
    })
  })

  it("someone who left the server is the speaker by name", () => {
    const r = quoteRequest(state(), msg({ author_id: 'U3' }), 'S1', 'n1', '2026-11-12')
    expect(r.lines[0]).toMatchObject({ speaker_id: null, speaker_name: 'kim' })
  })

  it('the local date', () => {
    expect(localDay(new Date(2026, 0, 5, 23, 30))).toBe('2026-01-05')
  })
})
