// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { Message } from './protocol/Message'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: unknown) => invoke(cmd, args) }))

import { jumpTo, jumpToPresent } from './jump'
import { app, resetState, ui } from './store.svelte'
import { addHistory, applyReady, emptyState, markHistory } from './state'
import { testReady } from './testReady'

const msg = (n: number): Message => ({
  id: `M${String(n).padStart(4, '0')}`, channel_id: 'C1', author_id: 'U2', kind: 'normal', content: `m${n}`,
  reply_to_id: null, created_at: '2026-10-01T00:00:00.000Z', edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null, quote: null, qoty_card: null,
})
const newestFirst = (from: number, n: number) => Array.from({ length: n }, (_, i) => msg(from + i)).reverse()

describe('jumpTo', () => {
  beforeEach(() => {
    resetState()
    ui.goTo = null
    ui.jump = null
    ui.openChannelId = null
    invoke.mockReset()
    const s = applyReady(emptyState(), testReady([{ id: 'C1', server_id: 'S1', kind: 'quotes', name: 'quotes', position: 0 }, { id: 'C2', server_id: 'S1', kind: 'text', name: 'general', position: 1 }]))
    app.state = markHistory(addHistory(s, 'C1', newestFirst(900, 50)), 'C1', false)
  })

  it("jumping to a loaded message doesn't refetch", async () => {
    const before = app.state.messages['C1']
    await jumpTo('C1', 'M0910')
    expect(invoke).not.toHaveBeenCalled()
    expect(app.state.messages['C1']).toBe(before)
    expect(ui.goTo).toBe('C1')
    expect(ui.jump).toMatchObject({ channelId: 'C1', messageId: 'M0910' })
    // the same target again is a new request
    const first = ui.jump!.seq
    await jumpTo('C1', 'M0910')
    expect(ui.jump!.seq).not.toBe(first)
  })

  it('jumping to a far message loads the window around it', async () => {
    invoke.mockImplementation(async (cmd) => (cmd === 'list_messages_around' ? newestFirst(100, 50) : undefined))
    await jumpTo('C1', 'M0125')
    expect(invoke).toHaveBeenCalledWith('list_messages_around', { channelId: 'C1', messageId: 'M0125' })
    expect(app.state.messages['C1'][0].id).toBe('M0100')
    expect(app.state.history['C1']).toEqual({ loaded: true, start: false, end: false })
    expect(ui.goTo).toBe('C1')
    expect(ui.jump).toMatchObject({ channelId: 'C1', messageId: 'M0125' })
    expect(ui.jump!.missing).toBeFalsy()
  })

  it("a message that can't be found is reported, not silently dropped", async () => {
    invoke.mockImplementation(async () => null) // the core's answer to a 404
    const before = app.state.messages['C1']
    await jumpTo('C1', 'M0001')
    expect(app.state.messages['C1']).toBe(before)
    expect(ui.jump).toMatchObject({ channelId: 'C1', messageId: 'M0001', missing: true })
  })

  it('a failed request says why instead of "not found"', async () => {
    invoke.mockImplementation(async () => { throw "can't reach the server: offline" })
    await jumpTo('C1', 'M0001')
    expect(ui.jump).toMatchObject({ channelId: 'C1', messageId: 'M0001', error: "can't reach the server: offline" })
    expect(ui.jump!.missing).toBeFalsy()
  })

  it('leaving the channel before the window lands drops the jump', async () => {
    let release: (p: Message[]) => void = () => {}
    invoke.mockImplementationOnce(() => new Promise((done) => (release = done as never)))
    const before = app.state.messages['C1']
    const pending = jumpTo('C1', 'M0125')
    ui.goTo = null // the shell switched to it...
    ui.openChannelId = 'C2' // ...and then you went elsewhere
    release(newestFirst(100, 50))
    await pending
    expect(app.state.messages['C1']).toBe(before)
    expect(ui.jump).toBeNull()
  })

  it('Jump to present reloads the latest and catches up on what it missed', async () => {
    invoke.mockImplementation(async (cmd) => (cmd === 'list_messages_around' ? newestFirst(100, 50) : undefined))
    await jumpTo('C1', 'M0125')
    ui.openChannelId = 'C1'
    let releaseLatest: (p: Message[]) => void = () => {}
    invoke.mockImplementation((cmd) =>
      cmd === 'list_messages' ? new Promise((done) => (releaseLatest = done as never))
        : cmd === 'list_messages_after' ? Promise.resolve([msg(2000)]) : Promise.resolve(undefined))
    const going = jumpToPresent('C1')
    // your message is confirmed live before the latest page answers, and isn't in it
    const { applyEvent } = await import('./state')
    app.state = applyEvent(app.state, { t: 'MessageCreated', d: { message: msg(2000), nonce: null } }, 0)
    releaseLatest(newestFirst(900, 50))
    expect(await going).toBe(true)
    expect(app.state.messages['C1'].at(-1)!.id).toBe('M2000')
    expect(app.state.history['C1'].end).toBe(true)
    expect(invoke).toHaveBeenCalledWith('list_messages_after', { channelId: 'C1', messageId: 'M0949' })
  })

  it('a jump made while Jump to present loads wins', async () => {
    invoke.mockImplementation(async (cmd) => (cmd === 'list_messages_around' ? newestFirst(100, 50) : undefined))
    await jumpTo('C1', 'M0125')
    ui.openChannelId = 'C1'
    let releaseLatest: (p: Message[]) => void = () => {}
    invoke.mockImplementation((cmd) =>
      cmd === 'list_messages' ? new Promise((done) => (releaseLatest = done as never)) : Promise.resolve(newestFirst(300, 50)))
    const going = jumpToPresent('C1')
    await jumpTo('C1', 'M0325')
    releaseLatest(newestFirst(900, 50))
    expect(await going).toBe(false)
    expect(app.state.messages['C1'][0].id).toBe('M0300')
    expect(app.state.history['C1'].end).toBe(false)
  })

  it('only the latest of two quick jumps lands', async () => {
    let releaseFirst: (p: Message[]) => void = () => {}
    invoke.mockImplementationOnce(() => new Promise((done) => (releaseFirst = done as never)))
    invoke.mockImplementationOnce(async () => newestFirst(300, 50))
    const first = jumpTo('C1', 'M0125')
    await jumpTo('C1', 'M0325')
    releaseFirst(newestFirst(100, 50))
    await first
    expect(app.state.messages['C1'][0].id).toBe('M0300')
    expect(ui.jump).toMatchObject({ messageId: 'M0325' })
  })
})
