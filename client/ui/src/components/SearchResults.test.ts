// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: unknown) => invoke(cmd, args) }))

import SearchResults from './SearchResults.svelte'
import { app, clock, ui } from '../lib/store.svelte'
import { closeSearch, search } from '../lib/search.svelte'
import { applyReady, emptyState } from '../lib/state'
import { testMe, testReady } from '../lib/testReady'
import type { Message } from '../lib/protocol/Message'
import type { SearchHit } from '../lib/protocol/SearchHit'

const sam = { id: 'U2', username: 'sam', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const msg = (id: string, channel = 'C1', more: Partial<Message> = {}): Message => ({
  id, channel_id: channel, author_id: 'U2', kind: 'normal', content: `text ${id}`,
  reply_to_id: null, created_at: new Date(Date.parse(clock.nowIso) - 3 * 3600_000).toISOString(), edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null, quote: null, qoty_card: null, clip: null, embeds: [],
  ...more,
})
const hit = (m: Message, text = m.content, highlights: SearchHit['highlights'] = []): SearchHit => ({ message: m, text, highlights, root: null })

let cmp: ReturnType<typeof mount> | undefined
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})
beforeEach(() => {
  invoke.mockReset()
  closeSearch()
  ui.goTo = null
  ui.jump = null
  ui.openReplies = null
  clock.nowIso = new Date().toISOString()
  app.state = applyReady(emptyState(), testReady(
    [
      { id: 'C1', server_id: 'S1', kind: 'text', name: 'general', position: 0 },
      { id: 'Q1', server_id: 'S1', kind: 'quotes', name: 'quotes', position: 1 },
      { id: 'K1', server_id: 'S1', kind: 'clips', name: 'clips', position: 2 },
    ],
    { members: [{ server_id: 'S1', members: [{ user: testMe, nickname: null }, { user: sam, nickname: null }] }] },
  ))
})

function show(state: Partial<typeof search>) {
  Object.assign(search, { open: true, scope: { kind: 'server', serverId: 'S1' }, words: 'pizza', chips: [], status: 'done', hits: [], total: 0, nextCursor: null, error: '', ...state })
  cmp = mount(SearchResults, { target: document.body })
  flushSync()
  return document.querySelector<HTMLElement>('.search-results')!
}
const cards = () => [...document.querySelectorAll<HTMLElement>('.result')]

describe('SearchResults', () => {
  it('says it is searching', () => {
    expect(show({ status: 'loading' }).textContent).toContain('Searching')
  })

  it('counts results, "50+" past a page', () => {
    expect(show({ hits: [hit(msg('M1'))], total: 1 }).querySelector('.count')!.textContent).toBe('1 result')
    unmount(cmp!)
    cmp = undefined
    expect(show({ hits: [hit(msg('M1'))], total: 51 }).querySelector('.count')!.textContent).toBe('50+ results')
  })

  it('shows who, where and when, with the matches marked', () => {
    show({ hits: [hit(msg('M1', 'C1', { content: 'pizza tonight' }), 'pizza tonight', [{ start: 0, end: 5 }])], total: 1 })
    const c = cards()[0]
    expect(c.textContent).toContain('sam')
    expect(c.textContent).toContain('#general')
    expect(c.textContent).toContain('3h ago')
    expect([...c.querySelectorAll('mark')].map((m) => m.textContent)).toEqual(['pizza'])
  })

  it('an image quote line shows its picture, not "[image]"', () => {
    const line = (over: object) => ({ id: 'L1', speaker_id: 'U2', speaker_name: null, text: null, media: null, caption: null, awards: 0, awarders: [], ...over })
    const q = msg('M9', 'Q1', {
      content: '[image]\n“pizza time” — sam',
      quote: { lines: [line({ id: 'L1', media: { hash: 'a'.repeat(64), still_hash: null, width: 40, height: 30 } }), line({ id: 'L2', text: 'pizza time' })], year: 2026 } as unknown as Message['quote'],
    })
    show({ hits: [hit(q, q.content, [])], total: 1 }) // nothing matched in a line: the first one shows
    const card = cards()[0]
    expect(card.textContent).not.toContain('[image]')
    const img = card.querySelector<HTMLImageElement>('img.line-thumb')!
    expect(img.getAttribute('src')).toContain('a'.repeat(64))
  })

  it('never renders the text as HTML', () => {
    const evil = '<img src=x onerror="window.pwned=1"> pizza'
    show({ hits: [hit(msg('M1', 'C1', { content: evil }), evil, [{ start: 0, end: 4 }])], total: 1 })
    expect(document.querySelector('.search-results img[src="x"]')).toBeNull()
    expect(cards()[0].textContent).toContain('<img src=x')
  })

  it('labels replies inside threads', () => {
    show({
      hits: [
        hit(msg('R1', 'Q1', { thread_root_id: 'Q9' })),
        hit(msg('R2', 'K1', { thread_root_id: 'V9' })),
        hit(msg('M3', 'C1')),
      ],
      total: 3,
    })
    expect(cards().map((c) => c.querySelector('.label')?.textContent ?? null)).toEqual(['reply in quote', 'reply in clip', null])
  })

  it('no matches: says so, with the filters that were on', () => {
    const el = show({ chips: [{ key: 'from', id: 'U2', label: 'sam' }], hits: [], total: 0 })
    expect(el.textContent).toContain('No messages match')
    expect(el.textContent).toContain('from: sam')
    expect(el.textContent).toContain('pizza')
  })

  it('a failure offers Retry', async () => {
    invoke.mockResolvedValue({ hits: [], next_cursor: null, total: 0 })
    const el = show({ status: 'error', error: "can't reach the server" })
    expect(el.querySelector('[role="alert"]')!.textContent).toContain("can't reach the server")
    ;[...el.querySelectorAll('button')].find((b) => b.textContent === 'Retry')!.click()
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith('search_messages', expect.anything()))
  })

  it('Load more asks for the next page', async () => {
    invoke.mockResolvedValue({ hits: [hit(msg('M0'))], next_cursor: null, total: null })
    const el = show({ hits: [hit(msg('M1'))], total: 51, nextCursor: 'M1' })
    ;[...el.querySelectorAll('button')].find((b) => b.textContent === 'Load more')!.click()
    await vi.waitFor(() => expect(cards()).toHaveLength(2))
    expect(invoke).toHaveBeenCalledWith('search_messages', expect.objectContaining({ query: expect.objectContaining({ cursor: 'M1' }) }))
  })

  it('a click jumps to the message and the results stay open', async () => {
    invoke.mockResolvedValue([msg('M1')])
    show({ hits: [hit(msg('M1'))], total: 1 })
    cards()[0].click()
    expect(ui.goTo).toBe('C1')
    await vi.waitFor(() => expect(ui.jump).toMatchObject({ messageId: 'M1' }))
    expect(search.open).toBe(true)
  })

  it('Escape or ✕ closes it', () => {
    const el = show({ hits: [hit(msg('M1'))], total: 1 })
    el.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
    flushSync()
    expect(search.open).toBe(false)
    unmount(cmp!)
    cmp = undefined
    show({})
    document.querySelector<HTMLButtonElement>('button[aria-label="Close search"]')!.click()
    expect(search.open).toBe(false)
  })

  it('a quote shows the line that matched', () => {
    const text = '“nothing” — sam\n“pizza forever” — jo'
    show({ hits: [hit(msg('Q2', 'Q1', { content: text }), text, [{ start: 17, end: 22 }])], total: 1 })
    expect(cards()[0].querySelector('.text')!.textContent).toBe('“pizza forever” — jo')
  })
})
