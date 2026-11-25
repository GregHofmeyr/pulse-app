// @vitest-environment jsdom
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { BuildLink } from '../lib/protocol/BuildLink'

const ORIGIN = 'https://pulsechat.co.za'
const invoke = vi.fn(async (cmd: string, _args?: unknown): Promise<unknown> => (cmd === 'embed_origin' ? ORIGIN : undefined))
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import BuildVideo from './BuildVideo.svelte'
import { forgetEmbedOrigin, player } from '../lib/linkPreviews.svelte'
import { linkAsEmbed } from '../lib/embeds'

const HASH = 'a'.repeat(64)
const yt: BuildLink = { url: 'https://www.youtube.com/watch?v=dQw4w9WgXcQ', title: 'Raven bunny build', thumb: { hash: HASH, width: 480, height: 360 }, provider: 'youtube', provider_id: 'dQw4w9WgXcQ' }
const planner: BuildLink = { url: 'https://planner.example/raven/42', title: 'Raven planner', thumb: null, provider: 'generic', provider_id: null }

let cmp: ReturnType<typeof mount> | undefined
async function settle() { for (let i = 0; i < 5; i++) await Promise.resolve(); await tick(); flushSync() }
async function show(link: BuildLink) {
  cmp = mount(BuildVideo, { target: document.body, props: { link, messageId: 'M1' } })
  flushSync()
  await settle()
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
    await settle()
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

  it('leaving the page stops the player', async () => {
    await show(yt)
    document.querySelector<HTMLButtonElement>('.build-video .play')!.click()
    await settle()
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
