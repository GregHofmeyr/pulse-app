// "Listening to…" (#275): wording, links, and how the state keeps it.
import { describe, expect, it } from 'vitest'
import { activityLine, artUrl, listenLink, listeningOf, songLine } from './activity'
import { applyEvent, applyReady, emptyState, stageShareIn } from './state'
import { testReady } from './testReady'
import type { Listening } from './protocol/Listening'
import type { StageShare } from './protocol/StageShare'

const ID = '4uLU6hMCjMI75M1A2tKUQC'
const song = (over: Partial<Listening> = {}): Listening => ({ title: 'Song', artist: 'The Band', album: null, spotify_id: null, art: null, ...over })
const sam = { id: 'U2', username: 'sam', display_name: 'sam', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }

describe('how it reads', () => {
  it('lines', () => {
    expect(activityLine(song())).toBe('Listening to Song · The Band')
    expect(songLine(song({ artist: null }))).toBe('Song')
  })

  it('a known song links to itself; otherwise a search that opens Spotify', () => {
    expect(listenLink(song({ spotify_id: `track:${ID}` }))).toEqual({ href: `https://open.spotify.com/track/${ID}`, label: 'Listen on Spotify' })
    expect(listenLink(song({ spotify_id: `episode:${ID}` })).href).toBe(`https://open.spotify.com/episode/${ID}`)
    expect(listenLink(song())).toEqual({ href: 'https://open.spotify.com/search/Song%20The%20Band', label: 'Open Spotify' })
    // nothing odd from the wire becomes a path
    expect(listenLink(song({ spotify_id: 'track:../../x', title: 'a/b?c' })).href).toBe('https://open.spotify.com/search/a%2Fb%3Fc%20The%20Band')
  })

  it('art only from a media hash', () => {
    expect(artUrl(song())).toBeNull()
    expect(artUrl(song({ art: { hash: 'not a hash', width: 1, height: 1 } }))).toBeNull()
    expect(artUrl(song({ art: { hash: 'a'.repeat(64), width: 300, height: 300 } }))).toContain('a'.repeat(64))
  })
})

describe('the state', () => {
  const base = () => applyReady(emptyState(), testReady([], { people: [{ user: sam, online: true, last_seen_at: null, status: 'online', listening: song() }] }))

  it('Ready carries it; an older server sends none', () => {
    expect(listeningOf(base(), 'U2')?.title).toBe('Song')
    const old = applyReady(emptyState(), testReady([], { people: [{ user: sam, online: true, last_seen_at: null, status: 'online' }] }))
    expect(listeningOf(old, 'U2')).toBeNull()
    expect(listeningOf(old, 'nobody')).toBeNull()
  })

  it('ListeningChanged sets and clears it; going offline drops it', () => {
    let s = applyEvent(base(), { t: 'ListeningChanged', d: { user_id: 'U2', listening: song({ title: 'Next' }) } }, 0)
    expect(listeningOf(s, 'U2')?.title).toBe('Next')
    s = applyEvent(s, { t: 'ListeningChanged', d: { user_id: 'U2', listening: null } }, 0)
    expect(listeningOf(s, 'U2')).toBeNull()
    s = applyEvent(s, { t: 'ListeningChanged', d: { user_id: 'U2', listening: song() } }, 0)
    // DND is still online: it stays
    s = applyEvent(s, { t: 'PresenceChanged', d: { user_id: 'U2', online: true, last_seen_at: null, status: 'dnd' } }, 0)
    expect(listeningOf(s, 'U2')).not.toBeNull()
    s = applyEvent(s, { t: 'PresenceChanged', d: { user_id: 'U2', online: false, last_seen_at: '2026-11-12T10:00:00Z', status: 'offline' } }, 0)
    expect(listeningOf(s, 'U2')).toBeNull()
    // someone unknown: nothing to change
    const same = applyEvent(s, { t: 'ListeningChanged', d: { user_id: 'U9', listening: song() } }, 0)
    expect(same).toBe(s)
  })

  it("StageNowPlaying updates that Jam's line only", () => {
    const share: StageShare = {
      id: 'a', channel_id: 'V1', shared_by: 'U2', shared_at: '2026-11-12T10:00:00.000Z', expires_at: '2026-11-12T13:00:00.000Z',
      embed: { url: 'https://open.spotify.com/socialsession/abcdefgh', provider: 'spotify', kind: 'card', provider_id: 'jam:abcdefgh', title: null, description: null, site_name: 'Spotify', author: null, thumb: null, start: null, vertical: false },
    }
    let s = applyEvent(base(), { t: 'StageShared', d: { share } }, 0)
    s = applyEvent(s, { t: 'StageNowPlaying', d: { channel_id: 'V1', share_id: 'a', now_playing: song({ title: 'Live' }) } }, 0)
    expect(stageShareIn(s, 'V1')?.now_playing?.title).toBe('Live')
    expect(applyEvent(s, { t: 'StageNowPlaying', d: { channel_id: 'V1', share_id: 'old', now_playing: null } }, 0)).toBe(s)
    s = applyEvent(s, { t: 'StageNowPlaying', d: { channel_id: 'V1', share_id: 'a', now_playing: null } }, 0)
    expect(stageShareIn(s, 'V1')?.now_playing).toBeUndefined()
  })
})
