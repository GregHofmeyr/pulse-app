import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { MESSAGE_DELAY_MS, MESSAGES_SHOWN, messageQuery, messageSearcher, type MessageResults } from './quickMessages'
import type { SearchHit } from './protocol/SearchHit'

const hit = (id: string) => ({ message: { id, channel_id: 'c1' }, text: id, highlights: [], root: null }) as unknown as SearchHit

describe('messageQuery', () => {
  it('searches two characters or more of plain text', () => {
    expect(messageQuery('pi')).toBe('pi')
    expect(messageQuery('  pizza night ')).toBe('pizza night')
  })
  it('not one character, not actions, not text with no words', () => {
    expect(messageQuery('')).toBeNull()
    expect(messageQuery('p')).toBeNull()
    expect(messageQuery(' p ')).toBeNull()
    expect(messageQuery('>mute')).toBeNull()
    expect(messageQuery('  > settings')).toBeNull()
    expect(messageQuery('?!')).toBeNull()
  })
})

describe('messageSearcher', () => {
  let states: MessageResults[]
  beforeEach(() => {
    vi.useFakeTimers()
    states = []
  })
  afterEach(() => vi.useRealTimers())
  const last = () => states[states.length - 1]

  it('waits for typing to pause, then asks once for the last query', async () => {
    const search = vi.fn(async (_q: string) => [hit('m1')])
    const s = messageSearcher({ search, onChange: (r) => states.push(r) })
    s.set('pi')
    vi.advanceTimersByTime(MESSAGE_DELAY_MS - 50)
    s.set('piz')
    vi.advanceTimersByTime(MESSAGE_DELAY_MS - 50)
    s.set('pizza')
    expect(search).not.toHaveBeenCalled()
    expect(last()).toEqual({ status: 'loading', query: 'pizza', hits: [] })
    await vi.advanceTimersByTimeAsync(MESSAGE_DELAY_MS)
    expect(search).toHaveBeenCalledTimes(1)
    expect(search).toHaveBeenCalledWith('pizza')
    expect(last()).toEqual({ status: 'done', query: 'pizza', hits: [hit('m1')] })
  })

  it("a newer query drops an older one's late answer", async () => {
    let answer: (h: SearchHit[]) => void = () => {}
    const search = vi.fn((q: string) => (q === 'old' ? new Promise<SearchHit[]>((r) => (answer = r)) : Promise.resolve([hit('new')])))
    const s = messageSearcher({ search, onChange: (r) => states.push(r) })
    s.set('old')
    await vi.advanceTimersByTimeAsync(MESSAGE_DELAY_MS)
    s.set('new')
    await vi.advanceTimersByTimeAsync(MESSAGE_DELAY_MS)
    answer([hit('stale')])
    await vi.runAllTimersAsync()
    expect(last()).toEqual({ status: 'done', query: 'new', hits: [hit('new')] })
    expect(states.some((x) => x.hits.some((h) => h.message.id === 'stale'))).toBe(false)
  })

  it('caps the hits', async () => {
    const many = Array.from({ length: 25 }, (_, i) => hit(`m${i}`))
    const s = messageSearcher({ search: async () => many, onChange: (r) => states.push(r) })
    s.set('pizza')
    await vi.advanceTimersByTimeAsync(MESSAGE_DELAY_MS)
    expect(last().hits).toHaveLength(MESSAGES_SHOWN)
    expect(MESSAGES_SHOWN).toBe(8)
  })

  it('the same query again changes nothing; none clears at once and cancels the wait', async () => {
    const search = vi.fn(async () => [hit('m1')])
    const s = messageSearcher({ search, onChange: (r) => states.push(r) })
    s.set('pizza')
    s.set('pizza')
    expect(states).toHaveLength(1)
    s.set(null)
    expect(last()).toEqual({ status: 'idle', query: '', hits: [] })
    await vi.runAllTimersAsync()
    expect(search).not.toHaveBeenCalled()
  })

  it('a failure says so quietly; dispose drops what is pending', async () => {
    const s = messageSearcher({ search: () => Promise.reject(new Error('offline')), onChange: (r) => states.push(r) })
    s.set('pizza')
    await vi.advanceTimersByTimeAsync(MESSAGE_DELAY_MS)
    expect(last()).toEqual({ status: 'error', query: 'pizza', hits: [] })
    const search = vi.fn(async () => [hit('m1')])
    const t = messageSearcher({ search, onChange: (r) => states.push(r) })
    t.set('pizza')
    t.dispose()
    await vi.runAllTimersAsync()
    expect(search).not.toHaveBeenCalled()
  })
})
