// @vitest-environment jsdom
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { flushSync, mount, unmount } from 'svelte'
import { compile } from 'svelte/compiler'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Attachment } from '../lib/protocol/Attachment'
import type { Build } from '../lib/protocol/Build'
import type { Message } from '../lib/protocol/Message'
import { FakeIntersectionObserver, fakeMatchMedia } from '../lib/testMotion'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args), convertFileSrc: (s: string) => s }))
vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)
// jsdom has no layout: a stand-in ResizeObserver, fired by the test with the width the page's body would have
const observers = new Set<{ cb: ResizeObserverCallback; els: Set<Element> }>()
vi.stubGlobal('ResizeObserver', class {
  o: { cb: ResizeObserverCallback; els: Set<Element> }
  constructor(cb: ResizeObserverCallback) {
    this.o = { cb, els: new Set() }
    observers.add(this.o)
  }
  observe(el: Element) { this.o.els.add(el) }
  unobserve(el: Element) { this.o.els.delete(el) }
  disconnect() { this.o.els.clear(); observers.delete(this.o) }
})
/** The build page's body is now `width` wide. */
function resize(width: number) {
  const el = document.querySelector('.build-page .body')!
  for (const o of observers) {
    if (!o.els.has(el)) continue
    o.cb([{ target: el, contentRect: { width, height: 800 } } as unknown as ResizeObserverEntry], {} as ResizeObserver)
  }
  flushSync()
}

import BuildPage from './BuildPage.svelte'
import BuildsChannel from './BuildsChannel.svelte'
import { app, handleReady, resetState, ui } from '../lib/store.svelte'
import { applyEvent } from '../lib/state'
import { openMessage } from '../lib/openMessage'
import { testMe, testReady } from '../lib/testReady'
import { windowFocus } from '../lib/windowFocus.svelte'
import { forgetPinnedBuild, pinnedBuild } from '../lib/pinnedBuild.svelte'

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
/** The calls of `cmd` (for `list_builds`, the browse feed's, not the Favourites section's own). */
const calls = (cmd: string) => invoke.mock.calls.filter((c) => c[0] === cmd && !(cmd === 'list_builds' && (c[1] as { favourite?: boolean })?.favourite))
let replies: Message[] = []
let pages: Message[][] = []
let fetched: Message | null = null

function answer(cmd: string, args?: unknown): unknown {
  if (cmd === 'attachment_urls') return Array.from({ length: 12 }, (_, i) => ({ id: `A${i}`, url: `https://files/${i}.webp`, expires_at: '2099-01-01T00:00:00Z' }))
  if (cmd === 'list_replies') return (args as { before: string | null }).before ? [] : replies
  // the Favourites section's own feed: none starred
  if (cmd === 'list_builds') return (args as { favourite?: boolean } | undefined)?.favourite ? [] : (pages.shift() ?? [])
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
  forgetPinnedBuild()
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
    expect(document.querySelectorAll('.build-view .tile')[3].getAttribute('aria-label')).toBe('Open screenshot 4, and 2 more')
  })

  it('a screenshot opens the image viewer', async () => {
    await page(build('M001', {}, { attachments: [1, 2, 3].map(shot) }))
    expect(document.querySelector('.viewer')).toBeNull()
    document.querySelectorAll<HTMLButtonElement>('.build-view .tile')[1].click()
    await flush()
    expect(document.querySelector('.viewer[role=dialog]')?.getAttribute('aria-label')).toBe('shot2.webp')
  })

  it('the author sees Edit, Archive and Delete; sam only Pin to side and Pop out; the owner Archive and Delete', async () => {
    // alex's own build
    await page(build('M001', {}, { author_id: 'U1' }))
    await openActions()
    expect(actionItems()).toEqual(['Pin to side', 'Pop out', 'Edit', 'Archive', 'Delete'])
    unmount(cmp!)
    // sam's build, alex neither its author nor the owner: only Pin to side (#619)
    await page(build('M001'))
    await openActions()
    expect(actionItems()).toEqual(['Pin to side', 'Pop out'])
    unmount(cmp!)
    // sam's build, alex the server's owner
    start('U1')
    await page(build('M001'))
    await openActions()
    expect(actionItems()).toEqual(['Pin to side', 'Pop out', 'Archive', 'Delete'])
  })

  // #619
  it('Pin to side pins it, and the menu then offers Unpin', async () => {
    await page(build('M001'))
    await openActions()
    ;[...document.querySelectorAll<HTMLElement>('.build-actions [role=menuitem]')].find((e) => e.textContent?.trim() === 'Pin to side')!.click()
    flushSync()
    expect(pinnedBuild.key).toEqual({ channelId: 'B1', messageId: 'M001' })
    expect(pinnedBuild.message?.id).toBe('M001')
    await openActions()
    expect(actionItems()).toEqual(['Unpin', 'Pop out'])
    ;[...document.querySelectorAll<HTMLElement>('.build-actions [role=menuitem]')].find((e) => e.textContent?.trim() === 'Unpin')!.click()
    flushSync()
    expect(pinnedBuild.key).toBeNull()
  })

  // #620
  it('Pop out sends the snapshot', async () => {
    await page(build('M001'))
    await openActions()
    ;[...document.querySelectorAll<HTMLElement>('.build-actions [role=menuitem]')].find((e) => e.textContent?.trim() === 'Pop out')!.click()
    await flush()
    const open = invoke.mock.calls.filter(([c]) => c === 'open_build_window').map(([, a]) => a as { snapshot: { message_id: string; title: string }; onTop: boolean })
    expect(open).toHaveLength(1)
    expect(open[0].onTop).toBe(true)
    expect(open[0].snapshot).toMatchObject({ message_id: 'M001', title: 'Build M001', author_name: 'sam' })
  })

  it('an archived build says so and offers Restore', async () => {
    await page(build('M001', { archived_at: '2026-10-05T00:00:00Z', archived_by: 'U1' }, { author_id: 'U1' }))
    expect(document.querySelector('.build-view .badge')?.textContent).toBe('ARCHIVED')
    await openActions()
    expect(actionItems()).toEqual(['Pin to side', 'Pop out', 'Edit', 'Restore', 'Delete'])
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

  it('Escape on the delete question closes it and puts the focus back on Actions', async () => {
    await page(build('M001', {}, { author_id: 'U1' }))
    await openActions()
    ;[...document.querySelectorAll<HTMLElement>('.build-actions [role=menuitem]')].find((e) => e.textContent?.trim() === 'Delete')!.click()
    await flush()
    const ask = document.querySelector<HTMLElement>('.build-actions [role=alertdialog]')!
    ask.querySelector<HTMLButtonElement>('button')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))
    flushSync()
    expect(document.querySelector('.build-actions [role=alertdialog]')).toBeNull()
    expect(document.activeElement).toBe(document.querySelector('.build-actions button[aria-haspopup=menu]'))
    expect(calls('delete_build')).toHaveLength(0)
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

  it('a second build opened from the page still goes Back to the scroll the browse view had', async () => {
    pages = [[build('M003'), build('M002'), build('M001')]]
    fetched = build('M009', { title: 'Raven bunny' })
    await channel()
    document.querySelector<HTMLElement>('.feed')!.scrollTop = 320
    document.querySelector<HTMLButtonElement>('[data-build-id="M001"] button.open')!.click()
    await flush()
    ui.openBuild = { channelId: 'B1', messageId: 'M009' }
    await flush()
    expect(document.querySelector('.build-view h1')?.textContent?.trim()).toBe('Raven bunny')
    document.querySelector<HTMLButtonElement>('.build-page .back')!.click()
    await flush()
    expect(document.querySelector<HTMLElement>('.feed')!.scrollTop).toBe(320)
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

// #618 testing feedback: the page's layout
describe('BuildPage: layout', () => {
  const yt = { url: 'https://www.youtube.com/watch?v=dQw4w9WgXcQ', title: 'Raven', thumb: null, provider: 'youtube' as const, provider_id: 'dQw4w9WgXcQ' }
  const toggle = () => document.querySelector<HTMLButtonElement>('.build-page .bar button.comments-toggle')!
  const layout = () => document.querySelector('.build-view')!.getAttribute('data-layout')
  const comments = () => document.querySelector<HTMLElement>('.build-page .comments')!

  it('Comments can be hidden and shown, and the page remembers it on this device', async () => {
    await page(build('M001'))
    expect(toggle().getAttribute('aria-expanded')).toBe('true')
    expect(toggle().getAttribute('aria-controls')).toBe(comments().id)
    expect(toggle().textContent).toContain('Comments')
    expect(comments().hidden).toBe(false)
    toggle().click()
    flushSync()
    expect(toggle().getAttribute('aria-expanded')).toBe('false')
    expect(comments().hidden).toBe(true)
    expect(localStorage.getItem('pulse.buildComments')).toBe('hidden')
    // another build's page on this device opens with them hidden too
    unmount(cmp!)
    await page(build('M002'))
    expect(toggle().getAttribute('aria-expanded')).toBe('false')
    expect(comments().hidden).toBe(true)
    toggle().click()
    flushSync()
    expect(comments().hidden).toBe(false)
    expect(localStorage.getItem('pulse.buildComments')).toBe('shown')
  })

  it('hiding Comments keeps what was being typed (the thread stays mounted)', async () => {
    await page(build('M001'))
    const box = document.querySelector<HTMLTextAreaElement>('.comments textarea')
    toggle().click()
    flushSync()
    expect(document.querySelector('.comments textarea')).toBe(box)
  })

  it('wide enough beside Comments: video left, screenshot grid right', async () => {
    await page(build('M001', { link: yt }, { attachments: [1, 2, 3, 4].map(shot) }))
    resize(1200)
    expect(layout()).toBe('wide')
    expect(document.querySelector('.build-page .body')!.classList.contains('stack')).toBe(false)
    const media = document.querySelector('.build-view .media')!
    expect(media.firstElementChild!.classList.contains('video')).toBe(true)
    expect(media.lastElementChild!.classList.contains('tiles')).toBe(true)
  })

  it('narrow with Comments shown: stacked; hiding Comments brings the wide layout back', async () => {
    await page(build('M001', { link: yt }, { attachments: [1, 2].map(shot) }))
    resize(900)
    expect(layout()).toBe('stacked')
    toggle().click()
    flushSync()
    expect(layout()).toBe('wide')
    toggle().click()
    flushSync()
    expect(layout()).toBe('stacked')
  })

  it('too narrow for a Comments column: Comments go under the build', async () => {
    await page(build('M001'))
    resize(600)
    expect(document.querySelector('.build-page .body')!.classList.contains('stack')).toBe(true)
    expect(layout()).toBe('stacked')
    resize(1000)
    expect(document.querySelector('.build-page .body')!.classList.contains('stack')).toBe(false)
  })

  it('never cramped: a narrow page stacks even with Comments hidden', async () => {
    localStorage.setItem('pulse.buildComments', 'hidden')
    await page(build('M001', { link: yt }, { attachments: [1, 2].map(shot) }))
    resize(500)
    expect(layout()).toBe('stacked')
    resize(700)
    expect(layout()).toBe('wide')
  })

  it('the meta line sits top right, on the caption line, muted and with its full text on hover', async () => {
    await page(build('M001'))
    const top = document.querySelector('.build-view .top')!
    expect(top.querySelector('.caption')).not.toBeNull()
    const meta = top.querySelector<HTMLElement>('.meta')!
    expect(meta.previousElementSibling).toBe(top.querySelector('.caption'))
    expect(meta.textContent).toMatch(/^sam · saved .+ · updated .+$/)
    expect(meta.getAttribute('data-tip')).toBe(meta.textContent)
    // above the title, not under it
    expect(top.compareDocumentPosition(document.querySelector('.build-view h1')!) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
  })

  it('Actions sit in the top row with Back, above the Comments column', async () => {
    await page(build('M001', {}, { author_id: 'U1' }))
    const bar = document.querySelector('.build-page .bar')!
    expect(bar.querySelector('.back')).not.toBeNull()
    expect(bar.querySelector('.build-actions button[aria-haspopup=menu]')?.textContent?.trim()).toBe('Actions')
    expect(document.querySelector('.build-view .build-actions')).toBeNull()
    expect(bar.querySelector('.comments-toggle')).not.toBeNull()
  })

  // #645 review: on the stacked layout Enlarge has nothing to fill, so it isn't offered and an earlier one doesn't apply
  it('Enlarge is offered only on the wide layout', async () => {
    localStorage.setItem('pulse.buildComments', 'hidden')
    await page(build('M001', { link: yt }, { attachments: [1, 2].map(shot) }))
    resize(500)
    expect(layout()).toBe('stacked')
    expect(document.querySelector('.build-video .enlarge')).toBeNull()
    resize(1200)
    document.querySelector<HTMLButtonElement>('.build-video .corner .enlarge')!.click()
    flushSync()
    expect(document.querySelector('.build-view .media.enlarged')).not.toBeNull()
    resize(500)
    expect(document.querySelector('.build-view .media.enlarged')).toBeNull()
    expect(document.querySelector('.build-video.enlarged')).toBeNull()
    expect(document.querySelector('.build-video .enlarge')).toBeNull()
  })

  // #618 testing: with Comments hidden the media row fills the page; beside Comments (and stacked) it keeps its sizes
  it('wide with Comments hidden: the media row fills the page; beside Comments or stacked it does not', async () => {
    await page(build('M001', { link: yt }, { attachments: [1, 2, 3, 4].map(shot) }))
    const fill = () => document.querySelector('.build-view .media')!.classList.contains('fill')
    const videoFill = () => document.querySelector('.build-video')!.classList.contains('fill')
    resize(1200)
    expect(layout()).toBe('wide')
    expect(fill()).toBe(false)
    expect(videoFill()).toBe(false)
    toggle().click()
    flushSync()
    expect(layout()).toBe('wide')
    expect(fill()).toBe(true)
    expect(videoFill()).toBe(true)
    // too narrow for the wide row even with Comments hidden: stacked, and the old rules
    resize(500)
    expect(layout()).toBe('stacked')
    expect(fill()).toBe(false)
    expect(videoFill()).toBe(false)
    resize(1200)
    expect(fill()).toBe(true)
    // enlarged keeps filling (the video the row, the strip under it)
    document.querySelector<HTMLButtonElement>('.build-video .corner .enlarge')!.click()
    flushSync()
    const media = document.querySelector('.build-view .media')!
    expect(media.classList.contains('enlarged')).toBe(true)
    expect(media.classList.contains('fill')).toBe(true)
  })

  // #645 review: a video with no screenshots takes the row's one column, not half of it
  it('a video with no screenshots gets the whole wide row', async () => {
    localStorage.setItem('pulse.buildComments', 'hidden')
    await page(build('M001', { link: yt }))
    resize(1200)
    expect(document.querySelector('.build-view .media.wide')!.classList.contains('no-tiles')).toBe(true)
  })

  // #645 review: the Actions error over the page is dismissed from the keyboard too
  it('an Actions error closes on Escape or its close button', async () => {
    invoke.mockImplementation(async (cmd: string, args?: unknown) => { if (cmd === 'set_build_archived') throw 'no can do'; return answer(cmd, args) })
    const fail = async () => {
      await openActions()
      ;[...document.querySelectorAll<HTMLElement>('.build-actions [role=menuitem]')].find((e) => e.textContent?.trim() === 'Archive')!.click()
      await flush()
      return document.querySelector<HTMLElement>('.build-actions .err[role=alert]')
    }
    await page(build('M001', {}, { author_id: 'U1' }))
    expect((await fail())?.textContent).toContain('no can do')
    document.body.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))
    flushSync()
    expect(document.querySelector('.build-actions .err')).toBeNull()
    const close = (await fail())!.querySelector<HTMLButtonElement>('button[aria-label="Dismiss error"]')!
    close.click()
    flushSync()
    expect(document.querySelector('.build-actions .err')).toBeNull()
  })

  it('Enlarge on the wide layout gives the video the full row', async () => {
    await page(build('M001', { link: yt }, { attachments: [1, 2].map(shot) }))
    resize(1200)
    expect(document.querySelector('.build-view .media.enlarged')).toBeNull()
    document.querySelector<HTMLButtonElement>('.build-video .corner .enlarge')!.click()
    flushSync()
    expect(document.querySelector('.build-view .media.enlarged')).not.toBeNull()
  })
})

// Greg's UI rule: nothing spills out of its container or jumps at any width (jsdom doesn't lay out: the rules that keep it in)
describe('BuildPage: containment', () => {
  const cssOf = (file: string) => compile(readFileSync(resolve(__dirname, file), 'utf8'), { filename: file, css: 'external', cssHash: () => 'x' }).css!.code
  const rule = (css: string, sel: string) => css.match(new RegExp(`(?:^|\\}|\\s)${sel}\\s*\\{([^}]*)\\}`))?.[1] ?? ''

  it('the top row keeps Back, Actions and the toggle on one line inside the page', () => {
    const css = cssOf('BuildPage.svelte')
    expect(rule(css, '\\.bar\\.x')).toMatch(/min-width:\s*0/)
    expect(rule(css, '\\.tools\\.x')).toMatch(/margin-left:\s*auto/)
    expect(rule(css, '\\.tools\\.x')).toMatch(/flex-shrink:\s*0/)
    expect(rule(css, '\\.main\\.x')).toMatch(/min-width:\s*0/)
    expect(rule(css, '\\.main\\.x')).toMatch(/overflow-x:\s*hidden/)
    expect(rule(css, '\\.comments\\[hidden\\]\\.x')).toMatch(/display:\s*none/)
    // the layout is picked from the page's width, never the window's
    expect(css).not.toMatch(/@media \((max|min)-width/)
  })

  it('the meta line ellipsises on the caption line; the caption wraps', () => {
    const css = cssOf('BuildView.svelte')
    expect(rule(css, '\\.top\\.x')).toMatch(/display:\s*flex/)
    // #645 review: the line wraps, so a meta that doesn't fit beside the caption takes its own line (and only
    // ellipsises when that is too narrow), rather than being squeezed to nothing in the pinned panel and pop-out
    expect(rule(css, '\\.top\\.x')).toMatch(/flex-wrap:\s*wrap/)
    expect(rule(css, '\\.meta\\.x')).toMatch(/flex:\s*0 1 auto/)
    expect(rule(css, '\\.top\\.x')).toMatch(/min-width:\s*0/)
    const meta = rule(css, '\\.meta\\.x')
    for (const r of [/min-width:\s*0/, /white-space:\s*nowrap/, /overflow:\s*hidden/, /text-overflow:\s*ellipsis/, /margin-left:\s*auto/, /color:\s*var\(--text-3\)/]) expect(meta).toMatch(r)
    expect(rule(css, '\\.caption\\.x')).toMatch(/overflow-wrap:\s*anywhere/)
  })

  it('the wide media row shrinks its two columns inside the page', () => {
    const css = cssOf('BuildView.svelte')
    expect(rule(css, '\\.media\\.wide\\.x')).toMatch(/grid-template-columns:\s*repeat\(2,\s*minmax\(0,\s*360px\)\)/)
    expect(rule(css, '\\.media\\.wide\\.x \\.tiles:where\\(\\.x\\)')).toMatch(/grid-template-columns:\s*repeat\(2,\s*minmax\(0,\s*1fr\)\)/)
    expect(rule(css, '\\.media\\.wide\\.enlarged\\.x \\.video:where\\(\\.x\\)')).toMatch(/grid-column:\s*1 \/ -1/)
    expect(rule(css, '\\.media\\.wide\\.no-tiles\\.x')).toMatch(/grid-template-columns:\s*minmax\(0,\s*640px\)/)
  })

  // #618 testing: Comments hidden, the media row fills the page: video ~60%, the 2×2 grid ~40%, capped and left-aligned
  it('the filling media row: fluid columns, fixed aspects and a width cap', () => {
    const css = cssOf('BuildView.svelte')
    const row = rule(css, '\\.media\\.wide\\.fill\\.x')
    expect(row).toMatch(/grid-template-columns:\s*minmax\(0,\s*3fr\)\s+minmax\(0,\s*2fr\)/)
    expect(row).toMatch(/max-width:\s*1400px/)
    expect(row).toMatch(/width:\s*100%/)
    expect(row).toMatch(/box-sizing:\s*border-box/)
    // its tiles keep one aspect, cropped to it, never capped short
    const tile = rule(css, '\\.media\\.wide\\.fill\\.x \\.tile:where\\(\\.x\\)')
    expect(tile).toMatch(/aspect-ratio:\s*4 \/ 3/)
    expect(tile).toMatch(/max-height:\s*none/)
    expect(rule(css, '\\.tile\\.x img:where\\(\\.x\\)')).toMatch(/object-fit:\s*cover/)
    // no screenshots: the video takes the row; no video: the screenshots an adapting grid across it
    expect(rule(css, '\\.media\\.wide\\.fill\\.no-tiles\\.x')).toMatch(/grid-template-columns:\s*minmax\(0,\s*1fr\)/)
    expect(rule(css, '\\.media\\.wide\\.fill\\.solo\\.x \\.tiles:where\\(\\.x\\)')).toMatch(/grid-template-columns:\s*repeat\(auto-fill,\s*minmax\(min\(320px,\s*100%\),\s*1fr\)\)/)
    // enlarged: the video the row and the strip under it, as before
    expect(rule(css, '\\.media\\.wide\\.fill\\.enlarged\\.x')).toMatch(/grid-template-columns:\s*repeat\(2,\s*minmax\(0,\s*1fr\)\)/)
    expect(rule(css, '\\.media\\.wide\\.fill\\.enlarged\\.x \\.tile:where\\(\\.x\\)')).toMatch(/aspect-ratio:\s*16 \/ 9/)
    // the player drops its 640px cap only when filling; it stays 16:9
    const video = cssOf('BuildVideo.svelte')
    expect(rule(video, '\\.build-video\\.x')).toMatch(/max-width:\s*640px/)
    expect(rule(video, '\\.build-video\\.fill\\.x')).toMatch(/max-width:\s*none/)
    expect(rule(video, '\\.screen\\.x')).toMatch(/aspect-ratio:\s*16 \/ 9/)
  })

  it("the player's controls show on hover and keyboard focus, and the link button ellipsises", () => {
    const css = cssOf('BuildVideo.svelte')
    expect(css).toMatch(/\.corner\.x:focus-within/)
    expect(css).toMatch(/\.build-video\.x:hover \.corner:where\(\.x\)/)
    expect(css).toMatch(/@media \(hover: none\)/)
    expect(rule(css, '\\.yt-title\\.x')).toMatch(/text-overflow:\s*ellipsis/)
    expect(rule(css, '\\.yt-title\\.x')).toMatch(/min-width:\s*0/)
    expect(rule(css, '\\.open-yt\\.x')).toMatch(/flex-shrink:\s*0/)
    expect(rule(css, '\\.open-yt\\.x')).toMatch(/font-size:\s*12px/)
  })

  it('colour tokens only', () => {
    for (const f of ['BuildPage.svelte', 'BuildView.svelte', 'BuildVideo.svelte', 'BuildActions.svelte']) {
      const src = readFileSync(resolve(__dirname, f), 'utf8')
      expect(src.slice(src.indexOf('<style>')).replace(/\/\*[\s\S]*?\*\//g, ''), f).not.toMatch(/#[0-9a-fA-F]{3,8}\b|rgba?\(|hsla?\(/)
    }
  })
})
