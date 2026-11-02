// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { Message } from './protocol/Message'
import type { SearchHit } from './protocol/SearchHit'
import type { SearchResults } from './protocol/SearchResults'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: unknown) => invoke(cmd, args) }))

import { closeSearch, loadMore, openHit, retrySearch, runSearch, search } from './search.svelte'
import { app, resetState, ui } from './store.svelte'
import { applyReady, emptyState } from './state'
import { testReady } from './testReady'

const msg = (id: string, channel = 'C1', more: Partial<Message> = {}): Message => ({
  id, channel_id: channel, author_id: 'U2', kind: 'normal', content: `text ${id}`,
  reply_to_id: null, created_at: '2026-10-01T00:00:00.000Z', edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null, quote: null, qoty_card: null, clip: null,
  ...more,
})
const hit = (m: Message, root: Message | null = null): SearchHit => ({ message: m, text: m.content, highlights: [], root })
const page = (hits: SearchHit[], next_cursor: string | null = null, total: number | null = hits.length): SearchResults => ({ hits, next_cursor, total })

/** A promise and the function that settles it. */
function deferred<T>() {
  let resolve!: (v: T) => void
  let reject!: (e: unknown) => void
  const p = new Promise<T>((a, b) => ((resolve = a), (reject = b)))
  return { p, resolve, reject }
}

const server = { kind: 'server', serverId: 'S1' } as const

describe('search store', () => {
  beforeEach(() => {
    resetState()
    closeSearch()
    ui.goTo = null
    ui.jump = null
    ui.openReplies = null
    ui.openClip = null
    invoke.mockReset()
    app.state = applyReady(emptyState(), testReady([
      { id: 'C1', server_id: 'S1', kind: 'text', name: 'general', position: 0 },
      { id: 'Q1', server_id: 'S1', kind: 'quotes', name: 'quotes', position: 1 },
      { id: 'K1', server_id: 'S1', kind: 'clips', name: 'clips', position: 2 },
    ]))
  })

  it('runs a search for a server with structured parameters', async () => {
    invoke.mockResolvedValueOnce(page([hit(msg('M2')), hit(msg('M1'))], null, 2))
    const run = runSearch(server, 'pizza', [{ key: 'from', id: 'U2', label: 'sam' }])
    expect(search.open).toBe(true)
    expect(search.status).toBe('loading')
    await run
    expect(invoke).toHaveBeenCalledWith('search_messages', {
      serverId: 'S1', channelId: null,
      query: { q: 'pizza', from: 'U2', in_channel: null, has: [], mentions: null, cursor: null },
    })
    expect(search.status).toBe('done')
    expect(search.hits.map((h) => h.message.id)).toEqual(['M2', 'M1'])
    expect(search.total).toBe(2)
  })

  it('searches a conversation by its channel', async () => {
    invoke.mockResolvedValueOnce(page([]))
    await runSearch({ kind: 'conversation', channelId: 'D1' }, 'x', [])
    expect(invoke).toHaveBeenCalledWith('search_messages', expect.objectContaining({ serverId: null, channelId: 'D1' }))
  })

  it('stale_answer_ignored: an older answer never overwrites a newer search', async () => {
    const first = deferred<SearchResults>()
    invoke.mockImplementationOnce(() => first.p).mockResolvedValueOnce(page([hit(msg('NEW'))]))
    const a = runSearch(server, 'old', [])
    await runSearch(server, 'new', [])
    first.resolve(page([hit(msg('OLD'))]))
    await a
    expect(search.hits.map((h) => h.message.id)).toEqual(['NEW'])
    expect(search.words).toBe('new')
  })

  it('load more appends the next page with the cursor', async () => {
    invoke.mockResolvedValueOnce(page([hit(msg('M3'))], 'M3', 51)).mockResolvedValueOnce(page([hit(msg('M2'))], null, null))
    await runSearch(server, 'pizza', [])
    await loadMore()
    expect(invoke).toHaveBeenLastCalledWith('search_messages', expect.objectContaining({ query: expect.objectContaining({ cursor: 'M3' }) }))
    expect(search.hits.map((h) => h.message.id)).toEqual(['M3', 'M2'])
    expect(search.total).toBe(51) // the count is the first page's
    expect(search.nextCursor).toBeNull()
  })

  it('a failure says so, and retry runs the same search again', async () => {
    invoke.mockRejectedValueOnce('the server is having trouble (500)').mockResolvedValueOnce(page([hit(msg('M1'))]))
    await runSearch(server, 'pizza', [])
    expect(search.status).toBe('error')
    expect(search.error).toContain('trouble')
    await retrySearch()
    expect(search.status).toBe('done')
    expect(invoke).toHaveBeenCalledTimes(2)
    expect(invoke.mock.calls[1]).toEqual(invoke.mock.calls[0])
  })

  it('close forgets the results', async () => {
    invoke.mockResolvedValueOnce(page([hit(msg('M1'))]))
    await runSearch(server, 'pizza', [])
    closeSearch()
    expect(search.open).toBe(false)
    expect(search.hits).toEqual([])
  })

  it('a plain hit jumps to the message', async () => {
    invoke.mockResolvedValue([msg('M1')])
    openHit(hit(msg('M1')))
    expect(ui.goTo).toBe('C1')
    await vi.waitFor(() => expect(ui.jump).toMatchObject({ channelId: 'C1', messageId: 'M1' }))
    expect(invoke).toHaveBeenCalledWith('list_messages_around', { channelId: 'C1', messageId: 'M1' })
  })

  it('a reply in a quote opens its replies panel, as a notification click does', () => {
    const root = msg('Q9', 'Q1')
    openHit(hit(msg('R1', 'Q1', { thread_root_id: 'Q9' }), root))
    expect(ui.goTo).toBe('Q1')
    expect(ui.openReplies).toEqual({ channelId: 'Q1', rootId: 'Q9' })
  })

  it('a clip, or a reply to one, opens that clip in its channel', () => {
    const clip = msg('V1', 'K1')
    openHit(hit(clip))
    expect(ui.goTo).toBe('K1')
    expect(ui.openClip).toEqual({ channelId: 'K1', clip })
    ui.openClip = null
    openHit(hit(msg('R2', 'K1', { thread_root_id: 'V1' }), clip))
    expect(ui.openClip).toEqual({ channelId: 'K1', clip })
  })

  it('the results stay open after a click', () => {
    search.open = true
    openHit(hit(msg('R1', 'Q1', { thread_root_id: 'Q9' }), msg('Q9', 'Q1')))
    expect(search.open).toBe(true)
  })
})
