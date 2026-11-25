// @vitest-environment jsdom
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
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
import { buildFeedKey } from '../lib/builds'
import { applyEvent, applyReady, emptyState } from '../lib/state'
import { testMe, testReady } from '../lib/testReady'
import { windowFocus } from '../lib/windowFocus.svelte'

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
/** What the Favourites section's own feed (`favourite: true`) answers (default: none). */
let favAnswer: (args: Record<string, unknown>) => Message[] = () => []
const list = (args: unknown) => {
  const a = args as Record<string, unknown>
  return a.favourite ? favAnswer(a) : answer(a)
}
let pages: Message[][] = []
let cmp: ReturnType<typeof mount> | undefined
const settle = () => new Promise((r) => setTimeout(r, 0))
/** The calls of `cmd` (for `list_builds`, the browse feed's: the Favourites feed's are `favCalls`). */
const calls = (cmd: string) => invoke.mock.calls.filter((c) => c[0] === cmd && !(cmd === 'list_builds' && (c[1] as { favourite?: boolean })?.favourite))
const favCalls = () => invoke.mock.calls.filter((c) => c[0] === 'list_builds' && (c[1] as { favourite?: boolean })?.favourite)
const order = () => [...document.querySelectorAll<HTMLElement>('[data-build-id]')].map((e) => e.dataset.buildId)

beforeEach(() => {
  invoke.mockReset()
  pages = []
  answer = () => pages.shift() ?? []
  favAnswer = () => []
  try { localStorage.clear() } catch { /* none */ }
  invoke.mockImplementation(async (cmd: string, args?: unknown) => {
    if (cmd === 'list_builds') return list(args)
    if (cmd === 'build_facets') return { total: 0, games: [] }
    if (cmd === 'list_replies') return []
    return undefined
  })
  Element.prototype.scrollTo = () => {}
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
    expect(calls('list_builds')[0][1]).toEqual({ channelId: 'B1', game: null, archived: false, favourite: false, q: null, before: null })
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
    // the header: All games, the count, and "+ Add build" (#618)
    expect(document.querySelector('.builds-head h2')?.textContent).toBe('All games')
    expect([...document.querySelectorAll('.builds-head button')].some((b) => b.textContent?.trim() === '+ Add build')).toBe(true)
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

  it('a search feed is thrown away when the words change or the channel closes', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] })
    answer = (a) => (a.q ? [build('M002', { title: 'Raven' })] : [build('M002', { title: 'Raven' }), build('M001')])
    cmp = mount(BuildsChannel, { target: document.body, props: { channelId: 'B1', serverId: 'S1' } })
    flushSync()
    await vi.advanceTimersByTimeAsync(0)
    const box = document.querySelector<HTMLInputElement>('input[type=search]')!
    const search = async (q: string) => {
      box.value = q
      box.dispatchEvent(new Event('input', { bubbles: true }))
      await vi.advanceTimersByTimeAsync(300)
      flushSync()
    }
    await search('raven')
    expect(Object.keys(app.state.buildFeeds)).toContain(buildFeedKey('B1', { q: 'raven' }))
    await search('bunny')
    // (each with its Favourites feed)
    expect(Object.keys(app.state.buildFeeds).sort()).toEqual(
      [buildFeedKey('B1', {}), buildFeedKey('B1', { favourite: true }), buildFeedKey('B1', { q: 'bunny' }), buildFeedKey('B1', { q: 'bunny', favourite: true })].sort(),
    )
    // back to the same words: fetched again, not the copy cached before
    await search('raven')
    expect(calls('list_builds').filter((c) => (c[1] as { q: string | null }).q === 'raven')).toHaveLength(2)
    unmount(cmp!)
    cmp = undefined
    // the whole channel's feed stays (it follows events); the search's goes
    expect(Object.keys(app.state.buildFeeds).sort()).toEqual([buildFeedKey('B1', {}), buildFeedKey('B1', { favourite: true })].sort())
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

  it('marks read when the newest build is on screen and the window is focused', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] })
    pages = [[build('M004'), build('M003')]]
    cmp = mount(BuildsChannel, { target: document.body, props: { channelId: 'B1', serverId: 'S1' } })
    flushSync()
    await vi.advanceTimersByTimeAsync(0)
    flushSync()
    windowFocus.focused = false
    FakeIntersectionObserver.set(document.querySelector('[data-build-id="M004"]')!, true)
    flushSync()
    await vi.advanceTimersByTimeAsync(1000)
    expect(calls('mark_read')).toHaveLength(0) // not focused
    windowFocus.focused = true
    flushSync()
    await vi.advanceTimersByTimeAsync(1000)
    expect(calls('mark_read').map((c) => c[1])).toEqual([{ channelId: 'B1', messageId: 'M004' }])
  })

  it('an empty channel says how to add one', async () => {
    pages = [[]]
    await show()
    expect(document.querySelector('.empty')?.textContent).toContain('No builds yet')
    expect(document.querySelector('.empty')?.textContent).toContain('+ Add build')
  })

  it('the empty state sits in the same place in gallery and list (no jump on switching)', async () => {
    pages = [[]]
    await show()
    const radio = (l: string) => [...document.querySelectorAll<HTMLButtonElement>('[role=radiogroup][aria-label="View"] [role=radio]')].find((b) => b.textContent === l)!
    // what's laid out around the copy: its own classes, its parent's, and every sibling before it
    // (an empty, padded grid above it in one view only is what pushed it down)
    const layout = () => {
      const e = document.querySelector<HTMLElement>('.empty')!
      const before: string[] = []
      for (let s = e.previousElementSibling; s; s = s.previousElementSibling) before.push(s.className)
      return { self: e.className, parent: e.parentElement!.className.replace(/\blist\b/, '').trim(), before }
    }
    const gallery = layout()
    radio('List').click()
    flushSync()
    expect(document.querySelector('.empty')?.textContent).toContain('No builds yet')
    expect(layout()).toEqual(gallery)
    expect(gallery.before).toEqual([])
    radio('Gallery').click()
    flushSync()
    expect(layout()).toEqual(gallery)
  })
})

describe('BuildsChannel: accessibility', () => {
  const radio = (l: string) => [...document.querySelectorAll<HTMLButtonElement>('[role=radiogroup][aria-label="View"] [role=radio]')].find((b) => b.textContent === l)!
  const link = { url: 'https://youtu.be/x', provider: 'youtube', thumb: null } as never

  it("a row's video tag reads as \"video\", not as the ▶ glyph", async () => {
    pages = [[build('M002', { link })]]
    await show()
    radio('List').click()
    flushSync()
    const tag = document.querySelector<HTMLElement>('.build-row .tag')!
    expect(tag.getAttribute('aria-hidden')).toBe('true')
    expect(tag.textContent).toBe('▶')
    expect(tag.parentElement!.querySelector('.sr-only')?.textContent).toBe('video')
  })

  it('an archived build dims only its picture, never its text', async () => {
    pages = [[build('M002', { archived_at: '2026-10-05T00:00:00.000Z', archived_by: 'U2' }), build('M001')]]
    await show()
    for (const v of ['Gallery', 'List']) {
      radio(v).click()
      flushSync()
      expect(document.querySelector('[data-build-id="M002"] .thumb')?.classList.contains('dim')).toBe(true)
      expect(document.querySelector('[data-build-id="M001"] .thumb')?.classList.contains('dim')).toBe(false)
    }
    const src = (p: string) => readFileSync(resolve(__dirname, p), 'utf8')
    for (const f of ['BuildCard.svelte', 'BuildRow.svelte']) expect(src(f)).not.toMatch(/\.archived[^{]*\{[^}]*opacity/)
    expect(src('BuildThumb.svelte')).toMatch(/\.thumb\.dim \{[^}]*opacity/)
  })
})

describe('BuildsChannel: right-click menu', () => {
  const mine = (n: string, b: Partial<Build> = {}) => build(n, b, { author_id: 'U1' })
  const card = (n: string) => document.querySelector<HTMLElement>(`.build-card[data-build-id="${n}"]`)!
  const opener = (n: string) => card(n).querySelector<HTMLElement>('.open')!
  const rightClick = (el: HTMLElement) => {
    el.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 20, clientY: 20 }))
    flushSync()
  }
  const items = () => [...document.querySelectorAll<HTMLElement>('.build-menu [role=menuitem]')].map((b) => b.textContent?.trim())
  const item = (l: string) => [...document.querySelectorAll<HTMLElement>('.build-menu [role=menuitem]')].find((b) => b.textContent?.trim().startsWith(l))!

  it('offers Edit, Archive and Delete only where buildRights allows', async () => {
    pages = [[mine('M002'), build('M001')]]
    await show()
    rightClick(card('M002'))
    expect(items()).toEqual(['Open', 'Favourite', 'Edit', 'Archive', 'Delete…', 'Select'])
    rightClick(card('M001'))
    expect(items()).toEqual(['Open', 'Favourite', 'Select'])
  })

  it("the server's owner gets Archive and Delete on anyone's, but not Edit", async () => {
    app.state = applyReady(emptyState(), { ...ready(), servers: [{ id: 'S1', name: 'Main', icon_hash: null, owner_id: 'U1' } as never] })
    pages = [[build('M001')]]
    await show()
    rightClick(card('M001'))
    expect(items()).toEqual(['Open', 'Favourite', 'Archive', 'Delete…', 'Select'])
  })

  it('opens from the keyboard (the menu key or Shift+F10) with the focus on its first item', async () => {
    pages = [[mine('M002')]]
    await show()
    opener('M002').focus()
    opener('M002').dispatchEvent(new KeyboardEvent('keydown', { key: 'F10', shiftKey: true, bubbles: true, cancelable: true }))
    flushSync()
    await settle()
    expect(items()[0]).toBe('Open')
    expect(document.activeElement?.textContent?.trim()).toBe('Open')
    item('Open').click()
    flushSync()
    expect(document.querySelector('.build-menu')).toBeNull()
  })

  it('the menu key opens it too, in the list as well', async () => {
    pages = [[mine('M002')]]
    await show()
    ;[...document.querySelectorAll<HTMLButtonElement>('[role=radiogroup][aria-label="View"] [role=radio]')].find((b) => b.textContent === 'List')!.click()
    flushSync()
    const row = document.querySelector<HTMLElement>('.build-row .open')!
    row.dispatchEvent(new KeyboardEvent('keydown', { key: 'ContextMenu', bubbles: true, cancelable: true }))
    flushSync()
    expect(items()).toEqual(['Open', 'Favourite', 'Edit', 'Archive', 'Delete…', 'Select'])
  })

  it('Open opens the build page; Archive archives it', async () => {
    pages = [[mine('M002')]]
    await show()
    rightClick(card('M002'))
    item('Archive').click()
    flushSync()
    expect(calls('set_build_archived')[0][1]).toEqual({ messageId: 'M002', archived: true })
    rightClick(card('M002'))
    item('Open').click()
    flushSync()
    expect(document.querySelector('[data-build-id="M002"].build-card')).toBeNull()
  })

  it('Delete asks first, then deletes', async () => {
    pages = [[mine('M002')]]
    await show()
    rightClick(card('M002'))
    item('Delete').click()
    flushSync()
    const dialog = document.querySelector<HTMLElement>('[role=dialog]')!
    expect(dialog.textContent).toContain('Delete this build?')
    expect(calls('delete_build')).toHaveLength(0)
    ;[...dialog.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent?.trim() === 'Cancel')!.click()
    flushSync()
    expect(document.querySelector('[role=dialog]')).toBeNull()
    rightClick(card('M002'))
    item('Delete').click()
    flushSync()
    ;[...document.querySelectorAll<HTMLButtonElement>('[role=dialog] button')].find((b) => b.textContent?.trim() === 'Delete')!.click()
    await settle()
    expect(calls('delete_build')[0][1]).toEqual({ messageId: 'M002' })
    expect(calls('delete_many_builds')).toHaveLength(0)
  })
})

describe('BuildsChannel: multi-select', () => {
  const mine = (n: string, b: Partial<Build> = {}) => build(n, b, { author_id: 'U1' })
  const card = (n: string) => document.querySelector<HTMLElement>(`[data-build-id="${n}"]`)!
  const opener = (n: string) => card(n).querySelector<HTMLElement>('.open')!
  const click = (n: string, shiftKey = false) => {
    opener(n).dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true, shiftKey }))
    flushSync()
  }
  const checked = () => [...document.querySelectorAll<HTMLElement>('[data-build-id] .open[role=checkbox]')].filter((e) => e.getAttribute('aria-checked') === 'true').map((e) => e.closest<HTMLElement>('[data-build-id]')!.dataset.buildId)
  const bar = () => document.querySelector<HTMLElement>('[role=toolbar][aria-label="Selected builds"]')
  const barButton = (l: string) => [...(bar()?.querySelectorAll<HTMLButtonElement>('button') ?? [])].find((b) => (b.getAttribute('aria-label') ?? b.textContent?.trim()) === l)
  const status = () => document.querySelector('[data-testid="builds-selection-status"]')?.textContent?.trim()
  const four = () => [mine('M004'), mine('M003'), build('M002'), mine('M001')]

  it('Shift+click starts selecting with that build; the bar says how many', async () => {
    pages = [four()]
    await show()
    expect(bar()).toBeNull()
    click('M003', true)
    expect(bar()).not.toBeNull()
    expect(checked()).toEqual(['M003'])
    expect(status()).toBe('1 selected')
    // every build shows its check state, as a checkbox
    expect(document.querySelectorAll('[data-build-id] .open[role=checkbox]')).toHaveLength(4)
    expect(opener('M004').getAttribute('aria-checked')).toBe('false')
    // the build page didn't open
    expect(document.querySelectorAll('.build-card')).toHaveLength(4)
  })

  it('"Select" in the menu starts it too', async () => {
    pages = [four()]
    await show()
    card('M002').dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 5, clientY: 5 }))
    flushSync()
    ;[...document.querySelectorAll<HTMLElement>('.build-menu [role=menuitem]')].find((b) => b.textContent?.trim() === 'Select')!.click()
    flushSync()
    expect(checked()).toEqual(['M002'])
  })

  it('a click toggles, Shift+click adds a range, in both views', async () => {
    pages = [four()]
    await show()
    click('M003', true)
    click('M001', true)
    expect(checked()).toEqual(['M003', 'M002', 'M001'])
    click('M004')
    expect(checked()).toEqual(['M004', 'M003', 'M002', 'M001'])
    click('M004')
    expect(checked()).toEqual(['M003', 'M002', 'M001'])
    expect(status()).toBe('3 selected')
    // the list keeps it
    ;[...document.querySelectorAll<HTMLButtonElement>('[role=radiogroup][aria-label="View"] [role=radio]')].find((b) => b.textContent === 'List')!.click()
    flushSync()
    expect(document.querySelectorAll('.build-row .open[role=checkbox][aria-checked=true]')).toHaveLength(3)
  })

  it('Escape or Cancel leaves it', async () => {
    pages = [four()]
    await show()
    click('M004', true)
    opener('M004').dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))
    flushSync()
    expect(bar()).toBeNull()
    expect(document.querySelector('[data-build-id] .open[role=checkbox]')).toBeNull()
    click('M004', true)
    barButton('Cancel')!.click()
    flushSync()
    expect(bar()).toBeNull()
    // a plain click opens again
    click('M004')
    expect(document.querySelector('.build-card')).toBeNull()
  })

  it('Clear empties the selection but keeps selecting', async () => {
    pages = [four()]
    await show()
    click('M004', true)
    click('M003')
    barButton('Clear')!.click()
    flushSync()
    expect(checked()).toEqual([])
    expect(status()).toBe('0 selected')
    expect(bar()).not.toBeNull()
  })

  it('Archive sends the builds you may change in one batch and says how many were skipped', async () => {
    pages = [four()]
    invoke.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === 'list_builds') return list(args)
      if (cmd === 'build_facets') return { total: 0, games: [] }
      if (cmd === 'archive_many_builds') return { done: 2, skipped: 0 }
      return undefined
    })
    await show()
    click('M003', true)
    click('M001', true)
    // sam's M002 isn't yours: the bar says so before you act
    expect(bar()!.textContent).toContain('1 not yours')
    barButton('Archive')!.click()
    await settle()
    expect(calls('archive_many_builds')).toEqual([['archive_many_builds', { channelId: 'B1', body: { message_ids: ['M003', 'M001'], archived: true } }]])
    const { ui } = await import('../lib/store.svelte')
    expect(ui.notice).toBe("Archived 2 builds. 1 skipped: they aren't yours to change.")
    expect(bar()).toBeNull()
  })

  it('Restore shows only with an archived build selected, and restores', async () => {
    pages = [[mine('M002', { archived_at: '2026-10-05T00:00:00.000Z', archived_by: 'U1' }), mine('M001')]]
    invoke.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === 'list_builds') return list(args)
      if (cmd === 'archive_many_builds') return { done: 1, skipped: 0 }
      return cmd === 'build_facets' ? { total: 0, games: [] } : undefined
    })
    await show()
    click('M001', true)
    expect(barButton('Restore')).toBeUndefined()
    expect(barButton('Archive')).toBeDefined()
    click('M002')
    barButton('Restore')!.click()
    await settle()
    expect(calls('archive_many_builds')[0][1]).toEqual({ channelId: 'B1', body: { message_ids: ['M002'], archived: false } })
  })

  it('Delete asks with the count first, then deletes in one batch', async () => {
    pages = [four()]
    invoke.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === 'list_builds') return list(args)
      if (cmd === 'delete_many_builds') return { done: 2, skipped: 1 }
      return cmd === 'build_facets' ? { total: 0, games: [] } : undefined
    })
    await show()
    click('M004', true)
    click('M003')
    barButton('Delete')!.click()
    flushSync()
    const dialog = document.querySelector<HTMLElement>('[role=dialog]')!
    expect(dialog.textContent).toContain('Delete 2 builds?')
    expect(calls('delete_many_builds')).toHaveLength(0)
    ;[...dialog.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent?.trim() === 'Delete 2 builds')!.click()
    await settle()
    expect(calls('delete_many_builds')[0][1]).toEqual({ channelId: 'B1', body: { message_ids: ['M004', 'M003'] } })
    const { ui } = await import('../lib/store.svelte')
    // the server skipped one (gone or changed meanwhile): said apart from any not yours
    expect(ui.notice).toBe('Deleted 2 builds. 1 skipped: gone or changed meanwhile.')
  })

  it('an action none of the selection allows is disabled', async () => {
    pages = [four()]
    await show()
    click('M002', true)
    expect(barButton('Archive')!.disabled).toBe(true)
    expect(barButton('Delete')!.disabled).toBe(true)
  })

  it('the events keep the view and the count current', async () => {
    pages = [four()]
    await show()
    click('M004', true)
    click('M003')
    app.state = applyEvent(app.state, { t: 'BuildsArchived', d: { channel_id: 'B1', message_ids: ['M004'], archived_at: '2026-10-06T00:00:00.000Z', archived_by: 'U1' } }, Date.now())
    flushSync()
    expect(card('M004')).toBeNull()
    expect(status()).toBe('1 selected')
    app.state = applyEvent(app.state, { t: 'MessageDeleted', d: { channel_id: 'B1', message_id: 'M003', author_id: 'U1', mentions: [], thread_root_id: null } }, Date.now())
    flushSync()
    expect(status()).toBe('0 selected')
  })

  it('a build that leaves the view leaves the selection: back again, it is not still selected', async () => {
    pages = [four()]
    await show()
    click('M004', true)
    click('M003')
    app.state = applyEvent(app.state, { t: 'BuildsArchived', d: { channel_id: 'B1', message_ids: ['M004'], archived_at: '2026-10-06T00:00:00.000Z', archived_by: 'U1' } }, Date.now())
    flushSync()
    app.state = applyEvent(app.state, { t: 'MessageUpdated', d: { message: mine('M004') } }, Date.now())
    flushSync()
    await settle()
    flushSync()
    expect(card('M004')).not.toBeNull()
    expect(checked()).toEqual(['M003'])
    expect(status()).toBe('1 selected')
  })

  it('while selecting, Tab skips the header under the bar (inert), and is back after', async () => {
    pages = [four()]
    await show()
    // (set as the property or the attribute)
    const inert = (el: HTMLElement | null) => {
      for (let e = el; e; e = e.parentElement) if (e.hasAttribute('inert') || (e as HTMLElement & { inert?: boolean }).inert === true) return true
      return false
    }
    const head = () => document.querySelector<HTMLElement>('.builds-head')!
    expect(inert(head())).toBe(false)
    click('M004', true)
    expect(inert(head())).toBe(true)
    // the bar itself isn't
    expect(inert(bar())).toBe(false)
    barButton('Cancel')!.click()
    flushSync()
    expect(inert(head())).toBe(false)
  })

  it("while selecting, a click on a build's star selects it rather than starring it", async () => {
    pages = [four()]
    await show()
    click('M004', true)
    for (const v of ['Gallery', 'List']) {
      ;[...document.querySelectorAll<HTMLButtonElement>('[role=radiogroup][aria-label="View"] [role=radio]')].find((b) => b.textContent === v)!.click()
      flushSync()
      card('M002').querySelector<HTMLButtonElement>('.star')!.click()
      flushSync()
      expect(checked()).toContain('M002')
      card('M002').querySelector<HTMLButtonElement>('.star')!.click()
      flushSync()
      expect(checked()).not.toContain('M002')
    }
    await settle()
    expect(calls('set_build_favourite')).toHaveLength(0)
  })

  it('narrow, the actions past the first and Clear move into a "More" menu', async () => {
    pages = [[mine('M002', { archived_at: '2026-10-05T00:00:00.000Z', archived_by: 'U1' }), mine('M001', { favourite: true })]]
    await show()
    click('M002', true)
    click('M001')
    const more = barButton('More actions')!
    expect(more.getAttribute('aria-haspopup')).toBe('menu')
    more.click()
    flushSync()
    const items = [...document.querySelectorAll<HTMLElement>('.select-more [role=menuitem]')].map((b) => b.textContent?.trim())
    expect(items).toEqual(['Favourite', 'Unfavourite', 'Archive', 'Restore', 'Delete', 'Clear'])
    // Escape closes the menu and keeps selecting
    document.querySelector<HTMLElement>('.select-more [role=menuitem]')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))
    flushSync()
    expect(document.querySelector('.select-more')).toBeNull()
    expect(bar()).not.toBeNull()
    // Clear from it
    more.click()
    flushSync()
    ;[...document.querySelectorAll<HTMLElement>('.select-more [role=menuitem]')].find((b) => b.textContent?.trim() === 'Clear')!.click()
    flushSync()
    expect(status()).toBe('0 selected')
    // the CSS: wide, More is hidden; narrow, the extra actions and Clear go and More shows; narrowest, every action
    const css = readFileSync(resolve(__dirname, 'BuildsSelectBar.svelte'), 'utf8').split('<style>')[1]
    expect(css).toMatch(/\.more \{[^}]*display: none/)
    expect(css).toMatch(/@container \(max-width: [34]\d\dpx\) \{[^}]*\.extra, \.clear \{ display: none; \}[^}]*\.more \{ display: (inline-)?(flex|grid|block)/)
    expect(css).toMatch(/@container \(max-width: 2\d\dpx\) \{[^}]*\.act \{ display: none; \}/)
  })

  it('the bar sits over the header (no jump), uses colour tokens only and announces the count', () => {
    const src = (p: string) => readFileSync(resolve(__dirname, p), 'utf8')
    const bar = src('BuildsSelectBar.svelte')
    expect(bar).toMatch(/\.select-bar \{[^}]*position: absolute;[^}]*inset: 0/)
    expect(bar).toMatch(/role="status"/)
    for (const f of ['BuildsSelectBar.svelte', 'BuildCard.svelte', 'BuildRow.svelte', 'BuildMenu.svelte']) {
      const style = src(f).split('<style>')[1] ?? ''
      expect(style).not.toMatch(/#[0-9a-fA-F]{3,8}\b|rgba?\(/)
    }
  })
})

describe('BuildsChannel: favourites', () => {
  const fav = (n: string, b: Partial<Build> = {}, m: Partial<Message> = {}) => build(n, { favourite: true, ...b }, m)
  const star = (n: string) => document.querySelector<HTMLButtonElement>(`[data-build-id="${n}"] .star`)!
  const sections = () => {
    const favs = [...document.querySelectorAll<HTMLElement>('[data-section="favourites"] [data-build-id]')].map((e) => e.dataset.buildId)
    const rest = [...document.querySelectorAll<HTMLElement>('[data-section="rest"] [data-build-id]')].map((e) => e.dataset.buildId)
    return { favs, rest }
  }
  const pickView = (l: string) => {
    ;[...document.querySelectorAll<HTMLButtonElement>('[role=radiogroup][aria-label="View"] [role=radio]')].find((b) => b.textContent === l)!.click()
    flushSync()
  }

  it('every card and row has a star, named for what it does', async () => {
    pages = [[build('M002'), fav('M001')]]
    await show()
    for (const v of ['Gallery', 'List']) {
      pickView(v)
      expect(star('M002').getAttribute('aria-label')).toBe('Favourite')
      expect(star('M002').getAttribute('aria-pressed')).toBe('false')
      expect(star('M001').getAttribute('aria-label')).toBe('Remove from favourites')
      expect(star('M001').getAttribute('aria-pressed')).toBe('true')
      // it's its own button, never inside the one that opens the build
      expect(star('M002').closest('.open')).toBeNull()
    }
  })

  it('shows on hover or focus, always when starred, and never moves anything (CSS)', () => {
    const src = (p: string) => readFileSync(resolve(__dirname, p), 'utf8')
    for (const f of ['BuildCard.svelte', 'BuildRow.svelte']) {
      const css = src(f).split('<style>')[1]
      expect(css).toMatch(/\.star \{[^}]*position: absolute;[^}]*opacity: 0/)
      expect(css).toMatch(/:hover \.star|:focus-within \.star/)
      expect(css).toMatch(/\.star\.on \{[^}]*opacity: 1/)
      expect(css).not.toMatch(/#[0-9a-fA-F]{3,8}\b|rgba?\(/)
    }
  })

  it('the star stars and unstars it', async () => {
    pages = [[build('M002'), fav('M001')]]
    await show()
    star('M002').click()
    star('M001').click()
    await settle()
    expect(calls('set_build_favourite').map((c) => c[1])).toEqual([{ messageId: 'M002', favourite: true }, { messageId: 'M001', favourite: false }])
    // the build page didn't open
    expect(document.querySelectorAll('.build-card')).toHaveLength(2)
  })

  it('Favourite and Unfavourite in the right-click menu', async () => {
    pages = [[build('M002'), fav('M001')]]
    await show()
    const menuItem = (n: string, l: string) => {
      document.querySelector<HTMLElement>(`[data-build-id="${n}"]`)!.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 5, clientY: 5 }))
      flushSync()
      return [...document.querySelectorAll<HTMLElement>('.build-menu [role=menuitem]')].find((b) => b.textContent?.trim() === l)
    }
    menuItem('M002', 'Favourite')!.click()
    flushSync()
    menuItem('M001', 'Unfavourite')!.click()
    await settle()
    expect(calls('set_build_favourite').map((c) => c[1])).toEqual([{ messageId: 'M002', favourite: true }, { messageId: 'M001', favourite: false }])
  })

  it('Favourite in the selection bar stars every selected build, anyone\'s, in one batch', async () => {
    pages = [[build('M003'), build('M002', {}, { author_id: 'U1' }), fav('M001')]]
    invoke.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === 'list_builds') return list(args)
      if (cmd === 'favourite_many_builds') return { done: 2, skipped: 0 }
      return cmd === 'build_facets' ? { total: 0, games: [] } : undefined
    })
    await show()
    const click = (n: string, shiftKey = false) => {
      document.querySelector<HTMLElement>(`[data-build-id="${n}"] .open`)!.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true, shiftKey }))
      flushSync()
    }
    click('M003', true)
    click('M002')
    click('M001')
    const btn = (l: string) => [...document.querySelectorAll<HTMLButtonElement>('[role=toolbar] button')].find((b) => b.getAttribute('aria-label') === l)
    expect(btn('Unfavourite')).toBeDefined()
    btn('Favourite')!.click()
    await settle()
    expect(calls('favourite_many_builds')[0][1]).toEqual({ channelId: 'B1', body: { message_ids: ['M003', 'M002'], favourite: true } })
    const { ui } = await import('../lib/store.svelte')
    expect(ui.notice).toBe('Added 2 builds to your favourites.')
  })

  it('favourites sit in their own section at the top, then a separator, then the rest (never twice), in both views', async () => {
    pages = [[build('M004'), fav('M003'), build('M002'), fav('M001')]]
    favAnswer = () => [fav('M003'), fav('M001')]
    await show()
    for (const v of ['Gallery', 'List']) {
      pickView(v)
      expect(sections()).toEqual({ favs: ['M003', 'M001'], rest: ['M004', 'M002'] })
      const head = document.querySelector<HTMLElement>('[data-section="favourites"] .section-head')!
      expect(head.textContent?.trim()).toBe('Favourites')
      expect(document.querySelector('[data-section="favourites"] + [role=separator]')).not.toBeNull()
    }
  })

  it('a star or unstar moves the build between the sections', async () => {
    pages = [[build('M002'), build('M001')]]
    await show()
    expect(document.querySelector('[data-section="favourites"]')).toBeNull()
    expect(document.querySelector('.feed .sep[role=separator]')).toBeNull()
    app.state = applyEvent(app.state, { t: 'BuildFavouriteChanged', d: { user_id: 'U1', channel_id: 'B1', message_ids: ['M001'], favourite: true } }, Date.now())
    flushSync()
    expect(sections()).toEqual({ favs: ['M001'], rest: ['M002'] })
    app.state = applyEvent(app.state, { t: 'BuildFavouriteChanged', d: { user_id: 'U1', channel_id: 'B1', message_ids: ['M001'], favourite: false } }, Date.now())
    flushSync()
    expect(document.querySelector('[data-section="favourites"]')).toBeNull()
  })

  // the sections split the feed of the filter in view (game, Show archived, words), so they follow it
  it('the section follows Show archived (and any filter: it splits the feed in view)', async () => {
    answer = (a) => (a.archived ? [fav('M004', { archived_at: '2026-10-05T00:00:00.000Z', archived_by: 'U2' }), build('M002')] : [build('M002')])
    favAnswer = (a) => (a.archived ? [fav('M004', { archived_at: '2026-10-05T00:00:00.000Z', archived_by: 'U2' })] : [])
    await show()
    // no favourites among the live builds: no section
    expect(document.querySelector('[data-section="favourites"]')).toBeNull()
    const toggle = document.querySelector<HTMLElement>('[role=switch]')
    toggle?.click()
    flushSync()
    await settle()
    flushSync()
    expect(sections()).toEqual({ favs: ['M004'], rest: ['M002'] })
  })

  it('the Favourites section loads from its own feed: a starred build on no loaded page still shows, never twice', async () => {
    const page = Array.from({ length: 30 }, (_, i) => build(id(200 - i)))
    page[3] = fav(id(197))
    pages = [page]
    favAnswer = () => [fav(id(197)), fav('M005')]
    await show()
    expect(favCalls()[0][1]).toEqual({ channelId: 'B1', game: null, archived: false, favourite: true, q: null, before: null })
    const { favs, rest } = sections()
    expect(favs).toEqual([id(197), 'M005'])
    expect(rest).toHaveLength(29)
    expect(rest).not.toContain(id(197))
  })

  it('the Favourites feed follows the filter in view: game, Show archived and words', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] })
    favAnswer = (a) => (a.q === 'raven' ? [fav('M002', { title: 'Raven' })] : [])
    answer = () => [build('M003')]
    cmp = mount(BuildsChannel, { target: document.body, props: { channelId: 'B1', serverId: 'S1' } })
    flushSync()
    await vi.advanceTimersByTimeAsync(0)
    const input = document.querySelector<HTMLInputElement>('.builds-head input[type=search]')!
    input.value = 'raven'
    input.dispatchEvent(new Event('input', { bubbles: true }))
    await vi.advanceTimersByTimeAsync(1000)
    flushSync()
    expect(favCalls().at(-1)![1]).toMatchObject({ q: 'raven', favourite: true })
    expect(sections().favs).toEqual(['M002'])
  })

  it('a build starred in another session, on no page here, is fetched and joins the section', async () => {
    pages = [[build('M009')]]
    invoke.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === 'list_builds') return list(args)
      if (cmd === 'get_build') return fav('M001')
      return cmd === 'build_facets' ? { total: 0, games: [] } : undefined
    })
    await show()
    app.state = applyEvent(app.state, { t: 'BuildFavouriteChanged', d: { user_id: 'U1', channel_id: 'B1', message_ids: ['M001'], favourite: true } }, Date.now())
    flushSync()
    await settle()
    flushSync()
    expect(calls('get_build')).toEqual([['get_build', { messageId: 'M001' }]])
    expect(sections()).toEqual({ favs: ['M001'], rest: ['M009'] })
  })

  it('a starred build archived out of the view and restored comes back starred (fetched as yours when no copy is held)', async () => {
    pages = [[fav('M002'), build('M001')]]
    favAnswer = () => [fav('M002')]
    invoke.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === 'list_builds') return list(args)
      if (cmd === 'get_build') return fav('M002')
      return cmd === 'build_facets' ? { total: 0, games: [] } : undefined
    })
    await show()
    expect(sections()).toEqual({ favs: ['M002'], rest: ['M001'] })
    app.state = applyEvent(app.state, { t: 'BuildsArchived', d: { channel_id: 'B1', message_ids: ['M002'], archived_at: '2026-10-06T00:00:00.000Z', archived_by: 'U2' } }, Date.now())
    flushSync()
    expect(sections()).toEqual({ favs: [], rest: ['M001'] })
    // the restore's event carries no one's star
    app.state = applyEvent(app.state, { t: 'MessageUpdated', d: { message: build('M002') } }, Date.now())
    flushSync()
    await settle()
    flushSync()
    expect(calls('get_build')).toEqual([['get_build', { messageId: 'M002' }]])
    expect(sections()).toEqual({ favs: ['M002'], rest: ['M001'] })
  })

  it('the section head and separator fit at any width, in tokens (CSS)', () => {
    const css = readFileSync(resolve(__dirname, 'BuildsChannel.svelte'), 'utf8').split('<style>')[1]
    expect(css).toMatch(/\.section-head \{[^}]*white-space: nowrap;[^}]*overflow: hidden;[^}]*text-overflow: ellipsis/)
    expect(css).not.toMatch(/#[0-9a-fA-F]{3,8}\b|rgba?\(/)
  })
})
