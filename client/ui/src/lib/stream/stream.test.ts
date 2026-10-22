import { describe, expect, it } from 'vitest'
import { generation, needsReannounce } from './stream.svelte'
import type { StreamInfo } from '../protocol/StreamInfo'

const info = (channel_id: string, user_id: string): StreamInfo => ({
  channel_id,
  user_id,
  settings: { height: 720, fps: 60, audio: true },
  status: 'live',
  viewers: [],
  started_at: '2026-10-23T10:00:00Z',
})

describe('needsReannounce', () => {
  it('reannounces_when_ready_lacks_my_stream', () => {
    // the server restarted: its streams are in memory, so Ready comes back without ours
    expect(needsReannounce('me', { channelId: 'c1' }, [])).toBe(true)
    // someone else's stream in the channel, or ours in another channel, is not ours here
    expect(needsReannounce('me', { channelId: 'c1' }, [info('c1', 'other'), info('c2', 'me')])).toBe(true)
  })
  it('no_reannounce_when_ready_has_it', () => {
    expect(needsReannounce('me', { channelId: 'c1' }, [info('c1', 'other'), info('c1', 'me')])).toBe(false)
  })
  it('no_reannounce_when_not_live', () => {
    expect(needsReannounce('me', null, [])).toBe(false)
    expect(needsReannounce('me', null, [info('c1', 'me')])).toBe(false)
  })
})

describe('generation', () => {
  it('a_newer_call_supersedes_an_older_one', () => {
    const g = generation()
    const first = g.next()
    expect(g.current(first)).toBe(true)
    const second = g.next()
    expect(g.current(first)).toBe(false)
    expect(g.current(second)).toBe(true)
  })
  it('cancel_invalidates_the_call_in_flight', () => {
    const g = generation()
    const t = g.next()
    g.cancel()
    expect(g.current(t)).toBe(false)
    expect(g.current(g.next())).toBe(true)
  })
})
