// @vitest-environment jsdom
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { compile } from 'svelte/compiler'
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Message } from '../lib/protocol/Message'
import { FakeIntersectionObserver, fakeMatchMedia } from '../lib/testMotion'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))
vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)

import ClipFeed from './ClipFeed.svelte'
import { app } from '../lib/store.svelte'
import { applyEvent, applyReady, emptyState } from '../lib/state'
import { feedOf } from '../lib/clips'
import { claim, playback, release } from '../lib/clipPlayback.svelte'
import { reactiveProps } from '../lib/testProps.svelte'
import { testMe, testReady } from '../lib/testReady'
import { windowFocus } from '../lib/windowFocus.svelte'

const sam = { id: 'U2', username: 'sam', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const ready = (lastRead: string | null = null) =>
  testReady([{ id: 'C1', server_id: 'S1', kind: 'clips', name: 'clips', position: 0 }], {
    members: [{ server_id: 'S1', members: [{ user: testMe, nickname: null }, { user: sam, nickname: null }] }],
    read_states: [{ channel_id: 'C1', last_read_message_id: lastRead, unread: 0, mentions: 0 }],
    games: [{ id: 'G1', server_id: 'S1', name: 'Apex', created_by: 'U1', created_at: '2026-10-01T00:00:00Z' }],
  })

const clip = (id: string, over: Partial<Message> = {}): Message => ({
  id, channel_id: 'C1', author_id: 'U2', kind: 'normal', content: `caption ${id}`, reply_to_id: null,
  created_at: '2026-10-01T10:00:00.000Z', edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null,
  quote: null, qoty_card: null,
  clip: { thumb_hash: id.toLowerCase().padEnd(64, '0'), duration_ms: 65000, width: 1280, height: 720, size: 1000, game_id: 'G1', reply_count: 0, last_reply_id: null, replies_seen: null },
  ...over,
})
const id = (n: number) => `M${String(n).padStart(3, '0')}`
/** Clips `from` down to `to` (newest first, as the server pages them). */
const range = (from: number, to: number) => Array.from({ length: from - to + 1 }, (_, i) => clip(id(from - i)))

let pages: Message[][] = []
let pendingPage: ((m: Message[]) => void) | null = null
let hold = false
let cmp: ReturnType<typeof mount> | undefined
const settle = () => new Promise((r) => setTimeout(r, 0))
const calls = (cmd: string) => invoke.mock.calls.filter((c) => c[0] === cmd)
const order = () => [...document.querySelectorAll<HTMLElement>('[data-clip-id]')].map((e) => e.dataset.clipId)
const card = (cid: string) => document.querySelector<HTMLElement>(`[data-clip-id="${cid}"]`)!

beforeEach(() => {
  invoke.mockReset()
  pages = []
  hold = false
  pendingPage = null
  invoke.mockImplementation(async (cmd: string) => {
    if (cmd === 'list_clips') {
      if (hold) return new Promise<Message[]>((r) => (pendingPage = r))
      return pages.shift() ?? []
    }
    if (cmd === 'clip_url') return { url: 'http://127.0.0.1:7895/clips/k.mp4?sig=1', expires_at: '2026-10-28T11:00:00Z' }
    if (cmd === 'edit_clip') return clip('M004')
    return undefined
  })
  app.state = applyReady(emptyState(), ready('M002'))
  release(claim('reset', () => {}))
  playback.volume = 1
  playback.muted = false
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
  vi.useRealTimers()
})

async function show(props: { layout: 'list' | 'wall'; filter?: { game?: string; author?: string } }) {
  const p = reactiveProps({ channelId: 'C1', filter: props.filter ?? {}, layout: props.layout })
  cmp = mount(ClipFeed, { target: document.body, props: p })
  flushSync()
  await settle()
  flushSync()
  await tick()
  return p
}

describe('ClipFeed', () => {
  it('list shows newest at the bottom with a NEW line', async () => {
    pages = [[clip('M004'), clip('M003'), clip('M002'), clip('M001')]]
    await show({ layout: 'list' })
    expect(calls('list_clips')[0][1]).toEqual({ channelId: 'C1', before: null, game: null, author: null })
    expect(order()).toEqual(['M001', 'M002', 'M003', 'M004'])
    const line = document.querySelector('[role="separator"]')!
    expect(line.textContent).toBe('NEW')
    expect(line.nextElementSibling?.getAttribute('data-clip-id')).toBe('M003')
    // a list item: name, game tag, caption, the player and the replies link
    const item = card('M004')
    expect(item.textContent).toContain('sam')
    expect(item.querySelector('.game')?.textContent).toBe('Apex')
    expect(item.textContent).toContain('caption M004')
    expect(item.querySelector('[aria-label="Clip player"]')).not.toBeNull()
  })

  it('shows "View N replies" and opens them', async () => {
    const onOpenReplies = vi.fn()
    pages = [[clip('M002', { clip: { ...clip('M002').clip!, reply_count: 1 } }), clip('M001', { clip: { ...clip('M001').clip!, reply_count: 3 } })]]
    cmp = mount(ClipFeed, { target: document.body, props: { channelId: 'C1', filter: {}, layout: 'list', onOpenReplies } })
    flushSync()
    await settle()
    flushSync()
    expect(card('M002').querySelector('.replies')?.textContent?.trim()).toBe('View 1 reply')
    const btn = card('M001').querySelector<HTMLButtonElement>('.replies')!
    expect(btn.textContent?.trim()).toBe('View 3 replies')
    btn.click()
    expect(onOpenReplies).toHaveBeenCalledWith(expect.objectContaining({ id: 'M001' }))
  })

  it('wall shows newest first three across', async () => {
    pages = [[clip('M004'), clip('M003'), clip('M002'), clip('M001')]]
    await show({ layout: 'wall' })
    expect(order()).toEqual(['M004', 'M003', 'M002', 'M001'])
    const thumb = card('M004').querySelector('img')!
    expect(thumb.getAttribute('loading')).toBe('lazy')
    expect(thumb.getAttribute('src')).toContain('m004')
    expect(card('M004').querySelector('.duration')?.textContent).toBe('1:05')
    expect(card('M004').querySelector('button[aria-label="Play full screen"]')).not.toBeNull()
    // the grid's own CSS: three columns, two under 900 px
    const grid = document.querySelector<HTMLElement>('.grid')!
    const hash = [...grid.classList].find((c) => c.startsWith('svelte-'))!
    const src = readFileSync(resolve(__dirname, 'ClipFeed.svelte'), 'utf8')
    const css = compile(src, { filename: 'ClipFeed.svelte', css: 'external', cssHash: () => hash }).css!.code
    const style = document.createElement('style')
    style.textContent = css
    document.head.append(style)
    expect(getComputedStyle(grid).gridTemplateColumns).toBe('repeat(3, minmax(0, 1fr))')
    expect(css).toMatch(/@media \(max-width: ?899\.98px\)\s*\{\s*\.grid\.[\w-]+\s*\{\s*grid-template-columns: ?repeat\(2, ?minmax\(0, ?1fr\)\)/)
    style.remove()
  })

  it('scrolling to the far end loads the next page', async () => {
    pages = [range(130, 101)]
    await show({ layout: 'wall' })
    expect(order()).toHaveLength(30)
    const sentinel = document.querySelector('.sentinel')!
    // it sits at the far end: the bottom of the wall
    expect(sentinel.previousElementSibling?.classList.contains('grid')).toBe(true)
    hold = true
    FakeIntersectionObserver.set(sentinel, true)
    FakeIntersectionObserver.set(sentinel, true) // one page at a time
    flushSync()
    await settle()
    expect(calls('list_clips')).toHaveLength(2)
    expect(calls('list_clips')[1][1]).toEqual({ channelId: 'C1', before: 'M101', game: null, author: null })
    pendingPage!(range(100, 95))
    await settle()
    flushSync()
    expect(order()).toHaveLength(36)
    expect(order().at(-1)).toBe('M095')
    // the start is reached: no more fetches, even with the sentinel still showing
    FakeIntersectionObserver.set(sentinel, true)
    await settle()
    expect(calls('list_clips')).toHaveLength(2)
  })

  it('the list pages older at the top and passes the filter', async () => {
    pages = [range(130, 101).map((m) => ({ ...m, author_id: 'U2' }))]
    await show({ layout: 'list', filter: { game: 'G1', author: 'U2' } })
    expect(calls('list_clips')[0][1]).toEqual({ channelId: 'C1', before: null, game: 'G1', author: 'U2' })
    const sentinel = document.querySelector('.sentinel')!
    expect(sentinel.nextElementSibling?.getAttribute('data-clip-id') ?? sentinel.nextElementSibling?.querySelector('[data-clip-id]')?.getAttribute('data-clip-id')).toBe('M101')
    pages = [range(100, 99)]
    FakeIntersectionObserver.set(sentinel, true)
    await settle()
    flushSync()
    expect(calls('list_clips')[1][1]).toEqual({ channelId: 'C1', before: 'M101', game: 'G1', author: 'U2' })
    expect(order()[0]).toBe('M099')
    expect(feedOf(app.state, 'C1', { game: 'G1', author: 'U2' })?.start).toBe(true)
  })

  it('"Try again" after a failed first page fetches it again', async () => {
    let fail = true
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'list_clips') {
        if (fail) throw 'Server unreachable'
        return [clip('M002'), clip('M001')]
      }
      return undefined
    })
    await show({ layout: 'wall' })
    expect(calls('list_clips')).toHaveLength(1)
    const alert = document.querySelector('[role="alert"]')!
    expect(alert.textContent).toContain('Server unreachable')
    await settle()
    expect(calls('list_clips')).toHaveLength(1) // no retry loop on its own
    fail = false
    ;[...alert.querySelectorAll('button')].find((b) => b.textContent === 'Try again')!.click()
    flushSync()
    await settle()
    flushSync()
    expect(calls('list_clips')).toHaveLength(2)
    expect(order()).toEqual(['M002', 'M001'])
    expect(document.querySelector('[role="alert"]')).toBeNull()
  })

  it('switching back to a filter whose first page failed fetches it again', async () => {
    let failGame = true
    invoke.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd !== 'list_clips') return undefined
      const game = (args as { game: string | null }).game
      if (game === 'G1' && failGame) throw 'Server unreachable'
      return game === 'G1' ? [clip('M003')] : [clip('M002'), clip('M001')]
    })
    const p = await show({ layout: 'wall', filter: { game: 'G1' } })
    expect(document.querySelector('[role="alert"]')).not.toBeNull()
    p.filter = {}
    flushSync()
    await settle()
    flushSync()
    expect(order()).toEqual(['M002', 'M001'])
    failGame = false
    p.filter = { game: 'G1' }
    flushSync()
    await settle()
    flushSync()
    expect(calls('list_clips').filter((c) => (c[1] as { game: string | null }).game === 'G1')).toHaveLength(2)
    expect(order()).toEqual(['M003'])
  })

  it('switching view keeps the clip in view', async () => {
    pages = [[clip('M004'), clip('M003'), clip('M002'), clip('M001')]]
    const p = await show({ layout: 'list' })
    const scroller = document.querySelector<HTMLElement>('.feed')!
    scroller.getBoundingClientRect = () => ({ top: 100, bottom: 500 }) as DOMRect
    // list order top to bottom: M001 (cut off above), M002 (cut off), M003 (whole), M004 (whole)
    const tops: Record<string, number> = { M001: -200, M002: 50, M003: 200, M004: 350 }
    for (const [cid, top] of Object.entries(tops)) card(cid).getBoundingClientRect = () => ({ top, bottom: top + 140 }) as DOMRect
    const scrolled: string[] = []
    Element.prototype.scrollIntoView = function (this: Element) { scrolled.push((this as HTMLElement).dataset.clipId ?? '') }
    p.layout = 'wall'
    flushSync()
    await tick()
    await settle()
    expect(order()[0]).toBe('M004') // the wall now
    expect(scrolled).toEqual(['M003'])
  })

  it('clicking a thumbnail plays it with one player at a time', async () => {
    pages = [[clip('M004'), clip('M003')]]
    await show({ layout: 'wall' })
    expect(document.querySelectorAll('[aria-label="Clip player"]')).toHaveLength(0)
    card('M004').querySelector<HTMLButtonElement>('button.thumb')!.click()
    flushSync()
    await settle()
    expect(card('M004').querySelector('[aria-label="Clip player"]')).not.toBeNull()
    expect(calls('clip_url').map((c) => c[1])).toEqual([{ messageId: 'M004' }]) // autoplay, with sound
    expect(playback.muted).toBe(false)
    card('M003').querySelector<HTMLButtonElement>('button.thumb')!.click()
    flushSync()
    await settle()
    expect(document.querySelectorAll('[aria-label="Clip player"]')).toHaveLength(1)
    expect(card('M003').querySelector('[aria-label="Clip player"]')).not.toBeNull()
    expect(card('M004').querySelector('button.thumb')).not.toBeNull()
  })

  it('unseen clips have a new badge in the wall', async () => {
    // read up to M002; M005 is mine, so never new to me
    pages = [[clip('M005', { author_id: 'U1' }), clip('M004'), clip('M003'), clip('M002'), clip('M001')]]
    await show({ layout: 'wall' })
    const badged = order().filter((cid) => card(cid!).querySelector('.badge-new'))
    expect(badged).toEqual(['M004', 'M003'])
    expect(card('M004').querySelector('.badge-new')?.textContent).toBe('new')
  })

  it('marks read when the newest clip is on screen and the window is focused', async () => {
    vi.useFakeTimers()
    pages = [[clip('M004'), clip('M003')]]
    cmp = mount(ClipFeed, { target: document.body, props: { channelId: 'C1', filter: {}, layout: 'list' } })
    flushSync()
    await vi.advanceTimersByTimeAsync(0)
    flushSync()
    windowFocus.focused = false
    FakeIntersectionObserver.set(card('M004'), true)
    flushSync()
    await vi.advanceTimersByTimeAsync(1000)
    expect(calls('mark_read')).toHaveLength(0) // not focused
    windowFocus.focused = true
    flushSync()
    await vi.advanceTimersByTimeAsync(1000)
    expect(calls('mark_read').map((c) => c[1])).toEqual([{ channelId: 'C1', messageId: 'M004' }])
  })

  it('edit and delete only for the poster', async () => {
    pages = [[clip('M004', { author_id: 'U1' }), clip('M003')]]
    await show({ layout: 'list' })
    expect(card('M003').querySelector('button[aria-label="Clip actions"]')).toBeNull()
    const open = () => {
      card('M004').querySelector<HTMLButtonElement>('button[aria-label="Clip actions"]')!.click()
      flushSync()
    }
    const item = (label: string) => [...document.querySelectorAll<HTMLElement>('[role="menuitem"]')].find((b) => b.textContent?.trim() === label)!
    // Edit: caption and game, through a dialog
    open()
    item('Edit').click()
    flushSync()
    const dialog = document.querySelector<HTMLElement>('[role="dialog"][aria-label="Edit clip"]')!
    expect(dialog.querySelector<HTMLInputElement>('input[aria-label="Game"]')!.value).toBe('Apex')
    const caption = dialog.querySelector<HTMLTextAreaElement>('textarea[aria-label="Caption"]')!
    expect(caption.value).toBe('caption M004')
    caption.value = '  what a shot  '
    caption.dispatchEvent(new Event('input', { bubbles: true }))
    flushSync()
    ;[...dialog.querySelectorAll('button')].find((b) => b.textContent?.trim() === 'Save')!.click()
    await settle()
    flushSync()
    expect(calls('edit_clip').map((c) => c[1])).toEqual([{ messageId: 'M004', body: { caption: 'what a shot', game_id: 'G1' } }])
    expect(document.querySelector('[aria-label="Edit clip"]')).toBeNull()
    // Delete asks first
    open()
    item('Delete').click()
    flushSync()
    await settle()
    const ask = document.querySelector<HTMLElement>('[role="alertdialog"]')!
    expect(document.activeElement).toBe([...ask.querySelectorAll('button')].find((b) => b.textContent?.trim() === 'Cancel'))
    expect(ask.textContent).toContain('Delete this clip? Its replies go too.')
    ;[...ask.querySelectorAll('button')].find((b) => b.textContent?.trim() === 'Cancel')!.click()
    flushSync()
    expect(calls('delete_message')).toHaveLength(0)
    open()
    item('Delete').click()
    flushSync()
    ;[...document.querySelector<HTMLElement>('[role="alertdialog"]')!.querySelectorAll('button')].find((b) => b.textContent?.trim() === 'Delete')!.click()
    await settle()
    expect(calls('delete_message').map((c) => c[1])).toEqual([{ messageId: 'M004' }])
    // the gateway's delete takes it out of the feed
    app.state = applyEvent(app.state, { t: 'MessageDeleted', d: { channel_id: 'C1', message_id: 'M004', author_id: 'U1', mentions: [], thread_root_id: null } }, Date.now())
    flushSync()
    expect(order()).toEqual(['M003'])
  })

  describe.each(['list', 'wall'] as const)('starting on a clip (%s)', (layout) => {
    it('a clip with no replies has Reply, which opens its panel; a reply then shows "View 1 reply"', async () => {
      const onOpenReplies = vi.fn()
      pages = [[clip('M001')]]
      cmp = mount(ClipFeed, { target: document.body, props: { channelId: 'C1', filter: {}, layout, onOpenReplies } })
      flushSync()
      await settle()
      flushSync()
      expect(card('M001').querySelector('.replies')).toBeNull()
      card('M001').querySelector<HTMLButtonElement>('button[aria-label="Reply"]')!.click()
      expect(onOpenReplies).toHaveBeenCalledWith(expect.objectContaining({ id: 'M001' }))
      const reply: Message = { ...clip('R001'), content: 'nice', clip: null, thread_root_id: 'M001' }
      app.state = applyEvent(app.state, { t: 'MessageCreated', d: { message: reply, nonce: null } }, Date.now())
      flushSync()
      expect(card('M001').querySelector('.replies')?.textContent?.trim()).toMatch(/^View 1 reply\b/)
    })

    it('no Reply where nothing opens replies', async () => {
      pages = [[clip('M001')]]
      await show({ layout })
      expect(card('M001').querySelector('button[aria-label="Reply"]')).toBeNull()
    })

    it('a clip with no reactions can get its first one', async () => {
      pages = [[clip('M001')]]
      await show({ layout })
      const add = card('M001').querySelectorAll<HTMLButtonElement>('button[aria-label="Add reaction"]')
      expect(add).toHaveLength(1)
      add[0].click()
      flushSync()
      ;[...document.querySelectorAll<HTMLButtonElement>('.cell')].find((c) => c.dataset.key === '👍')!.click()
      flushSync()
      await settle()
      flushSync()
      expect(calls('add_reaction').map((c) => c[1])).toEqual([{ messageId: 'M001', emoji: '👍' }])
      expect(card('M001').querySelector('.reactions .chip.mine')?.textContent?.trim()).toBe('1')
    })

    it('your own reaction on a clip comes off with a click', async () => {
      pages = [[clip('M001', { reactions: [{ emoji: '🔥', count: 2, me: true }] })]]
      await show({ layout })
      card('M001').querySelector<HTMLButtonElement>('.reactions .chip.mine')!.click()
      flushSync()
      await settle()
      flushSync()
      expect(calls('remove_reaction').map((c) => c[1])).toEqual([{ messageId: 'M001', emoji: '🔥' }])
      expect(card('M001').querySelector('.reactions .chip.mine')).toBeNull()
      expect(card('M001').querySelector('.reactions .chip')?.textContent?.trim()).toBe('1')
    })

    it('suggests your usual picks it does not have yet, any number per person', async () => {
      app.state = { ...app.state, frequentEmoji: ['👍', '🔥', '😂', '🎉'] }
      pages = [[clip('M001', { reactions: [{ emoji: '🔥', count: 1, me: true }] })]]
      await show({ layout })
      const sug = [...card('M001').querySelectorAll<HTMLButtonElement>('.chip.suggest')]
      expect(sug.map((b) => b.getAttribute('aria-label'))).toEqual(['React with 👍', 'React with 😂', 'React with 🎉'])
      sug[0].click()
      await settle()
      flushSync()
      // a clip keeps your other reactions (no take-back as on a quote)
      expect(calls('add_reaction').map((c) => c[1])).toEqual([{ messageId: 'M001', emoji: '👍' }])
      expect(calls('remove_reaction')).toHaveLength(0)
    })
  })
})
