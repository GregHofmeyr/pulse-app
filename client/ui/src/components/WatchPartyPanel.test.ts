// @vitest-environment jsdom
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const ORIGIN = 'https://pulsechat.co.za'
const invoke = vi.fn(async (cmd: string, _args?: unknown): Promise<unknown> => (cmd === 'embed_origin' ? ORIGIN : undefined))
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))
const handlers = vi.hoisted(() => ({} as Record<string, (e: { payload: unknown }) => void>))
vi.mock('@tauri-apps/api/event', () => ({
  listen: async (name: string, cb: (e: { payload: unknown }) => void) => {
    handlers[name] = cb
    return () => {}
  },
}))

import WatchPartyPanel from './WatchPartyPanel.svelte'
import { app, startListening } from '../lib/store.svelte'
import { applyReady, emptyState } from '../lib/state'
import { forgetEmbedOrigin } from '../lib/linkPreviews.svelte'
import { resetServerClock } from '../lib/serverClock'
import { setWatchParties, watchPrefs } from '../lib/watchParty.svelte'
import type { Ready } from '../lib/protocol/Ready'
import type { WatchParty } from '../lib/protocol/WatchParty'

const user = (id: string, name: string) => ({ id, username: name, display_name: name, avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } })
const alex = user('U1', 'alex')
const sam = user('U2', 'sam')
const ready: Ready = {
  me: alex, servers: [{ id: 'S1', name: 'Main', icon_hash: null }], channels: [{ id: 'V1', server_id: 'S1', kind: 'voice', name: 'Lounge', position: 0 }],
  members: [{ server_id: 'S1', members: [{ user: alex, nickname: null }, { user: sam, nickname: null }] }], dm_members: [], voice: [],
  people: [], read_states: [], mutes: [], hidden: [], latest: [], streams: [], custom_emoji: [], frequent_emoji: [], qoty: [], speaker_links: [], thread_seen: [], games: [], clip_views: [], private_nicknames: [], sounds: [], sound_favourites: [], frequent_sounds: [], server_voice: [], status: { status: 'online', dnd_until: null }, appear_offline_to: [], live_notify: [],
}
const party = (over: Partial<WatchParty> = {}): WatchParty => ({
  channel_id: 'V1', host_id: 'U2', video_id: 'dQw4w9WgXcQ', title: 'A video', position_ms: 0, playing: true, set_at_ms: Date.now(),
  everyone_controls: false, viewers: ['U2'], queue: [], ...over,
})

const mounted: ReturnType<typeof mount>[] = []
function show(p: WatchParty, inCall = true) {
  const host = document.createElement('div')
  document.body.append(host)
  mounted.push(mount(WatchPartyPanel, { target: host, props: { party: p, serverId: 'S1', inCall } }))
  flushSync()
  return host
}
async function settle() { for (let i = 0; i < 5; i++) await Promise.resolve(); await tick(); flushSync() }
const btn = (host: HTMLElement, label: string) => [...host.querySelectorAll('button')].find((b) => b.textContent?.trim() === label || b.getAttribute('aria-label') === label)
const actions = () => invoke.mock.calls.filter(([c]) => c === 'watch_party_action').map(([, a]) => (a as { action: unknown }).action)

beforeEach(() => {
  localStorage.clear()
  forgetEmbedOrigin()
  resetServerClock(null)
  app.state = applyReady(emptyState(), ready)
  watchPrefs.on = true
  invoke.mockClear()
})
afterEach(() => {
  while (mounted.length) unmount(mounted.pop()!)
  document.body.innerHTML = ''
})

describe('the watch party panel (#384)', () => {
  it('outside it: who hosts it and Join, which opens it without leaving voice', async () => {
    const host = show(party())
    expect(host.textContent).toContain('A video')
    expect(host.textContent).toContain('sam is hosting')
    expect(host.querySelector('iframe')).toBeNull()
    btn(host, 'Join watch party')!.click()
    await settle()
    expect(actions()).toEqual([{ action: 'join' }])
  })

  it('out of the call: no Join, a hint instead', () => {
    const host = show(party(), false)
    expect(btn(host, 'Join watch party')).toBeUndefined()
    expect(host.textContent).toContain('Join voice to watch')
  })

  it("watching: the party page's player, from where the party is, and Leave", async () => {
    const host = show(party({ viewers: ['U2', 'U1'], position_ms: 30_000, playing: false }))
    await settle()
    const f = host.querySelector('iframe')!
    expect(f.getAttribute('src')).toBe(`${ORIGIN}/embed/youtube/dQw4w9WgXcQ?party=1&start=30`)
    expect(f.getAttribute('sandbox')).toBe('allow-scripts allow-same-origin allow-presentation')
    expect(f.getAttribute('referrerpolicy')).toBe('strict-origin-when-cross-origin')
    // not the host: the controls say who has them
    const play = btn(host, 'Play')!
    expect(play.getAttribute('aria-disabled')).toBe('true')
    play.click()
    expect((host.querySelector('input.seek') as HTMLInputElement).disabled).toBe(true)
    btn(host, 'Leave watch party')!.click()
    await settle()
    expect(actions()).toEqual([{ action: 'leave' }])
  })

  it('the host plays and pauses for everyone, lets everyone control and ends it', async () => {
    const host = show(party({ host_id: 'U1', viewers: ['U1', 'U2'], playing: false, position_ms: 5_000 }))
    await settle()
    expect(host.textContent).toContain('you’re hosting')
    btn(host, 'Play')!.click()
    await settle()
    btn(host, 'Let everyone control')!.click()
    await settle()
    btn(host, 'End')!.click()
    await settle()
    expect(actions()).toEqual([{ action: 'play', position_ms: 5_000 }, { action: 'everyone_controls', on: true }])
    expect(invoke).toHaveBeenCalledWith('end_watch_party', { channelId: 'V1' })
  })

  it('drives the player to the party, only from its own frame and our origin', async () => {
    const host = show(party({ viewers: ['U2', 'U1'], position_ms: 60_000, playing: true, set_at_ms: Date.now() }))
    await settle()
    const f = host.querySelector('iframe')!
    const posted: [unknown, string][] = []
    Object.defineProperty(f, 'contentWindow', { value: { postMessage: (m: unknown, o: string) => posted.push([m, o]) } })
    const say = (data: unknown, source: unknown = f.contentWindow, origin = ORIGIN) =>
      window.dispatchEvent(new MessageEvent('message', { data, origin, source: source as Window }))
    // another origin, or another frame: ignored
    say({ pulse: 'party', state: 'info', player: 2, time: 0 }, f.contentWindow, 'https://evil.test')
    say({ pulse: 'party', state: 'info', player: 2, time: 0 }, window)
    expect(posted).toEqual([])
    // the player is paused at 0 while the party plays at a minute: play and seek
    say({ pulse: 'party', state: 'info', player: 2, time: 0, duration: 300 })
    await new Promise((r) => setTimeout(r, 1100))
    const cmds = posted.map(([m]) => m as { func: string; to?: number })
    expect(cmds[0]).toEqual({ pulse: 'party-cmd', func: 'volume', to: 100 })
    expect(cmds.some((c) => c.func === 'play')).toBe(true)
    const seek = cmds.find((c) => c.func === 'seek')!
    expect(seek.to).toBeGreaterThanOrEqual(60)
    expect(seek.to).toBeLessThan(63)
    expect(posted.every(([, o]) => o === ORIGIN)).toBe(true)
  })

  /** The panel mounted the way the voice panel does: its party follows the store. */
  function showLive(p: WatchParty) {
    app.state = { ...app.state, watchParties: { V1: p } }
    const host = document.createElement('div')
    document.body.append(host)
    mounted.push(mount(WatchPartyPanel, { target: host, props: { get party() { return app.state.watchParties.V1 }, serverId: 'S1', inCall: true } }))
    flushSync()
    return host
  }
  const change = (over: Partial<WatchParty>) => {
    app.state = { ...app.state, watchParties: { V1: { ...app.state.watchParties.V1, ...over } } }
    flushSync()
  }
  function wire(f: HTMLIFrameElement) {
    const posted: { func: string; to?: number }[] = []
    Object.defineProperty(f, 'contentWindow', { value: { postMessage: (m: { func: string; to?: number }) => posted.push(m) } })
    const say = (data: object) => window.dispatchEvent(new MessageEvent('message', { data: { pulse: 'party', state: 'info', ...data }, origin: ORIGIN, source: f.contentWindow }))
    return { posted, say }
  }

  it('a pause keeps the same player (no reload, which autoplays from the top of the page) and it stays paused', async () => {
    const host = showLive(party({ host_id: 'U1', viewers: ['U1', 'U2'], position_ms: 0, playing: true, set_at_ms: Date.now() - 5_000 }))
    await settle()
    const f = host.querySelector('iframe')!
    const src = f.getAttribute('src')
    expect(src).toContain('start=5')
    const { posted, say } = wire(f)
    // it's played on a few seconds since the page loaded
    say({ player: 1, time: 9.3, duration: 300 })
    // the pause comes back from the server (to the pauser too)
    change({ playing: false, position_ms: 9_400, set_at_ms: Date.now() })
    await settle()
    expect(host.querySelector('iframe')).toBe(f)
    expect(f.getAttribute('src')).toBe(src)
    expect(posted.filter((c) => c.func !== 'volume').map((c) => c.func)).toEqual(['pause'])
    // the player says it paused: nothing more is sent, ever (no play to fight it)
    say({ player: 2, time: 9.4 })
    posted.length = 0
    await new Promise((r) => setTimeout(r, 1100))
    expect(posted).toEqual([])
    // resumed: the same player plays on from where it paused
    change({ playing: true, position_ms: 9_400, set_at_ms: Date.now() })
    await settle()
    expect(host.querySelector('iframe')).toBe(f)
    expect(posted.map((c) => c.func)).toEqual(['play'])
  })

  it('a late joiner to a paused party gets a paused player at the paused moment', async () => {
    const host = showLive(party({ viewers: ['U2', 'U1'], position_ms: 95_000, playing: false, set_at_ms: Date.now() - 600_000 }))
    await settle()
    const f = host.querySelector('iframe')!
    expect(f.getAttribute('src')).toBe(`${ORIGIN}/embed/youtube/dQw4w9WgXcQ?party=1&start=95`)
    const { posted, say } = wire(f)
    // the page autoplays: the party says paused, so it's paused there, and seeked to the moment
    say({ player: 1, time: 95.2, duration: 300 })
    await new Promise((r) => setTimeout(r, 1100))
    expect(posted.filter((c) => c.func !== 'volume').map((c) => c.func)).toEqual(['pause'])
    expect(host.querySelector('[aria-label="Play"]')).not.toBeNull()
  })

  it('a click on the video is the panel’s, not YouTube’s: the host’s click pauses for everyone', async () => {
    const host = showLive(party({ host_id: 'U1', viewers: ['U1', 'U2'], playing: true, set_at_ms: Date.now() - 10_000 }))
    await settle()
    const shield = host.querySelector<HTMLElement>('.player .shield')!
    expect(shield).not.toBeNull()
    shield.click()
    await settle()
    expect(actions()).toEqual([{ action: 'pause', position_ms: expect.any(Number) }])
  })

  it('a click on the video by someone without control does nothing', async () => {
    const host = showLive(party({ viewers: ['U2', 'U1'], playing: true }))
    await settle()
    host.querySelector<HTMLElement>('.player .shield')!.click()
    await settle()
    expect(actions()).toEqual([])
  })

  it('the host’s player pausing on its own (a media key, the system’s media controls) pauses the party instead of being fought', async () => {
    const host = showLive(party({ host_id: 'U1', viewers: ['U1', 'U2'], playing: true, position_ms: 0, set_at_ms: Date.now() - 20_000 }))
    await settle()
    const { posted, say } = wire(host.querySelector('iframe')!)
    say({ player: 1, time: 20, duration: 300 })
    say({ player: 2, time: 20.1 })
    await settle()
    expect(actions()).toEqual([{ action: 'pause', position_ms: 20_100 }])
    expect(posted.some((c) => c.func === 'play')).toBe(false)
  })

  it('YouTube refusing to play framed offers the window instead', async () => {
    const host = show(party({ viewers: ['U2', 'U1'] }))
    await settle()
    const f = host.querySelector('iframe')!
    window.dispatchEvent(new MessageEvent('message', { data: { pulse: 'party', state: 'error', code: 153 }, origin: ORIGIN, source: f.contentWindow }))
    flushSync()
    expect(host.querySelector('iframe')).toBeNull()
    btn(host, 'Open in a window')!.click()
    expect(invoke.mock.calls.some(([c]) => c === 'open_embed_window')).toBe(true)
  })

  it('lists every queued video, not just the first few (#409)', async () => {
    const queue = Array.from({ length: 8 }, (_, i) => ({ video_id: `video${i}abcd`, added_by: 'U2' }))
    const host = show(party({ host_id: 'U1', viewers: ['U1'], queue }))
    await settle()
    const rows = host.querySelectorAll('.queue li')
    expect(rows.length).toBe(8)
    expect(rows[7].textContent).toContain('video7abcd')
    expect(host.textContent).not.toContain('more')
    const removes = host.querySelectorAll<HTMLButtonElement>('[aria-label="Remove from queue"]')
    removes[7].click()
    expect(actions()).toEqual([{ action: 'unqueue', index: 7 }])
  })

  it('the host’s player moves the party on when the video ends', async () => {
    const host = show(party({ host_id: 'U1', viewers: ['U1'], queue: [{ video_id: 'aaaaaaaaaaa', added_by: 'U2' }] }))
    await settle()
    expect(host.textContent).toContain('aaaaaaaaaaa')
    const f = host.querySelector('iframe')!
    window.dispatchEvent(new MessageEvent('message', { data: { pulse: 'party', state: 'info', player: 0 }, origin: ORIGIN, source: f.contentWindow }))
    window.dispatchEvent(new MessageEvent('message', { data: { pulse: 'party', state: 'info', player: 0 }, origin: ORIGIN, source: f.contentWindow }))
    await settle()
    expect(actions()).toEqual([{ action: 'next' }])
  })
})

describe('the Watch parties setting (#384)', () => {
  it('is on by default, and off leaves a party you are in and declines one handed to you', async () => {
    expect(watchPrefs.on).toBe(true)
    app.state = { ...app.state, watchParties: { V1: party({ viewers: ['U2', 'U1'] }) } }
    setWatchParties(false)
    expect(localStorage.getItem('pulse.watchParties')).toBe('off')
    await settle()
    expect(actions()).toEqual([{ action: 'leave' }])
    invoke.mockClear()
    await startListening()
    handlers['pulse://event']({ payload: { t: 'WatchPartyChanged', d: { party: party({ host_id: 'U1', viewers: ['U2', 'U1'] }), server_now_ms: Date.now() } } })
    await settle()
    expect(actions()).toEqual([{ action: 'leave' }])
    setWatchParties(true)
  })
})
