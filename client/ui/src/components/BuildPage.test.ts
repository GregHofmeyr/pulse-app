// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Attachment } from '../lib/protocol/Attachment'
import type { Build } from '../lib/protocol/Build'
import type { Message } from '../lib/protocol/Message'
import { FakeIntersectionObserver, fakeMatchMedia } from '../lib/testMotion'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args), convertFileSrc: (s: string) => s }))
vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)

import BuildPage from './BuildPage.svelte'
import BuildsChannel from './BuildsChannel.svelte'
import { app, handleReady, resetState, ui } from '../lib/store.svelte'
import { applyEvent } from '../lib/state'
import { openMessage } from '../lib/openMessage'
import { testMe, testReady } from '../lib/testReady'
import { windowFocus } from '../lib/windowFocus.svelte'

const sam = { id: 'U2', username: 'sam', display_name: 'sam', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const jo = { id: 'U3', username: 'jo', display_name: 'jo', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const ready = (owner = 'U3') =>
  testReady([{ id: 'B1', server_id: 'S1', kind: 'builds', name: 'raven-builds', position: 0 }], {
    servers: [{ id: 'S1', name: 'Main', icon_hash: null, owner_id: owner }],
    members: [{ server_id: 'S1', members: [{ user: testMe, nickname: null }, { user: sam, nickname: null }, { user: jo, nickname: null }] }],
    people: [testMe, sam, jo].map((user) => ({ user, online: true, last_seen_at: null, status: 'online' as const })),
    read_states: [{ channel_id: 'B1', last_read_message_id: null, unread: 0, mentions: 0 }],
    games: [{ id: 'G1', server_id: 'S1', name: 'The First Descendant', created_by: 'U1', created_at: '2026-10-01T00:00:00Z' }],
  })

const shot = (n: number): Attachment => ({
  id: `A${n}`, name: `shot${n}.webp`, content_type: 'image/webp', kind: 'image', size: 100, width: 1600, height: 900, duration_ms: null, expires_at: null, expired: false,
})
const build = (id: string, b: Partial<Build> = {}, m: Partial<Message> = {}): Message => ({
  id, channel_id: 'B1', author_id: 'U2', kind: 'normal', content: '', reply_to_id: null,
  created_at: '2026-10-01T10:00:00.000Z', edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null,
  quote: null, qoty_card: null, embeds: [], clip: null,
  build: {
    title: `Build ${id}`, title_auto: false, game_id: 'G1', game_guessed: false, link: null, notes: '**Max crit**, bunny spam', quick_drop: false,
    updated_at: '2026-10-02T10:00:00.000Z', archived_at: null, archived_by: null, reply_count: 0, last_reply_id: null, replies_seen: null, ...b,
  },
  ...m,
})
const comment = (id: string, author: string, content: string): Message => ({
  id, channel_id: 'B1', author_id: author, kind: 'normal', content, reply_to_id: null, created_at: '2026-10-03T10:00:00.000Z', edited_at: null,
  deleted: false, mentions: [], reactions: [], thread_root_id: 'M001', quote: null, qoty_card: null, embeds: [], clip: null,
})

let cmp: ReturnType<typeof mount> | undefined
const settle = () => new Promise((r) => setTimeout(r, 0))
const calls = (cmd: string) => invoke.mock.calls.filter((c) => c[0] === cmd)
let replies: Message[] = []
let pages: Message[][] = []
let fetched: Message | null = null

function answer(cmd: string, args?: unknown): unknown {
  if (cmd === 'attachment_urls') return Array.from({ length: 12 }, (_, i) => ({ id: `A${i}`, url: `https://files/${i}.webp`, expires_at: '2099-01-01T00:00:00Z' }))
  if (cmd === 'list_replies') return (args as { before: string | null }).before ? [] : replies
  if (cmd === 'list_builds') return pages.shift() ?? []
  if (cmd === 'build_facets') return { total: 3, games: [{ game_id: 'G1', count: 3 }] }
  if (cmd === 'get_build') return fetched
  return undefined
}

function start(owner = 'U3') {
  resetState()
  handleReady(ready(owner))
}

beforeEach(() => {
  Element.prototype.scrollTo = () => {}
  try { localStorage.clear() } catch { /* none */ }
  invoke.mockReset()
  invoke.mockImplementation(async (cmd: string, args?: unknown) => answer(cmd, args))
  replies = []
  pages = []
  fetched = null
  windowFocus.focused = true
  start()
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

async function flush() {
  flushSync()
  await settle()
  flushSync()
  await settle()
  flushSync()
}

async function page(m: Message, onBack = () => {}) {
  cmp = mount(BuildPage, { target: document.body, props: { message: m, serverId: 'S1', onBack } })
  await flush()
}

const actionItems = () => [...document.querySelectorAll<HTMLElement>('.build-actions [role=menuitem]')].map((e) => e.textContent?.trim())
async function openActions() {
  document.querySelector<HTMLButtonElement>('.build-actions button[aria-haspopup=menu]')!.click()
  flushSync()
}

describe('BuildPage', () => {
  it('the page shows caption, meta, notes and four tiles with +N', async () => {
    await page(build('M001', {}, { attachments: [1, 2, 3, 4, 5, 6].map(shot) }))
    expect(document.querySelector('.build-view .caption')?.textContent?.trim()).toBe('Builds · The First Descendant')
    expect(document.querySelector('.build-view h1')?.textContent?.trim()).toBe('Build M001')
    const meta = document.querySelector('.build-view .meta')!.textContent!
    expect(meta).toMatch(/^sam · saved .+ · updated .+$/)
    expect(document.querySelector('.build-view .notes strong')?.textContent).toBe('Max crit')
    expect(document.querySelectorAll('.build-view .tile')).toHaveLength(4)
    expect(document.querySelector('.build-view .more')?.textContent?.trim()).toBe('+2')
  })

  it('a screenshot opens the image viewer', async () => {
    await page(build('M001', {}, { attachments: [1, 2, 3].map(shot) }))
    expect(document.querySelector('.viewer')).toBeNull()
    document.querySelectorAll<HTMLButtonElement>('.build-view .tile')[1].click()
    await flush()
    expect(document.querySelector('.viewer[role=dialog]')?.getAttribute('aria-label')).toBe('shot2.webp')
  })

  it('the author sees Edit, Archive and Delete; sam sees no Actions; the owner sees Archive and Delete', async () => {
    // alex's own build
    await page(build('M001', {}, { author_id: 'U1' }))
    await openActions()
    expect(actionItems()).toEqual(['Edit', 'Archive', 'Delete'])
    unmount(cmp!)
    // sam's build, alex neither its author nor the owner
    await page(build('M001'))
    expect(document.querySelector('.build-actions')).toBeNull()
    unmount(cmp!)
    // sam's build, alex the server's owner
    start('U1')
    await page(build('M001'))
    await openActions()
    expect(actionItems()).toEqual(['Archive', 'Delete'])
  })

  it('an archived build says so and offers Restore', async () => {
    await page(build('M001', { archived_at: '2026-10-05T00:00:00Z', archived_by: 'U1' }, { author_id: 'U1' }))
    expect(document.querySelector('.build-view .badge')?.textContent).toBe('ARCHIVED')
    await openActions()
    expect(actionItems()).toEqual(['Edit', 'Restore', 'Delete'])
    ;[...document.querySelectorAll<HTMLElement>('.build-actions [role=menuitem]')].find((e) => e.textContent?.trim() === 'Restore')!.click()
    await flush()
    expect(calls('set_build_archived')[0][1]).toEqual({ messageId: 'M001', archived: false })
  })

  it('Delete asks first', async () => {
    await page(build('M001', {}, { author_id: 'U1' }))
    await openActions()
    ;[...document.querySelectorAll<HTMLElement>('.build-actions [role=menuitem]')].find((e) => e.textContent?.trim() === 'Delete')!.click()
    flushSync()
    expect(calls('delete_build')).toHaveLength(0)
    document.querySelector<HTMLButtonElement>('.build-actions [role=alertdialog] .btn-danger')!.click()
    await flush()
    expect(calls('delete_build')[0][1]).toEqual({ messageId: 'M001' })
  })

  it("the author's comments are tagged author", async () => {
    replies = [comment('R0002', 'U3', 'nice one'), comment('R0001', 'U2', 'thanks')]
    await page(build('M001'))
    expect(document.querySelector('.comments h2')?.textContent).toBe('Comments')
    const rows = [...document.querySelectorAll<HTMLElement>('.msg')]
    const row = (t: string) => rows.find((r) => r.querySelector('.content')?.textContent?.trim() === t)!
    expect(row('thanks').querySelector('.author-tag')?.textContent).toBe('author')
    expect(row('nice one').querySelector('.author-tag')).toBeNull()
  })

  it("the build's video sits in the media area; a build without a link has none", async () => {
    await page(build('M001', { link: { url: 'https://www.youtube.com/watch?v=dQw4w9WgXcQ', title: 'Raven', thumb: null, provider: 'youtube', provider_id: 'dQw4w9WgXcQ' } }))
    expect(document.querySelector('.build-view .media .build-video')).not.toBeNull()
    unmount(cmp!)
    await page(build('M001'))
    expect(document.querySelector('.build-video, .link-card')).toBeNull()
  })

  it('no comments yet says so', async () => {
    await page(build('M001'))
    expect(document.querySelector('.comments .empty')?.textContent).toBe('No comments yet.')
  })
})

describe('BuildsChannel: the build page', () => {
  async function channel() {
    cmp = mount(BuildsChannel, { target: document.body, props: { channelId: 'B1', serverId: 'S1' } })
    await flush()
  }

  it('Back returns to the browse view with its filter and scroll', async () => {
    pages = [[build('M003'), build('M002'), build('M001')], [build('M003'), build('M001')]]
    await channel()
    // pick a game, then scroll
    ;[...document.querySelectorAll<HTMLButtonElement>('.games button')].find((b) => b.textContent?.includes('The First Descendant'))!.click()
    await flush()
    expect(calls('list_builds').at(-1)![1]).toMatchObject({ game: 'G1' })
    const feed = document.querySelector<HTMLElement>('.feed')!
    feed.scrollTop = 320
    document.querySelector<HTMLButtonElement>('[data-build-id="M001"] button.open')!.click()
    await flush()
    expect(document.querySelector('.feed')).toBeNull()
    expect(document.querySelector('.build-view h1')?.textContent?.trim()).toBe('Build M001')
    document.querySelector<HTMLButtonElement>('.build-page .back')!.click()
    await flush()
    expect(document.querySelector('.build-view')).toBeNull()
    expect(document.querySelector('.builds-head h2')?.textContent).toBe('The First Descendant')
    expect(document.querySelector<HTMLElement>('.feed')!.scrollTop).toBe(320)
    // no refetch: the same feed came back
    expect(calls('list_builds')).toHaveLength(2)
  })

  it('a build deleted while its page is open says so, and stays gone', async () => {
    pages = [[build('M002'), build('M001')]]
    await channel()
    document.querySelector<HTMLButtonElement>('[data-build-id="M001"] button.open')!.click()
    await flush()
    expect(document.querySelector('.build-view h1')?.textContent?.trim()).toBe('Build M001')
    app.state = applyEvent(app.state, { t: 'MessageDeleted', d: { channel_id: 'B1', message_id: 'M001', author_id: 'U2', mentions: [], thread_root_id: null } }, Date.now())
    await flush()
    await flush()
    expect(document.querySelector('.build-view')).toBeNull()
    expect(document.querySelector('.build-page .gone')?.textContent).toBe('This build was deleted.')
  })

  it('an inbox comment on a build opens its page', async () => {
    fetched = build('M009', { title: 'Raven bunny' })
    expect(openMessage({ ...comment('R0001', 'U3', 'hi'), thread_root_id: 'M009' })).toBe(true)
    expect(ui.goTo).toBe('B1')
    expect(ui.openBuild).toEqual({ channelId: 'B1', messageId: 'M009' })
    await channel()
    expect(calls('get_build')[0][1]).toEqual({ messageId: 'M009' })
    expect(document.querySelector('.build-view h1')?.textContent?.trim()).toBe('Raven bunny')
    expect(ui.openBuild).toBeNull()
  })
})
