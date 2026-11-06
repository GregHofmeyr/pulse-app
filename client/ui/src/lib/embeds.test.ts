import { describe, expect, it } from 'vitest'
import { aspect, audioHeight, JAM_SUBTITLE, playable, playerSrc, popsOut, providerLabel, SANDBOX, spotifyCard } from './embeds'
import type { LinkEmbed } from './protocol/LinkEmbed'

const ORIGIN = 'https://pulsechat.co.za'
const e = (over: Partial<LinkEmbed> = {}): LinkEmbed => ({
  url: 'https://www.youtube.com/watch?v=dQw4w9WgXcQ', provider: 'youtube', kind: 'player', provider_id: 'dQw4w9WgXcQ',
  title: null, description: null, site_name: null, author: null, thumb: null, start: null, vertical: false, ...over,
})

describe('providerLabel', () => {
  it('names the known providers', () => {
    expect(providerLabel(e())).toBe('YouTube')
    expect(providerLabel(e({ provider: 'tiktok' }))).toBe('TikTok')
    expect(providerLabel(e({ provider: 'spotify' }))).toBe('Spotify')
    expect(providerLabel(e({ provider: 'soundcloud' }))).toBe('SoundCloud')
    expect(providerLabel(e({ provider: 'twitch' }))).toBe('Twitch')
  })
  it('says whether an Instagram link is a reel or a post', () => {
    expect(providerLabel(e({ provider: 'instagram', provider_id: 'reel:Abc_12' }))).toBe('Instagram · reel')
    expect(providerLabel(e({ provider: 'instagram', provider_id: 'p:Abc_12' }))).toBe('Instagram · post')
    expect(providerLabel(e({ provider: 'instagram', provider_id: null }))).toBe('Instagram')
  })
  it('a generic card uses its site name, else the host without www.', () => {
    expect(providerLabel(e({ provider: 'generic', kind: 'card', provider_id: null, site_name: 'Example News' }))).toBe('Example News')
    expect(providerLabel(e({ provider: 'generic', kind: 'card', provider_id: null, url: 'https://www.example.com/a' }))).toBe('example.com')
    expect(providerLabel(e({ provider: 'unknown', kind: 'card', provider_id: null, url: 'https://news.example.test/a' }))).toBe('news.example.test')
    expect(providerLabel(e({ provider: 'generic', kind: 'card', provider_id: null, url: 'not a url' }))).toBe('not a url')
  })
})

describe('playable', () => {
  it('a known player with an id', () => {
    expect(playable(e())).toBe(true)
    expect(playable(e({ provider: 'instagram', provider_id: 'reel:Abc' }))).toBe(true)
  })
  it('not a generic, unknown or id-less one', () => {
    expect(playable(e({ provider: 'generic', kind: 'card', provider_id: null }))).toBe(false)
    expect(playable(e({ provider: 'unknown' }))).toBe(false)
    expect(playable(e({ provider_id: null }))).toBe(false)
    expect(playable(e({ provider_id: '' }))).toBe(false)
    expect(playable(e({ kind: 'card' }))).toBe(false)
    // A kind a newer server sends that this build doesn't know is shown as a card.
    expect(playable(e({ kind: 'unknown' }))).toBe(false)
  })
})

describe('playerSrc', () => {
  it('points at our embed page with the id encoded', () => {
    expect(playerSrc(ORIGIN, e())).toBe('https://pulsechat.co.za/embed/youtube/dQw4w9WgXcQ')
    expect(playerSrc(ORIGIN + '/', e())).toBe('https://pulsechat.co.za/embed/youtube/dQw4w9WgXcQ')
    expect(playerSrc(ORIGIN, e({ provider: 'spotify', provider_id: 'track:4uLU6hMCjMI75M1A2tKUQC' })))
      .toBe('https://pulsechat.co.za/embed/spotify/track%3A4uLU6hMCjMI75M1A2tKUQC')
    expect(playerSrc(ORIGIN, e({ provider: 'soundcloud', provider_id: 'alex/a-track' })))
      .toBe('https://pulsechat.co.za/embed/soundcloud/alex%2Fa-track')
  })
  it('keeps a YouTube start, and only for YouTube', () => {
    expect(playerSrc(ORIGIN, e({ start: 42 }))).toBe('https://pulsechat.co.za/embed/youtube/dQw4w9WgXcQ?start=42')
    expect(playerSrc(ORIGIN, e({ start: 0 }))).toBe('https://pulsechat.co.za/embed/youtube/dQw4w9WgXcQ')
    // The embed page takes a start of at most a week; past that the player starts at the top.
    expect(playerSrc(ORIGIN, e({ start: 604800 }))).toBe('https://pulsechat.co.za/embed/youtube/dQw4w9WgXcQ?start=604800')
    expect(playerSrc(ORIGIN, e({ start: 604801 }))).toBe('https://pulsechat.co.za/embed/youtube/dQw4w9WgXcQ')
    expect(playerSrc(ORIGIN, e({ start: 4294967295 }))).toBe('https://pulsechat.co.za/embed/youtube/dQw4w9WgXcQ')
    expect(playerSrc(ORIGIN, e({ provider: 'tiktok', provider_id: '7300000000000000000', start: 42 })))
      .toBe('https://pulsechat.co.za/embed/tiktok/7300000000000000000')
  })
})

describe('aspect and sandbox', () => {
  it('16:9 by default, 9:16 when vertical, audio for Spotify and SoundCloud', () => {
    expect(aspect(e())).toBe('16 / 9')
    expect(aspect(e({ provider: 'tiktok', vertical: true }))).toBe('9 / 16')
    expect(aspect(e({ provider: 'spotify' }))).toBe('audio')
    expect(aspect(e({ provider: 'soundcloud' }))).toBe('audio')
    expect(audioHeight(e({ provider: 'spotify' }))).toBe(152)
    expect(audioHeight(e({ provider: 'soundcloud' }))).toBe(166)
  })
  it('no player may open popups', () => {
    for (const p of ['youtube', 'tiktok', 'spotify', 'soundcloud', 'twitch'] as const) {
      expect(SANDBOX[p]).toBe('allow-scripts allow-same-origin allow-presentation')
    }
  })
  it('Instagram pops out', () => {
    expect(popsOut(e({ provider: 'instagram', provider_id: 'p:Abc' }))).toBe(true)
    expect(popsOut(e())).toBe(false)
  })
})

describe('spotifyCard', () => {
  const sp = (provider_id: string | null, over: Partial<LinkEmbed> = {}) =>
    e({ url: 'https://open.spotify.com/x', provider: 'spotify', provider_id, ...over })
  it('plays-on-Spotify wording for what you listen to (Spotify\'s approved labels)', () => {
    for (const [kind, label] of [['track', 'Track'], ['album', 'Album'], ['playlist', 'Playlist'], ['episode', 'Episode'], ['show', 'Podcast']]) {
      expect(spotifyCard(sp(`${kind}:4uLU6hMCjMI75M1A2tKUQC`))).toEqual({ kind, label, action: 'Listen on Spotify', jam: false })
    }
  })
  it('“Open Spotify” for an artist and a Jam', () => {
    expect(spotifyCard(sp('artist:4uLU6hMCjMI75M1A2tKUQC'))).toEqual({ kind: 'artist', label: 'Artist', action: 'Open Spotify', jam: false })
    expect(spotifyCard(sp('jam:ABC123abc123', { kind: 'card' }))).toEqual({ kind: 'jam', label: 'Jam', action: 'Open Spotify', jam: true })
  })
  it('anything else is no Spotify card', () => {
    expect(spotifyCard(e())).toBeNull()
    expect(spotifyCard(sp(null))).toEqual({ kind: null, label: null, action: 'Open Spotify', jam: false })
    expect(spotifyCard(sp('user:abc'))).toEqual({ kind: null, label: null, action: 'Open Spotify', jam: false })
    expect(spotifyCard(e({ provider: 'generic', kind: 'card', provider_id: 'jam:ABC123abc123' }))).toBeNull()
  })
  it('the Jam card has Pulse\'s own subtitle', () => {
    expect(JAM_SUBTITLE).toMatch(/together/i)
  })
})
