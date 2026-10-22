import { describe, expect, it } from 'vitest'
import { generation, needsReannounce, streamApi } from './stream.svelte'
import { removeToast, toasts } from './viewer.svelte'
import { app } from '../store.svelte'
import { emptyState } from '../state'
import { voice } from '../voice.svelte'
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

describe('streamApi.onEvent', () => {
  const started = (channel: string, user: string) => ({ t: 'StreamStarted', d: { stream: info(channel, user) } }) as never
  const setup = (voiceChannel: string | null) => {
    app.state = { ...emptyState(), me: { id: 'me', username: 'me' } as never }
    voice.channelId = voiceChannel
    toasts.list = []
  }

  it('toasts_a_stream_started_in_my_voice_channel', () => {
    setup('c1')
    streamApi.onEvent(started('c1', 'other'))
    expect(toasts.list.map((t) => t.streamer)).toEqual(['other'])
    removeToast('c1', 'other')
  })
  it('nobody_else_gets_the_toast', () => {
    setup('c2')
    streamApi.onEvent(started('c1', 'other'))
    setup('c1')
    streamApi.onEvent(started('c1', 'me'))
    expect(toasts.list).toEqual([])
  })
  it('stream_ended_removes_its_toast', () => {
    setup('c1')
    streamApi.onEvent(started('c1', 'other'))
    streamApi.onEvent({ t: 'StreamEnded', d: { channel_id: 'c1', user_id: 'other', reason: 'stopped' } } as never)
    expect(toasts.list).toEqual([])
  })
})
