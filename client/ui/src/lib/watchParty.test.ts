import { describe, expect, it } from 'vitest'
import { clockText, correction, DRIFT_S, mayControl, partyPlayerSrc, partySeconds, validVideoId, watching, youtubeIdOf, YT } from './watchParty'
import type { WatchParty } from './protocol/WatchParty'
import type { LinkEmbed } from './protocol/LinkEmbed'

const party = (over: Partial<WatchParty> = {}): WatchParty => ({
  channel_id: 'V1', host_id: 'U1', source: 'youtube', video_id: 'dQw4w9WgXcQ', position_ms: 10_000, playing: true, set_at_ms: 1_000_000,
  everyone_controls: false, viewers: ['U1', 'U2'], queue: [], ...over,
})
const info = (state: number | null, time: number | null, duration: number | null = 300) => ({ state, time, duration })

describe('watch party sync (#384)', () => {
  it('video ids are strict', () => {
    expect(validVideoId('dQw4w9WgXcQ')).toBe(true)
    expect(validVideoId('a-b_c-d_e-f')).toBe(true)
    for (const bad of ['', 'short', 'dQw4w9WgXcQQ', 'dQw4w9WgXc!', '../../admin', null, undefined]) expect(validVideoId(bad)).toBe(false)
    const e = { provider: 'youtube', provider_id: 'dQw4w9WgXcQ' } as LinkEmbed
    expect(youtubeIdOf(e)).toBe('dQw4w9WgXcQ')
    expect(youtubeIdOf({ ...e, provider_id: 'x' })).toBeNull()
    expect(youtubeIdOf({ ...e, provider: 'twitch' })).toBeNull()
  })

  it('the position runs on by the server clock while playing, not while paused', () => {
    expect(partySeconds(party(), 1_000_000)).toBe(10)
    expect(partySeconds(party(), 1_004_500)).toBe(14.5)
    // a server stamp from just ahead of our guess never runs it backwards
    expect(partySeconds(party(), 999_000)).toBe(10)
    expect(partySeconds(party({ playing: false }), 9_999_999)).toBe(10)
  })

  it('who controls and who watches', () => {
    expect(mayControl(party(), 'U1')).toBe(true)
    expect(mayControl(party(), 'U2')).toBe(false)
    expect(mayControl(party({ everyone_controls: true }), 'U2')).toBe(true)
    expect(mayControl(party({ everyone_controls: true }), 'U3')).toBe(false)
    expect(mayControl(party(), null)).toBe(false)
    expect(watching(party(), 'U2')).toBe(true)
    expect(watching(party(), 'U3')).toBe(false)
    expect(watching(null, 'U1')).toBe(false)
  })

  it('corrects drift beyond DRIFT_S, and only then', () => {
    const now = 1_010_000 // the party is at 20 s
    expect(correction(party(), info(YT.playing, 20 + DRIFT_S - 0.1), now)).toEqual([])
    expect(correction(party(), info(YT.playing, 20 - DRIFT_S - 0.1), now)).toEqual([{ func: 'seek', to: 20 }])
    expect(correction(party(), info(YT.playing, 40), now)).toEqual([{ func: 'seek', to: 20 }])
  })

  it('plays and pauses to match, and waits while the player says nothing or buffers', () => {
    const now = 1_000_000
    expect(correction(party(), info(null, null), now)).toEqual([])
    expect(correction(party(), info(YT.paused, 10), now)).toEqual([{ func: 'play' }])
    expect(correction(party(), info(YT.unstarted, 0), now)).toEqual([{ func: 'play' }, { func: 'seek', to: 10 }])
    expect(correction(party({ playing: false }), info(YT.playing, 10), now)).toEqual([{ func: 'pause' }])
    expect(correction(party(), info(YT.buffering, 2), now)).toEqual([])
    expect(correction(party({ playing: false }), info(YT.buffering, 2), now)).toEqual([{ func: 'pause' }])
  })

  it('an ended video stays ended while the party runs past its end', () => {
    expect(correction(party(), info(YT.ended, 300, 300), 1_000_000 + 295_000)).toEqual([])
    // but a party seeked back into it restarts it
    expect(correction(party({ position_ms: 30_000 }), info(YT.ended, 300, 300), 1_000_000)).toEqual([{ func: 'play' }, { func: 'seek', to: 30 }])
  })

  it('the party page and the clock text', () => {
    expect(partyPlayerSrc('https://pulsechat.co.za/', 'dQw4w9WgXcQ', 42.9)).toBe('https://pulsechat.co.za/embed/youtube/dQw4w9WgXcQ?party=1&start=42')
    expect(partyPlayerSrc('https://pulsechat.co.za', 'dQw4w9WgXcQ')).toBe('https://pulsechat.co.za/embed/youtube/dQw4w9WgXcQ?party=1')
    expect(clockText(65)).toBe('1:05')
    expect(clockText(3723)).toBe('1:02:03')
    expect(clockText(-3)).toBe('0:00')
  })
})

describe('the queue (#464)', () => {
  it('reads YouTube links, and only YouTube’s own', async () => {
    const { youtubeIdFromLink, linkIn } = await import('./watchParty')
    for (const ok of ['https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=3', 'https://youtu.be/dQw4w9WgXcQ', 'https://m.youtube.com/shorts/dQw4w9WgXcQ', ' https://youtube.com/live/dQw4w9WgXcQ '])
      expect(youtubeIdFromLink(ok)).toBe('dQw4w9WgXcQ')
    for (const bad of ['', 'dQw4w9WgXcQ', 'https://youtube.com.evil.test/watch?v=dQw4w9WgXcQ', 'https://www.youtube.com/watch?v=short', 'javascript:alert(1)', 'https://www.youtube.com/channel/x'])
      expect(youtubeIdFromLink(bad)).toBeNull()
    expect(linkIn('# comment\r\nhttps://a.test/\nhttps://b.test/')).toBe('https://a.test/')
  })

  it('who may skip and edit, and when Up next shows', async () => {
    const { maySkip, mayEdit, upNextIn } = await import('./watchParty')
    const item = { source: 'youtube' as const, video_id: 'aaaaaaaaaaa', added_by: 'U2' }
    const p = party({ added_by: 'U2', queue: [item] })
    expect([maySkip(p, 'U1'), maySkip(p, 'U2'), maySkip(p, 'U3')]).toEqual([true, true, false])
    expect([mayEdit(p, item, 'U2'), mayEdit(p, { ...item, added_by: 'U3' }, 'U2')]).toEqual([true, false])
    expect(upNextIn(p, 290.2, 300)).toBe(10)
    expect(upNextIn(p, 200, 300)).toBeNull()
    expect(upNextIn(p, 290, null)).toBeNull()
    expect(upNextIn({ ...p, queue: [] }, 290, 300)).toBeNull()
    expect(upNextIn({ ...p, playing: false }, 290, 300)).toBeNull()
  })
})

describe('Twitch in the party (#459)', () => {
  it('reads Twitch links and cards, and builds their party pages', async () => {
    const { mediaFromLink, partyMediaOf, partyPlayerSrc } = await import('./watchParty')
    expect(mediaFromLink('https://www.twitch.tv/videos/2012345678?t=1h')).toEqual({ source: 'twitch_video', id: '2012345678' })
    expect(mediaFromLink('https://twitch.tv/Alex_1')).toEqual({ source: 'twitch_channel', id: 'alex_1' })
    expect(mediaFromLink('https://clips.twitch.tv/FunnyClip-abc')).toEqual({ source: 'twitch_clip', id: 'FunnyClip-abc' })
    expect(mediaFromLink('https://www.twitch.tv/alex/clip/FunnyClip-abc')).toEqual({ source: 'twitch_clip', id: 'FunnyClip-abc' })
    expect(mediaFromLink('https://youtu.be/dQw4w9WgXcQ')).toEqual({ source: 'youtube', id: 'dQw4w9WgXcQ' })
    for (const bad of ['https://www.twitch.tv/directory', 'https://www.twitch.tv/videos/v1', 'https://twitch.tv.evil.test/alex', 'https://www.twitch.tv/al.ex'])
      expect(mediaFromLink(bad)).toBeNull()
    const card = (provider_id: string) => ({ provider: 'twitch', provider_id }) as LinkEmbed
    expect(partyMediaOf(card('video:123'))).toEqual({ source: 'twitch_video', id: '123' })
    expect(partyMediaOf(card('channel:alex'))).toEqual({ source: 'twitch_channel', id: 'alex' })
    expect(partyMediaOf(card('Clip1'))).toEqual({ source: 'twitch_clip', id: 'Clip1' })
    expect(partyMediaOf(card('video:x'))).toBeNull()
    const o = 'https://pulsechat.co.za'
    expect(partyPlayerSrc(o, { source: 'twitch_video', id: '123' }, 90)).toBe(`${o}/embed/twitch/video%3A123?party=1&start=90`)
    expect(partyPlayerSrc(o, { source: 'twitch_channel', id: 'alex' }, 90)).toBe(`${o}/embed/twitch/channel%3Aalex?party=1`)
    expect(partyPlayerSrc(o, { source: 'twitch_clip', id: 'Clip1' }, 9)).toBe(`${o}/embed/twitch/Clip1`)
  })

  it('a live channel plays and pauses but never seeks; a clip takes no commands', () => {
    const live = party({ source: 'twitch_channel', video_id: 'alex', position_ms: 0, playing: true })
    expect(correction(live, info(YT.paused, 5, null), 1_000_000 + 600_000)).toEqual([{ func: 'play' }])
    expect(correction(live, info(YT.playing, 5, null), 1_000_000 + 600_000)).toEqual([])
    expect(correction({ ...live, playing: false }, info(YT.playing, 5, null), 0)).toEqual([{ func: 'pause' }])
    const clip = party({ source: 'twitch_clip', video_id: 'Clip1' })
    expect(correction(clip, info(YT.paused, 0), 9_999_999)).toEqual([])
  })
})
