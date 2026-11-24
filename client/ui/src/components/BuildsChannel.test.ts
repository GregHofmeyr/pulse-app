// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Message } from '../lib/protocol/Message'
import type { Build } from '../lib/protocol/Build'
import { FakeIntersectionObserver, fakeMatchMedia } from '../lib/testMotion'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args), convertFileSrc: (s: string) => s }))
vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)

import BuildsChannel from './BuildsChannel.svelte'
import { app } from '../lib/store.svelte'
import { applyReady, emptyState } from '../lib/state'
import { testMe, testReady } from '../lib/testReady'

const sam = { id: 'U2', username: 'sam', display_name: 'sam', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const ready = () =>
  testReady([{ id: 'B1', server_id: 'S1', kind: 'builds', name: 'raven-builds', position: 0 }, { id: 'B2', server_id: 'S1', kind: 'builds', name: 'more-builds', position: 1 }], {
    members: [{ server_id: 'S1', members: [{ user: testMe, nickname: null }, { user: sam, nickname: null }] }],
    read_states: [{ channel_id: 'B1', last_read_message_id: null, unread: 0, mentions: 0 }],
    games: [{ id: 'G1', server_id: 'S1', name: 'The First Descendant', created_by: 'U1', created_at: '2026-10-01T00:00:00Z' }],
  })

const build = (id: string, b: Partial<Build> = {}, m: Partial<Message> = {}): Message => ({
  id, channel_id: 'B1', author_id: 'U2', kind: 'normal', content: '', reply_to_id: null,
  created_at: '2026-10-01T10:00:00.000Z', edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null,
  quote: null, qoty_card: null, embeds: [], clip: null,
  build: {
    title: `Build ${id}`, title_auto: false, game_id: 'G1', game_guessed: false, link: null, notes: 'Max crit, bunny spam', quick_drop: false,
    updated_at: null, archived_at: null, archived_by: null, reply_count: 0, last_reply_id: null, replies_seen: null, ...b,
  },
  ...m,
})
const id = (n: number) => `M${String(n).padStart(3, '0')}`
const range = (from: number, to: number) => Array.from({ length: from - to + 1 }, (_, i) => build(id(from - i)))

/** What `list_builds` answers, by its arguments (default: the next of `pages`). */
let answer: (args: Record<string, unknown>) => Message[] = () => pages.shift() ?? []
let pages: Message[][] = []
let cmp: ReturnType<typeof mount> | undefined
const settle = () => new Promise((r) => setTimeout(r, 0))
const calls = (cmd: string) => invoke.mock.calls.filter((c) => c[0] === cmd)
const order = () => [...document.querySelectorAll<HTMLElement>('[data-build-id]')].map((e) => e.dataset.buildId)

beforeEach(() => {
  invoke.mockReset()
  pages = []
  answer = () => pages.shift() ?? []
  try { localStorage.clear() } catch { /* none */ }
  invoke.mockImplementation(async (cmd: string, args?: unknown) => {
    if (cmd === 'list_builds') return answer(args as Record<string, unknown>)
    if (cmd === 'build_facets') return { total: 0, games: [] }
    return undefined
  })
  app.state = applyReady(emptyState(), ready())
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
  vi.useRealTimers()
})

async function show(channelId = 'B1') {
  cmp = mount(BuildsChannel, { target: document.body, props: { channelId, serverId: 'S1' } })
  flushSync()
  await settle()
  flushSync()
}

describe('BuildsChannel: gallery and list', () => {
  it('shows builds newest first as cards', async () => {
    pages = [[build('M003'), build('M002', { game_id: null }), build('M001')]]
    await show()
    expect(calls('list_builds')[0][1]).toEqual({ channelId: 'B1', game: null, archived: false, q: null, before: null })
    expect(order()).toEqual(['M003', 'M002', 'M001'])
    const card = document.querySelector<HTMLElement>('.build-card[data-build-id="M003"]')!
    expect(card.querySelector('.caption')?.textContent?.trim()).toBe('The First Descendant')
    expect(card.querySelector('.title')?.textContent).toBe('Build M003')
    expect(card.querySelector('.notes')?.textContent).toBe('Max crit, bunny spam')
    expect(card.querySelector('.meta')?.textContent).toContain('sam')
    // the game is a caption, never a pill
    expect(card.querySelector('.chip, .pill')).toBeNull()
    // no game: no caption
    expect(document.querySelector('[data-build-id="M002"] .caption')).toBeNull()
    // the header: All games, the count, and no "+ Add build" yet (nothing inert ships)
    expect(document.querySelector('.builds-head h2')?.textContent).toBe('All games')
    expect(document.body.textContent).not.toContain('+ Add build')
  })

  it('the List toggle switches to rows and is remembered per channel', async () => {
    pages = [[build('M002'), build('M001')]]
    await show()
    const radio = (l: string) => [...document.querySelectorAll<HTMLButtonElement>('[role=radiogroup][aria-label="View"] [role=radio]')].find((b) => b.textContent === l)!
    expect(radio('Gallery').getAttribute('aria-checked')).toBe('true')
    radio('List').click()
    flushSync()
    expect(document.querySelectorAll('.build-row')).toHaveLength(2)
    expect(document.querySelectorAll('.build-card')).toHaveLength(0)
    unmount(cmp!)
    // again: still a list; another channel keeps the gallery
    pages = [[build('M002'), build('M001')]]
    await show()
    expect(document.querySelectorAll('.build-row')).toHaveLength(2)
    unmount(cmp!)
    pages = [[build('M009', {}, { channel_id: 'B2' })]]
    await show('B2')
    expect(document.querySelectorAll('.build-card')).toHaveLength(1)
  })

  it('a quick drop shows its badge', async () => {
    pages = [[build('M002', { quick_drop: true, game_id: null, notes: null })]]
    await show()
    const card = document.querySelector<HTMLElement>('[data-build-id="M002"]')!
    expect(card.querySelector('.badge')?.textContent).toBe('QUICK DROP')
    expect(card.classList.contains('archived')).toBe(false)
  })

  it('search narrows the list', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] })
    answer = (a) => (a.q === 'raven' ? [build('M002', { title: 'Raven' })] : [build('M003'), build('M002', { title: 'Raven' }), build('M001')])
    cmp = mount(BuildsChannel, { target: document.body, props: { channelId: 'B1', serverId: 'S1' } })
    flushSync()
    await vi.advanceTimersByTimeAsync(0)
    flushSync()
    expect(order()).toEqual(['M003', 'M002', 'M001'])
    const box = document.querySelector<HTMLInputElement>('input[type=search]')!
    box.value = 'raven'
    box.dispatchEvent(new Event('input', { bubbles: true }))
    await vi.advanceTimersByTimeAsync(100)
    expect(calls('list_builds')).toHaveLength(1) // debounced
    await vi.advanceTimersByTimeAsync(200)
    flushSync()
    expect(calls('list_builds').at(-1)![1]).toMatchObject({ q: 'raven' })
    expect(order()).toEqual(['M002'])
  })

  it('scrolling to the end loads older builds', async () => {
    pages = [range(40, 11), range(10, 1)]
    await show()
    expect(order()).toHaveLength(30)
    FakeIntersectionObserver.set(document.querySelector('.sentinel')!, true)
    await settle()
    flushSync()
    expect(calls('list_builds')[1][1]).toMatchObject({ before: id(11) })
    expect(order()).toHaveLength(40)
    expect(order().at(-1)).toBe(id(1))
  })

  it('an empty channel says how to add one', async () => {
    pages = [[]]
    await show()
    expect(document.querySelector('.empty')?.textContent).toContain('No builds yet')
    expect(document.querySelector('.empty')?.textContent).toContain('+ Add build')
  })
})
