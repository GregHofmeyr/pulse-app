// @vitest-environment jsdom
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { compile } from 'svelte/compiler'
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
import { clipPanelOpen } from '../lib/clipPrefs'
import { reactiveProps } from '../lib/testProps.svelte'
import { testMe, testReady } from '../lib/testReady'

const sam = { id: 'U2', username: 'sam', display_name: 'sam', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const kim = { id: 'U3', username: 'kim', display_name: 'kim', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const ready = (over = {}) =>
  testReady([{ id: 'C1', server_id: 'S1', kind: 'clips', name: 'highlights', position: 0 }], {
    members: [{ server_id: 'S1', members: [{ user: testMe, nickname: null }, { user: sam, nickname: null }, { user: kim, nickname: 'Kimmy' }] }],
    games: [
      { id: 'G1', server_id: 'S1', name: 'Apex', created_by: 'U1', created_at: '2026-10-01T00:00:00Z' },
      { id: 'G2', server_id: 'S1', name: 'Rocket League', created_by: 'U1', created_at: '2026-10-01T00:00:00Z' },
    ],
    ...over,
  })
const facets = { games: [{ game_id: 'G2', count: 4 }, { game_id: 'G1', count: 2 }], authors: [{ user_id: 'U2', count: 5 }, { user_id: 'U3', count: 1 }], total: 7 }

let cmp: ReturnType<typeof mount> | undefined
const settle = () => new Promise((r) => setTimeout(r, 0))
const calls = (cmd: string) => invoke.mock.calls.filter((c) => c[0] === cmd)
/** A row of the clips panel's `games` or `people` list, by its name. */
const row = (list: string, name: string) =>
  [...document.querySelectorAll<HTMLButtonElement>(`.clips-panel .${list} button`)].find((b) => b.querySelector('.name')?.textContent === name)!
const rows = (list: string) =>
  [...document.querySelectorAll<HTMLButtonElement>(`.clips-panel .${list} button`)].map((b) => [b.querySelector('.name')?.textContent, b.querySelector('.n')?.textContent])
const button = (text: string) => [...document.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent?.trim() === text)!

beforeEach(() => {
  invoke.mockReset()
  invoke.mockImplementation(async (cmd: string) => (cmd === 'list_clips' ? [] : cmd === 'clip_facets' ? facets : undefined))
  app.state = applyReady(emptyState(), ready())
  localStorage.clear() // the per-device panel choice: each test starts with it open
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
    onFilter: vi.fn(), onView: vi.fn(), onShare: vi.fn(), panel: true, onPanel: vi.fn(), panelId: 'clips-panel-C1', ...over,
  })
  cmp = mount(ClipsHeader, { target: document.body, props: p })
  flushSync()
  return p
}

describe('ClipsHeader', () => {
  it('no filter pills in the header any more (#663): the title is the channel name, not a toggle', () => {
    app.state = { ...app.state, clipFacets: { C1: facets } }
    header()
    expect(document.querySelectorAll('.chip, .toggle-chip').length).toBe(0)
    expect(document.querySelector('button.title')).toBeNull()
    expect(document.querySelector('h2')?.textContent).toBe('highlights')
  })

  it('the panel toggle sits at the right end, says whether the panel is open and which one it opens', () => {
    const p = header()
    const t = document.querySelector<HTMLButtonElement>('button[aria-controls="clips-panel-C1"]')!
    expect(t.getAttribute('aria-expanded')).toBe('true')
    expect(t.getAttribute('aria-label')).toBe('Hide filters panel')
    const head = document.querySelector('.head')!
    expect(head.lastElementChild).toBe(t) // rightmost, as in Builds
    t.click()
    expect(p.onPanel).toHaveBeenCalledTimes(1)
    p.panel = false
    flushSync()
    expect(t.getAttribute('aria-expanded')).toBe('false')
    expect(t.getAttribute('aria-label')).toBe('Show filters panel')
  })

  it('with the panel closed, a filter that is on still shows (so a filtered list never looks like everything)', () => {
    app.state = { ...app.state, clipFacets: { C1: facets } }
    const p = header({ panel: false, filter: { game: 'G2', author: 'U3' } })
    expect(document.querySelector('.filtering .what')?.textContent).toBe('Rocket League · Kimmy')
    document.querySelector<HTMLButtonElement>('button[aria-label="Clear filters"]')!.click()
    expect(p.onFilter).toHaveBeenLastCalledWith({})
    // open, the panel says it instead
    p.panel = true
    flushSync()
    expect(document.querySelector('.filtering')).toBeNull()
    // closed with no filter: nothing
    p.panel = false
    p.filter = {}
    flushSync()
    expect(document.querySelector('.filtering')).toBeNull()
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

  it('"Share a clip" is the accent button the rest of the app uses, not a native one', () => {
    header()
    const share = button('Share a clip')
    // the shared classes (#173: it had its own copy, without their hover)
    expect(share.classList.contains('btn')).toBe(true)
    expect(share.classList.contains('btn-primary')).toBe(true)
    const hash = [...share.classList].find((c) => c.startsWith('svelte-'))!
    const src = readFileSync(resolve(__dirname, 'ClipsHeader.svelte'), 'utf8')
    const css = compile(src, { filename: 'ClipsHeader.svelte', css: 'external', cssHash: () => hash }).css!.code
    const rule = css.match(/\.share\.[\w-]+\s*\{([^}]*)\}/)?.[1] ?? ''
    expect(rule).not.toMatch(/background|border-radius/)
  })
})

describe('ClipsChannel', () => {
  const clip = (id: string): Message => ({
    id, channel_id: 'C1', author_id: 'U2', kind: 'normal', content: '', reply_to_id: null, created_at: '2026-10-01T10:00:00.000Z',
    edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null, quote: null, qoty_card: null, embeds: [],
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
    expect(row('games', 'Apex')).toBeTruthy() // the panel, from the loaded facets
  })

  it('opens in the view you chose, and filtering swaps the feed', async () => {
    app.state = applyReady(emptyState(), ready({ clip_views: [{ channel_id: 'C1', view: 'list' }] }))
    cmp = mount(ClipsChannel, { target: document.body, props: { channelId: 'C1', serverId: 'S1' } })
    flushSync()
    await settle()
    flushSync()
    expect(document.querySelector('.grid')).toBeNull()
    expect(document.querySelector('.feed.list')).not.toBeNull()
    row('games', 'Apex').click()
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

  async function openChannel(id = 'C1') {
    cmp = mount(ClipsChannel, { target: document.body, props: { channelId: id, serverId: 'S1' } })
    flushSync()
    await settle()
    flushSync()
  }

  it('the games panel replaces the pills (#663): All games with the total, each game with its count, then People', async () => {
    await openChannel()
    expect(document.querySelectorAll('.chip, .toggle-chip').length).toBe(0)
    const panel = document.querySelector<HTMLElement>('aside.clips-panel')!
    expect(panel).not.toBeNull()
    expect(panel.id).toBe(document.querySelector('[aria-controls]')!.getAttribute('aria-controls'))
    expect(rows('games')).toEqual([['All games', '7'], ['Rocket League', '4'], ['Apex', '2']])
    expect([...panel.querySelectorAll('h3.menu-label')].map((h) => h.textContent)).toEqual(['Games', 'People'])
    expect(panel.querySelectorAll(':scope > [role="separator"]').length).toBe(1)
    expect(rows('people')).toEqual([['Everyone', '7'], ['sam', '5'], ['Kimmy', '1']])
    expect(row('games', 'All games').getAttribute('aria-pressed')).toBe('true')
    expect(row('people', 'Everyone').getAttribute('aria-pressed')).toBe('true')
  })

  it('a server without the total: "All games" and "Everyone" show no number rather than a guess', async () => {
    const { total: _, ...old } = facets
    invoke.mockImplementation(async (cmd: string) => (cmd === 'list_clips' ? [] : cmd === 'clip_facets' ? old : undefined))
    await openChannel()
    expect(row('games', 'All games').querySelector('.n')?.textContent).toBe('')
    expect(row('people', 'Everyone').querySelector('.n')?.textContent).toBe('')
  })

  it('picking a game, then a person, filters the feed and highlights them; All games clears only the game', async () => {
    await openChannel()
    row('games', 'Rocket League').click()
    flushSync()
    await settle()
    expect(calls('list_clips').at(-1)![1]).toEqual({ channelId: 'C1', before: null, game: 'G2', author: null })
    expect(row('games', 'Rocket League').getAttribute('aria-pressed')).toBe('true')
    expect(row('games', 'All games').getAttribute('aria-pressed')).toBe('false')
    row('people', 'Kimmy').click()
    flushSync()
    await settle()
    expect(calls('list_clips').at(-1)![1]).toEqual({ channelId: 'C1', before: null, game: 'G2', author: 'U3' })
    expect(row('people', 'Kimmy').getAttribute('aria-pressed')).toBe('true')
    row('games', 'All games').click()
    flushSync()
    await settle()
    expect(calls('list_clips').at(-1)![1]).toEqual({ channelId: 'C1', before: null, game: null, author: 'U3' })
  })

  it('the header button collapses the panel, remembered for this channel on the next visit', async () => {
    await openChannel()
    const toggle = () => document.querySelector<HTMLButtonElement>('button[aria-controls]')!
    expect(toggle().getAttribute('aria-expanded')).toBe('true')
    toggle().click()
    flushSync()
    expect(document.querySelector('.clips-panel')).toBeNull()
    expect(toggle().getAttribute('aria-expanded')).toBe('false')
    expect(clipPanelOpen('C1')).toBe(false)
    expect(clipPanelOpen('C2')).toBe(true) // per channel
    unmount(cmp!)
    cmp = undefined
    await openChannel()
    expect(document.querySelector('.clips-panel')).toBeNull()
    toggle().click()
    flushSync()
    expect(document.querySelector('.clips-panel')).not.toBeNull()
    expect(clipPanelOpen('C1')).toBe(true)
  })

  it('a clip\'s replies take the right side while open: the games panel steps aside and comes back', async () => {
    Element.prototype.scrollTo = () => {} // jsdom has none
    const replied = { ...clip('M1'), clip: { ...clip('M1').clip!, reply_count: 2 } }
    invoke.mockImplementation(async (cmd: string) => (cmd === 'list_clips' ? [replied] : cmd === 'clip_facets' ? facets : cmd === 'list_replies' ? [] : undefined))
    app.state = applyReady(emptyState(), ready({ clip_views: [{ channel_id: 'C1', view: 'list' }] }))
    await openChannel()
    expect(document.querySelector('.clips-panel')).not.toBeNull()
    button('View 2 replies').click()
    flushSync()
    await settle()
    expect(document.querySelector('.replies-panel')).not.toBeNull()
    expect(document.querySelector('.clips-panel')).toBeNull()
    expect(clipPanelOpen('C1')).toBe(true) // not a choice to close it
    document.querySelector<HTMLButtonElement>('.replies-panel button[aria-label="Close replies"]')!.click()
    flushSync()
    await settle()
    flushSync()
    expect(document.querySelector('.replies-panel')).toBeNull()
    expect(document.querySelector('.clips-panel')).not.toBeNull()
  })

  it('layout: the feed gives way to the panel (min-width 0), so nothing spills at any width', () => {
    const src = readFileSync(resolve(__dirname, 'ClipsChannel.svelte'), 'utf8').split('<style>')[1]
    expect(src).toMatch(/\.body\s*\{[^}]*display:\s*flex[^}]*min-height:\s*0/)
    expect(src).toMatch(/\.main\s*\{[^}]*flex:\s*1[^}]*min-width:\s*0/)
  })
})
