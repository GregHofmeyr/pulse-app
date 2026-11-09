// @vitest-environment jsdom
// Polls (#157): our own view of a poll is fetched on PollChanged (for polls we show), events never undo it, and a
// slow answer never puts back what a later one changed.
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { Message } from './protocol/Message'
import type { Poll } from './protocol/Poll'

const view = (over: Partial<Poll> = {}): Poll => ({
  question: 'Q', options: ['a', 'b'], multi: false, anonymous: false, ends_at: null, closed_at: null, closed: false,
  voters: 0, mine: [], results: null, ...over,
})
let pending: ((v: unknown) => void)[] = []
const invoke = vi.fn(async (cmd: string, _args?: unknown): Promise<unknown> => {
  if (cmd === 'get_poll' || cmd === 'vote_poll') return new Promise((ok) => pending.push(ok))
  return undefined
})
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import { applyEvent, emptyState } from './state'
import { app } from './store.svelte'
import { handlePollEvent, resetPolls, votePoll } from './polls.svelte'

const msg = (over: Partial<Message> = {}): Message => ({
  id: 'M1', channel_id: 'C1', author_id: 'U2', kind: 'normal', content: '📊 Poll: Q', reply_to_id: null,
  created_at: '2026-11-09T12:00:00Z', edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null,
  quote: null, qoty_card: null, clip: null, embeds: [], poll: view(), ...over,
})
const flush = () => new Promise((r) => setTimeout(r, 0))
const held = () => app.state.messages.C1?.find((m) => m.id === 'M1')

beforeEach(() => {
  resetPolls()
  invoke.mockClear()
  pending = []
  app.state = { ...emptyState(), me: { id: 'U1' } as never, messages: { C1: [msg()] } }
})

describe('live poll views', () => {
  it('fetches our view when a poll we show changes, and only then', async () => {
    handlePollEvent({ t: 'PollChanged', d: { channel_id: 'C1', message_id: 'M1' } })
    handlePollEvent({ t: 'PollChanged', d: { channel_id: 'C1', message_id: 'NOPE' } })
    await flush()
    expect(invoke.mock.calls.map(([c, a]) => [c, a])).toEqual([['get_poll', { messageId: 'M1' }]])
    pending[0](view({ voters: 1, results: [{ votes: 1, voters: [] }, { votes: 0, voters: [] }], mine: [0] }))
    await flush()
    expect(held()?.poll?.mine).toEqual([0])
  })

  it('folds changes during a fetch into one more fetch', async () => {
    for (let i = 0; i < 3; i++) handlePollEvent({ t: 'PollChanged', d: { channel_id: 'C1', message_id: 'M1' } })
    await flush()
    expect(invoke).toHaveBeenCalledTimes(1)
    pending[0](view())
    await flush()
    await flush()
    expect(invoke).toHaveBeenCalledTimes(2)
  })

  it('a slow fetch does not undo a later vote', async () => {
    handlePollEvent({ t: 'PollChanged', d: { channel_id: 'C1', message_id: 'M1' } })
    await flush()
    const voted = votePoll(msg(), [1])
    await flush()
    pending[1](view({ mine: [1], voters: 1 }))
    await voted
    pending[0](view({ mine: [], voters: 0 }))
    await flush()
    expect(held()?.poll?.mine).toEqual([1])
  })

  it('an event keeps the view we fetched', () => {
    app.state = applyEvent(app.state, { t: 'PollChanged', d: { channel_id: 'C1', message_id: 'M1' } }, 0)
    const mine = view({ mine: [1], voters: 1, results: [{ votes: 0, voters: [] }, { votes: 1, voters: [] }] })
    app.state = { ...app.state, messages: { C1: [msg({ poll: mine })] } }
    app.state = applyEvent(app.state, { t: 'MessageCreated', d: { message: msg(), nonce: null } }, 0)
    expect(held()?.poll).toEqual(mine)
    app.state = applyEvent(app.state, { t: 'MessageUpdated', d: { message: msg({ content: 'x' }) } }, 0)
    expect(held()?.poll).toEqual(mine)
    expect(held()?.content).toBe('x')
  })

  it('our own new poll from another session fetches the creator view', async () => {
    handlePollEvent({ t: 'MessageCreated', d: { message: msg({ id: 'M2', author_id: 'U1' }), nonce: null } })
    handlePollEvent({ t: 'MessageCreated', d: { message: msg({ id: 'M3', author_id: 'U2' }), nonce: null } })
    await flush()
    expect(invoke.mock.calls.map(([, a]) => a)).toEqual([{ messageId: 'M2' }])
  })
})
