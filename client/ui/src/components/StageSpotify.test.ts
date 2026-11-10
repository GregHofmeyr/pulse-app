// @vitest-environment jsdom
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import StageSpotify from './StageSpotify.svelte'
import { app } from '../lib/store.svelte'
import { applyReady, emptyState } from '../lib/state'
import { reloadStageView, setMode, modeFor } from '../lib/stageShareView.svelte'
import { resetServerClock } from '../lib/serverClock'
import type { Ready } from '../lib/protocol/Ready'
import type { StageShare } from '../lib/protocol/StageShare'

const me = { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const sam = { id: 'U2', username: 'sam', display_name: 'sam', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const ready: Ready = {
  me, servers: [{ id: 'S1', name: 'Main', icon_hash: null }], channels: [{ id: 'V1', server_id: 'S1', kind: 'voice', name: 'Lounge', position: 0 }],
  members: [{ server_id: 'S1', members: [{ user: me, nickname: null }, { user: sam, nickname: null }] }], dm_members: [], voice: [],
  people: [], read_states: [], mutes: [], hidden: [], latest: [], streams: [], custom_emoji: [], frequent_emoji: [], qoty: [], speaker_links: [], thread_seen: [], games: [], clip_views: [], private_nicknames: [], sounds: [], sound_favourites: [], frequent_sounds: [], server_voice: [], status: { status: 'online', dnd_until: null }, appear_offline_to: [], live_notify: [],
}
const HASH = 'b'.repeat(64)
const share = (over: Partial<StageShare> = {}, embed: Partial<StageShare['embed']> = {}): StageShare => ({
  id: 'S-1', channel_id: 'V1', shared_by: 'U2', shared_at: new Date().toISOString(), expires_at: new Date(Date.now() + 3 * 3600_000).toISOString(),
  embed: { url: 'https://open.spotify.com/track/abc', provider: 'spotify', kind: 'player', provider_id: 'track:abc', title: 'Midnight City', description: null, site_name: 'Spotify', author: 'M83', thumb: { hash: HASH, width: 300, height: 300 }, start: null, vertical: false, ...embed },
  ...over,
})

const mounted: ReturnType<typeof mount>[] = []
function show(s: StageShare, place: 'top' | 'corner', inCall = true) {
  const host = document.createElement('div')
  document.body.append(host)
  mounted.push(mount(StageSpotify, { target: host, props: { share: s, serverId: 'S1', inCall, place } }))
  flushSync()
  return host
}
const btn = (host: HTMLElement, label: string) => [...host.querySelectorAll('button, a')].find((b) => b.textContent?.trim() === label || b.getAttribute('aria-label') === label) as HTMLElement | undefined

beforeEach(() => {
  localStorage.clear()
  reloadStageView()
  resetServerClock(null)
  app.state = applyReady(emptyState(), ready)
  invoke.mockClear()
})
afterEach(() => {
  while (mounted.length) unmount(mounted.pop()!)
  document.body.innerHTML = ''
})

describe('the strip above the tiles', () => {
  it('shows the art, title, who shared it, the Spotify icon and their wording', () => {
    const host = show(share(), 'top')
    const strip = host.querySelector('.sp-strip')!
    expect(strip.textContent).toContain('Midnight City')
    expect(strip.textContent).toContain('M83 · Shared by sam')
    expect(strip.querySelector('[aria-label="Spotify"]')).not.toBeNull()
    const open = btn(host, 'Listen on Spotify') as HTMLAnchorElement
    expect(open.getAttribute('href')).toBe('https://open.spotify.com/track/abc')
    expect(open.getAttribute('target')).toBe('_blank')
    // the artwork uncropped, with nothing laid over it
    const img = strip.querySelector('.art img')!
    expect(img.getAttribute('src')).toContain(HASH)
    expect(img.parentElement!.children).toHaveLength(1)
    // the corner part draws nothing while it's the strip
    expect(show(share(), 'corner').children).toHaveLength(0)
  })

  it('a Jam: Join the Jam, and "may have ended" an hour on', () => {
    const old = new Date(Date.now() - 61 * 60_000).toISOString()
    const host = show(share({ shared_at: old }, { provider_id: 'jam:xyz', kind: 'card', title: 'Join the Jam', author: null, thumb: null }), 'top')
    expect(btn(host, 'Join the Jam')).toBeDefined()
    expect(host.textContent).toContain('Spotify Jam')
    expect(host.textContent).toContain('Shared by sam · may have ended')
  })

  it('Stop sharing asks the server to take that share down; only for people in the call', async () => {
    const host = show(share(), 'top')
    btn(host, 'Stop sharing')!.click()
    await tick()
    expect(invoke).toHaveBeenCalledWith('stop_stage_share', { channelId: 'V1', shareId: 'S-1' })
    expect(btn(show(share(), 'top', false), 'Stop sharing')).toBeUndefined()
  })

  it('minimise makes it the pill; ✕ hides it for you', () => {
    const top = show(share(), 'top')
    const corner = show(share(), 'corner')
    btn(top, 'Minimise')!.click()
    flushSync()
    expect(top.querySelector('.sp-strip')).toBeNull()
    expect(corner.querySelector('.pill')).not.toBeNull()
    expect(modeFor('S-1')).toBe('pill')
    setMode('S-1', 'strip')
    flushSync()
    btn(top, 'Hide for me')!.click()
    flushSync()
    expect(top.children).toHaveLength(0)
    expect(corner.children).toHaveLength(0)
  })
})

describe('the pill and the corner card', () => {
  it('hovering the pill previews the corner card; a click opens it', () => {
    setMode('S-1', 'pill')
    const host = show(share(), 'corner')
    const pill = host.querySelector<HTMLElement>('.pill')!
    expect(host.querySelector('.corner-card')).toBeNull()
    pill.dispatchEvent(new PointerEvent('pointerenter'))
    flushSync()
    const peek = host.querySelector('.corner-card')!
    expect(peek.textContent).toContain('Midnight City')
    // a preview has no controls
    expect(btn(host, 'Hide for me')).toBeUndefined()
    pill.click()
    flushSync()
    expect(host.querySelector('.pill')).toBeNull()
    expect(btn(host, 'Hide for me')).toBeDefined()
    expect(btn(host, 'Listen on Spotify')).toBeDefined()
  })

  it('the corner card goes back to the strip, minimises again, or hides', () => {
    setMode('S-1', 'corner')
    const host = show(share(), 'corner')
    btn(host, 'Minimise')!.click()
    flushSync()
    expect(modeFor('S-1')).toBe('pill')
    setMode('S-1', 'corner')
    flushSync()
    btn(host, 'Show above the tiles')!.click()
    flushSync()
    expect(modeFor('S-1')).toBe('strip')
  })
})
