// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { Message } from './protocol/Message'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: unknown) => invoke(cmd, args) }))

import { jumpTo } from './jump'
import { app, resetState, ui } from './store.svelte'
import { addHistory, applyReady, emptyState, markHistory } from './state'

const msg = (n: number): Message => ({
  id: `M${String(n).padStart(4, '0')}`, channel_id: 'C1', author_id: 'U2', kind: 'normal', content: `m${n}`,
  reply_to_id: null, created_at: '2026-10-01T00:00:00.000Z', edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null, quote: null, qoty_card: null,
})
const newestFirst = (from: number, n: number) => Array.from({ length: n }, (_, i) => msg(from + i)).reverse()
const me = { id: 'U1', username: 'alex', avatar_hash: null }

describe('jumpTo', () => {
  beforeEach(() => {
    resetState()
    ui.goTo = null
    ui.jump = null
    invoke.mockReset()
    const s = applyReady(emptyState(), {
      me, servers: [{ id: 'S1', name: 'Main', icon_hash: null }],
      channels: [{ id: 'C1', server_id: 'S1', kind: 'quotes', name: 'quotes', position: 0 }, { id: 'C2', server_id: 'S1', kind: 'text', name: 'general', position: 1 }],
      members: [{ server_id: 'S1', members: [{ user: me, nickname: null }] }],
      dm_members: [], voice: [], people: [], read_states: [], mutes: [], hidden: [], latest: [], streams: [], custom_emoji: [], frequent_emoji: [], qoty: [], speaker_links: [], thread_seen: [],
    })
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
    invoke.mockImplementation(async () => { throw 'not found' })
    const before = app.state.messages['C1']
    await jumpTo('C1', 'M0001')
    expect(app.state.messages['C1']).toBe(before)
    expect(ui.jump).toMatchObject({ channelId: 'C1', messageId: 'M0001', missing: true })
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
