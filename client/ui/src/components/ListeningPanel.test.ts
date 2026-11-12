// @vitest-environment jsdom
// "Listening to" on a profile card (#275): art, song, artist, Spotify's icon, and the approved link-out.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, describe, expect, it } from 'vitest'
import ListeningPanel from './ListeningPanel.svelte'
import type { Listening } from '../lib/protocol/Listening'

const ID = '4uLU6hMCjMI75M1A2tKUQC'
let cmp: ReturnType<typeof mount> | undefined
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})
const show = (listening: Listening) => {
  cmp = mount(ListeningPanel, { target: document.body, props: { listening } })
  flushSync()
}

describe('ListeningPanel', () => {
  it('a known song: its art and Listen on Spotify to it', () => {
    show({ title: 'Song', artist: 'The Band', album: 'Album', spotify_id: `track:${ID}`, art: { hash: 'a'.repeat(64), width: 300, height: 300 } })
    expect(document.querySelector('.title')!.textContent).toBe('Song')
    expect(document.querySelector('.sub')!.textContent).toBe('The Band')
    expect(document.querySelector('img.art')!.getAttribute('src')).toContain('a'.repeat(64))
    expect(document.querySelector('.spotify-icon')).not.toBeNull()
    const a = document.querySelector<HTMLAnchorElement>('a.open')!
    expect([a.textContent, a.getAttribute('href')]).toEqual(['Listen on Spotify', `https://open.spotify.com/track/${ID}`])
  })

  it('no id (Windows): no art, and Open Spotify to a search', () => {
    show({ title: 'Song', artist: null, album: null, spotify_id: null, art: null })
    expect(document.querySelector('img')).toBeNull()
    expect(document.querySelector('.sub')).toBeNull()
    expect(document.querySelector('a.open')!.textContent).toBe('Open Spotify')
  })
})
