// @vitest-environment jsdom
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { LinkEmbed as Embed } from '../lib/protocol/LinkEmbed'

const ORIGIN = 'https://pulsechat.co.za'
let origin: string | null = ORIGIN
const invoke = vi.fn(async (cmd: string, _args?: unknown): Promise<unknown> => (cmd === 'embed_origin' ? origin : undefined))
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import LinkEmbed from './LinkEmbed.svelte'
import { forgetEmbedOrigin, player } from '../lib/linkPreviews.svelte'

const HASH = 'a'.repeat(64)
const yt = (over: Partial<Embed> = {}): Embed => ({
  url: 'https://www.youtube.com/watch?v=dQw4w9WgXcQ', provider: 'youtube', kind: 'player', provider_id: 'dQw4w9WgXcQ',
  title: 'A video', description: 'About it', site_name: 'YouTube', author: 'alex', thumb: { hash: HASH, width: 480, height: 360 },
  start: null, vertical: false, ...over,
})

const mounted: ReturnType<typeof mount>[] = []
const onError = vi.fn()
function show(embed: Embed, { mine = false, messageId = 'M1' } = {}) {
  const host = document.createElement('div')
  document.body.append(host)
  mounted.push(mount(LinkEmbed, { target: host, props: { embed, messageId, mine, onError } }))
  flushSync()
  return host
}
async function settle() { for (let i = 0; i < 5; i++) await Promise.resolve(); await tick(); flushSync() }
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
    await settle()
    expect(host.textContent).toContain(evil)
    const imgs = [...host.querySelectorAll('img')]
    expect(imgs).toHaveLength(1) // only the thumbnail
    expect(imgs[0].getAttribute('src')).toContain(HASH)
    expect(host.querySelector('[onerror]')).toBeNull()
  })

  it('the title links to the URL, opening outside the app', async () => {
    const host = show(yt())
    await settle()
    const a = [...host.querySelectorAll('a')].find((x) => x.textContent === 'A video')!
    expect(a.getAttribute('href')).toBe('https://www.youtube.com/watch?v=dQw4w9WgXcQ')
    expect(a.getAttribute('target')).toBe('_blank')
    expect(a.getAttribute('rel')).toBe('noopener noreferrer')
  })

  it('clicking play swaps the thumbnail for our embed page', async () => {
    const host = show(yt({ start: 30 }))
    await settle()
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

  it('starting a second player stops the first', async () => {
    const a = show(yt())
    const b = show(yt({ url: 'https://youtu.be/aaaaaaaaaaa', provider_id: 'aaaaaaaaaaa' }), { messageId: 'M2' })
    await settle()
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
    await settle()
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
    await settle()
    expect(host.textContent).toContain('Instagram · reel')
    host.querySelector<HTMLButtonElement>('button.play')!.click()
    flushSync()
    expect(invoke).toHaveBeenCalledWith('open_embed_window', { provider: 'instagram', id: 'reel:Abc_12' })
    expect(iframes()).toHaveLength(0)
  })

  it('a generic card has no play control', async () => {
    const host = show(yt({ url: 'https://example.com/a', provider: 'generic', kind: 'card', provider_id: null, site_name: 'Example' }))
    await settle()
    expect(host.querySelector('.play')).toBeNull()
    expect(host.textContent).toContain('Example')
  })

  it('“Remove preview” only for the sender, and it hides this link', async () => {
    const theirs = show(yt())
    await settle()
    expect(theirs.querySelector('button[aria-label="Remove preview"]')).toBeNull()
    const mine = show(yt(), { mine: true, messageId: 'M9' })
    await settle()
    mine.querySelector<HTMLButtonElement>('button[aria-label="Remove preview"]')!.click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('hide_embed', { messageId: 'M9', url: 'https://www.youtube.com/watch?v=dQw4w9WgXcQ' })
  })

  it('a failed hide reports through onError', async () => {
    const host = show(yt(), { mine: true })
    await settle()
    invoke.mockImplementationOnce(async () => { throw 'not allowed' })
    host.querySelector<HTMLButtonElement>('button[aria-label="Remove preview"]')!.click()
    await settle()
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
    await settle()
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

  it('asks the core for the embed origin once', async () => {
    show(yt())
    show(yt(), { messageId: 'M2' })
    await settle()
    expect(invoke.mock.calls.filter(([c]) => c === 'embed_origin')).toHaveLength(1)
  })
})
