// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { LinkEmbed as Embed } from '../lib/protocol/LinkEmbed'

const ORIGIN = 'https://pulsechat.co.za'
let origin: string | null = ORIGIN
const invoke = vi.fn(async (cmd: string, _args?: unknown): Promise<unknown> => (cmd === 'embed_origin' ? origin : undefined))
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import LinkEmbed from './LinkEmbed.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import { voice } from '../lib/voice.svelte'
import { forgetEmbedOrigin, player } from '../lib/linkPreviews.svelte'
import { watchPrefs } from '../lib/watchParty.svelte'
import type { WatchParty } from '../lib/protocol/WatchParty'
import { flushMicrotasks } from '../lib/testSettle'

const HASH = 'a'.repeat(64)
const yt = (over: Partial<Embed> = {}): Embed => ({
  url: 'https://www.youtube.com/watch?v=dQw4w9WgXcQ', provider: 'youtube', kind: 'player', provider_id: 'dQw4w9WgXcQ',
  title: 'A video', description: 'About it', site_name: 'YouTube', author: 'alex', thumb: { hash: HASH, width: 480, height: 360 },
  start: null, vertical: false, ...over,
})

const mounted: ReturnType<typeof mount>[] = []
const onError = vi.fn()
function show(embed: Embed, { mine = false, messageId = 'M1', channelId = null as string | null } = {}) {
  const host = document.createElement('div')
  document.body.append(host)
  mounted.push(mount(LinkEmbed, { target: host, props: { embed, messageId, channelId, mine, onError } }))
  flushSync()
  return host
}
const iframes = () => [...document.querySelectorAll('iframe')]

beforeEach(() => {
  origin = ORIGIN
  forgetEmbedOrigin()
  invoke.mockClear()
  onError.mockReset()
})
afterEach(() => {
  while (mounted.length) unmount(mounted.pop()!)
  document.body.innerHTML = ''
  vi.unstubAllGlobals()
})

describe('LinkEmbed', () => {
  it('renders provider text as text, never as HTML', async () => {
    const evil = '<img src=x onerror="alert(1)">'
    const host = show(yt({ title: evil, description: evil, author: evil, site_name: evil }))
    await flushMicrotasks(6)
    expect(host.textContent).toContain(evil)
    const imgs = [...host.querySelectorAll('img')]
    expect(imgs).toHaveLength(1) // only the thumbnail
    expect(imgs[0].getAttribute('src')).toContain(HASH)
    expect(host.querySelector('[onerror]')).toBeNull()
  })

  it('the title links to the URL, opening outside the app', async () => {
    const host = show(yt())
    await flushMicrotasks(6)
    const a = [...host.querySelectorAll('a')].find((x) => x.textContent === 'A video')!
    expect(a.getAttribute('href')).toBe('https://www.youtube.com/watch?v=dQw4w9WgXcQ')
    expect(a.getAttribute('target')).toBe('_blank')
    expect(a.getAttribute('rel')).toBe('noopener noreferrer')
  })

  it('clicking play swaps the thumbnail for our embed page', async () => {
    const host = show(yt({ start: 30 }))
    await flushMicrotasks(6)
    expect(iframes()).toHaveLength(0)
    host.querySelector<HTMLButtonElement>('button.play')!.click()
    flushSync()
    const f = iframes()
    expect(f).toHaveLength(1)
    expect(f[0].getAttribute('src')!.startsWith('https://pulsechat.co.za/embed/youtube/')).toBe(true)
    expect(f[0].getAttribute('src')).toBe('https://pulsechat.co.za/embed/youtube/dQw4w9WgXcQ?start=30')
    expect(f[0].getAttribute('sandbox')).toBe('allow-scripts allow-same-origin allow-presentation')
    expect(f[0].getAttribute('allow')).toBe('autoplay; encrypted-media; fullscreen; picture-in-picture')
    expect(f[0].getAttribute('referrerpolicy')).toBe('strict-origin-when-cross-origin')
    expect(f[0].getAttribute('title')).toBeTruthy()
    expect(host.querySelector('img')).toBeNull()
  })

  // #292: on Linux the video plays in a window of its own; when it closes (the player errors) or the video ends, our
  // embed page says so, and the card goes back to its thumbnail and play button instead of a dead player.
  const fromPage = (data: unknown, source: Window | null, o = ORIGIN) => window.dispatchEvent(new MessageEvent('message', { data, origin: o, source }))
  for (const state of ['ended', 'error'] as const) {
    it(`the player saying it ${state === 'ended' ? 'ended' : 'failed (its pop-out closed)'} brings the card back`, async () => {
      const host = show(yt())
      await flushMicrotasks(6)
      host.querySelector<HTMLButtonElement>('button.play')!.click()
      flushSync()
      const f = host.querySelector('iframe')!
      fromPage({ pulse: 'player', state }, f.contentWindow)
      flushSync()
      expect(host.querySelector('iframe')).toBeNull()
      expect(host.querySelector('img')!.getAttribute('src')).toContain(HASH)
      expect(host.querySelector('button.play')).not.toBeNull()
      expect(player.active).toBeNull()
      // and plays again from it
      host.querySelector<HTMLButtonElement>('button.play')!.click()
      flushSync()
      expect(host.querySelector('iframe')).not.toBeNull()
    })
  }

  it("ignores a player message from another origin, another card's player, or anything that isn't one", async () => {
    const a = show(yt())
    const b = show(yt({ url: 'https://youtu.be/aaaaaaaaaaa', provider_id: 'aaaaaaaaaaa' }), { messageId: 'M2' })
    await flushMicrotasks(6)
    a.querySelector<HTMLButtonElement>('button.play')!.click()
    flushSync()
    const f = a.querySelector('iframe')!
    fromPage({ pulse: 'player', state: 'ended' }, f.contentWindow, 'https://evil.example')
    fromPage({ pulse: 'player', state: 'ended' }, window)
    fromPage({ pulse: 'player', state: 'playing' }, f.contentWindow)
    fromPage('{"pulse":"player","state":"ended"}', f.contentWindow)
    fromPage(null, f.contentWindow)
    flushSync()
    expect(a.querySelector('iframe')).not.toBeNull()
    expect(b.querySelector('iframe')).toBeNull()
  })

  it('an open player can be closed back to its card', async () => {
    const host = show(yt())
    await flushMicrotasks(6)
    expect(host.querySelector('button.close-player')).toBeNull()
    host.querySelector<HTMLButtonElement>('button.play')!.click()
    flushSync()
    const close = host.querySelector<HTMLButtonElement>('button.close-player')!
    expect(close.getAttribute('aria-label')).toBe('Close player')
    close.click()
    flushSync()
    expect(host.querySelector('iframe')).toBeNull()
    expect(host.querySelector('button.play')).not.toBeNull()
    expect(host.querySelector('img')).not.toBeNull()
  })

  it('starting a second player stops the first', async () => {
    const a = show(yt())
    const b = show(yt({ url: 'https://youtu.be/aaaaaaaaaaa', provider_id: 'aaaaaaaaaaa' }), { messageId: 'M2' })
    await flushMicrotasks(6)
    a.querySelector<HTMLButtonElement>('button.play')!.click()
    flushSync()
    expect(a.querySelector('iframe')).not.toBeNull()
    b.querySelector<HTMLButtonElement>('button.play')!.click()
    flushSync()
    expect(a.querySelector('iframe')).toBeNull()
    expect(b.querySelector('iframe')).not.toBeNull()
    expect(iframes()).toHaveLength(1)
  })

  it('without an embed origin the play control opens the link instead', async () => {
    origin = null
    const host = show(yt())
    await flushMicrotasks(6)
    const play = host.querySelector<HTMLAnchorElement>('a.play')!
    expect(play).not.toBeNull()
    expect(play.getAttribute('href')).toBe('https://www.youtube.com/watch?v=dQw4w9WgXcQ')
    expect(play.getAttribute('target')).toBe('_blank')
    expect(play.getAttribute('rel')).toBe('noopener noreferrer')
    play.addEventListener('click', (e) => e.preventDefault()) // jsdom would try to navigate
    play.click()
    flushSync()
    expect(iframes()).toHaveLength(0)
    expect(host.querySelector('button.play')).toBeNull()
  })

  it('Instagram opens the pop-out window', async () => {
    const host = show(yt({ url: 'https://www.instagram.com/reel/Abc_12/', provider: 'instagram', provider_id: 'reel:Abc_12', title: null, thumb: null, site_name: 'Instagram' }))
    await flushMicrotasks(6)
    expect(host.textContent).toContain('Instagram · reel')
    host.querySelector<HTMLButtonElement>('button.play')!.click()
    flushSync()
    expect(invoke).toHaveBeenCalledWith('open_embed_window', { provider: 'instagram', id: 'reel:Abc_12', start: null })
    expect(iframes()).toHaveLength(0)
    // Instagram never plays inline, so there's no player to move
    expect(host.querySelector('button.pop-out')).toBeNull()
  })

  // #88: YouTube's error 153 means it won't play framed in the app (no Referer it trusts).
  it('YouTube refusing to play framed (error 153) moves it to the pop-out window, and the rest follow', async () => {
    const host = show(yt({ start: 42 }))
    const other = show(yt({ url: 'https://youtu.be/aaaaaaaaaaa', provider_id: 'aaaaaaaaaaa' }), { messageId: 'M2' })
    await flushMicrotasks(6)
    host.querySelector<HTMLButtonElement>('button.play')!.click()
    flushSync()
    const f = host.querySelector('iframe')!
    fromPage({ pulse: 'player', state: 'error', code: 153 }, f.contentWindow)
    flushSync()
    expect(iframes()).toHaveLength(0)
    expect(player.active).toBeNull()
    expect(invoke).toHaveBeenCalledWith('open_embed_window', { provider: 'youtube', id: 'dQw4w9WgXcQ', start: 42 })
    // another YouTube card now opens the window too, never an iframe
    invoke.mockClear()
    other.querySelector<HTMLButtonElement>('button.play')!.click()
    flushSync()
    expect(iframes()).toHaveLength(0)
    expect(invoke).toHaveBeenCalledWith('open_embed_window', { provider: 'youtube', id: 'aaaaaaaaaaa', start: null })
  })

  it('any other player error only brings the card back', async () => {
    const host = show(yt())
    await flushMicrotasks(6)
    host.querySelector<HTMLButtonElement>('button.play')!.click()
    flushSync()
    fromPage({ pulse: 'player', state: 'error', code: 150 }, host.querySelector('iframe')!.contentWindow)
    flushSync()
    expect(iframes()).toHaveLength(0)
    expect(invoke).not.toHaveBeenCalledWith('open_embed_window', expect.anything())
    host.querySelector<HTMLButtonElement>('button.play')!.click()
    flushSync()
    expect(iframes()).toHaveLength(1)
  })

  it("a playing Twitch clip can move to the pop-out window (its refusal can't be seen from outside)", async () => {
    const host = show(yt({ url: 'https://clips.twitch.tv/FunnyClip', provider: 'twitch', provider_id: 'FunnyClip', site_name: 'Twitch' }))
    await flushMicrotasks(6)
    host.querySelector<HTMLButtonElement>('button.play')!.click()
    flushSync()
    expect(iframes()).toHaveLength(1)
    const move = host.querySelector<HTMLButtonElement>('button.pop-out')!
    expect(move.getAttribute('aria-label')).toBe('Open in a window')
    move.click()
    flushSync()
    expect(iframes()).toHaveLength(0)
    expect(invoke).toHaveBeenCalledWith('open_embed_window', { provider: 'twitch', id: 'FunnyClip', start: null })
    // just this once: the next play is inline again
    host.querySelector<HTMLButtonElement>('button.play')!.click()
    flushSync()
    expect(iframes()).toHaveLength(1)
  })

  it('a generic card has no play control', async () => {
    const host = show(yt({ url: 'https://example.com/a', provider: 'generic', kind: 'card', provider_id: null, site_name: 'Example' }))
    await flushMicrotasks(6)
    expect(host.querySelector('.play')).toBeNull()
    expect(host.textContent).toContain('Example')
  })

  it('“Remove preview” only for the sender, and it hides this link', async () => {
    const theirs = show(yt())
    await flushMicrotasks(6)
    expect(theirs.querySelector('button[aria-label="Remove preview"]')).toBeNull()
    const mine = show(yt(), { mine: true, messageId: 'M9' })
    await flushMicrotasks(6)
    mine.querySelector<HTMLButtonElement>('button[aria-label="Remove preview"]')!.click()
    await flushMicrotasks(6)
    expect(invoke).toHaveBeenCalledWith('hide_embed', { messageId: 'M9', url: 'https://www.youtube.com/watch?v=dQw4w9WgXcQ' })
  })

  it('a failed hide reports through onError', async () => {
    const host = show(yt(), { mine: true })
    await flushMicrotasks(6)
    invoke.mockImplementationOnce(async () => { throw 'not allowed' })
    host.querySelector<HTMLButtonElement>('button[aria-label="Remove preview"]')!.click()
    await flushMicrotasks(6)
    expect(onError).toHaveBeenCalledWith('not allowed')
  })

  it('scrolling far away stops the player', async () => {
    let cb: ((es: { isIntersecting: boolean }[]) => void) | undefined
    let opts: IntersectionObserverInit | undefined
    vi.stubGlobal('IntersectionObserver', class {
      constructor(c: typeof cb, o: IntersectionObserverInit) { cb = c; opts = o }
      observe() {}
      disconnect() {}
    })
    const host = show(yt())
    await flushMicrotasks(6)
    host.querySelector<HTMLButtonElement>('button.play')!.click()
    flushSync()
    expect(opts?.rootMargin).toBe('400px')
    cb!([{ isIntersecting: true }])
    flushSync()
    expect(host.querySelector('iframe')).not.toBeNull()
    cb!([{ isIntersecting: false }])
    flushSync()
    expect(host.querySelector('iframe')).toBeNull()
    expect(player.active).toBeNull()
  })

  describe('Spotify', () => {
    const SP = '4uLU6hMCjMI75M1A2tKUQC'
    const sp = (over: Partial<Embed> = {}): Embed => yt({
      url: `https://open.spotify.com/track/${SP}`, provider: 'spotify', provider_id: `track:${SP}`, title: 'A song',
      description: null, author: 'alex', site_name: 'Spotify', ...over,
    })
    const jam = (): Embed => sp({
      url: 'https://open.spotify.com/socialsession/ABC123abc123', kind: 'card', provider_id: 'jam:ABC123abc123',
      title: 'Join the Jam', author: null, thumb: null,
    })
    const link = (host: HTMLElement, text: string) => [...host.querySelectorAll('a')].find((a) => a.textContent?.trim() === text)

    describe('Share to stage (#234)', () => {
      const shareBtn = (host: HTMLElement) => [...host.querySelectorAll('button')].find((b) => b.textContent?.trim() === 'Share to stage')
      beforeEach(() => {
        app.state = { ...emptyState(), channels: {
          V1: { id: 'V1', server_id: 'S1', kind: 'voice', name: 'Lounge', position: 0 },
          C1: { id: 'C1', server_id: 'S1', kind: 'text', name: 'general', position: 0 },
        } }
        voice.channelId = 'V1'
        voice.connection = 'connected'
      })
      afterEach(() => {
        voice.channelId = null
        voice.connection = 'disconnected'
      })

      it('on a Spotify card in the chat of the call you are in', async () => {
        const host = show(jam(), { channelId: 'V1' })
        await flushMicrotasks(6)
        shareBtn(host)!.click()
        await flushMicrotasks(6)
        expect(invoke).toHaveBeenCalledWith('share_to_stage', { channelId: 'V1', url: 'https://open.spotify.com/socialsession/ABC123abc123' })
      })

      it('says "On the stage" while that very link is up there', async () => {
        app.state = { ...app.state, stageShares: { V1: [{ id: 'SH1', channel_id: 'V1', shared_by: 'U2', shared_at: '', expires_at: '', embed: jam() }] } }
        const host = show(jam(), { channelId: 'V1' })
        await flushMicrotasks(6)
        expect(shareBtn(host)).toBeUndefined()
        expect(host.textContent).toContain('On the stage')
        expect(shareBtn(show(sp(), { channelId: 'V1' }))).toBeDefined()
      })

      it('not out of the call, in a text chat, or on another provider', async () => {
        expect(shareBtn(show(sp(), { channelId: 'C1' }))).toBeUndefined()
        expect(shareBtn(show(yt(), { channelId: 'V1' }))).toBeUndefined()
        voice.connection = 'connecting'
        flushSync()
        expect(shareBtn(show(sp(), { channelId: 'V1' }))).toBeUndefined()
        voice.connection = 'connected'
        voice.channelId = 'V2'
        flushSync()
        expect(shareBtn(show(sp(), { channelId: 'V1' }))).toBeUndefined()
      })

      it('a refusal shows as the message\'s error', async () => {
        const host = show(sp(), { channelId: 'V1' })
        await flushMicrotasks(6)
        invoke.mockImplementationOnce(async () => { throw { kind: 'rejected', message: "that isn't a Spotify link" } })
        shareBtn(host)!.click()
        await flushMicrotasks(6)
        expect(onError).toHaveBeenCalledOnce()
      })
    })

    it('no coloured bar down the left edge of any card', async () => {
      for (const e of [sp(), yt()]) {
        const host = show(e)
        await flushMicrotasks(6)
        expect(host.querySelector<HTMLElement>('.embed')!.style.getPropertyValue('--brand')).toBe('')
      }
    })

    it('a track: kind, title, artist, uncovered artwork and “Listen on Spotify”', async () => {
      const host = show(sp())
      await flushMicrotasks(6)
      expect(host.querySelector('.label')!.textContent).toContain('Spotify · Track')
      expect(host.textContent).toContain('A song')
      expect(host.querySelector('.sub')!.textContent).toBe('alex')
      const open = link(host, 'Listen on Spotify')!
      expect(open.getAttribute('href')).toBe(`https://open.spotify.com/track/${SP}`)
      expect(open.getAttribute('target')).toBe('_blank')
      expect(open.getAttribute('rel')).toBe('noopener noreferrer')
      // Spotify's rules: nothing drawn over the artwork.
      const art = host.querySelector('.art')!
      expect(art.querySelector('img')!.getAttribute('src')).toContain(HASH)
      expect(art.querySelector('.play, svg')).toBeNull()
    })

    it('the attribution is the single-colour Spotify icon', async () => {
      const host = show(sp())
      await flushMicrotasks(6)
      const icon = host.querySelector('.label svg[role="img"]')!
      expect(icon.getAttribute('aria-label')).toBe('Spotify')
      expect(icon.querySelector('path')!.getAttribute('fill')).toBe('currentColor')
      expect(Number(icon.getAttribute('width'))).toBeGreaterThanOrEqual(21)
    })

    it('“Play” starts the inline player through our embed page', async () => {
      const host = show(sp())
      await flushMicrotasks(6)
      const play = [...host.querySelectorAll('button')].find((b) => b.textContent?.trim() === 'Play')!
      play.click()
      flushSync()
      expect(iframes().map((f) => f.getAttribute('src'))).toEqual([`https://pulsechat.co.za/embed/spotify/track%3A${SP}`])
      expect(link(host, 'Listen on Spotify')).toBeDefined()
    })

    it('without an embed origin there is no Play, only the link out', async () => {
      origin = null
      const host = show(sp())
      await flushMicrotasks(6)
      expect([...host.querySelectorAll('button')].some((b) => b.textContent?.trim() === 'Play')).toBe(false)
      expect(link(host, 'Listen on Spotify')).toBeDefined()
    })

    it('an artist says “Open Spotify”; a podcast is named so', async () => {
      const a = show(sp({ url: `https://open.spotify.com/artist/${SP}`, provider_id: `artist:${SP}` }))
      const p = show(sp({ url: `https://open.spotify.com/show/${SP}`, provider_id: `show:${SP}` }), { messageId: 'M2' })
      await flushMicrotasks(6)
      expect(link(a, 'Open Spotify')).toBeDefined()
      expect(link(a, 'Listen on Spotify')).toBeUndefined()
      expect(p.querySelector('.label')!.textContent).toContain('Spotify · Podcast')
      expect(link(p, 'Listen on Spotify')).toBeDefined()
    })

    it('a Jam: “Join the Jam”, our subtitle, “Open Spotify” and no player', async () => {
      const host = show(jam())
      await flushMicrotasks(6)
      expect(host.querySelector('.label')!.textContent).toContain('Spotify · Jam')
      expect(host.querySelector('.title')!.textContent).toBe('Join the Jam')
      expect(host.querySelector('.sub')!.textContent).toMatch(/together/i)
      const open = link(host, 'Open Spotify')!
      expect(open.getAttribute('href')).toBe('https://open.spotify.com/socialsession/ABC123abc123')
      expect(open.getAttribute('target')).toBe('_blank')
      expect(host.querySelector('.play')).toBeNull()
      expect([...host.querySelectorAll('button')].some((b) => b.textContent?.trim() === 'Play')).toBe(false)
      expect(iframes()).toHaveLength(0)
      expect(host.querySelector('img')).toBeNull()
    })
  })

  it('asks the core for the embed origin once', async () => {
    show(yt())
    show(yt(), { messageId: 'M2' })
    await flushMicrotasks(6)
    expect(invoke.mock.calls.filter(([c]) => c === 'embed_origin')).toHaveLength(1)
  })
})

describe('watch party (#384)', () => {
  const btn = (host: HTMLElement, text: string) => [...host.querySelectorAll('button')].find((b) => b.textContent?.trim() === text)
  const party = (over: Partial<WatchParty> = {}): WatchParty => ({
    channel_id: 'V1', host_id: 'U1', source: 'youtube', video_id: 'aaaaaaaaaaa', position_ms: 0, playing: true, set_at_ms: 0, everyone_controls: false,
    viewers: ['U1'], queue: [], ...over,
  })
  beforeEach(() => {
    app.state = { ...emptyState(), me: { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }, channels: {
      V1: { id: 'V1', server_id: 'S1', kind: 'voice', name: 'Lounge', position: 0 },
      D1: { id: 'D1', server_id: null, kind: 'dm', name: '', position: 0 },
    } }
    voice.channelId = 'V1'
    voice.connection = 'connected'
    watchPrefs.on = true
  })
  afterEach(() => {
    voice.channelId = null
    voice.connection = 'disconnected'
    watchPrefs.on = true
  })

  it('a YouTube card starts one in the voice channel you are in, from any chat', async () => {
    const host = show(yt({ provider_id: 'dQw4w9WgXcQ' }), { channelId: 'C9' })
    await flushMicrotasks(6)
    btn(host, 'Start watch party')!.click()
    await flushMicrotasks(6)
    expect(invoke).toHaveBeenCalledWith('start_watch_party', { channelId: 'V1', source: 'youtube', videoId: 'dQw4w9WgXcQ', title: 'A video' })
  })

  it('a Twitch VOD, channel or clip card starts one too (#459)', async () => {
    const tw = (provider_id: string) => yt({ url: 'https://www.twitch.tv/x', provider: 'twitch', provider_id, site_name: 'Twitch' })
    for (const [id, source, videoId] of [['video:123', 'twitch_video', '123'], ['channel:alex', 'twitch_channel', 'alex'], ['FunnyClip-1', 'twitch_clip', 'FunnyClip-1']]) {
      invoke.mockClear()
      const host = show(tw(id))
      await flushMicrotasks(6)
      btn(host, 'Start watch party')!.click()
      await flushMicrotasks(6)
      expect(invoke).toHaveBeenCalledWith('start_watch_party', { channelId: 'V1', source, videoId, title: 'A video' })
    }
    expect(btn(show(tw('video:v1')), 'Start watch party')).toBeUndefined()
  })

  it('not out of a server call, with watch parties off, or for a bad id', async () => {
    voice.connection = 'connecting'
    expect(btn(show(yt()), 'Start watch party')).toBeUndefined()
    voice.connection = 'connected'
    voice.channelId = 'D1'
    expect(btn(show(yt()), 'Start watch party')).toBeUndefined()
    voice.channelId = 'V1'
    watchPrefs.on = false
    expect(btn(show(yt()), 'Start watch party')).toBeUndefined()
    watchPrefs.on = true
    expect(btn(show(yt({ provider_id: 'bad' })), 'Start watch party')).toBeUndefined()
    expect(btn(show(yt()), 'Start watch party')).toBeDefined()
  })

  it('with one on: the host plays it there or queues it, a viewer queues it, and the video playing says so', async () => {
    app.state = { ...app.state, watchParties: { V1: party() } }
    let host = show(yt())
    expect(btn(host, 'Play in watch party')).toBeDefined()
    btn(host, 'Add to queue')!.click()
    await flushMicrotasks(6)
    expect(invoke).toHaveBeenCalledWith('watch_party_action', { channelId: 'V1', action: { action: 'enqueue', source: 'youtube', video_id: 'dQw4w9WgXcQ', title: 'A video' } })
    // sam hosts, alex watches: queue only
    app.state = { ...app.state, watchParties: { V1: party({ host_id: 'U2', viewers: ['U2', 'U1'] }) } }
    host = show(yt())
    expect(btn(host, 'Play in watch party')).toBeUndefined()
    expect(btn(host, 'Add to queue')).toBeDefined()
    // in the call but not watching: still queues it (#464)
    app.state = { ...app.state, watchParties: { V1: party({ host_id: 'U2', viewers: ['U2'] }) } }
    host = show(yt())
    expect(btn(host, 'Play in watch party')).toBeUndefined()
    expect(btn(host, 'Add to queue')).toBeDefined()
    app.state = { ...app.state, watchParties: { V1: party({ source: 'youtube' as const, video_id: 'dQw4w9WgXcQ' }) } }
    expect(show(yt()).textContent).toContain('In the watch party')
  })
})
