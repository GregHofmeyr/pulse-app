// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { BuildLink } from '../lib/protocol/BuildLink'

const ORIGIN = 'https://pulsechat.co.za'
const invoke = vi.fn(async (cmd: string, _args?: unknown): Promise<unknown> => (cmd === 'embed_origin' ? ORIGIN : undefined))
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import BuildVideo from './BuildVideo.svelte'
import { forgetEmbedOrigin, player } from '../lib/linkPreviews.svelte'
import { linkAsEmbed } from '../lib/embeds'
import { flushMicrotasks } from '../lib/testSettle'

const HASH = 'a'.repeat(64)
const yt: BuildLink = { url: 'https://www.youtube.com/watch?v=dQw4w9WgXcQ', title: 'Raven bunny build', thumb: { hash: HASH, width: 480, height: 360 }, provider: 'youtube', provider_id: 'dQw4w9WgXcQ' }
const planner: BuildLink = { url: 'https://planner.example/raven/42', title: 'Raven planner', thumb: null, provider: 'generic', provider_id: null }

let cmp: ReturnType<typeof mount> | undefined
async function show(link: BuildLink, canEnlarge = true) {
  cmp = mount(BuildVideo, { target: document.body, props: { link, messageId: 'M1', canEnlarge } })
  flushSync()
  await flushMicrotasks(6)
}

beforeEach(() => {
  forgetEmbedOrigin()
  invoke.mockClear()
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('linkAsEmbed', () => {
  it('a youtube link is a player; anything else a card', () => {
    expect(linkAsEmbed(yt)).toMatchObject({ url: yt.url, provider: 'youtube', kind: 'player', provider_id: 'dQw4w9WgXcQ', title: 'Raven bunny build' })
    expect(linkAsEmbed(planner)).toMatchObject({ provider: 'generic', kind: 'card', provider_id: null })
  })
})

describe('BuildVideo', () => {
  it('a youtube build plays in place on click', async () => {
    await show(yt)
    expect(document.querySelector('iframe')).toBeNull()
    document.querySelector<HTMLButtonElement>('.build-video .play')!.click()
    await flushMicrotasks(6)
    const frame = document.querySelector('.build-video iframe')!
    expect(frame.getAttribute('src')).toBe(`${ORIGIN}/embed/youtube/dQw4w9WgXcQ`)
    expect(frame.getAttribute('sandbox')).toBeTruthy()
  })

  it('enlarge fills the media area', async () => {
    await show(yt)
    const enlarge = document.querySelector<HTMLButtonElement>('.build-video .enlarge')!
    expect(enlarge.getAttribute('aria-pressed')).toBe('false')
    expect(document.querySelector('.build-video.enlarged')).toBeNull()
    enlarge.click()
    flushSync()
    expect(enlarge.getAttribute('aria-pressed')).toBe('true')
    expect(document.querySelector('.build-video.enlarged')).not.toBeNull()
  })

  // #645 review: its name and tip stay "Enlarge video" (aria-pressed says it's on); without room to fill it isn't shown
  it('Enlarge keeps its name, and is only offered where it can enlarge', async () => {
    await show(yt)
    const enlarge = document.querySelector<HTMLButtonElement>('.build-video .enlarge')!
    enlarge.click()
    flushSync()
    expect(enlarge.getAttribute('aria-label')).toBe('Enlarge video')
    expect(enlarge.getAttribute('data-tip')).toBe('Enlarge video')
    unmount(cmp!)
    cmp = undefined
    document.body.innerHTML = ''
    await show(yt, false)
    expect(document.querySelector('.build-video .enlarge')).toBeNull()
  })

  // #618 testing: the raw link text read messy; a small button opens the video on YouTube instead
  it('opens on YouTube from a small button beside the player, not raw link text', async () => {
    await show(yt)
    const open = document.querySelector<HTMLAnchorElement>('.build-video .under a.open-yt')!
    expect(open.textContent?.replace(/\s+/g, ' ').trim()).toBe('Open on YouTube ↗')
    expect(open.getAttribute('href')).toBe(yt.url)
    expect(open.getAttribute('target')).toBe('_blank')
    expect(open.getAttribute('aria-label')).toBe('Open on YouTube (opens in your browser)')
    expect(open.querySelector('[aria-hidden=true]')?.textContent).toBe('↗')
    // the play affordance stays the big one; the title is plain, ellipsised text with its full text on hover
    expect(document.querySelector('.build-video .under a.title')).toBeNull()
    const t = document.querySelector<HTMLElement>('.build-video .under .yt-title')!
    expect(t.textContent).toBe('Raven bunny build')
    expect(t.getAttribute('data-tip')).toBe('Raven bunny build')
  })

  it('a link with no title shows only the button, never the raw url', async () => {
    await show({ ...yt, title: null })
    expect(document.querySelector('.build-video .under .yt-title')).toBeNull()
    expect(document.querySelector('.build-video')!.textContent).not.toContain('youtube.com')
  })

  // #618 testing: Enlarge lives on the player's top-right, as a video player's controls do, and Tab reaches it
  it('enlarge is an icon button on the player, reachable by keyboard', async () => {
    await show(yt)
    const enlarge = document.querySelector<HTMLButtonElement>('.build-video .screen .corner button.enlarge')!
    expect(enlarge.getAttribute('aria-label')).toBe('Enlarge video')
    expect(enlarge.hasAttribute('tabindex')).toBe(false)
    expect(enlarge.disabled).toBe(false)
    enlarge.focus()
    expect(document.activeElement).toBe(enlarge)
    enlarge.click()
    flushSync()
    // a toggle: its name and tooltip stay, its pressed state (and icon) say which way it goes (#645 review)
    expect(enlarge.getAttribute('aria-label')).toBe('Enlarge video')
    expect(enlarge.getAttribute('aria-pressed')).toBe('true')
    expect(enlarge.getAttribute('data-tip')).toBe('Enlarge video')
    // nothing of it is left under the player
    expect(document.querySelector('.build-video .under .enlarge')).toBeNull()
  })

  it('while playing, Close player sits beside enlarge on the player', async () => {
    await show(yt)
    expect(document.querySelector('.build-video .corner .stop')).toBeNull()
    document.querySelector<HTMLButtonElement>('.build-video .play')!.click()
    await flushMicrotasks(6)
    const close = document.querySelector<HTMLButtonElement>('.build-video .screen .corner button.stop')!
    expect(close.getAttribute('aria-label')).toBe('Close player')
    close.click()
    await flushMicrotasks(6)
    expect(document.querySelector('.build-video iframe')).toBeNull()
  })

  it('leaving the page stops the player', async () => {
    await show(yt)
    document.querySelector<HTMLButtonElement>('.build-video .play')!.click()
    await flushMicrotasks(6)
    expect(player.active).not.toBeNull()
    unmount(cmp!)
    cmp = undefined
    expect(player.active).toBeNull()
  })

  it('a planner link is a card that opens in the browser', async () => {
    await show(planner)
    expect(document.querySelector('.build-video .play')).toBeNull()
    expect(document.querySelector('iframe')).toBeNull()
    const a = document.querySelector<HTMLAnchorElement>('a.link-card')!
    expect(a.getAttribute('href')).toBe(planner.url)
    expect(a.getAttribute('target')).toBe('_blank')
    expect(a.textContent).toContain('Raven planner')
  })
})
