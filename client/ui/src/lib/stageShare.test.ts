import { describe, expect, it } from 'vitest'
import { ANYONE_IN_CALL_MAY_STOP, JAM_MAY_HAVE_ENDED_MS, mayStop, sharePhrase, stageCard } from './stageShare'
import type { StageShare } from './protocol/StageShare'

const at = Date.parse('2026-11-10T10:00:00.000Z')
const share = (provider_id: string, over: Partial<StageShare['embed']> = {}): StageShare => ({
  id: 's1', channel_id: 'V1', shared_by: 'U2', shared_at: new Date(at).toISOString(), expires_at: new Date(at + 3 * 3600_000).toISOString(),
  embed: { url: 'https://open.spotify.com/x', provider: 'spotify', kind: 'player', provider_id, title: 'Midnight City', description: null, site_name: 'Spotify', author: 'M83', thumb: null, start: null, vertical: false, ...over },
})

describe('stageCard', () => {
  it('a Jam: Join the Jam, and "may have ended" after an hour', () => {
    const jam = share('jam:abc', { kind: 'card', title: 'Join the Jam', author: null })
    expect(stageCard(jam, 'alex', at)).toEqual({ title: 'Spotify Jam', subtitle: 'Listen together · Shared by alex', action: 'Join the Jam', jam: true })
    expect(stageCard(jam, 'alex', at + JAM_MAY_HAVE_ENDED_MS - 1).subtitle).toBe('Listen together · Shared by alex')
    expect(stageCard(jam, 'alex', at + JAM_MAY_HAVE_ENDED_MS).subtitle).toBe('Shared by alex · may have ended')
  })

  it('a song: its title, artist and Listen on Spotify; never "may have ended"', () => {
    expect(stageCard(share('track:x'), 'sam', at + 2 * JAM_MAY_HAVE_ENDED_MS)).toEqual({ title: 'Midnight City', subtitle: 'M83 · Shared by sam', action: 'Listen on Spotify', jam: false })
  })

  it('without an artist the kind stands in; without a title the link; an artist opens Spotify', () => {
    expect(stageCard(share('playlist:x', { author: null }), 'kim', at).subtitle).toBe('Playlist · Shared by kim')
    expect(stageCard(share('track:x', { title: null }), 'kim', at).title).toBe('https://open.spotify.com/x')
    expect(stageCard(share('artist:x', { author: null }), 'kim', at).action).toBe('Open Spotify')
  })
})

describe('mayStop', () => {
  it('anyone in the call; nobody outside it', () => {
    expect(ANYONE_IN_CALL_MAY_STOP).toBe(true)
    expect(mayStop(share('track:x'), 'U3', true)).toBe(true)
    expect(mayStop(share('track:x'), 'U2', false)).toBe(false)
  })
})

it('sharePhrase words each kind as the server does', () => {
  expect(['jam', 'track', 'show', 'odd'].map(sharePhrase)).toEqual(['a Jam', 'a song', 'a podcast', 'something'])
})
