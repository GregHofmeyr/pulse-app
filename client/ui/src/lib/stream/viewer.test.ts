import { describe, expect, it } from 'vitest'
import { admit, dropToast, layerFor, liveEndSound, MAX_TOASTS, pendingWatchReady, pushToast, toastFor, watchDisabledReason, type Toast } from './viewer'

describe('toastFor', () => {
  it('only_people_in_that_voice_channel_get_the_toast', () => {
    expect(toastFor({ channelId: 'c1', streamer: 'u2' }, 'c1', 'me')).toBe(true)
    expect(toastFor({ channelId: 'c1', streamer: 'u2' }, 'c2', 'me')).toBe(false)
    expect(toastFor({ channelId: 'c1', streamer: 'u2' }, null, 'me')).toBe(false)
  })
  it('you_do_not_get_a_toast_for_your_own_stream', () => {
    expect(toastFor({ channelId: 'c1', streamer: 'me' }, 'c1', 'me')).toBe(false)
  })
})

describe('watchDisabledReason', () => {
  it('needs_windows_and_the_same_voice_channel', () => {
    expect(watchDisabledReason({ windows: true, voiceChannelId: 'c1', channelId: 'c1' })).toBeNull()
    expect(watchDisabledReason({ windows: true, voiceChannelId: 'c2', channelId: 'c1' })).toBe('Join voice to watch')
    expect(watchDisabledReason({ windows: true, voiceChannelId: null, channelId: 'c1' })).toBe('Join voice to watch')
    expect(watchDisabledReason({ windows: false, voiceChannelId: 'c1', channelId: 'c1' })).toBe("Watching from Linux isn't supported yet")
  })
  it('linux_wins_over_join_voice_because_joining_would_not_help', () => {
    expect(watchDisabledReason({ windows: false, voiceChannelId: null, channelId: 'c1' })).toBe("Watching from Linux isn't supported yet")
  })
})

describe('toast stack', () => {
  const t = (streamer: string): Toast => ({ channelId: 'c1', streamer, name: streamer })
  it('stacks_two_and_dedupes_the_same_streamer', () => {
    let s: Toast[] = []
    s = pushToast(s, t('a'))
    s = pushToast(s, t('b'))
    s = pushToast(s, t('a'))
    expect(s.map((x) => x.streamer)).toEqual(['b', 'a'])
  })
  it('keeps_only_the_newest_few', () => {
    let s: Toast[] = []
    for (const n of ['a', 'b', 'c', 'd', 'e']) s = pushToast(s, t(n))
    expect(s).toHaveLength(MAX_TOASTS)
    expect(s[s.length - 1].streamer).toBe('e')
  })
  it('drops_by_stream', () => {
    const s = [t('a'), t('b')]
    expect(dropToast(s, 'c1', 'a').map((x) => x.streamer)).toEqual(['b'])
    expect(dropToast(s, 'c2', 'a')).toHaveLength(2)
  })
})

describe('watch list', () => {
  it('preview_is_always_low_full_follows_quality', () => {
    expect(layerFor('preview', 'auto')).toBe('low')
    expect(layerFor('preview', 'saver')).toBe('low')
    expect(layerFor('full', 'auto')).toBe('high')
    expect(layerFor('full', 'saver')).toBe('low')
  })
  it('admit_drops_the_oldest_at_the_cap', () => {
    expect(admit(['a'], 'b', 1)).toEqual({ list: ['b'], dropped: ['a'] })
    expect(admit(['a', 'b'], 'c', 2)).toEqual({ list: ['b', 'c'], dropped: ['a'] })
    expect(admit(['a'], 'b', 3)).toEqual({ list: ['a', 'b'], dropped: [] })
    expect(admit([], 'a', 1)).toEqual({ list: ['a'], dropped: [] })
  })
})

describe('pendingWatchReady', () => {
  const k = { channelId: 'c1', streamer: 'u2' }
  it('a_join_and_watch_is_ready_once_connected_in_that_channel', () => {
    expect(pendingWatchReady(k, { channelId: 'c1', connection: 'connected' })).toBe(true)
    expect(pendingWatchReady(k, { channelId: 'c1', connection: 'connecting' })).toBe(false)
    expect(pendingWatchReady(k, { channelId: 'c2', connection: 'connected' })).toBe(false)
    expect(pendingWatchReady(k, { channelId: null, connection: 'disconnected' })).toBe(false)
    expect(pendingWatchReady(null, { channelId: 'c1', connection: 'connected' })).toBe(false)
  })
})

describe('liveEndSound', () => {
  const end = (userId: string, reason: 'stopped' | 'left_voice' | 'crashed' | 'timed_out' = 'stopped', channelId = 'c1') => ({ channelId, userId, reason })
  const st = (voiceChannelId: string | null, watching: boolean) => ({ voiceChannelId, meId: 'me', watching })

  it('someone_elses_stream_in_my_channel_plays', () => {
    expect(liveEndSound(end('sam'), st('c1', false))).toBe(true)
  })
  it('someone_elses_stream_elsewhere_plays_only_when_watched', () => {
    expect(liveEndSound(end('sam'), st('c2', false))).toBe(false)
    expect(liveEndSound(end('sam'), st(null, false))).toBe(false)
    expect(liveEndSound(end('sam'), st('c2', true))).toBe(true)
  })
  it('my_own_stream_plays_only_when_the_server_ended_it', () => {
    expect(liveEndSound(end('me', 'timed_out'), st('c1', false))).toBe(true)
    expect(liveEndSound(end('me', 'crashed'), st('c1', false))).toBe(true)
    expect(liveEndSound(end('me', 'stopped'), st('c1', false))).toBe(false)
    expect(liveEndSound(end('me', 'left_voice'), st('c1', false))).toBe(false)
  })
  it('my_own_stream_in_another_channel_is_silent', () => {
    expect(liveEndSound(end('me', 'timed_out'), st('c2', false))).toBe(false)
    expect(liveEndSound(end('me', 'crashed'), st(null, false))).toBe(false)
  })
})
