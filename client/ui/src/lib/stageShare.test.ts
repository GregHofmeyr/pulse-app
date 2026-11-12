import { describe, expect, it } from 'vitest'
import { ANYONE_IN_CALL_MAY_STOP, JAM_MAY_HAVE_ENDED_MS, mayStop, sharePhrase, spotifyLinkIn, stageCard } from './stageShare'
import type { StageShare } from './protocol/StageShare'

const at = Date.parse('2026-11-10T10:00:00.000Z')
const share = (provider_id: string, over: Partial<StageShare['embed']> = {}): StageShare => ({
  id: 's1', channel_id: 'V1', shared_by: 'U2', shared_at: new Date(at).toISOString(), expires_at: new Date(at + 3 * 3600_000).toISOString(),
  embed: { url: 'https://open.spotify.com/x', provider: 'spotify', kind: 'player', provider_id, title: 'Midnight City', description: null, site_name: 'Spotify', author: 'M83', thumb: null, start: null, vertical: false, ...over },
})

describe('stageCard', () => {
  it('a Jam: Join the Jam, and "may have ended" after an hour', () => {
    const jam = share('jam:abc', { kind: 'card', title: 'Join the Jam', author: null })
    expect(stageCard(jam, 'alex', at)).toEqual({ title: 'Spotify Jam', subtitle: 'Listen together · Shared by alex', action: 'Join the Jam', jam: true, nowPlaying: null })
    expect(stageCard(jam, 'alex', at + JAM_MAY_HAVE_ENDED_MS - 1).subtitle).toBe('Listen together · Shared by alex')
    expect(stageCard(jam, 'alex', at + JAM_MAY_HAVE_ENDED_MS).subtitle).toBe('Shared by alex · may have ended')
  })

  it('a song: its title, artist and Listen on Spotify; never "may have ended"', () => {
    expect(stageCard(share('track:x'), 'sam', at + 2 * JAM_MAY_HAVE_ENDED_MS)).toEqual({ title: 'Midnight City', subtitle: 'M83 · Shared by sam', action: 'Listen on Spotify', jam: false, nowPlaying: null })
  })

  it('without an artist the kind stands in; without a title the link; an artist opens Spotify', () => {
    expect(stageCard(share('playlist:x', { author: null }), 'kim', at).subtitle).toBe('Playlist · Shared by kim')
    expect(stageCard(share('track:x', { title: null }), 'kim', at).title).toBe('https://open.spotify.com/x')
    expect(stageCard(share('artist:x', { author: null }), 'kim', at).action).toBe('Open Spotify')
  })
})

describe('a Jam\'s Now playing (#275)', () => {
  const song = { title: 'Song', artist: 'The Band', album: null, spotify_id: null, art: null }
  it('a Jam shows what its sharer plays; anything else never does', () => {
    const jam = { ...share('jam:abc', { kind: 'card' }), now_playing: song }
    expect(stageCard(jam, 'alex', at).nowPlaying).toBe('Now playing · Song · The Band')
    expect(stageCard({ ...jam, now_playing: { ...song, artist: null } }, 'alex', at).nowPlaying).toBe('Now playing · Song')
    expect(stageCard({ ...share('track:x'), now_playing: song }, 'alex', at).nowPlaying).toBeNull()
  })
})

describe('mayStop', () => {
  it('anyone in the call; nobody outside it', () => {
    expect(ANYONE_IN_CALL_MAY_STOP).toBe(true)
    expect(mayStop(share('track:x'), 'U3', true)).toBe(true)
    expect(mayStop(share('track:x'), 'U2', false)).toBe(false)
  })
})

it('sharePhrase words each kind as the share lines in the history do', () => {
  expect(['jam', 'track', 'show', 'odd'].map(sharePhrase)).toEqual(['a Jam', 'a song', 'a podcast', 'something'])
})

describe('spotifyLinkIn (the composer nudge, #288)', () => {
  const ID = '4uLU6hMCjMI75M1A2tKUQC'
  it('finds a track, album, playlist, episode, show or artist link in a message, keyed without its ?si=', () => {
    for (const kind of ['track', 'album', 'playlist', 'episode', 'show', 'artist']) {
      const url = `https://open.spotify.com/${kind}/${ID}?si=abc`
      expect(spotifyLinkIn(`listen to this ${url} now`)).toEqual({ url, key: `${kind}:${ID}` })
    }
    // another copy of the same link is the same link
    expect(spotifyLinkIn(`https://open.spotify.com/intl-de/track/${ID}?si=xyz`)?.key).toBe(`track:${ID}`)
  })

  it('a Jam: its invite, its app URI and a spotify.link short link', () => {
    expect(spotifyLinkIn('join https://open.spotify.com/socialsession/AbC123abc_-x?si=1')).toEqual({ url: 'https://open.spotify.com/socialsession/AbC123abc_-x?si=1', key: 'jam:AbC123abc_-x' })
    expect(spotifyLinkIn('spotify:socialsession:AbC123abc_-x')?.key).toBe('jam:AbC123abc_-x')
    expect(spotifyLinkIn('https://spotify.link/AbCd12Ef')).toEqual({ url: 'https://spotify.link/AbCd12Ef', key: 'short:AbCd12Ef' })
  })

  it('leaves punctuation round it out', () => {
    expect(spotifyLinkIn(`(https://open.spotify.com/track/${ID}).`)?.url).toBe(`https://open.spotify.com/track/${ID}`)
    expect(spotifyLinkIn(`https://open.spotify.com/track/${ID}?si=a, so good`)?.url).toBe(`https://open.spotify.com/track/${ID}?si=a`)
  })

  it('nothing for other links, look-alikes or a link still being typed', () => {
    for (const t of [
      'no link here',
      'https://example.com/track/x',
      `https://open.spotify.com.example.com/track/${ID}`,
      `https://xopen.spotify.com/track/${ID}`,
      'https://open.spotify.com/track/4uLU6h',
      'https://open.spotify.com/',
      `https://open.spotify.com/user/${ID}`,
      'https://spotify.link/',
      `spotify:track:${ID}`,
    ]) expect(spotifyLinkIn(t), t).toBeNull()
  })
})
