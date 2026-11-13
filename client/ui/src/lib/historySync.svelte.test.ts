// @vitest-environment jsdom
// Loaded messages after a reconnect (#263): a Ready refetches only a chat's latest page, so the older pages (or a
// jump's window) still loaded keep the reactions, edits and deletes they had before the drop. Each chat with messages
// loaded is marked, and when it is next shown its loaded range is fetched again and set right in place.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

type Args = { channelId: string; before?: string | null; messageId?: string }
const invoke = vi.fn(async (_cmd: string, _args?: Args): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import { historySync, noteMessageEvent, refreshHistory } from './historySync.svelte'
import { app, handleReady, resetState } from './store.svelte'
import { addHistory, applyEvent, applyRefreshed, loadWindow, markHistory, PAGE } from './state'
import { testReady } from './testReady'
import type { Message } from './protocol/Message'

const TEXT = { id: 'C1', server_id: 'S1', kind: 'text' as const, name: 'general', position: 0 }
const OTHER = { id: 'C2', server_id: 'S1', kind: 'text' as const, name: 'random', position: 1 }
const id = (n: number) => `M${String(n).padStart(4, '0')}`
const msg = (n: number, over: Partial<Message> = {}, channel = 'C1'): Message => ({
  id: id(n), channel_id: channel, author_id: 'U2', kind: 'normal', content: `m${n}`, reply_to_id: null, created_at: '2026-11-13T09:00:00.000Z',
  edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null, quote: null, qoty_card: null, clip: null, embeds: [],
  ...over,
})
const range = <T = Message>(from: number, to: number, f: (n: number) => T = ((n: number) => msg(n)) as (n: number) => T) => Array.from({ length: to - from + 1 }, (_, i) => f(from + i))
const held = (n: number) => app.state.messages['C1'].find((m) => m.id === id(n))
const thumbs = [{ emoji: '👍', count: 1, me: false }]

/** The server's channel C1: messages 1..`last`, each as `f` makes it (the copy as it is now). */
function serve(last: number, f: (n: number) => Message | null = (n) => msg(n)) {
  const all = range(1, last, f).filter((m): m is Message => m !== null)
  invoke.mockImplementation(async (cmd: string, a?: Args) => {
    if (cmd === 'list_messages') {
      const below = a?.before ? all.filter((m) => m.id < a.before!) : all
      return below.slice(-PAGE)
    }
    if (cmd === 'list_messages_after') return all.filter((m) => m.id > a!.messageId!).slice(0, PAGE)
    if (cmd === 'list_messages_around') {
      const i = all.findIndex((m) => m.id === a!.messageId)
      if (i < 0) return null
      return all.slice(Math.max(0, i - PAGE / 2), i + 1 + (PAGE - 1 - PAGE / 2))
    }
    return undefined
  })
  return all
}
const calls = (cmd: string) => invoke.mock.calls.filter(([c]) => c === cmd).length

beforeEach(() => {
  resetState()
  invoke.mockReset()
  invoke.mockResolvedValue(undefined)
  handleReady(testReady([TEXT, OTHER]))
  // scrolled back through three pages: 101..250 of a channel of 250
  app.state = markHistory(addHistory(app.state, 'C1', range(101, 250)), 'C1', false)
})
afterEach(() => resetState())

describe('loaded messages after a reconnect (#263)', () => {
  it('marks every chat with messages loaded', () => {
    expect(historySync.stale).toEqual({})
    app.state = addHistory(app.state, 'C2', [msg(1, {}, 'C2')])
    handleReady(testReady([TEXT, OTHER]))
    expect(historySync.stale).toEqual({ C1: true, C2: true })
  })

  it("an older page's reactions, edits and deletes from while away are set right", async () => {
    handleReady(testReady([TEXT, OTHER]))
    // while away: 120 got a reaction, 130 was edited, 140 deleted (soft), 150 removed for good (a hard delete)
    serve(250, (n) =>
      n === 120 ? msg(n, { reactions: thumbs })
      : n === 130 ? msg(n, { content: 'edited', edited_at: '2026-11-13T10:00:00.000Z' })
      : n === 140 ? msg(n, { deleted: true, content: '' })
      : n === 150 ? null
      : msg(n))
    await refreshHistory('C1')
    expect(held(120)?.reactions).toEqual(thumbs)
    expect(held(130)?.content).toBe('edited')
    expect(held(140)?.deleted).toBe(true)
    expect(held(150)).toBeUndefined()
    expect(app.state.messages['C1']).toHaveLength(149) // nothing added, nothing else lost
    expect(historySync.stale).toEqual({})
  })

  it('fetches only the loaded range, in as few pages as it takes', async () => {
    serve(250)
    await refreshHistory('C1')
    // the window around the oldest loaded, then forward pages to the newest loaded: never the whole channel
    expect(calls('list_messages_around')).toBe(1)
    expect(calls('list_messages_after')).toBe(3)
    expect(invoke).not.toHaveBeenCalledWith('list_messages_after', expect.objectContaining({ messageId: id(250) }))
  })

  it('leaves the list as it was when nothing changed', async () => {
    const before = app.state.messages['C1']
    serve(250)
    await refreshHistory('C1')
    expect(app.state.messages['C1']).toBe(before)
  })

  it("a jump's window is set right too, and stays a window", async () => {
    serve(400, (n) => (n === 10 ? msg(n, { reactions: thumbs }) : msg(n)))
    app.state = loadWindow(app.state, 'C1', range(1, 30), id(5))
    const before = app.state.history['C1']
    await refreshHistory('C1')
    expect(held(10)?.reactions).toEqual(thumbs)
    expect(app.state.messages['C1']).toHaveLength(30)
    expect(app.state.history['C1']).toEqual(before)
  })

  it('a removed oldest message is dropped and the rest still refreshed', async () => {
    serve(250, (n) => (n === 101 ? null : n === 102 ? msg(n, { content: 'edited' }) : msg(n)))
    await refreshHistory('C1')
    expect(held(101)).toBeUndefined()
    expect(held(102)?.content).toBe('edited')
  })

  it('a live change while it loads wins over the fetched copy', async () => {
    let release: () => void = () => {}
    const gate = new Promise<void>((r) => (release = r))
    serve(250)
    const plain = invoke.getMockImplementation()!
    invoke.mockImplementation(async (cmd: string, a?: Args) => {
      if (cmd === 'list_messages_around') await gate
      return plain(cmd, a)
    })
    const done = refreshHistory('C1')
    const e = { t: 'ReactionAdded' as const, d: { channel_id: 'C1', message_id: id(110), emoji: '🔥', user_id: 'U3', count: 1 } }
    app.state = applyEvent(app.state, e, 0)
    noteMessageEvent(e)
    release()
    await done
    expect(held(110)?.reactions.map((r) => r.emoji)).toEqual(['🔥'])
  })

  it('a failed fetch leaves the list as it was, to be tried after the next reconnect', async () => {
    handleReady(testReady([TEXT, OTHER]))
    const before = app.state.messages['C1']
    invoke.mockImplementation(async () => { throw "can't reach the server" })
    await refreshHistory('C1')
    expect(app.state.messages['C1']).toBe(before)
  })

  it('messages added after it started are not taken for removed ones', async () => {
    serve(250)
    const plain = invoke.getMockImplementation()!
    invoke.mockImplementation(async (cmd: string, a?: Args) => {
      const r = await plain(cmd, a)
      // a live message arrives while the last page is on its way (the server's page doesn't have it)
      if (cmd === 'list_messages_after' && a?.messageId === id(225)) {
        app.state = applyEvent(app.state, { t: 'MessageCreated', d: { message: msg(251), nonce: null } }, 0)
      }
      return r
    })
    await refreshHistory('C1')
    expect(held(251)).toBeDefined()
  })
})

describe('applyRefreshed', () => {
  it('replaces held copies in the span, drops held ones the span lacks, and adds nothing', () => {
    const s = applyRefreshed(app.state, 'C1', [msg(100), msg(101, { content: 'x' }), msg(103)], (m) => m >= id(101) && m <= id(103), new Set())
    expect(s.messages['C1'].slice(0, 3).map((m) => [m.id, m.content])).toEqual([[id(101), 'x'], [id(103), 'm103'], [id(104), 'm104']])
  })
})
