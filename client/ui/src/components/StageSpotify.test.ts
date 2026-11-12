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
import { PILL_CLOSE_GRACE, PILL_OPEN_DELAY } from '../lib/hoverIntent'

/** Icon.svelte's expand (two arrows out to opposite corners). */
const EXPAND_PATH = 'M14 4h6v6M20 4l-7 7M10 20H4v-6M4 20l7-7'
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

describe('keyboard focus follows the view', () => {
  it('minimise hands it to the pill, opening hands it to the card, back to the strip to its Minimise', async () => {
    const top = show(share(), 'top')
    const corner = show(share(), 'corner')
    const settle = async () => { for (let i = 0; i < 3; i++) await tick(); flushSync() }
    btn(top, 'Minimise')!.focus()
    btn(top, 'Minimise')!.click()
    await settle()
    expect(document.activeElement).toBe(corner.querySelector('.pill'))
    ;(document.activeElement as HTMLElement).click()
    await settle()
    expect(corner.querySelector('.corner-card')!.contains(document.activeElement)).toBe(true)
    btn(corner, 'Show above the tiles')!.focus()
    btn(corner, 'Show above the tiles')!.click()
    await settle()
    expect(document.activeElement).toBe(btn(top, 'Minimise'))
  })
})

// #288: the pill itself grows into the corner card, on a hover that means it or a click, and shrinks back.
describe('the pill grows into the corner card', () => {
  const corner = () => {
    setMode('S-1', 'pill')
    return show(share(), 'corner')
  }
  const zone = (host: HTMLElement) => host.querySelector<HTMLElement>('.stage-corner')!
  // as the browser sends them: enter and leave stay on the element, the rest bubble
  const at = (type: string, x: number, y: number) => new PointerEvent(type, { clientX: x, clientY: y, pointerType: 'mouse', bubbles: !/enter|leave/.test(type) })
  const card = (host: HTMLElement) => host.querySelector('.corner-card')
  const advance = (ms: number) => {
    vi.advanceTimersByTime(ms)
    flushSync()
  }
  afterEach(() => vi.useRealTimers())

  it('focusing the window or the pill opens nothing, nor does the pointer just being there (the refocus bug)', () => {
    vi.useFakeTimers()
    const host = corner()
    const pill = host.querySelector<HTMLElement>('.pill')!
    pill.focus()
    window.dispatchEvent(new FocusEvent('focus'))
    // what a window coming back to the front sends with the pointer already over the pill: no real movement
    zone(host).dispatchEvent(at('pointerenter', 50, 20))
    zone(host).dispatchEvent(at('pointermove', 50, 20))
    zone(host).dispatchEvent(at('pointerover', 50, 20))
    advance(2000)
    expect(card(host)).toBeNull()
    expect(document.activeElement).toBe(pill)
    // a move without an enter first (refocus sends one) isn't a hover either
    zone(host).dispatchEvent(at('pointerleave', 0, 0))
    zone(host).dispatchEvent(at('pointermove', 60, 22))
    advance(2000)
    expect(card(host)).toBeNull()
  })

  it('a hover that means it (the pointer moving on the pill, then a short rest) grows it into the card, controls and all', () => {
    vi.useFakeTimers()
    const host = corner()
    zone(host).dispatchEvent(at('pointerenter', 50, 20))
    zone(host).dispatchEvent(at('pointermove', 54, 21))
    advance(PILL_OPEN_DELAY - 50)
    expect(card(host)).toBeNull()
    advance(60)
    expect(card(host)!.textContent).toContain('Midnight City')
    // the pill became the card: one thing, so moving over it never crosses a gap
    expect(host.querySelector('.pill')).toBeNull()
    expect(btn(host, 'Hide for me')).toBeDefined()
    expect(btn(host, 'Listen on Spotify')).toBeDefined()
  })

  it('shrinks back once the pointer has been gone a moment; coming back in time keeps it', () => {
    vi.useFakeTimers()
    const host = corner()
    zone(host).dispatchEvent(at('pointerenter', 50, 20))
    zone(host).dispatchEvent(at('pointermove', 54, 21))
    advance(PILL_OPEN_DELAY)
    zone(host).dispatchEvent(at('pointerleave', 0, 0))
    advance(PILL_CLOSE_GRACE - 100)
    expect(card(host)).not.toBeNull()
    zone(host).dispatchEvent(at('pointerenter', 50, 20))
    advance(PILL_CLOSE_GRACE * 2)
    expect(card(host)).not.toBeNull()
    zone(host).dispatchEvent(at('pointerleave', 0, 0))
    advance(PILL_CLOSE_GRACE)
    expect(card(host)).toBeNull()
    expect(host.querySelector('.pill')).not.toBeNull()
    expect(modeFor('S-1')).toBe('pill')
  })

  it('a press in a card opened by hover keeps it open, as if it had been clicked open', () => {
    vi.useFakeTimers()
    const host = corner()
    zone(host).dispatchEvent(at('pointerenter', 50, 20))
    zone(host).dispatchEvent(at('pointermove', 54, 21))
    advance(PILL_OPEN_DELAY)
    card(host)!.dispatchEvent(at('pointerdown', 60, 30))
    zone(host).dispatchEvent(at('pointerleave', 0, 0))
    advance(PILL_CLOSE_GRACE * 3)
    expect(card(host)).not.toBeNull()
  })

  it('a click opens it until a click outside or Escape; Escape hands the focus back to the pill', async () => {
    vi.useFakeTimers()
    const host = corner()
    host.querySelector<HTMLElement>('.pill')!.click()
    for (let i = 0; i < 3; i++) await tick()
    flushSync()
    expect(card(host)).not.toBeNull()
    expect(card(host)!.contains(document.activeElement)).toBe(true)
    // opened on purpose: the pointer leaving doesn't close it
    zone(host).dispatchEvent(at('pointerleave', 0, 0))
    advance(PILL_CLOSE_GRACE * 3)
    expect(card(host)).not.toBeNull()
    card(host)!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))
    for (let i = 0; i < 3; i++) await tick()
    flushSync()
    expect(card(host)).toBeNull()
    expect(document.activeElement).toBe(host.querySelector('.pill'))
    // Escape with the pointer still there: it needs a new hover to open again
    zone(host).dispatchEvent(at('pointermove', 70, 25))
    advance(PILL_OPEN_DELAY * 3)
    expect(card(host)).toBeNull()

    host.querySelector<HTMLElement>('.pill')!.click()
    for (let i = 0; i < 3; i++) await tick()
    flushSync()
    card(host)!.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }))
    flushSync()
    expect(card(host)).not.toBeNull()
    document.body.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }))
    flushSync()
    expect(card(host)).toBeNull()
  })

  it('the keyboard: Enter on the pill opens it with the focus inside; tabbing out of it closes it', async () => {
    const host = corner()
    const outside = document.createElement('button')
    document.body.append(outside)
    const pill = host.querySelector<HTMLElement>('.pill')!
    expect(pill.getAttribute('aria-expanded')).toBe('false')
    pill.focus()
    pill.click() // what Enter or Space on a button does
    for (let i = 0; i < 3; i++) await tick()
    flushSync()
    const inside = document.activeElement as HTMLElement
    expect(card(host)!.contains(inside)).toBe(true)
    inside.dispatchEvent(new FocusEvent('focusout', { bubbles: true, relatedTarget: outside }))
    flushSync()
    expect(card(host)).toBeNull()
  })

  it('its controls sit inside the card: Show above the tiles (an expand icon), Minimise, Hide for me, then the Spotify button and Stop sharing', async () => {
    const host = corner()
    host.querySelector<HTMLElement>('.pill')!.click()
    for (let i = 0; i < 3; i++) await tick()
    flushSync()
    const c = card(host)!
    for (const l of ['Show above the tiles', 'Minimise', 'Hide for me', 'Listen on Spotify', 'Stop sharing']) expect(c.contains(btn(host, l)!), l).toBe(true)
    const expand = btn(host, 'Show above the tiles')!
    expect(expand.classList.contains('icon-btn')).toBe(true)
    expect(expand.getAttribute('data-tip')).toBe('Show above the tiles')
    expect(expand.querySelector('svg path')!.getAttribute('d')).toBe(EXPAND_PATH)
    // Minimise shrinks it into the pill (focus there); the expand goes back to the strip
    btn(host, 'Minimise')!.focus()
    btn(host, 'Minimise')!.click()
    for (let i = 0; i < 3; i++) await tick()
    flushSync()
    expect(card(host)).toBeNull()
    expect(document.activeElement).toBe(host.querySelector('.pill'))
    expect(modeFor('S-1')).toBe('pill')
    host.querySelector<HTMLElement>('.pill')!.click()
    for (let i = 0; i < 3; i++) await tick()
    flushSync()
    btn(host, 'Show above the tiles')!.click()
    flushSync()
    expect(modeFor('S-1')).toBe('strip')
    setMode('S-1', 'pill')
    flushSync()
    host.querySelector<HTMLElement>('.pill')!.click()
    for (let i = 0; i < 3; i++) await tick()
    flushSync()
    btn(host, 'Hide for me')!.click()
    flushSync()
    expect(modeFor('S-1')).toBe('hidden')
  })
})

describe("a Jam's Now playing (#275)", () => {
  const NOW = 'c'.repeat(64)
  const jam = (now_playing?: StageShare['now_playing']) =>
    share({ now_playing }, { url: 'https://open.spotify.com/socialsession/abcdefgh', kind: 'card', provider_id: 'jam:abcdefgh', title: null, author: null })

  it("the strip shows the sharer's song under the Jam, with its art; without one, the Jam's own", () => {
    const host = show(jam({ title: 'Live song', artist: 'The Band', album: null, spotify_id: null, art: { hash: NOW, width: 640, height: 640 } }), 'top')
    const strip = host.querySelector('.sp-strip')!
    expect(strip.querySelector('.sub.now')!.textContent).toBe('Now playing · Live song · The Band')
    expect(strip.querySelector('.art img')!.getAttribute('src')).toContain(NOW)
    // still the Jam's own button
    expect(btn(host, 'Join the Jam')).toBeDefined()
    const plain = show(jam(), 'top')
    expect(plain.querySelector('.sub.now')).toBeNull()
    expect(plain.querySelector('.art img')!.getAttribute('src')).toContain(HASH)
  })

  it('the pill names the song playing', () => {
    setMode('S-1', 'pill')
    const host = show(jam({ title: 'Live song', artist: null, album: null, spotify_id: null, art: null }), 'corner')
    expect(host.querySelector('.pill-title')!.textContent).toBe('Live song')
  })
})
