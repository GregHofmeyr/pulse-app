import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { Message } from './protocol/Message'
import { applyEvent, emptyState, type AppState } from './state'
import { noteReactionEvent, react } from './reactions'
import { api } from './tauri'

vi.mock('./tauri', () => ({
  api: { addReaction: vi.fn(), removeReaction: vi.fn(), listReactors: vi.fn() },
  errorText: (e: unknown) => String(e),
}))

const quote: Message = {
  id: 'M1', channel_id: 'Q1', author_id: 'U2', kind: 'normal', content: '', reply_to_id: null, created_at: '2026-10-01T00:00:00.000Z',
  edited_at: null, deleted: false, mentions: [], thread_root_id: null, qoty_card: null, clip: null,
  quote: { lines: [], year: 2026, said_on: null, reply_count: 0, last_reply_id: null, replies_seen: null, wins: [] },
  reactions: [{ emoji: 'a', count: 2, me: true }, { emoji: 'c', count: 1, me: false }],
}
const store = (start: AppState) => {
  let s = start
  return { get: () => s, set: (n: AppState) => (s = n) }
}
const start = (): AppState => ({ ...emptyState(), me: { id: 'U1', username: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }, messages: { Q1: [quote] } })
const reactions = (s: AppState) => s.messages['Q1'][0].reactions

beforeEach(() => {
  vi.resetAllMocks()
})

describe('react on a quote (one per person)', () => {
  it('swaps my reaction locally and keeps it when the server agrees', async () => {
    vi.mocked(api.addReaction).mockResolvedValue(undefined)
    const st = store(start())
    await react(st.get, st.set, 'Q1', 'M1', 'b', true, 'U1', () => {}, true)
    expect(reactions(st.get())).toEqual([{ emoji: 'a', count: 1, me: false }, { emoji: 'c', count: 1, me: false }, { emoji: 'b', count: 1, me: true }])
  })

  it('a failed add gives the displaced reaction back', async () => {
    vi.mocked(api.addReaction).mockRejectedValue(new Error('no'))
    vi.mocked(api.listReactors).mockRejectedValue(new Error('no')) // the fallback cannot settle it either
    const errors: string[] = []
    const st = store(start())
    await react(st.get, st.set, 'Q1', 'M1', 'b', true, 'U1', (m) => errors.push(m), true)
    expect(reactions(st.get()).find((r) => r.emoji === 'a')).toEqual({ emoji: 'a', count: 2, me: true })
    expect(reactions(st.get()).find((r) => r.emoji === 'b')).toBeUndefined()
    expect(errors).toHaveLength(1)
  })

  it('a failed add that the reactor list shows did not happen also restores it', async () => {
    vi.mocked(api.addReaction).mockRejectedValue(new Error('no'))
    vi.mocked(api.listReactors).mockImplementation(async (_id, key) => (key === 'b' ? [] : ['U1', 'U3']))
    const st = store(start())
    await react(st.get, st.set, 'Q1', 'M1', 'b', true, 'U1', () => {}, true)
    expect(reactions(st.get())).toEqual(expect.arrayContaining([{ emoji: 'a', count: 2, me: true }]))
    expect(reactions(st.get()).some((r) => r.emoji === 'b')).toBe(false)
  })

  it('an add that landed despite the error keeps the displacement', async () => {
    const st = store(start())
    vi.mocked(api.addReaction).mockImplementation(async () => {
      // the server applied it: it took back "a" and added "b" before the response was lost
      for (const e of [
        { t: 'ReactionRemoved', d: { channel_id: 'Q1', message_id: 'M1', emoji: 'a', user_id: 'U1', count: 1 } },
        { t: 'ReactionAdded', d: { channel_id: 'Q1', message_id: 'M1', emoji: 'b', user_id: 'U1', count: 1 } },
      ] as const) {
        noteReactionEvent(e, 'U1')
        st.set(applyEvent(st.get(), e, 0))
      }
      throw new Error('lost')
    })
    vi.mocked(api.listReactors).mockRejectedValue(new Error('no'))
    await react(st.get, st.set, 'Q1', 'M1', 'b', true, 'U1', () => {}, true)
    expect(reactions(st.get()).find((r) => r.emoji === 'a')).toEqual({ emoji: 'a', count: 1, me: false })
    expect(reactions(st.get()).find((r) => r.emoji === 'b')).toEqual({ emoji: 'b', count: 1, me: true })
  })

  it('another user removing the displaced emoji meanwhile: restored with the event\'s count, not +1', async () => {
    const st = store(start())
    vi.mocked(api.addReaction).mockImplementation(async () => {
      // someone else took their "a" back: the server's count (still including me) is 1
      const e = { t: 'ReactionRemoved', d: { channel_id: 'Q1', message_id: 'M1', emoji: 'a', user_id: 'U3', count: 1 } } as const
      noteReactionEvent(e, 'U1')
      st.set(applyEvent(st.get(), e, 0))
      throw new Error('no')
    })
    vi.mocked(api.listReactors).mockRejectedValue(new Error('no'))
    await react(st.get, st.set, 'Q1', 'M1', 'b', true, 'U1', () => {}, true)
    expect(reactions(st.get()).find((r) => r.emoji === 'a')).toEqual({ emoji: 'a', count: 1, me: true })
  })

  it('toggles of one quote share a queue, so a swap and its follow-up reach the server in order', async () => {
    const calls: string[] = []
    let release!: () => void
    vi.mocked(api.addReaction).mockImplementation(async (_id, key) => {
      calls.push(`add ${key}`)
      if (key === 'b') await new Promise<void>((r) => (release = r))
    })
    vi.mocked(api.removeReaction).mockImplementation(async (_id, key) => void calls.push(`remove ${key}`))
    const st = store(start())
    const first = react(st.get, st.set, 'Q1', 'M1', 'b', true, 'U1', () => {}, true)
    await vi.waitFor(() => expect(calls).toEqual(['add b']))
    // "a" was displaced locally; clicking it again must wait for the first add to finish.
    const second = react(st.get, st.set, 'Q1', 'M1', 'a', true, 'U1', () => {}, true)
    await new Promise((r) => setTimeout(r, 20))
    expect(calls).toEqual(['add b'])
    release()
    await Promise.all([first, second])
    expect(calls).toEqual(['add b', 'add a'])
  })

  it('reacts to a reply in its thread', async () => {
    vi.mocked(api.addReaction).mockResolvedValue(undefined)
    const r: Message = { ...quote, id: 'R1', quote: null, thread_root_id: 'M1', reactions: [] }
    const st = store({ ...start(), threads: { M1: [r] } })
    await react(st.get, st.set, 'Q1', 'R1', 'x', true, 'U1', () => {})
    expect(st.get().threads['M1'][0].reactions).toEqual([{ emoji: 'x', count: 1, me: true }])
  })
})
