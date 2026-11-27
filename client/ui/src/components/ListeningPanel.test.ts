// @vitest-environment jsdom
// "Listening to" on a profile card (#275): art, song, artist, Spotify's icon, and the approved link-out; and Share to
// stage (#321) for a known song while you're in a server voice call.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import ListeningPanel from './ListeningPanel.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import { voice } from '../lib/voice.svelte'
import type { Listening } from '../lib/protocol/Listening'
import type { LinkEmbed } from '../lib/protocol/LinkEmbed'
import { settle } from '../lib/testSettle'

const ID = '4uLU6hMCjMI75M1A2tKUQC'
const known = (): Listening => ({ title: 'Song', artist: 'The Band', album: 'Album', spotify_id: `track:${ID}`, art: { hash: 'a'.repeat(64), width: 300, height: 300 } })
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
    show(known())
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

  it('another player (#322): its name, a generic icon, and no Spotify link, art or share', () => {
    show({ title: 'Song', artist: 'Kim', album: 'Album', spotify_id: null, art: null, player: 'youtube_music' })
    expect(document.querySelector('.head')!.textContent).toBe('Listening on YouTube Music')
    expect(document.querySelector('.spotify-icon')).toBeNull()
    expect(document.querySelector('.player-icon svg')!.getAttribute('aria-label')).toBe('YouTube Music')
    expect(document.querySelector('a.open')).toBeNull()
    expect(document.querySelector('.actions')).toBeNull()
    expect(document.querySelector('section')!.getAttribute('aria-label')).toBe('Listening to Song on YouTube Music')
  })

  it('a browser: "Listening in a browser"', () => {
    show({ title: 'Song', artist: 'Kim', album: 'Album', spotify_id: null, art: null, player: 'browser' })
    expect(document.querySelector('.head')!.textContent).toBe('Listening in a browser')
    expect(document.querySelector('a.open')).toBeNull()
  })

  describe('Share to stage (#321)', () => {
    const shareBtn = () => [...document.querySelectorAll('button')].find((b) => b.textContent?.trim() === 'Share to stage')
    beforeEach(() => {
      invoke.mockClear()
      app.state = { ...emptyState(), channels: {
        V1: { id: 'V1', server_id: 'S1', kind: 'voice', name: 'Lounge', position: 0 },
        D1: { id: 'D1', server_id: null, kind: 'dm', name: '', position: 0 },
      } } as typeof app.state
      voice.channelId = 'V1'
      voice.connection = 'connected'
    })
    afterEach(() => {
      voice.channelId = null
      voice.connection = 'disconnected'
    })

    it('shares the song to the stage of the call you are in', async () => {
      show(known())
      shareBtn()!.click()
      await settle(3)
      expect(invoke).toHaveBeenCalledWith('share_to_stage', { channelId: 'V1', url: `https://open.spotify.com/track/${ID}` })
    })

    it('says "On the stage" while that song is up there', () => {
      const embed = { url: `https://open.spotify.com/track/${ID}` } as LinkEmbed
      app.state = { ...app.state, stageShares: { V1: [{ id: 'SH1', channel_id: 'V1', shared_by: 'U2', shared_at: '', expires_at: '', embed }] } }
      show(known())
      expect(shareBtn()).toBeUndefined()
      expect(document.body.textContent).toContain('On the stage')
    })

    it('not without a song id, out of a call, while connecting, or in a DM call', () => {
      show({ ...known(), spotify_id: null })
      expect(shareBtn()).toBeUndefined()
      unmount(cmp!); cmp = undefined
      voice.connection = 'connecting'
      show(known())
      expect(shareBtn()).toBeUndefined()
      unmount(cmp!); cmp = undefined
      voice.connection = 'connected'
      voice.channelId = 'D1'
      show(known())
      expect(shareBtn()).toBeUndefined()
      unmount(cmp!); cmp = undefined
      voice.channelId = null
      show(known())
      expect(shareBtn()).toBeUndefined()
    })

    it('a refusal shows under the song', async () => {
      invoke.mockImplementationOnce(async () => { throw { kind: 'rejected', message: 'too many shares, try again soon' } })
      show(known())
      shareBtn()!.click()
      await settle(3)
      expect(document.querySelector('.err')!.textContent).toContain('too many shares')
    })
  })
})
