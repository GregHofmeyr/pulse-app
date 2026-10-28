// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Message } from '../lib/protocol/Message'
import { FakeIntersectionObserver, fakeMatchMedia } from '../lib/testMotion'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))
const listeners = new Map<string, (e: { payload: unknown }) => void>()
vi.mock('@tauri-apps/api/event', () => ({
  listen: async (name: string, fn: (e: { payload: unknown }) => void) => {
    listeners.set(name, fn)
    return () => {}
  },
}))
vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)

import ClipsHeader from './ClipsHeader.svelte'
import ClipsChannel from './ClipsChannel.svelte'
import { app, startListening } from '../lib/store.svelte'
import { applyReady, emptyState } from '../lib/state'
import { clipView } from '../lib/clips'
import { reactiveProps } from '../lib/testProps.svelte'
import { testMe, testReady } from '../lib/testReady'

const sam = { id: 'U2', username: 'sam', avatar_hash: null }
const kim = { id: 'U3', username: 'kim', avatar_hash: null }
const ready = (over = {}) =>
  testReady([{ id: 'C1', server_id: 'S1', kind: 'clips', name: 'highlights', position: 0 }], {
    members: [{ server_id: 'S1', members: [{ user: testMe, nickname: null }, { user: sam, nickname: null }, { user: kim, nickname: 'Kimmy' }] }],
    games: [
      { id: 'G1', server_id: 'S1', name: 'Apex', created_by: 'U1', created_at: '2026-10-01T00:00:00Z' },
      { id: 'G2', server_id: 'S1', name: 'Rocket League', created_by: 'U1', created_at: '2026-10-01T00:00:00Z' },
    ],
    ...over,
  })
const facets = { games: [{ game_id: 'G2', count: 4 }, { game_id: 'G1', count: 2 }], authors: [{ user_id: 'U2', count: 5 }, { user_id: 'U3', count: 1 }] }

let cmp: ReturnType<typeof mount> | undefined
const settle = () => new Promise((r) => setTimeout(r, 0))
const calls = (cmd: string) => invoke.mock.calls.filter((c) => c[0] === cmd)
const button = (text: string) => [...document.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent?.trim() === text)!

beforeEach(() => {
  invoke.mockReset()
  invoke.mockImplementation(async (cmd: string) => (cmd === 'list_clips' ? [] : cmd === 'clip_facets' ? facets : undefined))
  app.state = applyReady(emptyState(), ready())
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
  vi.useRealTimers()
})

function header(over: Record<string, unknown> = {}) {
  const p = reactiveProps({
    channelId: 'C1', serverId: 'S1', filter: {} as { game?: string; author?: string }, view: 'wall' as 'list' | 'wall',
    onFilter: vi.fn(), onView: vi.fn(), onShare: vi.fn(), ...over,
  })
  cmp = mount(ClipsHeader, { target: document.body, props: p })
  flushSync()
  return p
}

describe('ClipsHeader', () => {
  it('chips filter by game and person', () => {
    app.state = { ...app.state, clipFacets: { C1: facets } }
    const p = header()
    const chips = [...document.querySelectorAll<HTMLButtonElement>('.chip')].map((b) => b.textContent?.trim())
    expect(chips).toEqual(['All', 'Apex', 'Rocket League', 'By person ▾'])
    expect(button('All').getAttribute('aria-pressed')).toBe('true')
    button('Rocket League').click()
    expect(p.onFilter).toHaveBeenLastCalledWith({ game: 'G2' })
    // the filter comes back as a prop
    p.filter = { game: 'G2' }
    flushSync()
    expect(button('Rocket League').getAttribute('aria-pressed')).toBe('true')
    expect(button('All').getAttribute('aria-pressed')).toBe('false')
    // a person, with their clip counts
    button('By person ▾').click()
    flushSync()
    const people = [...document.querySelectorAll<HTMLElement>('[role="menuitemradio"]')]
    expect(people.map((m) => m.textContent?.replace(/\s+/g, ' ').trim())).toEqual(['Everyone', 'sam 5', 'Kimmy 1'])
    people[2].click()
    flushSync()
    expect(p.onFilter).toHaveBeenLastCalledWith({ game: 'G2', author: 'U3' })
    expect(document.querySelector('[role="menu"]')).toBeNull()
    p.filter = { game: 'G2', author: 'U3' }
    flushSync()
    expect(button('Kimmy ▾').getAttribute('aria-pressed')).toBe('true')
    // All clears both
    button('All').click()
    expect(p.onFilter).toHaveBeenLastCalledWith({})
  })

  it('the toggle is remembered via set_clip_view', async () => {
    const p = header()
    const list = document.querySelector<HTMLButtonElement>('button[aria-label="List view"]')!
    const wall = document.querySelector<HTMLButtonElement>('button[aria-label="Wall view"]')!
    expect(wall.getAttribute('aria-pressed')).toBe('true')
    expect(list.getAttribute('aria-pressed')).toBe('false')
    list.click()
    expect(clipView(app.state, 'C1')).toBe('list') // optimistic
    expect(calls('set_clip_view').map((c) => c[1])).toEqual([{ channelId: 'C1', view: 'list' }])
    expect(p.onView).toHaveBeenCalledWith('list')
    // a refusal puts it back
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'set_clip_view') throw 'nope'
      return undefined
    })
    p.view = 'list'
    flushSync()
    wall.click()
    expect(clipView(app.state, 'C1')).toBe('wall')
    await settle()
    expect(clipView(app.state, 'C1')).toBe('list')
  })

  it('"Share a clip" calls onShare', () => {
    const p = header()
    button('Share a clip').click()
    expect(p.onShare).toHaveBeenCalled()
  })
})

describe('ClipsChannel', () => {
  const clip = (id: string): Message => ({
    id, channel_id: 'C1', author_id: 'U2', kind: 'normal', content: '', reply_to_id: null, created_at: '2026-10-01T10:00:00.000Z',
    edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null, quote: null, qoty_card: null,
    clip: { thumb_hash: 'a'.repeat(64), duration_ms: 5000, width: 1280, height: 720, size: 1, game_id: 'G1', reply_count: 0, last_reply_id: null, replies_seen: null },
  })

  it('a new clips channel opens in the wall', async () => {
    invoke.mockImplementation(async (cmd: string) => (cmd === 'list_clips' ? [clip('M2'), clip('M1')] : cmd === 'clip_facets' ? facets : undefined))
    cmp = mount(ClipsChannel, { target: document.body, props: { channelId: 'C1', serverId: 'S1' } })
    flushSync()
    await settle()
    flushSync()
    expect(document.body.textContent).toContain('highlights')
    expect(document.querySelector('button[aria-label="Wall view"]')?.getAttribute('aria-pressed')).toBe('true')
    expect(document.querySelector('.grid')).not.toBeNull()
    expect(calls('clip_facets').map((c) => c[1])).toEqual([{ channelId: 'C1' }])
    expect(button('Apex')).toBeTruthy() // chips from the loaded facets
  })

  it('opens in the view you chose, and filtering swaps the feed', async () => {
    app.state = applyReady(emptyState(), ready({ clip_views: [{ channel_id: 'C1', view: 'list' }] }))
    cmp = mount(ClipsChannel, { target: document.body, props: { channelId: 'C1', serverId: 'S1' } })
    flushSync()
    await settle()
    flushSync()
    expect(document.querySelector('.grid')).toBeNull()
    expect(document.querySelector('.feed.list')).not.toBeNull()
    button('Apex').click()
    flushSync()
    await settle()
    expect(calls('list_clips').at(-1)![1]).toEqual({ channelId: 'C1', before: null, game: 'G1', author: null })
  })

  it('reloads the chips a second after clip events', async () => {
    vi.useFakeTimers()
    cmp = mount(ClipsChannel, { target: document.body, props: { channelId: 'C1', serverId: 'S1' } })
    flushSync()
    await vi.advanceTimersByTimeAsync(0)
    expect(calls('clip_facets')).toHaveLength(1)
    await startListening()
    for (const id of ['M3', 'M4']) listeners.get('pulse://event')!({ payload: { t: 'MessageCreated', d: { message: clip(id), nonce: null } } })
    // another channel's clip, or a reply, isn't a change to these chips
    listeners.get('pulse://event')!({ payload: { t: 'MessageCreated', d: { message: { ...clip('M5'), channel_id: 'C9' }, nonce: null } } })
    await vi.advanceTimersByTimeAsync(900)
    expect(calls('clip_facets')).toHaveLength(1)
    await vi.advanceTimersByTimeAsync(200)
    expect(calls('clip_facets')).toHaveLength(2) // once for both
  })

  it('an older chips response never replaces a newer one', async () => {
    vi.useFakeTimers()
    const later = { games: [{ game_id: 'G1', count: 9 }], authors: [] }
    let first: ((f: unknown) => void) | null = null
    let n = 0
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'list_clips') return []
      if (cmd === 'clip_facets') return ++n === 1 ? new Promise((r) => (first = r)) : later
      return undefined
    })
    cmp = mount(ClipsChannel, { target: document.body, props: { channelId: 'C1', serverId: 'S1' } })
    flushSync()
    await vi.advanceTimersByTimeAsync(0)
    await startListening()
    listeners.get('pulse://event')!({ payload: { t: 'MessageCreated', d: { message: clip('M3'), nonce: null } } })
    await vi.advanceTimersByTimeAsync(1100)
    expect(calls('clip_facets')).toHaveLength(2)
    expect(app.state.clipFacets.C1).toEqual(later)
    first!(facets) // the on-open load answers last
    await vi.advanceTimersByTimeAsync(0)
    expect(app.state.clipFacets.C1).toEqual(later)
  })

  it('opens a clip\'s replies in the side panel with the clip pinned', async () => {
    Element.prototype.scrollTo = () => {} // jsdom has none
    const replied = { ...clip('M1'), clip: { ...clip('M1').clip!, reply_count: 2 } }
    invoke.mockImplementation(async (cmd: string) => (cmd === 'list_clips' ? [replied] : cmd === 'clip_facets' ? facets : cmd === 'list_replies' ? [] : undefined))
    app.state = applyReady(emptyState(), ready({ clip_views: [{ channel_id: 'C1', view: 'list' }] }))
    cmp = mount(ClipsChannel, { target: document.body, props: { channelId: 'C1', serverId: 'S1' } })
    flushSync()
    await settle()
    flushSync()
    button('View 2 replies').click()
    flushSync()
    await settle()
    const panel = document.querySelector<HTMLElement>('.replies-panel')!
    expect(panel).not.toBeNull()
    expect(panel.querySelector('.pinned [data-clip-id="M1"]')).not.toBeNull()
    expect(panel.querySelector('.pinned .replies')).toBeNull() // no replies link on the pinned clip
  })

  it('a delete prompt focuses its own Cancel when the clip is also pinned in its panel', async () => {
    Element.prototype.scrollTo = () => {} // jsdom has none
    const mine = { ...clip('M1'), author_id: 'U1', clip: { ...clip('M1').clip!, reply_count: 1 } }
    invoke.mockImplementation(async (cmd: string) => (cmd === 'list_clips' ? [mine] : cmd === 'clip_facets' ? facets : cmd === 'list_replies' ? [] : undefined))
    app.state = applyReady(emptyState(), ready({ clip_views: [{ channel_id: 'C1', view: 'list' }] }))
    cmp = mount(ClipsChannel, { target: document.body, props: { channelId: 'C1', serverId: 'S1' } })
    flushSync()
    await settle()
    flushSync()
    button('View 1 reply').click()
    flushSync()
    await settle()
    const askIn = async (root: Element) => {
      root.querySelector<HTMLButtonElement>('button[aria-label="Clip actions"]')!.click()
      flushSync()
      ;[...root.querySelectorAll<HTMLElement>('[role="menuitem"]')].find((b) => b.textContent?.trim() === 'Delete')!.click()
      flushSync()
      await settle()
      return root.querySelector<HTMLElement>('[role="alertdialog"] .keep')!
    }
    await askIn(document.querySelector('.feed [data-clip-id="M1"]')!)
    const panelCancel = await askIn(document.querySelector('.replies-panel [data-clip-id="M1"]')!)
    expect(document.activeElement).toBe(panelCancel)
  })
})
